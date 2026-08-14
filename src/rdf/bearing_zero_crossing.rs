use crate::config::{AgcConfig, ConfidenceWeights, DopplerConfig};
use crate::error::Result;
use crate::signal_processing::{ZeroCrossingDetector, power_to_db};
use std::f32::consts::PI;

use super::bearing::{
    MAX_PHASE_VARIANCE, MIN_POWER_THRESHOLD, bearing_uncertainty_deg, wrap_phase_diff,
};

const DEFAULT_SINGLE_CROSSING_COHERENCE: f32 = 0.5;

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
        confidence_weights: ConfidenceWeights,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        Ok(Self {
            base: BearingCalculatorBase::new(
                doppler_config,
                agc_config,
                confidence_weights,
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
            confidence: metrics.combined_score(self.base.confidence_weights()),
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

        let expected_crossings = self.base.work_buffer.len() as f32 / samples_per_rotation;
        let signal_strength = if expected_crossings > 0.0 {
            (crossings.len() as f32 / expected_crossings).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Spread of the per-crossing phases about the bearing they were
        // averaged into, scored the same way the correlation method scores the
        // spread of its sub-window phases.
        //
        // What this replaced measured the regularity of the intervals between
        // crossings instead. That is a different quantity: a tone whose
        // crossings are evenly spaced but sitting at the wrong phase scores
        // well on it, and it says nothing about how much the phases disagree
        // with each other. Both calculators report `coherence` into the same
        // confidence weighting, so the two have to mean the same thing or the
        // score changes meaning when the method is switched.
        let mut phase_variance = 0.0f32;
        let coherence = if crossings.len() >= 2 {
            let variance = crossings
                .iter()
                .map(|&crossing_idx| {
                    let samples_since_tick = self.base.samples_since_tick(north_tick, crossing_idx);
                    let angle = samples_since_tick / samples_per_rotation * 2.0 * PI;
                    let deviation = wrap_phase_diff(angle, avg_phase);
                    deviation * deviation
                })
                .sum::<f32>()
                / crossings.len() as f32;
            phase_variance = variance;
            (1.0 - variance / MAX_PHASE_VARIANCE).clamp(0.0, 1.0)
        } else {
            DEFAULT_SINGLE_CROSSING_COHERENCE
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
            snr_db,
            coherence,
            signal_strength,
            bearing_uncertainty_deg: bearing_uncertainty_deg(
                phase_variance,
                crossings.len(),
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
            ConfidenceWeights::default(),
            sample_rate,
            1,
        );

        assert!(
            calc.is_ok(),
            "Should be able to create ZeroCrossingBearingCalculator"
        );
    }

    /// Coherence must fall as the per-crossing phases disagree.
    ///
    /// The metric this replaced scored the regularity of the intervals
    /// between crossings, which is a different quantity and is why the two
    /// bearing methods could report the same field meaning different things.
    /// Driving the phases apart directly is the way to see that the number
    /// now tracks phase agreement.
    #[test]
    fn test_coherence_falls_as_crossing_phases_disagree() {
        let sample_rate = 48_000.0f32;
        let doppler_config = DopplerConfig::default();
        let period = sample_rate / doppler_config.expected_freq;

        let coherence_with_scatter = |scatter_fraction: f32| -> f32 {
            let calc = ZeroCrossingBearingCalculator::new(
                &DopplerConfig::default(),
                &AgcConfig::default(),
                ConfidenceWeights::default(),
                sample_rate,
                1,
            )
            .expect("calculator");

            // Crossings one rotation apart, each displaced from where the
            // rotation says it belongs by an alternating fraction of a turn.
            let crossings: Vec<f32> = (0..16)
                .map(|k| {
                    let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
                    k as f32 * period + sign * scatter_fraction * period
                })
                .collect();

            let tick = NorthTick {
                sample_index: 0,
                period: Some(period),
                lock_quality: Some(1.0),
                phase_variance: None,
                fractional_sample_offset: 0.0,
                phase: 0.0,
                frequency: 2.0 * PI / period,
            };

            let (sum_x, sum_y) = crossings
                .iter()
                .map(|&c| {
                    let angle = calc.base.samples_since_tick(&tick, c) / period * 2.0 * PI;
                    (angle.cos(), angle.sin())
                })
                .fold((0.0f32, 0.0f32), |(ax, ay), (x, y)| (ax + x, ay + y));
            let avg_phase = sum_y.atan2(sum_x);

            calc.calculate_metrics(&crossings, period, &tick, avg_phase)
                .coherence
        };

        let agreed = coherence_with_scatter(0.0);
        let spread = coherence_with_scatter(0.1);
        let scattered = coherence_with_scatter(0.25);

        assert!(
            agreed > 0.99,
            "Crossings all at the same phase should be fully coherent, got {agreed}"
        );
        assert!(
            spread < agreed,
            "A tenth of a turn of scatter should cost coherence: {spread} against {agreed}"
        );
        assert!(
            scattered < spread,
            "A quarter turn of scatter should cost more still: {scattered} against {spread}"
        );
        // A quarter turn either way is half the circle apart, which is most of
        // the way to carrying no common phase at all.
        assert!(
            scattered < 0.3,
            "A quarter turn of scatter should leave little coherence, got {scattered}"
        );
    }
}
