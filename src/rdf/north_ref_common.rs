use crate::config::{NorthPulseEstimator, NorthTickConfig};

/// Ceiling on the estimator window as a fraction of one rotation, so it can
/// never reach far enough to take in a neighbouring pulse.
const MAX_CENTROID_HALF_WIDTH_FRACTION: f32 = 0.2;
use crate::error::{RdfError, Result};
use crate::signal_processing::FirHighpass;
#[cfg(test)]
use crate::signal_processing::db_to_amplitude;

/// Validate the settings both trackers share.
///
/// Each message names the setting, what it was, and what would fix it: a
/// tracker that silently detects nothing is far harder to diagnose from a
/// bearing display than a refusal to start.
pub(super) fn validate_north_tick_config(config: &NorthTickConfig, sample_rate: f32) -> Result<()> {
    let finite = |name: &str, value: f32| -> Result<()> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(RdfError::Config(format!(
                "north_tick.{name} must be a finite number, got {value}"
            )))
        }
    };

    finite("gain_db", config.gain_db)?;
    if !(-60.0..=60.0).contains(&config.gain_db) {
        return Err(RdfError::Config(format!(
            "north_tick.gain_db is {} dB, outside the supported -60 to 60; a gain this far              from unity usually means the wrong channel is being read",
            config.gain_db
        )));
    }

    finite("expected_pulse_amplitude", config.expected_pulse_amplitude)?;
    if config.expected_pulse_amplitude <= 0.0 || config.expected_pulse_amplitude > 1.0 {
        return Err(RdfError::Config(format!(
            "north_tick.expected_pulse_amplitude is {}, must be within (0, 1]; it is the              pulse height in full-scale units after gain",
            config.expected_pulse_amplitude
        )));
    }

    // The threshold is a fraction of the pulse height the detector expects,
    // so the only way to misconfigure it is to ask for a fraction that is not
    // one. What this used to have to check -- a threshold sitting above the
    // amplitude it would meet, which a gain change alone could bring about
    // and which silently emitted no ticks -- cannot be expressed any more.
    finite("threshold_fraction", config.threshold_fraction)?;
    if !(0.0..1.0).contains(&config.threshold_fraction) || config.threshold_fraction == 0.0 {
        return Err(RdfError::Config(format!(
            "north_tick.threshold_fraction is {}, must be within (0, 1); it is the \
             fraction of the expected filtered pulse height a detection has to clear, \
             and at 1 or above no pulse can ever cross it",
            config.threshold_fraction
        )));
    }

    finite("fir_highpass_length_us", config.fir_highpass_length_us)?;
    if highpass_taps(config, sample_rate) < 3 {
        return Err(RdfError::Config(format!(
            "north_tick.fir_highpass_length_us is {} us, which is fewer than 3 taps at a \
             {} Hz sample rate; lengthen the filter",
            config.fir_highpass_length_us, sample_rate
        )));
    }

    finite("highpass_cutoff", config.highpass_cutoff)?;
    finite("highpass_transition_hz", config.highpass_transition_hz)?;
    if config.highpass_transition_hz <= 0.0 {
        return Err(RdfError::Config(format!(
            "north_tick.highpass_transition_hz is {}, must be greater than 0",
            config.highpass_transition_hz
        )));
    }
    let nyquist = sample_rate / 2.0;
    if config.highpass_cutoff <= config.highpass_transition_hz {
        return Err(RdfError::Config(format!(
            "north_tick.highpass_cutoff ({} Hz) must be above highpass_transition_hz ({} Hz),              or the stopband has no width",
            config.highpass_cutoff, config.highpass_transition_hz
        )));
    }
    if config.highpass_cutoff >= nyquist {
        return Err(RdfError::Config(format!(
            "north_tick.highpass_cutoff is {} Hz, at or above the {} Hz Nyquist frequency for              a {} Hz sample rate; nothing would pass",
            config.highpass_cutoff, nyquist, sample_rate
        )));
    }

    finite("max_coast_ms", config.max_coast_ms)?;
    if config.max_coast_ms < 0.0 {
        return Err(RdfError::Config(format!(
            "north_tick.max_coast_ms is {}, must be 0 or greater; 0 disables coasting",
            config.max_coast_ms
        )));
    }

    finite("gate_sigma", config.gate_sigma)?;
    if config.gate_sigma < 0.0 {
        return Err(RdfError::Config(format!(
            "north_tick.gate_sigma is {}, must be 0 or greater",
            config.gate_sigma
        )));
    }

    Ok(())
}

