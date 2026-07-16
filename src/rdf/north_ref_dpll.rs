use crate::config::{LockQualityWeights, NorthTickConfig};
use crate::constants::FREQUENCY_EPSILON;
use crate::error::{RdfError, Result};
use crate::rdf::NorthTick;
use crate::signal_processing::{FirHighpass, PeakDetector};
use std::collections::VecDeque;
use std::f32::consts::PI;

use super::north_ref_common::{
    derive_delay_compensation, derive_peak_timing, preprocess_north_buffer,
};

const MIN_TICK_SPACING_FRACTION: f32 = 0.75;
const DEAD_TIME_FRACTION_OF_MAX_PERIOD: f32 = 0.8;
const MAX_PHASE_TIMING_CORRECTION_SAMPLES: f32 = 0.1;
const MAX_TOTAL_FRACTIONAL_OFFSET_SAMPLES: f32 = 0.5;
const MIN_PHASE_CORRECTION_SAMPLES: usize = 16;
const MAX_PHASE_STD_FOR_CORRECTION_RAD: f32 = 0.25;
const LOCK_STATS_WINDOW_TICKS: usize = 128;

struct RollingWindowStats {
    window: VecDeque<f32>,
    max_len: usize,
    sum: f64,
    sum_sq: f64,
}

impl RollingWindowStats {
    fn new(max_len: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(max_len),
            max_len,
            sum: 0.0,
            sum_sq: 0.0,
        }
    }

    fn update(&mut self, value: f32) {
        if self.window.len() == self.max_len
            && let Some(old) = self.window.pop_front()
        {
            let old = old as f64;
            self.sum -= old;
            self.sum_sq -= old * old;
        }

        self.window.push_back(value);
        let v = value as f64;
        self.sum += v;
        self.sum_sq += v * v;
    }

    fn count(&self) -> usize {
        self.window.len()
    }

    fn mean(&self) -> Option<f32> {
        let n = self.window.len();
        if n == 0 {
            None
        } else {
            Some((self.sum / n as f64) as f32)
        }
    }

    fn variance(&self) -> Option<f32> {
        let n = self.window.len();
        if n < 2 {
            return None;
        }
        let n_f64 = n as f64;
        let mean = self.sum / n_f64;
        let var = (self.sum_sq / n_f64) - mean * mean;
        Some(var.max(0.0) as f32)
    }

    fn std_dev(&self) -> Option<f32> {
        self.variance().map(f32::sqrt)
    }
}

pub struct DpllNorthTracker {
    gain: f32,
    highpass: FirHighpass,
    peak_detector: PeakDetector,
    pulse_peak_offset: f32,
    last_tick_sample: Option<usize>,

    // PLL state
    phase: f32,     // Current phase estimate (radians, 0-2π)
    frequency: f32, // Frequency estimate (radians/sample)

    // PLL parameters
    kp: f32, // Proportional gain
    ki: f32, // Integral gain

    // Frequency limits (radians/sample)
    min_omega: f32,
    max_omega: f32,

    sample_counter: usize,
    sample_rate: f32,

    // Rolling statistics for lock quality
    phase_error_stats: RollingWindowStats,
    freq_stats: RollingWindowStats,
    lock_quality_weights: LockQualityWeights,

    // Pre-allocated buffer for filtering
    filter_buffer: Vec<f32>,
}

impl DpllNorthTracker {
    #[inline]
    fn wrap_phase(phase: f32) -> f32 {
        phase.rem_euclid(2.0 * PI)
    }

    #[inline]
    fn wrap_phase_error(phase_error: f32) -> f32 {
        (phase_error + PI).rem_euclid(2.0 * PI) - PI
    }

