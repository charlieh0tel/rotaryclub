use crate::config::{AgcConfig, DopplerConfig};
use crate::error::Result;
use crate::rdf::{BearingMeasurement, NorthTick};
use crate::signal_processing::{
    AutomaticGainControl, BandpassFilter, MovingAverage, ZeroCrossingDetector, phase_to_bearing,
};
use std::f32::consts::PI;

pub struct ZeroCrossingBearingCalculator {
    agc: AutomaticGainControl,
    bandpass: BandpassFilter,
    zero_detector: ZeroCrossingDetector,
    sample_counter: usize,
    bearing_smoother: MovingAverage,
}

impl ZeroCrossingBearingCalculator {
    pub fn new(
        doppler_config: &DopplerConfig,
        agc_config: &AgcConfig,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        Ok(Self {
            agc: AutomaticGainControl::new(agc_config, sample_rate as u32),
            bandpass: BandpassFilter::new(
                doppler_config.bandpass_low,
                doppler_config.bandpass_high,
                sample_rate,
                doppler_config.filter_order,
            )?,
            zero_detector: ZeroCrossingDetector::new(doppler_config.zero_cross_hysteresis),
            sample_counter: 0,
            bearing_smoother: MovingAverage::new(smoothing),
        })
    }

    /// Process doppler channel and calculate bearing relative to north tick
    pub fn process_buffer(
        &mut self,
        doppler_buffer: &[f32],
        north_tick: &NorthTick,
    ) -> Option<BearingMeasurement> {
        // Apply AGC to normalize signal amplitude
        let mut normalized = doppler_buffer.to_vec();
        self.agc.process_buffer(&mut normalized);

        // Filter doppler tone
        let mut filtered = normalized;
        self.bandpass.process_buffer(&mut filtered);

        // Find zero crossings
        let crossings = self.zero_detector.find_all_crossings(&filtered);

        eprintln!("BearingCalc: sample_counter={}, tick.sample_index={}, buffer_len={}, crossings={}",
                  self.sample_counter, north_tick.sample_index, doppler_buffer.len(), crossings.len());

        if crossings.is_empty() {
            eprintln!("BearingCalc: No zero crossings found");
            self.sample_counter += doppler_buffer.len();
            return None;
        }

        // Get rotation period
        let samples_per_rotation = north_tick.period?;

        // Use the first crossing in the buffer
        let crossing_idx = crossings[0];
        let global_crossing = self.sample_counter + crossing_idx;

        eprintln!("BearingCalc: crossing_idx={}, global_crossing={}, samples_per_rotation={}",
                  crossing_idx, global_crossing, samples_per_rotation);

        // Calculate samples elapsed since north tick
        let samples_since_tick = if global_crossing >= north_tick.sample_index {
            (global_crossing - north_tick.sample_index) as f32
        } else {
            // Handle wrap-around (shouldn't normally happen)
            eprintln!("BearingCalc: Wrap-around detected! global_crossing < tick.sample_index");
            self.sample_counter += doppler_buffer.len();
            return None;
        };

        // Calculate phase in radians
        let phase = (samples_since_tick / samples_per_rotation) * 2.0 * PI;

        // Convert to bearing (0-360 degrees)
        let raw_bearing = phase_to_bearing(phase);

        // Apply smoothing
        let smoothed_bearing = self.bearing_smoother.add(raw_bearing);

        self.sample_counter += doppler_buffer.len();

        Some(BearingMeasurement {
            bearing_degrees: smoothed_bearing,
            raw_bearing,
            confidence: self.calculate_confidence(&crossings),
            timestamp_samples: global_crossing,
        })
    }

    /// Calculate confidence metric based on signal quality
    fn calculate_confidence(&self, crossings: &[usize]) -> f32 {
        // Simple confidence: more crossings = better signal
        // In a real implementation, could use SNR, coherence, etc.
        let crossing_rate = crossings.len() as f32;
        if crossing_rate > 0.0 {
            (crossing_rate / 10.0).min(1.0)
        } else {
            0.0
        }
    }

    /// Reset calculator state
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.sample_counter = 0;
        self.zero_detector.reset();
        self.bearing_smoother.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgcConfig;

    #[test]
    fn test_zero_crossing_bearing_calculator_creation() {
        let doppler_config = DopplerConfig::default();
        let agc_config = AgcConfig::default();
        let sample_rate = 48000.0;
        let calc = ZeroCrossingBearingCalculator::new(&doppler_config, &agc_config, sample_rate, 1);
        assert!(calc.is_ok(), "Should be able to create ZeroCrossingBearingCalculator");
    }
}