/// Tap count for the highpass, derived from its length in time.
///
/// Forced odd so the filter stays Type I linear phase, which is what makes
/// its group delay an exact half-integer number of samples and therefore
/// exactly compensable.
pub(super) fn highpass_taps(config: &NorthTickConfig, sample_rate: f32) -> usize {
    let taps = (config.fir_highpass_length_us * 1e-6 * sample_rate).round() as usize;
    if taps.is_multiple_of(2) {
        taps + 1
    } else {
        taps
    }
}

/// Watches the north channel for the failure that reports nothing: a level
/// too low to cross the detection threshold.
///
/// Below that point detection does not degrade, it stops -- there are no
/// ticks, so no bearings, and no metric that would show a problem. The
/// threshold has wide margin (detection holds to a pulse amplitude of 0.3
/// against the 0.8 expected, per `examples/north_threshold_sweep`), so a
/// channel that falls through it is badly wrong rather than marginal, and
/// worth saying so out loud.
pub(super) struct QuietChannelWatch {
    samples_since_detection: usize,
    warn_after_samples: usize,
    warned: bool,
}

impl QuietChannelWatch {
    /// Roughly a second of rotations before complaining, so a dropout the
    /// tracker is coasting through does not trip it.
    pub(super) fn new(sample_rate: f32) -> Self {
        Self {
            samples_since_detection: 0,
            warn_after_samples: (sample_rate.max(1.0)) as usize,
            warned: false,
        }
    }

    /// Whether the channel has been silent long enough to have complained
    /// about it.
    pub(super) fn is_quiet(&self) -> bool {
        self.warned
    }

    /// Samples since a pulse was last detected.
    pub(super) fn samples_since_detection(&self) -> usize {
        self.samples_since_detection
    }

    pub(super) fn note_detections(&mut self, count: usize, peak_amplitude: f32, threshold: f32) {
        if count == 0 {
            return;
        }
        if self.warned {
            log::info!(
                "north reference pulses detected again after {:.2} s",
                self.samples_since_detection as f32 / self.warn_after_samples.max(1) as f32
            );
        }
        self.samples_since_detection = 0;
        self.warned = false;

        // Detected, but only just: the margin measured on real captures is a
        // factor of two or more, so anything under that is worth flagging
        // before it disappears entirely.
        if peak_amplitude > 0.0 && peak_amplitude < threshold * 1.5 {
            log::debug!(
                "north pulses are close to the detection threshold: peak {:.3} against \
                 threshold {:.3}",
                peak_amplitude,
                threshold
            );
        }
    }

    pub(super) fn advance(&mut self, samples: usize, threshold: f32) {
        self.samples_since_detection = self.samples_since_detection.saturating_add(samples);
        if !self.warned && self.samples_since_detection >= self.warn_after_samples {
            self.warned = true;
            log::warn!(
                "no north reference pulses detected for {:.1} s; the north channel may be too \
                 quiet to cross the {:.2} detection threshold, or wired to the wrong input",
                self.samples_since_detection as f32 / self.warn_after_samples.max(1) as f32,
                threshold
            );
        }
    }
}

pub(super) struct PeakTiming {
    /// Offset from group delay to the point the estimator reports for an
    /// impulse arriving exactly on a sample.
    pub pulse_reference_offset: f32,
    pub peak_search_window_samples: usize,
}

/// Half-width of the window the estimator takes its moment over, in samples.
///
/// Expressed in time by the estimator and converted here, so it means the
/// same thing at any sample rate, and bounded well inside one rotation so the
/// window can never reach a neighbouring pulse.
pub(super) fn centroid_half_width(
    estimator: NorthPulseEstimator,
    sample_rate: f32,
    nominal_period_samples: f32,
) -> usize {
    if estimator.weight_exponent() == 0 {
        return 0;
    }
    let ceiling = (nominal_period_samples * MAX_CENTROID_HALF_WIDTH_FRACTION).max(2.0) as usize;
    let samples = (estimator.window_half_width_us() * 1e-6 * sample_rate).round() as usize;
    samples.clamp(1, ceiling)
}

