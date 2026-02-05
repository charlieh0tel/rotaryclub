use crate::config::{AgcConfig, DopplerConfig};
use crate::error::Result;
use std::f32::consts::PI;

use super::bearing::MIN_POWER_THRESHOLD;
const COHERENCE_WINDOW_COUNT: usize = 4;
const MAX_PHASE_VARIANCE: f32 = PI * PI / 3.0;
const MIN_SIGNAL_STRENGTH_POWER: f32 = 0.01;

use super::bearing::phase_to_bearing;
use super::bearing_calculator_base::BearingCalculatorBase;
use super::{BearingCalculator, BearingMeasurement, ConfidenceMetrics, NorthTick};

/// Correlation-based bearing calculator using I/Q demodulation
///
/// Calculates bearing by correlating the filtered Doppler tone with sin/cos
/// reference signals at the rotation frequency, extracting phase via atan2.
/// Uses DPLL phase/frequency from NorthTick for accurate reference generation.
///
/// This method achieves sub-degree accuracy (<1°) and is more robust to noise
/// than zero-crossing detection, at the cost of slightly higher CPU usage.
pub struct CorrelationBearingCalculator {
    base: BearingCalculatorBase,
}

impl CorrelationBearingCalculator {
    /// Create a new correlation-based bearing calculator
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
        })
    }

    fn process_buffer_impl(
        &mut self,
        doppler_buffer: &[f32],
        north_tick: &NorthTick,
    ) -> Option<BearingMeasurement> {
        self.base.preprocess(doppler_buffer);

        // Calculate base offset from north tick
        let base_offset = match self.base.offset_from_north_tick(north_tick) {
            Some(offset) => offset,
            None => {
                self.base.advance_counter(doppler_buffer.len());
                return None;
            }
        };

        // Use DPLL's tracked frequency directly
        let omega = north_tick.frequency;
        if omega <= 0.0 {
            self.base.advance_counter(doppler_buffer.len());
            return None;
        }

        // I/Q demodulation: correlate with cos and sin using DPLL's phase tracking
        // base_offset is already (sample_counter - tick.sample_index), i.e., samples since tick.
        // Account for FIR filter group delay in the doppler path.
        let mut i_sum = 0.0;
        let mut q_sum = 0.0;
        let mut power_sum = 0.0;
        let group_delay = self.base.filter_group_delay() as f32;
        let tick_adjustment = self.base.north_tick_timing_adjustment();

        for (idx, &sample) in self.base.work_buffer.iter().enumerate() {
            // Samples since the north tick, compensated for filter delay
            let samples_since_tick = (base_offset + idx) as f32 - group_delay + tick_adjustment;
            // Phase from DPLL: start at tick phase, advance by omega per sample
            let phase = north_tick.phase + samples_since_tick * omega;

            i_sum += sample * phase.cos();
            q_sum += sample * phase.sin();
            power_sum += sample * sample;
        }

        // Normalize by buffer length
        let n = self.base.work_buffer.len() as f32;
        let i = i_sum / n;
        let q = q_sum / n;

        // Calculate signal power for confidence metric
        let signal_power = power_sum / n;
        let correlation_magnitude = (i * i + q * q).sqrt();

        // Calculate confidence metrics
        let metrics =
            self.calculate_metrics(north_tick, base_offset, signal_power, correlation_magnitude);

        // Extract bearing directly from I/Q
        // Our signal is: A * sin(ω*t - φ) where φ is the bearing (note the minus!)
        // Correlating with sin(ω*t) and cos(ω*t) gives:
        // I ≈ A * sin(-φ) = -A * sin(φ)
        // Q ≈ A * cos(-φ) = A * cos(φ)
        // Therefore: -φ = atan2(I, Q), so φ = -atan2(I, Q)
        let bearing_phase = -i.atan2(q);

        // Normalize phase to [0, 2π)
        let normalized_phase = bearing_phase.rem_euclid(2.0 * PI);

        // Convert to bearing (0-360 degrees)
        let raw_bearing = phase_to_bearing(normalized_phase);

        // Apply smoothing
        let smoothed_bearing = self.base.smooth_bearing(raw_bearing);

        self.base.advance_counter(doppler_buffer.len());

        Some(BearingMeasurement {
            bearing_degrees: smoothed_bearing,
            raw_bearing,
            confidence: metrics.combined_score(),
            metrics,
        })
    }

    fn calculate_metrics(
        &self,
        north_tick: &NorthTick,
        base_offset: usize,
        signal_power: f32,
        correlation_magnitude: f32,
    ) -> ConfidenceMetrics {
        let n = self.base.work_buffer.len();
        if n < COHERENCE_WINDOW_COUNT || signal_power < MIN_POWER_THRESHOLD {
            return ConfidenceMetrics::default();
        }

        // --- SNR Estimation ---
        let correlated_power = correlation_magnitude * correlation_magnitude;
        let noise_power = (signal_power - correlated_power).max(MIN_POWER_THRESHOLD);
        let snr_db = 10.0 * (correlated_power / noise_power).log10();

        // --- Coherence Estimation ---
        // Split buffer into sub-windows and compute phase in each
        let window_size = n / COHERENCE_WINDOW_COUNT;
        let mut phases = [0.0f32; COHERENCE_WINDOW_COUNT];
        let group_delay = self.base.filter_group_delay() as f32;
        let omega = north_tick.frequency;

        let tick_adjustment = self.base.north_tick_timing_adjustment();
        for (win_idx, phase) in phases.iter_mut().enumerate() {
            let start = win_idx * window_size;
            let end = start + window_size;

            let mut i_win = 0.0;
            let mut q_win = 0.0;

            for (idx, &sample) in self.base.work_buffer[start..end].iter().enumerate() {
                let samples_since_tick =
                    (base_offset + start + idx) as f32 - group_delay + tick_adjustment;
                let p = north_tick.phase + samples_since_tick * omega;
                i_win += sample * p.cos();
                q_win += sample * p.sin();
            }

            *phase = (-i_win).atan2(q_win);
        }

        // Calculate phase variance (circular variance)
        let mean_phase = phases.iter().sum::<f32>() / COHERENCE_WINDOW_COUNT as f32;
        let phase_variance: f32 = phases
            .iter()
            .map(|p| {
                let diff = (p - mean_phase).rem_euclid(2.0 * PI);
                let wrapped = if diff > PI { diff - 2.0 * PI } else { diff };
                wrapped * wrapped
            })
            .sum::<f32>()
            / COHERENCE_WINDOW_COUNT as f32;

        let max_variance = MAX_PHASE_VARIANCE;
        let coherence = (1.0 - phase_variance / max_variance).clamp(0.0, 1.0);

        // --- Signal Strength ---
        let signal_strength = if signal_power > MIN_SIGNAL_STRENGTH_POWER {
            (correlation_magnitude / signal_power.sqrt()).clamp(0.0, 1.0)
        } else {
            0.0
        };

        ConfidenceMetrics {
            snr_db,
            coherence,
            signal_strength,
        }
    }
}

