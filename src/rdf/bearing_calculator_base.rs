use crate::config::{AgcConfig, DopplerConfig};
use crate::error::Result;
use crate::signal_processing::{AutomaticGainControl, IirButterworthBandpass, MovingAverage};

use super::NorthTick;

/// Shared signal processing components for bearing calculators
///
/// Contains the common AGC, bandpass filter, smoother, and work buffer
/// used by all bearing calculator implementations.
pub struct BearingCalculatorBase {
    agc: AutomaticGainControl,
    bandpass: IirButterworthBandpass,
    pub sample_counter: usize,
    bearing_smoother: MovingAverage,
    pub work_buffer: Vec<f32>,
}

impl BearingCalculatorBase {
    /// Create a new bearing calculator base with shared components
    pub fn new(
        doppler_config: &DopplerConfig,
        agc_config: &AgcConfig,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        Ok(Self {
            agc: AutomaticGainControl::new(agc_config, sample_rate as u32),
            bandpass: IirButterworthBandpass::new(
                doppler_config.bandpass_low,
                doppler_config.bandpass_high,
                sample_rate,
                doppler_config.filter_order,
            )?,
            sample_counter: 0,
            bearing_smoother: MovingAverage::new(smoothing),
            work_buffer: Vec::new(),
        })
    }

    /// Preprocess the input buffer: copy to work buffer, apply AGC and bandpass filter
    pub fn preprocess(&mut self, input: &[f32]) {
        self.work_buffer.clear();
        self.work_buffer.extend_from_slice(input);
        self.agc.process_buffer(&mut self.work_buffer);
        self.bandpass.process_buffer(&mut self.work_buffer);
    }

    /// Calculate the sample offset from the north tick
    ///
    /// Returns `None` if the north tick is in the future (invalid state).
    pub fn offset_from_north_tick(&self, north_tick: &NorthTick) -> Option<usize> {
        if self.sample_counter >= north_tick.sample_index {
            Some(self.sample_counter - north_tick.sample_index)
        } else {
            None
        }
    }

    /// Apply smoothing to a raw bearing value
    pub fn smooth_bearing(&mut self, raw_bearing: f32) -> f32 {
        self.bearing_smoother.add(raw_bearing)
    }

    /// Advance the sample counter by the given amount
    pub fn advance_counter(&mut self, samples: usize) {
        self.sample_counter += samples;
    }
}