/// Sub-sample arrival time of a pulse, relative to `peak_index`.
///
/// `tail` holds the filtered samples immediately preceding `buffer`, so a
/// peak resolved across a buffer boundary -- which the detector reports at a
/// negative index -- still has a symmetric window. Indices are relative to
/// the start of `buffer`.
///
/// The window is expected to be fillable: the detector holds a crossing until
/// the samples after its peak exist, and the retained tail covers the samples
/// before. Returning zero here would not be a coarser measurement but a
/// biased one, since the delay compensation still references the estimator,
/// so it is a fallback against buffers too short to hold the context at all
/// rather than something that happens in normal operation.
pub(super) fn estimate_fraction(
    tail: &[f32],
    buffer: &[f32],
    peak_index: isize,
    estimator: NorthPulseEstimator,
    half_width: usize,
) -> f32 {
    let exponent = estimator.weight_exponent();
    if exponent == 0 {
        return 0.0;
    }
    let clip = estimator.clips_negative();

    let sample_at = |index: isize| -> Option<f32> {
        if index >= 0 {
            buffer.get(index as usize).copied()
        } else {
            let from_end = (-index) as usize;
            tail.len()
                .checked_sub(from_end)
                .and_then(|i| tail.get(i).copied())
        }
    };

    let half = half_width as isize;
    let mut weighted = 0.0f64;
    let mut total = 0.0f64;
    for index in (peak_index - half)..=(peak_index + half) {
        let Some(sample) = sample_at(index) else {
            return 0.0;
        };
        let value = if clip { sample.max(0.0) } else { sample.abs() } as f64;
        let weight = value.powi(exponent);
        weighted += weight * index as f64;
        total += weight;
    }

    if total > 0.0 {
        (weighted / total - peak_index as f64) as f32
    } else {
        0.0
    }
}

/// Retain the trailing filtered samples a later buffer needs to complete an
/// estimator window that straddles the boundary.
pub(super) fn retain_tail(tail: &mut Vec<f32>, buffer: &[f32], len: usize) {
    if len == 0 {
        tail.clear();
        return;
    }
    if buffer.len() >= len {
        tail.clear();
        tail.extend_from_slice(&buffer[buffer.len() - len..]);
    } else {
        let overflow = (tail.len() + buffer.len()).saturating_sub(len);
        tail.drain(..overflow.min(tail.len()));
        tail.extend_from_slice(buffer);
    }
}

pub(super) struct DelayCompensation {
    pub delay_samples: usize,
    pub fractional_sample_offset: f32,
}

/// Absolute detection threshold, from the fraction the configuration carries.
///
/// The detector runs on the highpassed signal, where the pulse peaks at the
/// expected amplitude times the filter's peak response, so that product is
/// what the fraction is of. `effective_pulse_amplitude` is the expected
/// amplitude the tracker actually presents to the filter: the configured one
/// when the AGC is driving the level to it, and that times the static gain
/// when it is not.
pub(super) fn detection_threshold(
    threshold_fraction: f32,
    effective_pulse_amplitude: f32,
    highpass: &FirHighpass,
) -> f32 {
    threshold_fraction * effective_pulse_amplitude * highpass.peak_response()
}

pub(super) fn derive_peak_timing(
    highpass: &FirHighpass,
    threshold: f32,
    effective_pulse_amplitude: f32,
    estimator: NorthPulseEstimator,
    centroid_half_width: usize,
) -> PeakTiming {
    let threshold_crossing_offset =
        highpass.threshold_crossing_offset(threshold, effective_pulse_amplitude);
    let peak_offset = highpass.peak_offset();
    let search_from_response =
        ((peak_offset - threshold_crossing_offset).max(0.0)).ceil() as usize + 3;

    let peak_search_window_samples = search_from_response;

    // Each estimator reports a different point on the same filtered pulse.
    // Referencing the delay compensation to that point keeps the emitted tick
    // time -- and any north-offset calibration against it -- unchanged when
    // the estimator changes.
    let pulse_reference_offset = match estimator.weight_exponent() {
        0 => peak_offset,
        exponent => {
            highpass.centroid_offset(centroid_half_width, exponent, estimator.clips_negative())
        }
    };

    PeakTiming {
        pulse_reference_offset,
        peak_search_window_samples,
    }
}

