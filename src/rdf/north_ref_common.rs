use crate::signal_processing::FirHighpass;

pub(super) struct PeakTiming {
    pub pulse_peak_offset: f32,
    pub peak_search_window_samples: usize,
}

pub(super) struct DelayCompensation {
    pub delay_samples: usize,
    pub fractional_sample_offset: f32,
}

pub(super) fn derive_peak_timing(
    highpass: &FirHighpass,
    threshold: f32,
    effective_pulse_amplitude: f32,
) -> PeakTiming {
    let threshold_crossing_offset =
        highpass.threshold_crossing_offset(threshold, effective_pulse_amplitude);
    let pulse_peak_offset = highpass.peak_offset();
    let peak_search_window_samples =
        ((pulse_peak_offset - threshold_crossing_offset).max(0.0)).ceil() as usize + 3;

    PeakTiming {
        pulse_peak_offset,
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
