use crate::config::{AgcConfig, DopplerConfig};
use crate::error::Result;
use crate::signal_processing::ZeroCrossingDetector;
use std::f32::consts::PI;

const DEFAULT_SINGLE_CROSSING_COHERENCE: f32 = 0.5;
const MIN_POWER_THRESHOLD: f32 = 1e-10;
const SNR_DB_OFFSET: f32 = 40.0;
const MAX_SNR_DB: f32 = 40.0;

use super::bearing::phase_to_bearing;
use super::bearing_calculator_base::BearingCalculatorBase;
use super::{BearingMeasurement, ConfidenceMetrics, NorthTick};

/// Zero-crossing based bearing calculator
///
/// Calculates bearing by detecting zero-crossings in the filtered Doppler tone
/// and measuring phase offset relative to north tick pulses.
///
/// This method is simple and fast (~7° accuracy) but less robust to noise than
/// correlation-based methods.
pub struct ZeroCrossingBearingCalculator {
    base: BearingCalculatorBase,
    zero_detector: ZeroCrossingDetector,
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
            base: BearingCalculatorBase::new(doppler_config, agc_config, sample_rate, smoothing)?,
            zero_detector: ZeroCrossingDetector::new(doppler_config.zero_cross_hysteresis),
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
        self.base.preprocess(doppler_buffer);

        // Find zero crossings in the filtered buffer
        let crossings = self
            .zero_detector
            .find_all_crossings(&self.base.work_buffer);

        if crossings.is_empty() {
            self.base.advance_counter(doppler_buffer.len());
            return None;
        }

        // Calculate base offset from north tick
        let base_offset = match self.base.offset_from_north_tick(north_tick) {
            Some(offset) => offset,
            None => {
                self.base.advance_counter(doppler_buffer.len());
                return None;
            }
        };

        // Get rotation period
        let samples_per_rotation = north_tick.period?;

        // To robustly calculate the bearing, we average the phase of all detected
        // crossings. This is done by converting each phase angle to a vector,
        // summing the vectors, and finding the angle of the resultant vector.
        // Account for FIR filter group delay in timing calculation.
        // Also subtract 0.5 for zero-crossing timing: the detector triggers when
        // sample > threshold, but the true zero crossing occurred between the
        // previous sample and current sample (on average 0.5 samples earlier).
        // Add the north tick timing adjustment for FIR highpass filter effects.
        let group_delay = self.base.filter_group_delay() as f32;
        let tick_adjustment = self.base.north_tick_timing_adjustment();
        let (sum_x, sum_y) = crossings
            .iter()
            .map(|&crossing_idx| {
                let samples_since_tick =
                    (base_offset + crossing_idx) as f32 - group_delay - 0.5 + tick_adjustment;
                let phase_fraction = samples_since_tick / samples_per_rotation;
                let angle = phase_fraction * 2.0 * PI;
                (angle.cos(), angle.sin())
            })
            .fold((0.0, 0.0), |(acc_x, acc_y), (x, y)| (acc_x + x, acc_y + y));

        let avg_phase = sum_y.atan2(sum_x);

        // Convert to bearing (0-360 degrees)
        let raw_bearing = phase_to_bearing(avg_phase);

        // Apply smoothing
        let smoothed_bearing = self.base.smooth_bearing(raw_bearing);

        self.base.advance_counter(doppler_buffer.len());

        let metrics = self.calculate_metrics(&crossings, samples_per_rotation);

        Some(BearingMeasurement {
            bearing_degrees: smoothed_bearing,
            raw_bearing,
            confidence: metrics.combined_score(),
            metrics,
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
        // A rising-edge detector should find one crossing per rotation.
        let expected_crossings = self.base.work_buffer.len() as f32 / samples_per_rotation;
        let signal_strength = if expected_crossings > 0.0 {
            (crossings.len() as f32 / expected_crossings).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // --- Coherence from crossing regularity ---
        let coherence = if crossings.len() >= 2 {
            // The interval between rising-edge crossings should be one full period.
            let expected_interval = samples_per_rotation;
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
            DEFAULT_SINGLE_CROSSING_COHERENCE
        };

        // --- SNR Estimation from signal amplitude ---
        let signal_power: f32 = self.base.work_buffer.iter().map(|s| s * s).sum::<f32>()
            / self.base.work_buffer.len() as f32;
        let snr_db = if signal_power > MIN_POWER_THRESHOLD {
            10.0 * signal_power.log10() + SNR_DB_OFFSET
        } else {
            0.0
        };

        ConfidenceMetrics {
            snr_db: snr_db.clamp(0.0, MAX_SNR_DB),
            coherence,
            signal_strength,
        }
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
