use crate::config::{AgcConfig, DopplerConfig};
use crate::error::Result;
use crate::rdf::{BearingMeasurement, ConfidenceMetrics, NorthTick};
use crate::signal_processing::{
    AutomaticGainControl, BandpassFilter, MovingAverage, ZeroCrossingDetector, phase_to_bearing,
};
use std::f32::consts::PI;

/// Zero-crossing based bearing calculator
///
/// Calculates bearing by detecting zero-crossings in the filtered Doppler tone
/// and measuring phase offset relative to north tick pulses.
///
/// This method is simple and fast (~7° accuracy) but less robust to noise than
/// correlation-based methods.
pub struct ZeroCrossingBearingCalculator {
    agc: AutomaticGainControl,
    bandpass: BandpassFilter,
    zero_detector: ZeroCrossingDetector,
    sample_counter: usize,
    bearing_smoother: MovingAverage,
    work_buffer: Vec<f32>,
}

impl ZeroCrossingBearingCalculator {
    /// Create a new zero-crossing bearing calculator
    ///
    /// # Arguments
    /// * `doppler_config` - Doppler processing configuration
    /// * `agc_config` - AGC configuration
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `smoothing` - Moving average window size
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
            work_buffer: Vec::new(),
        })
    }

    /// Process Doppler channel and calculate bearing relative to north tick
    ///
    /// Returns a bearing measurement if successful, or `None` if no valid
    /// bearing could be calculated (e.g., no zero-crossings detected).
    ///
    /// # Arguments
    /// * `doppler_buffer` - Audio samples from Doppler channel
    /// * `north_tick` - Most recent north reference tick
    pub fn process_buffer(
        &mut self,
        doppler_buffer: &[f32],
        north_tick: &NorthTick,
    ) -> Option<BearingMeasurement> {
        // Apply AGC to normalize signal amplitude
        self.work_buffer.clear();
        self.work_buffer.extend_from_slice(doppler_buffer);
        self.agc.process_buffer(&mut self.work_buffer);

        // Filter doppler tone
        self.bandpass.process_buffer(&mut self.work_buffer);

        // Find zero crossings
        let crossings = self.zero_detector.find_all_crossings(&self.work_buffer);

        if crossings.is_empty() {
            self.sample_counter += doppler_buffer.len();
            return None;
        }

        // Get rotation period
        let samples_per_rotation = north_tick.period?;

        // Use the first crossing in the buffer
        let crossing_idx = crossings[0];
        let global_crossing = self.sample_counter + crossing_idx;

        // Calculate samples elapsed since north tick
        let samples_since_tick = if global_crossing >= north_tick.sample_index {
            (global_crossing - north_tick.sample_index) as f32
        } else {
            // Handle wrap-around (shouldn't normally happen)
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

        let metrics = self.calculate_metrics(&crossings, samples_per_rotation);

        Some(BearingMeasurement {
            bearing_degrees: smoothed_bearing,
            raw_bearing,
            confidence: metrics.combined_score(),
            metrics,
            timestamp_samples: global_crossing,
        })
    }

    fn calculate_metrics(
        &self,
        crossings: &[usize],
        samples_per_rotation: f32,
    ) -> ConfidenceMetrics {
        if crossings.is_empty() {
            return ConfidenceMetrics::default();
        }

        // --- Signal Strength ---
        // Based on crossing count - more crossings indicates stronger signal
        let expected_crossings = (self.work_buffer.len() as f32 / samples_per_rotation) * 2.0;
        let signal_strength = if expected_crossings > 0.0 {
            (crossings.len() as f32 / expected_crossings).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // --- Coherence from crossing regularity ---
        // Measure how regular the crossing intervals are
        let coherence = if crossings.len() >= 2 {
            let expected_interval = samples_per_rotation / 2.0;
            let mut interval_errors = Vec::with_capacity(crossings.len() - 1);

            for window in crossings.windows(2) {
                let interval = (window[1] - window[0]) as f32;
                let error = ((interval - expected_interval) / expected_interval).abs();
                interval_errors.push(error);
            }

            let mean_error: f32 =
                interval_errors.iter().sum::<f32>() / interval_errors.len() as f32;
            (1.0 - mean_error.min(1.0)).clamp(0.0, 1.0)
        } else {
            0.5
        };

        // --- SNR Estimation from signal amplitude ---
        // Use signal power from the work buffer
        let signal_power: f32 =
            self.work_buffer.iter().map(|s| s * s).sum::<f32>() / self.work_buffer.len() as f32;
        let snr_db = if signal_power > 1e-10 {
            10.0 * signal_power.log10() + 40.0
        } else {
            0.0
        };

        ConfidenceMetrics {
            snr_db: snr_db.clamp(0.0, 40.0),
            coherence,
            signal_strength,
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
        assert!(
            calc.is_ok(),
            "Should be able to create ZeroCrossingBearingCalculator"
        );
    }
}
