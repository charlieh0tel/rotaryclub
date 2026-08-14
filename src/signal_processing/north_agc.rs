//! Slow automatic gain control for the north reference channel.
//!
//! The north channel's pulse amplitude varies by a factor of nearly four
//! across the two radios in `data/`, 0.21 to 0.78, against a configured
//! expectation of 0.8, and the doppler AGC does not reach this channel. What
//! the detection threshold meets has therefore been whatever the receiver
//! happened to deliver.
//!
//! This is peak-referenced rather than RMS-referenced, which the doppler AGC
//! is and which would be wrong here. The pulse is a roughly 1.2-sample event
//! every 30, a duty cycle of 0.04, so the RMS of a clean pulse train is a
//! fifth of the pulse amplitude and the doppler AGC's target of 0.3 would
//! imply an amplitude of 1.5 -- past full scale, and clipping on all three
//! captures. An RMS reference would also track the rotation rate, because the
//! duty cycle does.
//!
//! It adapts on detected pulses once there are any. Before that there is a
//! chicken and egg: a receiver quiet enough to need the gain is quiet enough
//! that nothing is detected, so gating purely on detections leaves it stuck at
//! zero forever, which is what the first version of this did.
//!
//! The way out is that a pulse train and a noise floor look nothing alike. A
//! 1.2-sample pulse every 30 has a peak twenty-five times its own mean
//! absolute value; white noise in a buffer of a few hundred samples has a peak
//! about four times its own. So before the first detection the gain adapts to
//! the buffer peak, but only when the buffer looks like pulses by that measure,
//! and a silent or noisy channel is left alone.

/// Slow peak-referenced gain for the north channel.
pub struct NorthPulseAgc {
    /// The filtered peak a pulse should produce once gain is right.
    target_peak: f32,
    gain: f32,
    min_gain: f32,
    max_gain: f32,
    /// Weight given to a single observation, from the time constant.
    alpha: f32,
    observations: u64,
}

/// Peak over mean absolute value, above which a buffer is taken to hold
/// pulses rather than noise. A pulse train at the shipped rate measures about
/// 25 and white noise about 4.
const PULSE_CREST_FACTOR: f32 = 10.0;

impl NorthPulseAgc {
    /// * `target_peak` - filtered peak amplitude to drive detections towards
    /// * `time_constant_secs` - how long the gain takes to settle
    /// * `pulse_rate_hz` - how often observations are expected to arrive
    pub fn new(
        target_peak: f32,
        time_constant_secs: f32,
        pulse_rate_hz: f32,
        min_gain: f32,
        max_gain: f32,
    ) -> Self {
        // One observation per pulse, so the per-observation weight that gives
        // the requested settling time depends on how fast pulses arrive.
        let observations_per_constant = (time_constant_secs * pulse_rate_hz).max(1.0);
        Self {
            target_peak,
            gain: 1.0,
            min_gain,
            max_gain,
            alpha: 1.0 - (-1.0 / observations_per_constant).exp(),
            observations: 0,
        }
    }

    /// The gain to apply to the next buffer.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Whether the gain has had time to mean anything.
    pub fn settled(&self) -> bool {
        self.observations > 0
    }

    /// Note a buffer that has produced no detections.
    ///
    /// Only useful before anything has ever been detected, and only acted on
    /// when the buffer looks like a pulse train rather than a noise floor.
    /// Once detections are arriving they are the better evidence and this does
    /// nothing.
    pub fn observe_undetected(&mut self, filtered: &[f32]) {
        if self.observations > 0 || filtered.is_empty() {
            return;
        }
        let peak = filtered.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let mean_abs = filtered.iter().map(|s| s.abs()).sum::<f32>() / filtered.len() as f32;
        if mean_abs <= f32::EPSILON || peak / mean_abs < PULSE_CREST_FACTOR {
            return;
        }
        // Deliberately not counted as an observation: this is a guess at the
        // level from an undetected buffer, and the first real detection should
        // still be free to move the gain the whole way.
        let wanted = (self.gain * self.target_peak / peak).clamp(self.min_gain, self.max_gain);
        self.gain = wanted;
    }

    /// Note the filtered peak of a detected pulse.
    ///
    /// `filtered_peak` is measured after the current gain was applied, so the
    /// gain that would have landed it on target is the current one scaled by
    /// how far off it was.
    pub fn observe(&mut self, filtered_peak: f32) {
        if !filtered_peak.is_finite() || filtered_peak <= f32::EPSILON {
            return;
        }
        let wanted =
            (self.gain * self.target_peak / filtered_peak).clamp(self.min_gain, self.max_gain);
        // The first observation moves the gain most of the way, so a receiver
        // far from the expected level is usable within a rotation or two
        // rather than after the full time constant.
        let alpha = if self.observations == 0 {
            1.0
        } else {
            self.alpha
        };
        self.gain = (1.0 - alpha) * self.gain + alpha * wanted;
        self.gain = self.gain.clamp(self.min_gain, self.max_gain);
        self.observations = self.observations.saturating_add(1);
    }
}
