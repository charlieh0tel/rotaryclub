use crate::config::{NorthPulseEstimator, NorthTickConfig};

/// Ceiling on the estimator window as a fraction of one rotation, so it can
/// never reach far enough to take in a neighbouring pulse.
const MAX_CENTROID_HALF_WIDTH_FRACTION: f32 = 0.2;
use crate::error::{RdfError, Result};
use crate::signal_processing::FirHighpass;

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

    finite("threshold", config.threshold)?;
    if config.threshold <= 0.0 {
        return Err(RdfError::Config(format!(
            "north_tick.threshold is {}, must be greater than 0",
            config.threshold
        )));
    }
    if config.threshold >= config.expected_pulse_amplitude {
        return Err(RdfError::Config(format!(
            "north_tick.threshold ({}) is at or above expected_pulse_amplitude ({}), so no              pulse can ever cross it; lower the threshold or raise gain_db",
            config.threshold, config.expected_pulse_amplitude
        )));
    }

    if config.fir_highpass_taps < 3 {
        return Err(RdfError::Config(format!(
            "north_tick.fir_highpass_taps is {}, must be at least 3; an even count is              rounded up to keep the filter linear phase",
            config.fir_highpass_taps
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