pub(super) fn derive_delay_compensation(
    highpass: &FirHighpass,
    pulse_peak_offset: f32,
) -> DelayCompensation {
    let group_delay = highpass.group_delay_samples() as f32;
    let total_delay = group_delay + pulse_peak_offset;
    let delay_samples = total_delay.round().max(0.0) as usize;
    let fractional_sample_offset = delay_samples as f32 - total_delay;

    DelayCompensation {
        delay_samples,
        fractional_sample_offset,
    }
}

/// Re-anchor an effective tick time onto the nearest sample index.
///
/// Callers accumulate corrections against a whole-sample base; once the total
/// exceeds half a sample the nearest index is a different one. Splitting here
/// keeps the reported fraction inside half a sample, so consumers never have
/// to reason about an offset that points past a neighbouring sample.
pub(super) fn split_effective_time(base_sample: usize, offset: f32) -> (usize, f32) {
    if !offset.is_finite() {
        return (base_sample, 0.0);
    }
    let whole = offset.round();
    let index = (base_sample as i64 + whole as i64).max(0) as usize;
    let fraction = offset - whole;
    (index, fraction)
}

pub(super) fn preprocess_north_buffer(
    filter_buffer: &mut Vec<f32>,
    input: &[f32],
    gain: f32,
    highpass: &mut FirHighpass,
) {
    filter_buffer.resize(input.len(), 0.0);
    filter_buffer.copy_from_slice(input);
    if gain != 1.0 {
        for sample in filter_buffer.iter_mut() {
            *sample *= gain;
        }
    }
    highpass.process_buffer(filter_buffer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RdfConfig;

    fn default_highpass(config: &NorthTickConfig, sample_rate: f32) -> FirHighpass {
        FirHighpass::new(
            config.highpass_cutoff,
            sample_rate,
            highpass_taps(config, sample_rate),
            config.highpass_transition_hz,
        )
        .expect("filter")
    }

    /// The fraction reproduces the absolute threshold it replaced.
    ///
    /// 0.15 of full scale was measured and settled, and the change to a
    /// fraction was meant to leave it exactly where it was. Pinned here
    /// because nothing else would notice it drifting: a threshold slightly
    /// off shows up as a slightly different detection cliff, which no test
    /// asserts directly.
    #[test]
    fn test_the_default_fraction_reproduces_the_measured_threshold() {
        let config = RdfConfig::default();
        let sample_rate = config.audio.sample_rate as f32;
        let highpass = default_highpass(&config.north_tick, sample_rate);
        let threshold = detection_threshold(
            config.north_tick.threshold_fraction,
            config.north_tick.expected_pulse_amplitude,
            &highpass,
        );
        assert!(
            (threshold - 0.15).abs() < 1e-4,
            "default fraction gives a threshold of {threshold}, not the 0.15 it replaced"
        );
    }

    /// The gain no longer moves the pulse out from under the threshold.
    ///
    /// This is the whole point of the change. An absolute threshold stayed
    /// put while the signal it met scaled with the gain, so attenuation
    /// silently defeated detection and validation had to reject it. Derived,
    /// the margin is the same at any gain.
    #[test]
    fn test_the_margin_is_invariant_under_gain() {
        let config = RdfConfig::default();
        let sample_rate = config.audio.sample_rate as f32;
        let highpass = default_highpass(&config.north_tick, sample_rate);
        let expected = config.north_tick.expected_pulse_amplitude;

        for gain_db in [-20.0f32, -6.0, 0.0, 6.0, 20.0] {
            let gain = db_to_amplitude(gain_db);
            let threshold = detection_threshold(
                config.north_tick.threshold_fraction,
                expected * gain,
                &highpass,
            );
            let pulse_peak = expected * gain * highpass.peak_response();
            let margin = threshold / pulse_peak;
            assert!(
                (margin - config.north_tick.threshold_fraction).abs() < 1e-5,
                "at {gain_db} dB the threshold is {margin} of the pulse, not the \
                 configured {}",
                config.north_tick.threshold_fraction
            );
        }
    }
}