    #[inline]
    fn stable_enough_for_phase_correction(&self) -> bool {
        if self.phase_error_stats.count() < MIN_PHASE_CORRECTION_SAMPLES {
            return false;
        }
        self.phase_error_stats
            .std_dev()
            .map(|s| s.is_finite() && s <= MAX_PHASE_STD_FOR_CORRECTION_RAD)
            .unwrap_or(false)
    }

    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        if !sample_rate.is_finite() || sample_rate <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick sample_rate must be finite and > {}, got {}",
                FREQUENCY_EPSILON, sample_rate
            )));
        }

        let initial_freq = config.dpll.initial_frequency_hz;
        if !initial_freq.is_finite() || initial_freq <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.initial_frequency_hz must be finite and > {}, got {}",
                FREQUENCY_EPSILON, initial_freq
            )));
        }

        let natural_frequency_hz = config.dpll.natural_frequency_hz;
        if !natural_frequency_hz.is_finite() || natural_frequency_hz <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.natural_frequency_hz must be finite and > {}, got {}",
                FREQUENCY_EPSILON, natural_frequency_hz
            )));
        }

        let damping_ratio = config.dpll.damping_ratio;
        if !damping_ratio.is_finite() || damping_ratio < 0.0 {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.damping_ratio must be finite and >= 0, got {}",
                damping_ratio
            )));
        }

        let frequency_min_hz = config.dpll.frequency_min_hz;
        let frequency_max_hz = config.dpll.frequency_max_hz;
        if !frequency_min_hz.is_finite() || frequency_min_hz <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.frequency_min_hz must be finite and > {}, got {}",
                FREQUENCY_EPSILON, frequency_min_hz
            )));
        }
        if !frequency_max_hz.is_finite() || frequency_max_hz <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.frequency_max_hz must be finite and > {}, got {}",
                FREQUENCY_EPSILON, frequency_max_hz
            )));
        }
        if frequency_min_hz >= frequency_max_hz {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.frequency_min_hz ({}) must be < north_tick.dpll.frequency_max_hz ({})",
                frequency_min_hz, frequency_max_hz
            )));
        }

        // Derive the detector dead time from the configured tracking band so
        // it can never reject valid ticks at frequency_max_hz (a fixed
        // min_interval_ms of 0.6 ms capped detection at ~1714 Hz while the
        // band advertised 1800 Hz).
        let min_samples =
            (DEAD_TIME_FRACTION_OF_MAX_PERIOD * sample_rate / frequency_max_hz) as usize;
        let gain = 10.0_f32.powf(config.gain_db / 20.0);

        // Initial frequency estimate from config
        let omega = 2.0 * PI * initial_freq / sample_rate;

        // PLL gains — the loop updates once per detected tick, not once per
        // sample. Normalize the natural frequency to the tick rate and scale
        // the integral gain by the expected update interval in samples.
        let tick_rate = initial_freq;
        let samples_per_tick = sample_rate / tick_rate;
        let wn = 2.0 * PI * config.dpll.natural_frequency_hz / tick_rate;
        let zeta = config.dpll.damping_ratio;
        let kp = 2.0 * zeta * wn;
        let ki = wn * wn / samples_per_tick;

        // Calculate frequency limits in radians/sample
        let min_omega = 2.0 * PI * config.dpll.frequency_min_hz / sample_rate;
        let max_omega = 2.0 * PI * config.dpll.frequency_max_hz / sample_rate;

        let highpass = FirHighpass::new(
            config.highpass_cutoff,
            sample_rate,
            config.fir_highpass_taps,
            config.highpass_transition_hz,
        )?;

        let effective_pulse_amplitude = (config.expected_pulse_amplitude * gain).max(f32::EPSILON);
        let peak_timing =
            derive_peak_timing(&highpass, config.threshold, effective_pulse_amplitude);

        Ok(Self {
            gain,
            highpass,
            peak_detector: PeakDetector::with_peak_search_window(
                config.threshold,
                min_samples,
                peak_timing.peak_search_window_samples,
            ),
            pulse_peak_offset: peak_timing.pulse_peak_offset,
            last_tick_sample: None,
            phase: 0.0,
            frequency: omega,
            kp,
            ki,
            min_omega,
            max_omega,
            sample_counter: 0,
            sample_rate,
            phase_error_stats: RollingWindowStats::new(LOCK_STATS_WINDOW_TICKS),
            freq_stats: RollingWindowStats::new(LOCK_STATS_WINDOW_TICKS),
            lock_quality_weights: config.lock_quality_weights,
            filter_buffer: Vec::new(),
        })
    }

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        preprocess_north_buffer(
            &mut self.filter_buffer,
            buffer,
            self.gain,
            &mut self.highpass,
        );

        let peaks = self.peak_detector.find_all_peaks(&self.filter_buffer);

        let delay = derive_delay_compensation(&self.highpass, self.pulse_peak_offset);

        let mut ticks = Vec::with_capacity(peaks.len());

        let mut last_sample_idx = 0;
        for &(peak_idx, _amplitude) in &peaks {
            if peak_idx < last_sample_idx {
                continue;
            }
            // Advance PLL phase from last_sample_idx to peak_idx
            let samples_to_advance = peak_idx - last_sample_idx;
            self.phase += self.frequency * samples_to_advance as f32;
            self.phase = Self::wrap_phase(self.phase);

            let global_sample = self.sample_counter.saturating_add(peak_idx);
            let compensated_sample = global_sample.saturating_sub(delay.delay_samples);
            let period_estimate = 2.0 * PI / self.frequency;
            if let Some(last) = self.last_tick_sample {
                let min_spacing = period_estimate * MIN_TICK_SPACING_FRACTION;
                let delta = compensated_sample.saturating_sub(last) as f32;
                if delta < min_spacing {
                    last_sample_idx = peak_idx;
                    continue;
                }
            }

            // Phase error: how far are we from expected zero phase?
            // When we detect a tick, we expect phase to be near 0
            let phase_error = Self::wrap_phase_error(-self.phase);

            // Track phase error for variance calculation
            self.phase_error_stats.update(phase_error);

            // Convert phase error to a bounded fractional timing correction,
            // but only once lock statistics indicate stable tracking.
            // phase_error = -phase, so positive NCO phase at the peak means the
            // oscillator's zero crossing occurred phase/frequency samples earlier;
            // the correction must shift the tick earlier (negative), i.e.
            // phase_error/frequency = -phase/frequency.
            let phase_timing_correction = if self.stable_enough_for_phase_correction()
                && self.frequency > FREQUENCY_EPSILON
            {
                (phase_error / self.frequency).clamp(
                    -MAX_PHASE_TIMING_CORRECTION_SAMPLES,
                    MAX_PHASE_TIMING_CORRECTION_SAMPLES,
                )
            } else {
                0.0
            };

            let fractional_sample_offset =
                (delay.fractional_sample_offset + phase_timing_correction).clamp(
                    -MAX_TOTAL_FRACTIONAL_OFFSET_SAMPLES,
                    MAX_TOTAL_FRACTIONAL_OFFSET_SAMPLES,
                );

            // Update frequency and phase with PI controller
            self.frequency += self.ki * phase_error;
            self.phase += self.kp * phase_error;

            // Clamp frequency to configured range
            self.frequency = self.frequency.clamp(self.min_omega, self.max_omega);

            // Track frequency for stability calculation
            self.freq_stats.update(self.frequency);

            // Wrap phase after correction
            self.phase = Self::wrap_phase(self.phase);

            // Calculate period in samples from current frequency estimate
            let period = 2.0 * PI / self.frequency;

            // Compensate for filter delay: the filtered output at this sample
            // corresponds to an input pulse that occurred earlier by the
            // configured delay compensation.
            // For bearing calculation, the tick itself defines north reference (phase = 0).
            // Jitter is represented by sample_index timing; using absolute DPLL oscillator
            // phase here would introduce reference drift across rotations.
            self.last_tick_sample = Some(compensated_sample);
            ticks.push(NorthTick {
                sample_index: compensated_sample,
                period: Some(period),
                lock_quality: self.lock_quality(),
                fractional_sample_offset,
                phase: 0.0,
                frequency: self.frequency,
            });

            last_sample_idx = peak_idx;
        }

        // Advance phase for remaining samples after the last peak
        if last_sample_idx < buffer.len() {
            let remaining = buffer.len() - last_sample_idx;
            self.phase += self.frequency * remaining as f32;
            self.phase = Self::wrap_phase(self.phase);
        }

        self.sample_counter += buffer.len();
        ticks
    }

    pub fn rotation_frequency(&self) -> Option<f32> {
        if self.frequency > 0.0 {
            Some(self.frequency * self.sample_rate / (2.0 * PI))
        } else {
            None
        }
    }

    pub fn phase_error_variance(&self) -> Option<f32> {
        self.phase_error_stats.variance()
    }

    pub fn lock_quality(&self) -> Option<f32> {
        if self.phase_error_stats.count() < 2 || self.freq_stats.count() < 2 {
            return None;
        }

        // Phase error std dev in radians - lower is better
        // A well-locked PLL should have phase error < 0.1 rad (~6 degrees)
        let phase_std = self.phase_error_stats.std_dev()?.abs();
        let phase_score = (1.0 - phase_std / PI).clamp(0.0, 1.0);

        // Frequency stability - lower variance relative to mean is better
        let freq_mean = self.freq_stats.mean()?;
        let freq_std = self.freq_stats.std_dev()?;
        let freq_cv = if freq_mean.abs() > FREQUENCY_EPSILON {
            (freq_std / freq_mean).abs()
        } else {
            1.0
        };
        let freq_score = (1.0 - freq_cv * 100.0).clamp(0.0, 1.0);

        // Combined score using configured weights
        Some(
            self.lock_quality_weights.phase_weight * phase_score
                + self.lock_quality_weights.frequency_weight * freq_score,
        )
    }

    pub fn filtered_buffer(&self) -> &[f32] {
        &self.filter_buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DpllConfig, NorthTickConfig};

    #[test]
    fn test_dpll_north_tick_detection() {
        let config = NorthTickConfig::default();
        let sample_rate = 48000.0;
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        // Generate signal with pulses at 1602 Hz (every 30 samples approx)
        let samples_per_pulse = (sample_rate / 1602.0) as usize;
        let mut ticks_detected = 0;

        for _ in 0..40 {
            let mut signal = vec![0.0; samples_per_pulse];
            signal[5] = 0.8; // Pulse near start

            let ticks = tracker.process_buffer(&signal);
            if !ticks.is_empty() {
                ticks_detected += ticks.len();
            }
        }

        // May detect fewer initially due to FIR transient
        assert!(
            ticks_detected >= 30,
            "Should detect most ticks with FIR filter"
        );

        if let Some(freq) = tracker.rotation_frequency() {
            assert!(
                (freq - 1602.0).abs() < 50.0,
                "Rotation frequency {} should be close to 1602 Hz",
                freq
            );
        }
    }

    #[test]
    fn test_dpll_north_tick_delay_compensation_with_gain() {
        let sample_rate = 48000.0;
        let config = NorthTickConfig {
            gain_db: 20.0,
            dpll: DpllConfig {
                initial_frequency_hz: 480.0,
                natural_frequency_hz: 10.0,
                damping_ratio: 0.707,
                frequency_min_hz: 300.0,
                frequency_max_hz: 800.0,
            },
            ..Default::default()
        };
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        let pulse_positions = [100, 200, 300, 400, 500];
        let mut signal = vec![0.0f32; 1000];
        for &pos in &pulse_positions {
            signal[pos] = config.expected_pulse_amplitude;
        }

        let ticks = tracker.process_buffer(&signal);
        assert!(
            ticks.len() == pulse_positions.len(),
            "Expected {} ticks, got {}",
            pulse_positions.len(),
            ticks.len()
        );

        for tick in &ticks {
            let closest_pulse = pulse_positions
                .iter()
                .min_by_key(|&&p| (p as isize - tick.sample_index as isize).abs())
                .unwrap();
            let error = (*closest_pulse as isize - tick.sample_index as isize).abs();
            assert!(
                error <= 2,
                "Tick sample_index {} too far from expected pulse {}",
                tick.sample_index,
                closest_pulse
            );
        }
    }

    #[test]
    fn test_dpll_locks_to_true_frequency() {
        // Regression test for an off-by-one in phase advancement that made
        // the loop lock to sample_rate/(period-1) instead of sample_rate/period
        // (484.85 Hz instead of 480 Hz for a 100-sample period at 48 kHz).
        let sample_rate = 48_000.0;
        let config = NorthTickConfig {
            dpll: DpllConfig {
                initial_frequency_hz: 480.0,
                natural_frequency_hz: 10.0,
                damping_ratio: 0.707,
                frequency_min_hz: 300.0,
                frequency_max_hz: 800.0,
            },
            ..Default::default()
        };
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        // Pulses at exactly 480 Hz (period 100 samples), split across buffers
        // so cross-buffer phase accounting is exercised too.
        let period = 100;
        let buffer_len = 1024;
        let total_samples = 50_000;
        let mut signal = vec![0.0f32; total_samples];
        for idx in (50..total_samples).step_by(period) {
            signal[idx] = config.expected_pulse_amplitude;
        }
        for buffer in signal.chunks(buffer_len) {
            tracker.process_buffer(buffer);
        }

        let freq = tracker
            .rotation_frequency()
            .expect("tracker should be tracking a frequency");
        assert!(
            (freq - 480.0).abs() < 0.5,
            "DPLL locked to {} Hz, expected 480 Hz",
            freq
        );
    }

    #[test]
    fn test_dpll_detects_ticks_at_frequency_max() {
        // Regression test: the peak-detector dead time used to be a fixed
        // 0.6 ms (28 samples @ 48 kHz), which rejected every other tick above
        // ~1714 Hz even though the configured band extends to 1800 Hz.
        let sample_rate = 48_000.0;
        let config = NorthTickConfig::default();
        let freq_max = config.dpll.frequency_max_hz;
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        let period = sample_rate as f64 / freq_max as f64;
        let total_samples = 48_000usize;
        let n_pulses = ((total_samples as f64 - 100.0) / period) as usize;
        let mut signal = vec![0.0f32; total_samples];
        for k in 0..n_pulses {
            signal[(50.0 + k as f64 * period).round() as usize] = config.expected_pulse_amplitude;
        }

        let mut ticks = 0;
        for buffer in signal.chunks(1024) {
            ticks += tracker.process_buffer(buffer).len();
        }
        assert!(
            ticks >= n_pulses * 9 / 10,
            "detected {} of {} ticks at frequency_max {} Hz",
            ticks,
            n_pulses,
            freq_max
        );
    }

    #[test]
    fn test_dpll_phase_correction_reduces_timing_error() {
        // Regression test for a sign inversion in the fractional timing
        // correction. Pulses at fractional period 30.4 samples (~1578.9 Hz
        // @ 48 kHz) land on quantized integer samples; the phase correction
        // should recover sub-sample timing. Measured steady-state RMS error:
        // correct sign 0.20 samples, correction disabled 0.28, inverted
        // sign 0.37 — the 0.25 bound fails both regressions.
        let sample_rate = 48_000.0;
        let config = NorthTickConfig {
            dpll: DpllConfig {
                initial_frequency_hz: 1_578.9,
                natural_frequency_hz: 15.0,
                damping_ratio: 0.707,
                frequency_min_hz: 1_400.0,
                frequency_max_hz: 1_800.0,
            },
            ..Default::default()
        };
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        let period = 30.4f64;
        let total_samples = 120_000usize;
        let n_pulses = ((total_samples as f64 - 100.0) / period) as usize;
        let true_times: Vec<f64> = (0..n_pulses).map(|k| 50.0 + k as f64 * period).collect();
        let mut signal = vec![0.0f32; total_samples];
        for t in &true_times {
            signal[t.round() as usize] = config.expected_pulse_amplitude;
        }

        let mut ticks = Vec::new();
        for buffer in signal.chunks(1024) {
            ticks.extend(tracker.process_buffer(buffer));
        }
        assert!(ticks.len() > 1000, "got {} ticks", ticks.len());

        // Steady state: last half of the run.
        let steady = &ticks[ticks.len() / 2..];
        let errors: Vec<f64> = steady
            .iter()
            .map(|t| {
                let measured = t.sample_index as f64 + t.fractional_sample_offset as f64;
                true_times
                    .iter()
                    .map(|&tt| measured - tt)
                    .min_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap())
                    .unwrap()
            })
            .collect();
        let mean = errors.iter().sum::<f64>() / errors.len() as f64;
        let rms_about_mean =
            (errors.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / errors.len() as f64).sqrt();
        let n_corrected = steady
            .iter()
            .filter(|t| t.fractional_sample_offset.abs() > 1e-6)
            .count();
        assert!(
            n_corrected > steady.len() / 2,
            "phase correction should be active in steady state ({} of {} ticks corrected)",
            n_corrected,
            steady.len()
        );
        assert!(
            rms_about_mean < 0.25,
            "steady-state RMS timing error {:.4} samples exceeds 0.25",
            rms_about_mean
        );
    }

    #[test]
    fn test_dpll_fractional_timing_correction_is_bounded() {
        let sample_rate = 48_000.0;
        let config = NorthTickConfig {
            dpll: DpllConfig {
                initial_frequency_hz: 1_602.0,
                natural_frequency_hz: 15.0,
                damping_ratio: 0.707,
                frequency_min_hz: 1_400.0,
                frequency_max_hz: 1_800.0,
            },
            ..Default::default()
        };
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        let nominal_period = (sample_rate / config.dpll.initial_frequency_hz).round() as isize;
        let mut signal = vec![0.0f32; 4096];
        for k in 0..110isize {
            let jitter = match k % 4 {
                0 => -1,
                1 => 0,
                2 => 1,
                _ => 0,
            };
            let idx = 60 + k * nominal_period + jitter;
            if idx >= 0 && (idx as usize) < signal.len() {
                signal[idx as usize] = config.expected_pulse_amplitude;
            }
        }

        let ticks = tracker.process_buffer(&signal);
        assert!(!ticks.is_empty(), "Expected at least one detected tick");

        for tick in ticks {
            assert!(
                tick.fractional_sample_offset.is_finite(),
                "fractional_sample_offset must be finite"
            );
            assert!(
                tick.fractional_sample_offset.abs() <= MAX_TOTAL_FRACTIONAL_OFFSET_SAMPLES + 1e-6,
                "fractional_sample_offset {} exceeds bound {}",
                tick.fractional_sample_offset,
                MAX_TOTAL_FRACTIONAL_OFFSET_SAMPLES
            );
        }
    }

    #[test]
    fn test_dpll_rejects_non_positive_initial_frequency() {
        let sample_rate = 48_000.0;
        let mut config = NorthTickConfig::default();
        config.dpll.initial_frequency_hz = 0.0;

        match DpllNorthTracker::new(&config, sample_rate) {
            Err(RdfError::Config(msg)) => {
                assert!(
                    msg.contains("initial_frequency_hz"),
                    "Unexpected message: {msg}"
                );
            }
            Err(err) => panic!("Expected configuration error, got {err}"),
            Ok(_) => panic!("Expected configuration error, got Ok"),
        }
    }

    #[test]
    fn test_dpll_rejects_invalid_frequency_bounds() {
        let sample_rate = 48_000.0;
        let mut config = NorthTickConfig::default();
        config.dpll.frequency_min_hz = 1800.0;
        config.dpll.frequency_max_hz = 1400.0;

        match DpllNorthTracker::new(&config, sample_rate) {
            Err(RdfError::Config(msg)) => {
                assert!(
                    msg.contains("frequency_min_hz") && msg.contains("frequency_max_hz"),
                    "Unexpected message: {msg}"
                );
            }
            Err(err) => panic!("Expected configuration error, got {err}"),
            Ok(_) => panic!("Expected configuration error, got Ok"),
        }
    }
}