impl BearingCalculator for CorrelationBearingCalculator {
    fn process_buffer(
        &mut self,
        doppler_buffer: &[f32],
        north_tick: &NorthTick,
    ) -> Option<BearingMeasurement> {
        self.process_buffer_impl(doppler_buffer, north_tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgcConfig;
    use std::f32::consts::PI;

    #[test]
    fn test_correlation_bearing_calculator_creation() {
        let doppler_config = DopplerConfig::default();
        let agc_config = AgcConfig::default();
        let sample_rate = 48000.0;
        let calc = CorrelationBearingCalculator::new(&doppler_config, &agc_config, sample_rate, 1);
        assert!(
            calc.is_ok(),
            "Should be able to create CorrelationBearingCalculator"
        );
    }

    #[test]
    fn test_bearing_from_known_phase() {
        let sample_rate = 48000.0;
        let mut doppler_config = DopplerConfig::default();
        doppler_config.expected_freq = 480.0;
        doppler_config.bandpass_low = 400.0;
        doppler_config.bandpass_high = 560.0;

        let agc_config = AgcConfig::default();
        let mut calc =
            CorrelationBearingCalculator::new(&doppler_config, &agc_config, sample_rate, 1)
                .unwrap();

        let samples_per_rotation = sample_rate / doppler_config.expected_freq; // 100.0
        let omega = 2.0 * PI / samples_per_rotation;
        let north_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase: 0.0,
            frequency: omega,
        };

        let omega = 2.0 * PI * doppler_config.expected_freq / sample_rate;
        let bearing_radians = 45.0f32.to_radians(); // Target bearing is 45 degrees

        // Generate a signal A*sin(ωt - φ)
        let buffer: Vec<f32> = (0..300)
            .map(|i| (omega * i as f32 - bearing_radians).sin())
            .collect();

        // Assume base_offset inside process_buffer will be 0
        let measurement = calc.process_buffer(&buffer, &north_tick);

        assert!(measurement.is_some(), "Should produce a measurement");
        let bearing = measurement.unwrap().raw_bearing;

        // The calculated bearing should be close to the known phase
        // Allow some tolerance for filter effects and processing
        assert!(
            (bearing - 45.0).abs() < 5.0,
            "Bearing calculation was incorrect. Got {}, expected 45.0",
            bearing
        );
    }
}
