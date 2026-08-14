use crate::config::{AgcConfig, ConfidenceConfig, DopplerConfig};
use crate::error::Result;
use crate::signal_processing::power_to_db;
use std::f32::consts::PI;

use super::bearing::{
    MIN_POWER_THRESHOLD, bearing_uncertainty_deg, circular_mean_phase, wrap_phase_diff,
};
/// Fewest windows the coherence estimate will use. Two is the least that has
/// a spread at all; it is reached only when the buffer is shorter than two
/// rotations.
const MIN_COHERENCE_WINDOWS: usize = 2;
/// Most windows it will use, which caps the per-tick work on long buffers.
/// Beyond this each window covers more than one rotation again, which is the
/// old behaviour but with enough points to be a variance.
const MAX_COHERENCE_WINDOWS: usize = 64;
const MIN_SIGNAL_STRENGTH_POWER: f32 = 0.01;
// Below this normalized correlation magnitude there is no Doppler tone to
// measure (a dead channel yields i = q = 0 and atan2(0, 0) would report a
// confident 0 degrees); samples are normalized to +/-1.0, so real noise
// floors sit orders of magnitude above this.
const MIN_CORRELATION_MAGNITUDE: f32 = 1e-6;

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
    preprocessed_len: usize,
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
            preprocessed_len: 0,
        })
    }

    fn process_tick_impl(&mut self, north_tick: &NorthTick) -> Option<BearingMeasurement> {
        if self.base.work_buffer.is_empty() {
            return None;
        }

        // Use DPLL's tracked frequency directly
        let omega = north_tick.frequency;
        if !omega.is_finite() || omega <= 0.0 || !north_tick.phase.is_finite() {
            return None;
        }

        // I/Q demodulation: correlate with cos and sin using DPLL's phase tracking
        // base_offset is (buffer_start - tick.sample_index), can be negative.
        // Account for FIR filter group delay in the doppler path.
        let mut i_sum = 0.0;
        let mut q_sum = 0.0;
        let mut power_sum = 0.0;

        for (idx, &sample) in self.base.work_buffer.iter().enumerate() {
            let samples_since_tick = self.base.samples_since_tick(north_tick, idx as f32);
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

        // No measurable Doppler tone (e.g. muted or disconnected channel):
        // suppress the measurement rather than emit atan2(0, 0) = 0 degrees,
        // and keep the smoothing window uncontaminated.
        if correlation_magnitude < MIN_CORRELATION_MAGNITUDE {
            return None;
        }

        // Calculate confidence metrics
        let metrics = self.calculate_metrics(north_tick, signal_power, correlation_magnitude);

        // No signal-strength gate here. This method's signal strength is the
        // fraction of power that correlated with the reference, which falls
        // when the reference is wrong as readily as when the channel is dead
        // -- a rotation rate mismatch takes it below a half on a perfectly
        // live signal, and low SNR takes it low enough to suppress bearings
        // that are poor rather than absent. Absence is already caught above,
        // by the correlation magnitude and the signal power floor.

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

        Some(BearingMeasurement {
            bearing_degrees: smoothed_bearing,
            raw_bearing,
            confidence: metrics.score(self.base.confidence()),
            metrics,
        })
    }

    fn calculate_metrics(
        &self,
        north_tick: &NorthTick,
        signal_power: f32,
        correlation_magnitude: f32,
    ) -> ConfidenceMetrics {
        let n = self.base.work_buffer.len();
        if n < MIN_COHERENCE_WINDOWS
            || !signal_power.is_finite()
            || !correlation_magnitude.is_finite()
            || signal_power < MIN_POWER_THRESHOLD
        {
            return ConfidenceMetrics::default();
        }

        // --- SNR Estimation ---
        // For a clean sine, I^2 + Q^2 = A^2 / 4 while signal_power = A^2 / 2.
        // Multiply by 2 to estimate full correlated signal power.
        let correlated_power = (2.0 * correlation_magnitude * correlation_magnitude)
            .max(0.0)
            .min(signal_power);
        let noise_power = (signal_power - correlated_power).max(MIN_POWER_THRESHOLD);
        let snr_db = power_to_db(correlated_power / noise_power);

        // --- Phase spread ---
        // One window per rotation, so the spread being measured is the spread
        // of independent bearing estimates. It feeds the uncertainty figure.
        //
        // This used to cut the buffer into four regardless of its length. At
        // the shipped buffer size that made each window average some four
        // rotations, which averages away most of the noise the metric exists
        // to detect, and four points is thin evidence for a variance besides.
        // Worse, four windows that agree with each other can be wrong
        // together: with the tone buried in noise the quarters still matched
        // to a couple of degrees while the bearing was forty degrees out, so
        // the metric read fully coherent on a measurement that was useless.
        let samples_per_rotation = north_tick
            .period
            .filter(|p| p.is_finite() && *p >= 1.0)
            .unwrap_or_else(|| 2.0 * PI / north_tick.frequency.max(f32::EPSILON));
        let window_count = ((n as f32 / samples_per_rotation) as usize)
            .clamp(MIN_COHERENCE_WINDOWS, MAX_COHERENCE_WINDOWS);
        let window_size = n / window_count;
        if window_size == 0 {
            return ConfidenceMetrics::default();
        }

        let omega = north_tick.frequency;
        let mut phases = [0.0f32; MAX_COHERENCE_WINDOWS];
        let phases = &mut phases[..window_count];
        for (win_idx, phase) in phases.iter_mut().enumerate() {
            let start = win_idx * window_size;
            let end = start + window_size;

            let mut i_win = 0.0;
            let mut q_win = 0.0;

            for (idx, &sample) in self.base.work_buffer[start..end].iter().enumerate() {
                let samples_since_tick = self
                    .base
                    .samples_since_tick(north_tick, (start + idx) as f32);
                let p = north_tick.phase + samples_since_tick * omega;
                i_win += sample * p.cos();
                q_win += sample * p.sin();
            }

            *phase = (-i_win).atan2(q_win);
        }

        // Calculate phase variance (circular variance)
        let mean_phase = circular_mean_phase(phases);
        let phase_variance: f32 = phases
            .iter()
            .map(|p| {
                let wrapped = wrap_phase_diff(*p, mean_phase);
                wrapped * wrapped
            })
            .sum::<f32>()
            / window_count as f32;

        let bearing_uncertainty_deg = bearing_uncertainty_deg(
            Some(phase_variance),
            self.base.independent_estimates(),
            north_tick,
        );

        // --- Signal Strength ---
        let signal_strength = if signal_power > MIN_SIGNAL_STRENGTH_POWER {
            (correlated_power / signal_power).sqrt().clamp(0.0, 1.0)
        } else {
            0.0
        };

        ConfidenceMetrics {
            snr_db,
            signal_strength,
            bearing_uncertainty_deg,
        }
    }
}

