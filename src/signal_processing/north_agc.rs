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
//! It follows the median of the recent pulse amplitudes, not their mean or
//! their maximum. Averaging every detected peak is a positive feedback loop
//! under noise, and detection gating does not break it because the false
//! detections are detections: a noise trigger has a small peak, the average
//! reads that as gain being too low, the gain rises, and more noise clears
//! the threshold. Measured, at a north channel noise of 0.10 RMS that took
//! detection from 0.979 to 0.791 and false positives from 0.000 to 0.177. A
//! peak hold has the opposite failure, following a noise spike riding on a
//! pulse down. The median follows neither end.
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

use std::collections::VecDeque;

/// Slow peak-referenced gain for the north channel.
pub struct NorthPulseAgc {
    /// The filtered peak a pulse should produce once gain is right.
    target_peak: f32,
    gain: f32,
    min_gain: f32,
    max_gain: f32,
    /// Recent pulse amplitudes at the filter input, oldest first. The gain
    /// follows their median.
    recent: VecDeque<f32>,
    /// Weight given to a single gain step, from the time constant.
    alpha: f32,
    observations: u64,
    /// Level implied by the last undetected buffer that looked like pulses,
    /// awaiting confirmation by a second one; see observe_undetected.
    undetected_candidate: Option<f32>,
}

/// Detected peaks kept for the median. Long enough that a run of noise
/// triggers cannot carry it, short enough to follow a real level change
/// within the time constant.
const ROBUST_WINDOW: usize = 64;

/// Accepted pulses after which the gain stops moving.
///
/// The reference tick does not change once the hardware is running, so the
/// gain has one job -- to find the level at startup -- and every adaptation
/// after that is exposure without benefit. Measured, an AGC that keeps
/// adapting degrades steadily as the channel gets noisier, because above
/// about a quarter RMS most detections are noise and no amplitude drawn from
/// them means anything. At the shipped rotation rate this is about a third of
/// a second.
const OBSERVATIONS_BEFORE_FREEZE: u64 = 512;

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
            recent: VecDeque::with_capacity(ROBUST_WINDOW),
            alpha: 1.0 - (-1.0 / observations_per_constant).exp(),
            observations: 0,
            undetected_candidate: None,
        }
    }

    /// Median of the recent pulse amplitudes, or None before there are enough
    /// to be worth a median.
    ///
    /// A median rather than a mean or a maximum, because noise contaminates
    /// an amplitude estimate from both ends and each of the other two follows
    /// one end of it. Averaging every detected peak follows the small ones --
    /// noise triggers just over the threshold -- so the gain rises and admits
    /// more of them, which measured as false positives going from 0.000 to
    /// 0.177 at a north noise of 0.10. A peak hold follows the large ones --
    /// a noise spike riding on a pulse -- so the gain falls and detection with
    /// it. The median follows neither.
    fn median_input_peak(&self) -> Option<f32> {
        if self.recent.len() < 8 {
            return None;
        }
        let mut sorted: Vec<f32> = self.recent.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        Some(sorted[sorted.len() / 2]).filter(|v| *v > f32::EPSILON)
    }

    /// The gain to apply to the next buffer.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Whether the gain has had time to mean anything.
    pub fn settled(&self) -> bool {
        self.observations > 0
    }

    /// Whether the gain has stopped moving.
    pub fn frozen(&self) -> bool {
        self.observations >= OBSERVATIONS_BEFORE_FREEZE
    }

    /// Allow the gain to move again.
    ///
    /// Freezing is what makes this safe -- converge once and then stop, so
    /// there is no standing exposure to whatever the channel does later --
    /// but taken absolutely it means a level that genuinely changes can never
    /// be followed. A receiver swapped, a volume nudged, an interface
    /// renegotiated: the frozen gain is then wrong and detection falls away
    /// with no way back.
    ///
    /// The caller unfreezes on a long silence, which is the evidence that the
    /// gain it converged to has stopped working. That keeps the safety
    /// property, because nothing can drag the gain while detections are still
    /// arriving; it only reconsiders once what it settled on has demonstrably
    /// failed. The current gain is kept as the starting point, and the next
    /// observation is free to move it the whole way.
    /// The undetected-buffer candidate is deliberately kept: unfreeze fires
    /// on every quiet buffer, and clearing it here would reset the two-
    /// buffer confirmation in observe_undetected before it could ever
    /// complete.
    pub fn unfreeze(&mut self) {
        self.observations = 0;
        self.recent.clear();
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
            self.undetected_candidate = None;
            return;
        }
        // Deliberately not counted as an observation: this is a guess at the
        // level from an undetected buffer, and the first real detection should
        // still be free to move the gain the whole way.
        let level = peak / self.gain.max(f32::EPSILON);
        // A single buffer is not enough: one noise impulse in an otherwise
        // quiet buffer has exactly the crest factor of a pulse. A pulse
        // train is periodic, so it shows the same level in consecutive
        // buffers; demand two in a row within a factor of two before the
        // gain moves.
        if let Some(prev) = self.undetected_candidate
            && level <= prev * 2.0
            && level >= prev * 0.5
        {
            self.gain = (self.target_peak / level).clamp(self.min_gain, self.max_gain);
        }
        self.undetected_candidate = Some(level);
    }

    /// Note the filtered peak of a detected pulse.
    ///
    /// `filtered_peak` is measured after the current gain was applied, so it is
    /// referred back through the gain before entering the median window.
    pub fn observe(&mut self, filtered_peak: f32) {
        if self.frozen() || !filtered_peak.is_finite() || filtered_peak <= f32::EPSILON {
            return;
        }
        // Referred back through the current gain, so what accumulates is a
        // property of the signal rather than of what the gain happened to be.
        let input_peak = filtered_peak / self.gain.max(f32::EPSILON);
        if self.recent.len() == ROBUST_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(input_peak);
        let Some(level) = self.median_input_peak() else {
            return;
        };
        let wanted = (self.target_peak / level).clamp(self.min_gain, self.max_gain);
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
