use crate::config::{AgcConfig, BearingMethod, ConfidenceConfig, DopplerConfig};
use crate::error::Result;
use crate::signal_processing::{ZeroCrossingDetector, power_to_db};
use std::f32::consts::PI;

use super::bearing::{MIN_POWER_THRESHOLD, bearing_uncertainty_deg, resultant_length_from_snr};

use super::bearing::phase_to_bearing;
use super::bearing_calculator_base::BearingCalculatorBase;
use super::{BearingCalculator, BearingMeasurement, ConfidenceMetrics, NorthTick};

/// Zero-crossing based bearing calculator
///
/// Calculates bearing by detecting zero-crossings in the filtered Doppler tone
/// and measuring phase offset relative to north tick pulses.
///
/// This method achieves sub-degree accuracy (<1°) with sub-sample interpolation,
/// similar to correlation-based detection but with lower CPU usage and less
/// noise robustness.
pub struct ZeroCrossingBearingCalculator {
    base: BearingCalculatorBase,
    zero_detector: ZeroCrossingDetector,
    preprocessed_len: usize,
    crossings: Vec<f32>,
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
        confidence: ConfidenceConfig,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        Ok(Self {
            base: BearingCalculatorBase::new(
                doppler_config,
                agc_config,
                confidence,
                sample_rate,
                smoothing,
            )?,
            zero_detector: ZeroCrossingDetector::new(doppler_config.zero_cross_hysteresis),
            preprocessed_len: 0,
            crossings: Vec::new(),
        })
    }

    fn process_tick_impl(&mut self, north_tick: &NorthTick) -> Option<BearingMeasurement> {
        if self.crossings.is_empty() {
            return None;
        }

        // Get rotation period
        let samples_per_rotation = north_tick.period?;
        if !samples_per_rotation.is_finite()
            || samples_per_rotation <= 0.0
            || !north_tick.phase.is_finite()
        {
            return None;
        }

        // To robustly calculate the bearing, we average the phase of all detected
        // crossings. This is done by converting each phase angle to a vector,
        // summing the vectors, and finding the angle of the resultant vector.
        // Account for FIR filter group delay in timing calculation.
        // The zero crossing detector provides sub-sample interpolation.
        // Add the north tick timing adjustment for FIR highpass filter effects.
        let (sum_x, sum_y) = self
            .crossings
            .iter()
            .map(|&crossing_idx| {
                let samples_since_tick = self.base.samples_since_tick(north_tick, crossing_idx);
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

        let metrics =
            self.calculate_metrics(&self.crossings, samples_per_rotation, north_tick, avg_phase);

        Some(BearingMeasurement {
            bearing_degrees: smoothed_bearing,
            raw_bearing,
            confidence: metrics.score(self.base.confidence()),
            signal_present: metrics.signal_strength
                >= self
                    .base
                    .confidence()
                    .resolved_min_signal_strength(BearingMethod::ZeroCrossing),
            metrics,
        })
    }

    fn calculate_metrics(
        &self,
        crossings: &[f32],
        samples_per_rotation: f32,
        north_tick: &NorthTick,
        avg_phase: f32,
    ) -> ConfidenceMetrics {
        if crossings.is_empty() {
            return ConfidenceMetrics::default();
        }

        // Crossing density against the density the rotation tone implies,
        // scored by how close it is in *either* direction. This used to clamp
        // the ratio at 1, which discarded the only direction that
        // discriminates: broadband noise crosses zero far more often than a
        // 1602 Hz tone, so hiss ran a ratio of 4.3 to 5.8 and the clamp folded
        // it onto the same 1.000 that a perfect tone reads. Measured, the two
        // populations do not overlap at all once the excess is kept -- real
        // signal spans 0.99 to 1.29 across every channel condition in
        // METRICS.md.
        //
        // Below 1 this is exactly the old fraction-of-expected-crossings, so
        // nothing that already worked changes.
        let expected_crossings = self.base.work_buffer.len() as f32 / samples_per_rotation;
        let signal_strength = if expected_crossings > 0.0 {
            let ratio = crossings.len() as f32 / expected_crossings;
            if ratio > 0.0 {
                ratio.min(ratio.recip()).clamp(0.0, 1.0)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // --- SNR Estimation via projection onto ideal Doppler sine ---
        // Reconstruct an ideal sine wave at the known bearing phase and north tick
        // frequency, then measure how much of the actual signal correlates with it.
        let omega = north_tick.frequency;
        let snr_db = if omega > 0.0 {
            let mut projection_sum = 0.0f32;
            let mut power_sum = 0.0f32;

            for (idx, &sample) in self.base.work_buffer.iter().enumerate() {
                let samples_since_tick = self.base.samples_since_tick(north_tick, idx as f32);
                let phase = north_tick.phase + samples_since_tick * omega;
                let ideal = (phase - avg_phase).sin();
                projection_sum += sample * ideal;
                power_sum += sample * sample;
            }

            let n = self.base.work_buffer.len() as f32;
            let projection = projection_sum / n;
            let signal_power = power_sum / n;

            // projection ≈ A/2 for signal A*sin(ωt - φ), since sin² averages to 1/2.
            // Correlated power = 2 * projection² reconstructs the full signal power.
            let correlated_power = (2.0 * projection * projection).max(0.0).min(signal_power);
            let noise_power = (signal_power - correlated_power).max(MIN_POWER_THRESHOLD);
            power_to_db(correlated_power / noise_power)
        } else {
            0.0
        };

        ConfidenceMetrics {
            tone_peak: self.base.work_buffer.iter().copied().fold(0.0f32, f32::max),
            resultant_length: resultant_length_from_snr(snr_db),
            snr_db,
            signal_strength,
            bearing_uncertainty_deg: bearing_uncertainty_deg(
                snr_db,
                self.base.independent_looks(),
                north_tick,
            ),
        }
    }
}

impl BearingCalculator for ZeroCrossingBearingCalculator {
    fn preprocess(&mut self, doppler_buffer: &[f32]) {
        self.base.preprocess(doppler_buffer);
        self.preprocessed_len = doppler_buffer.len();
        // Find zero crossings once per buffer
        self.crossings = self
            .zero_detector
            .find_all_crossings(&self.base.work_buffer);
    }

    fn process_tick(&mut self, north_tick: &NorthTick) -> Option<BearingMeasurement> {
        self.process_tick_impl(north_tick)
    }

    fn advance_samples(&mut self, samples: usize) {
        self.base.advance_counter(samples);
    }

    fn advance_buffer(&mut self) {
        self.base.advance_counter(self.preprocessed_len);
    }

    fn filtered_buffer(&self) -> &[f32] {
        &self.base.work_buffer
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

        let calc = ZeroCrossingBearingCalculator::new(
            &doppler_config,
            &agc_config,
            ConfidenceConfig::default(),
            sample_rate,
            1,
        );

        assert!(
            calc.is_ok(),
            "Should be able to create ZeroCrossingBearingCalculator"
        );
    }

    /// An unknown reference must suppress the figure entirely.
    ///
    /// The doppler term can always be computed from the SNR, so this is the
    /// only thing that can withhold it, and it must: an unknown reference is
    /// not a perfect one.
    #[test]
    fn test_unknown_reference_suppresses_the_uncertainty() {
        let sample_rate = 48_000.0f32;
        let doppler_config = DopplerConfig::default();
        let period = sample_rate / doppler_config.expected_freq;
        let omega = 2.0 * PI / period;

        let measure = |reference: Option<f32>| -> Option<f32> {
            let mut calc = ZeroCrossingBearingCalculator::new(
                &doppler_config,
                &AgcConfig::default(),
                ConfidenceConfig::default(),
                sample_rate,
                1,
            )
            .expect("calculator");
            let tick = NorthTick {
                sample_index: 0,
                period: Some(period),
                lock_quality: Some(1.0),
                phase_variance: reference,
                fractional_sample_offset: 0.0,
                phase: 0.0,
                frequency: omega,
            };
            let buffer: Vec<f32> = (0..4096)
                .map(|i| (omega * i as f32 - 45.0f32.to_radians()).sin())
                .collect();
            calc.process_buffer(&buffer, &tick)
                .expect("a bearing")
                .metrics
                .bearing_uncertainty_deg
        };

        assert!(
            measure(None).is_none(),
            "an unknown reference must give none"
        );
        assert!(
            measure(Some(0.0)).is_some(),
            "a known reference must give a figure"
        );
    }
}