impl BearingCalculator for CorrelationBearingCalculator {
    fn preprocess(&mut self, doppler_buffer: &[f32]) {
        self.base.preprocess(doppler_buffer);
        self.preprocessed_len = doppler_buffer.len();
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
    use std::f32::consts::PI;

    #[test]
    fn test_correlation_bearing_calculator_creation() {
        let doppler_config = DopplerConfig::default();
        let agc_config = AgcConfig::default();
        let sample_rate = 48000.0;
        let calc = CorrelationBearingCalculator::new(
            &doppler_config,
            &agc_config,
            ConfidenceConfig::default(),
            sample_rate,
            1,
        );
        assert!(
            calc.is_ok(),
            "Should be able to create CorrelationBearingCalculator"
        );
    }

    #[test]
    fn test_bearing_from_known_phase() {
        let sample_rate = 48000.0;
        let doppler_config = DopplerConfig {
            expected_freq: 480.0,
            bandpass_low: 400.0,
            bandpass_high: 560.0,
            ..Default::default()
        };

        let agc_config = AgcConfig::default();
        let mut calc = CorrelationBearingCalculator::new(
            &doppler_config,
            &agc_config,
            ConfidenceConfig::default(),
            sample_rate,
            1,
        )
        .unwrap();

        let samples_per_rotation = sample_rate / doppler_config.expected_freq; // 100.0
        let omega = 2.0 * PI / samples_per_rotation;
        let north_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase_variance: Some(0.0),
            fractional_sample_offset: 0.0,
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

    #[test]
    fn test_fractional_tick_offset_improves_alignment() {
        let sample_rate = 48_000.0;
        let doppler_config = DopplerConfig {
            expected_freq: 1_602.0,
            bandpass_low: 1_500.0,
            bandpass_high: 1_700.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();
        let mut calc_uncorrected = CorrelationBearingCalculator::new(
            &doppler_config,
            &agc_config,
            ConfidenceConfig::default(),
            sample_rate,
            1,
        )
        .unwrap();
        let mut calc_corrected = CorrelationBearingCalculator::new(
            &doppler_config,
            &agc_config,
            ConfidenceConfig::default(),
            sample_rate,
            1,
        )
        .unwrap();

        let samples_per_rotation = sample_rate / doppler_config.expected_freq;
        let omega = 2.0 * PI / samples_per_rotation;
        let true_fractional_offset = 0.4;
        let expected_bearing = 120.0f32;
        let bearing_radians = expected_bearing.to_radians();

        // Signal is generated relative to a tick that lands at +0.4 samples.
        let buffer: Vec<f32> = (0..4800)
            .map(|i| (omega * (i as f32 - true_fractional_offset) - bearing_radians).sin())
            .collect();

        let tick_uncorrected = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase_variance: Some(0.0),
            fractional_sample_offset: 0.0,
            phase: 0.0,
            frequency: omega,
        };
        let tick_corrected = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase_variance: Some(0.0),
            fractional_sample_offset: true_fractional_offset,
            phase: 0.0,
            frequency: omega,
        };

        let m_uncorrected = calc_uncorrected
            .process_buffer(&buffer, &tick_uncorrected)
            .unwrap();
        let m_corrected = calc_corrected
            .process_buffer(&buffer, &tick_corrected)
            .unwrap();

        let angle_error = |measured: f32, expected: f32| {
            let mut e = measured - expected;
            if e > 180.0 {
                e -= 360.0;
            } else if e < -180.0 {
                e += 360.0;
            }
            e.abs()
        };

        let err_uncorrected = angle_error(m_uncorrected.raw_bearing, expected_bearing);
        let err_corrected = angle_error(m_corrected.raw_bearing, expected_bearing);

        assert!(
            err_corrected < err_uncorrected,
            "Expected fractional offset correction to reduce error (uncorrected {}, corrected {})",
            err_uncorrected,
            err_corrected
        );
        assert!(
            err_corrected < 10.0,
            "Corrected bearing error too large: {}",
            err_corrected
        );
    }

    #[test]
    fn test_silent_doppler_channel_yields_no_bearing() {
        // A dead Doppler channel used to produce atan2(0, 0) = 0 and a
        // stream of confident 0-degree bearings.
        let sample_rate = 48000.0;
        let doppler_config = DopplerConfig {
            expected_freq: 480.0,
            bandpass_low: 400.0,
            bandpass_high: 560.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();
        let mut calc = CorrelationBearingCalculator::new(
            &doppler_config,
            &agc_config,
            ConfidenceConfig::default(),
            sample_rate,
            1,
        )
        .unwrap();

        let samples_per_rotation = sample_rate / doppler_config.expected_freq;
        let omega = 2.0 * PI / samples_per_rotation;
        let north_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase_variance: Some(0.0),
            fractional_sample_offset: 0.0,
            phase: 0.0,
            frequency: omega,
        };

        let buffer = vec![0.0f32; 4800];
        let measurement = calc.process_buffer(&buffer, &north_tick);
        assert!(
            measurement.is_none(),
            "Silent Doppler channel must not produce a bearing, got {:?}",
            measurement.map(|m| m.raw_bearing)
        );
    }

    #[test]
    fn test_circular_phase_mean_wraparound() {
        let phases = [
            179.0_f32.to_radians(),
            -179.0_f32.to_radians(),
            178.0_f32.to_radians(),
            -178.0_f32.to_radians(),
        ];

        let mean = circular_mean_phase(&phases);
        let error_to_pi = wrap_phase_diff(mean, PI).abs();
        assert!(
            error_to_pi < 0.1,
            "Expected circular mean near pi, got {} rad (error {})",
            mean,
            error_to_pi
        );
    }

    #[test]
    fn test_correlation_confidence_uses_the_configured_half_point() {
        let sample_rate = 48000.0;
        let doppler_config = DopplerConfig {
            expected_freq: 480.0,
            bandpass_low: 400.0,
            bandpass_high: 560.0,
            ..Default::default()
        };
        let samples_per_rotation = sample_rate / doppler_config.expected_freq;
        let omega = 2.0 * PI / samples_per_rotation;

        let confidence_at = |half_confidence_deg: f32| -> f32 {
            let mut calc = CorrelationBearingCalculator::new(
                &doppler_config,
                &AgcConfig::default(),
                ConfidenceConfig {
                    half_confidence_deg,
                    ..Default::default()
                },
                sample_rate,
                1,
            )
            .unwrap();

            let north_tick = NorthTick {
                sample_index: 0,
                period: Some(samples_per_rotation),
                lock_quality: None,
                // A reference of known-zero scatter, so what these measure is
                // the phase spread alone. None would mean "not estimable" and
                // suppress the figure entirely, which is its own test.
                phase_variance: Some(0.0),
                fractional_sample_offset: 0.0,
                phase: 0.0,
                frequency: omega,
            };
            let bearing_radians = 45.0f32.to_radians();
            let buffer: Vec<f32> = (0..4800)
                .map(|i| (omega * i as f32 - bearing_radians).sin())
                .collect();
            calc.process_buffer(&buffer, &north_tick)
                .unwrap()
                .confidence
        };

        // The same clean signal, judged against a demanding standard and a
        // lax one. Confidence is a statement about the uncertainty relative
        // to what the caller asked for, so it has to move with that.
        let lax = confidence_at(30.0);
        let strict = confidence_at(0.01);
        assert!(
            lax > 0.9,
            "A clean signal judged against 30 degrees should score high, got {lax}"
        );
        assert!(
            strict < 0.1,
            "The same signal judged against a hundredth of a degree should \
             score low, got {strict}"
        );
    }

    #[test]
    fn test_correlation_metrics_clean_signal() {
        let sample_rate = 48000.0;
        let doppler_config = DopplerConfig {
            expected_freq: 480.0,
            bandpass_low: 400.0,
            bandpass_high: 560.0,
            ..Default::default()
        };
        let agc_config = AgcConfig::default();
        let mut calc = CorrelationBearingCalculator::new(
            &doppler_config,
            &agc_config,
            ConfidenceConfig::default(),
            sample_rate,
            1,
        )
        .unwrap();

        let samples_per_rotation = sample_rate / doppler_config.expected_freq;
        let omega = 2.0 * PI / samples_per_rotation;
        let north_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase_variance: Some(0.0),
            fractional_sample_offset: 0.0,
            phase: 0.0,
            frequency: omega,
        };

        let bearing_radians = 45.0f32.to_radians();
        let buffer: Vec<f32> = (0..4800)
            .map(|i| (omega * i as f32 - bearing_radians).sin())
            .collect();

        let measurement = calc.process_buffer(&buffer, &north_tick).unwrap();
        assert!(
            measurement.metrics.signal_strength > 0.95,
            "Expected near-unit signal strength for clean sine, got {}",
            measurement.metrics.signal_strength
        );
        assert!(
            measurement.metrics.snr_db > 5.0,
            "Expected high SNR for clean sine, got {} dB",
            measurement.metrics.snr_db
        );
    }

