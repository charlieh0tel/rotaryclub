use crate::config::{AgcConfig, ConfidenceConfig, DopplerConfig};
use crate::error::{RdfError, Result};
use crate::signal_processing::{AutomaticGainControl, FirBandpass, MovingAverage};

use super::NorthTick;
use super::bearing::validate_confidence_config;

/// Shared signal processing components for bearing calculators
///
/// Contains the common AGC, bandpass filter, smoother, and work buffer
/// used by all bearing calculator implementations.
pub struct BearingCalculatorBase {
    agc: AutomaticGainControl,
    bandpass: FirBandpass,
    filter_group_delay: usize,
    /// The configured trim, converted from microseconds to samples once.
    north_tick_timing_adjustment: f32,
    confidence: ConfidenceConfig,
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
        confidence: ConfidenceConfig,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        if smoothing == 0 {
            return Err(RdfError::Config(
                "bearing smoothing_window must be at least 1".to_string(),
            ));
        }
        validate_confidence_config(&confidence)?;

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
            north_tick_timing_adjustment: doppler_config.north_tick_timing_adjustment_us
                * 1e-6
                * sample_rate,
            confidence,
            sample_counter: 0,
            buffer_start_sample: 0,
            bearing_smoother_cos: MovingAverage::new(smoothing),
            bearing_smoother_sin: MovingAverage::new(smoothing),
            work_buffer: Vec::new(),
        })
    }

    /// How many independent estimates the last preprocessed buffer can yield.
    ///
    /// Not the number of rotations in it. The bandpass carries roughly its own
    /// length of signal history, so two estimates taken closer together than
    /// the filter's impulse response share most of their input and are not
    /// separate looks at the bearing. What averaging earns is the root of this
    /// count, not the root of the rotation count.
    ///
    /// Measured against a real capture: the reported bearings scatter by 23.8
    /// degrees locally, the per-rotation spread is 78.3, and 78.3 over the
    /// root of this count is 27.6 -- against 13.4 if every rotation were
    /// counted independent, which would understate the error by half.
    pub fn independent_estimates(&self) -> f32 {
        let filter_len = (2 * self.filter_group_delay + 1) as f32;
        (self.work_buffer.len() as f32 / filter_len).max(1.0)
    }

    /// Get the confidence weights for combining metrics
    pub fn confidence(&self) -> &ConfidenceConfig {
        &self.confidence
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

    /// Calculate samples elapsed since north tick for a buffer sample index.
    ///
    /// `sample_index_in_buffer` can be fractional (for interpolated indices).
    /// This includes FIR group-delay compensation, configured timing trim, and
    /// tracker-provided fractional tick offset.
    pub fn samples_since_tick(&self, north_tick: &NorthTick, sample_index_in_buffer: f32) -> f32 {
        self.offset_from_north_tick(north_tick) as f32 + sample_index_in_buffer
            - self.filter_group_delay as f32
            + self.north_tick_timing_adjustment
            - north_tick.fractional_sample_offset
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
