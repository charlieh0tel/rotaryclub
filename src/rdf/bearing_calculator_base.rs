use crate::config::{AgcConfig, DopplerConfig};
use crate::error::Result;
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
        let bandpass = FirBandpass::new_default(
            doppler_config.bandpass_low,
            doppler_config.bandpass_high,
            sample_rate,
        )?;
        let filter_group_delay = bandpass.group_delay_samples();

        Ok(Self {
            agc: AutomaticGainControl::new(agc_config, sample_rate as u32),
            bandpass,
            filter_group_delay,
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
        self.sample_counter.checked_sub(north_tick.sample_index)
    }

    /// Get the fractional north tick timing adjustment in samples
    ///
    /// The north tick detector triggers when `sample > threshold`, which occurs
    /// at the first integer sample above threshold. The actual threshold crossing
    /// (the conceptual "tick" moment) happens somewhere in the previous inter-sample
    /// interval. On average, this is 0.5 samples before the detection point.
    ///
    /// This adjustment compensates for that discrete-sampling effect.
    pub fn north_tick_timing_adjustment(&self) -> f32 {
        0.5
    }

    /// Get the filter group delay in samples
    ///
    /// The FIR bandpass filter introduces a group delay. When calculating phase,
    /// the filtered output at buffer index `idx` corresponds to input sample
    /// `(base_offset + idx - filter_group_delay)` relative to the north tick.
    pub fn filter_group_delay(&self) -> usize {
        self.filter_group_delay
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
