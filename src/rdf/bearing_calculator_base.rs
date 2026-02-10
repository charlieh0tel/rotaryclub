use crate::config::{AgcConfig, ConfidenceWeights, DopplerConfig};
use crate::error::{RdfError, Result};
use crate::signal_processing::{AutomaticGainControl, FirBandpass, MovingAverage};

use super::NorthTick;

/// Shared signal processing components for bearing calculators
///
/// Contains the common AGC, bandpass filter, smoother, and work buffer
/// used by all bearing calculator implementations.
pub struct BearingCalculatorBase {
    agc: AutomaticGainControl,
    bandpass: FirBandpass,
    filter_group_delay: usize,
    north_tick_timing_adjustment: f32,
    confidence_weights: ConfidenceWeights,
    pub sample_counter: usize,
    buffer_start_sample: usize,
    bearing_smoother_cos: MovingAverage,
    bearing_smoother_sin: MovingAverage,
    pub work_buffer: Vec<f32>,
}

impl BearingCalculatorBase {
    /// Create a new bearing calculator base with shared components
    pub fn new(
        doppler_config: &DopplerConfig,
        agc_config: &AgcConfig,
        confidence_weights: ConfidenceWeights,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        if smoothing == 0 {
            return Err(RdfError::Config(
                "bearing smoothing_window must be at least 1".to_string(),
            ));
        }

        let bandpass = FirBandpass::new(
            doppler_config.bandpass_low,
            doppler_config.bandpass_high,
            sample_rate,
            doppler_config.bandpass_taps,
            doppler_config.bandpass_transition_hz,
        )?;
        let filter_group_delay = bandpass.group_delay_samples();

        Ok(Self {
            agc: AutomaticGainControl::new(agc_config, sample_rate),
            bandpass,
            filter_group_delay,
            north_tick_timing_adjustment: doppler_config.north_tick_timing_adjustment,
            confidence_weights,
            sample_counter: 0,
            buffer_start_sample: 0,
            bearing_smoother_cos: MovingAverage::new(smoothing),
            bearing_smoother_sin: MovingAverage::new(smoothing),
            work_buffer: Vec::new(),
        })
    }

    /// Get the confidence weights for combining metrics
    pub fn confidence_weights(&self) -> &ConfidenceWeights {
        &self.confidence_weights
    }

    /// Preprocess the input buffer: copy to work buffer, apply AGC and bandpass filter.
    /// Also records the buffer start position for multi-tick processing.
    pub fn preprocess(&mut self, input: &[f32]) {
        self.buffer_start_sample = self.sample_counter;
        self.work_buffer.clear();
        self.work_buffer.extend_from_slice(input);
        self.agc.process_buffer(&mut self.work_buffer);
        self.bandpass.process_buffer(&mut self.work_buffer);
    }

    /// Calculate the sample offset from the north tick using buffer_start_sample.
    /// Returns buffer_start_sample - tick.sample_index (can be negative if tick is
    /// within the current buffer).
    pub fn offset_from_north_tick(&self, north_tick: &NorthTick) -> isize {
        self.buffer_start_sample as isize - north_tick.sample_index as isize
    }

    /// Get the fractional north tick timing adjustment in samples
    ///
    /// This is a fine-trim applied after tracker compensation.
    /// Default is 0.5 for backward-compatible calibration.
    pub fn north_tick_timing_adjustment(&self) -> f32 {
        self.north_tick_timing_adjustment
    }

    /// Get the filter group delay in samples
    ///
    /// The FIR bandpass filter introduces a group delay. When calculating phase,
    /// the filtered output at buffer index `idx` corresponds to input sample
    /// `(base_offset + idx - filter_group_delay)` relative to the north tick.
    pub fn filter_group_delay(&self) -> usize {
        self.filter_group_delay
    }

    /// Apply circular smoothing to a raw bearing value.
    /// Uses vector averaging (cos/sin components) to handle 0°/360° wraparound.
    pub fn smooth_bearing(&mut self, raw_bearing: f32) -> f32 {
        let rad = raw_bearing.to_radians();
        let avg_cos = self.bearing_smoother_cos.add(rad.cos());
        let avg_sin = self.bearing_smoother_sin.add(rad.sin());
        avg_sin.atan2(avg_cos).to_degrees().rem_euclid(360.0)
    }

    /// Advance the sample counter by the given amount
    pub fn advance_counter(&mut self, samples: usize) {
        self.sample_counter += samples;
    }
}