    /// The phase spread must see noise that varies rotation to rotation.
    ///
    /// The windows used to be four regardless of buffer length, each covering
    /// many rotations. Per-rotation wobble averages to nothing inside such a
    /// window, so the four agreed with each other and the metric reported a
    /// clean signal. Driving the phase alternately either side of the true
    /// bearing is that case exactly: every window mean is the true bearing,
    /// and no individual rotation is. The spread now feeds the uncertainty
    /// figure, so that is what this asserts on.
    ///
    /// The metric is driven directly rather than through `process_buffer`
    /// because the Doppler bandpass would filter the wobble back out before
    /// coherence ever saw it.
    #[test]
    fn test_uncertainty_sees_rotation_to_rotation_phase_noise() {
        let sample_rate = 48000.0;
        let doppler_config = DopplerConfig {
            expected_freq: 480.0,
            bandpass_low: 400.0,
            bandpass_high: 560.0,
            ..Default::default()
        };
        let samples_per_rotation = sample_rate / doppler_config.expected_freq;
        let omega = 2.0 * PI / samples_per_rotation;
        let north_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase_variance: Some(0.0),
            fractional_sample_offset: 0.0,
            phase: 0.0,
            frequency: omega,
        };

        let uncertainty_with_wobble = |wobble: f32| -> f32 {
            let mut calc = CorrelationBearingCalculator::new(
                &doppler_config,
                &AgcConfig::default(),
                ConfidenceConfig::default(),
                sample_rate,
                1,
            )
            .unwrap();

            let bearing_radians = 45.0f32.to_radians();
            let per_rotation = samples_per_rotation as usize;
            calc.base.work_buffer = (0..4800)
                .map(|i| {
                    let sign = if (i / per_rotation).is_multiple_of(2) {
                        1.0
                    } else {
                        -1.0
                    };
                    (omega * i as f32 - bearing_radians + sign * wobble).sin()
                })
                .collect();

            let (mut i_sum, mut q_sum, mut power_sum) = (0.0f32, 0.0f32, 0.0f32);
            for (idx, &sample) in calc.base.work_buffer.iter().enumerate() {
                let phase = north_tick.phase
                    + calc.base.samples_since_tick(&north_tick, idx as f32) * omega;
                i_sum += sample * phase.cos();
                q_sum += sample * phase.sin();
                power_sum += sample * sample;
            }
            let n = calc.base.work_buffer.len() as f32;
            let (i, q) = (i_sum / n, q_sum / n);

            calc.calculate_metrics(&north_tick, power_sum / n, (i * i + q * q).sqrt())
                .bearing_uncertainty_deg
                .expect("an uncertainty")
        };

        let steady = uncertainty_with_wobble(0.0);
        let wobbling = uncertainty_with_wobble(0.5);

        assert!(
            steady < 1.0,
            "A tone held at one phase should claim little uncertainty, got {steady}"
        );
        // Half a radian either side is 28.6 degrees of per-estimate spread.
        // The figure reports the uncertainty of their average, so it is that
        // over the root of the independent count: 4800 samples of buffer over
        // 127 taps of filter is 37.8 independent looks, and 28.6 over the root
        // of 37.8 is 4.7.
        assert!(
            (3.0..7.0).contains(&wobbling),
            "Half a radian of per-rotation phase noise, averaged over about \
             38 independent looks, should land near 4.7 degrees: got \
             {wobbling} against {steady} for the same tone held steady"
        );
    }
}
