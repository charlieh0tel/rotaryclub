use crate::config::{LockQualityWeights, NorthTickConfig};
use crate::constants::FREQUENCY_EPSILON;
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{FirHighpass, PeakDetector};
use rolling_stats::Stats;
use std::f32::consts::PI;

pub struct DpllNorthTracker {
    highpass: FirHighpass,
    peak_detector: PeakDetector,
    threshold_crossing_offset: f32,

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

    // Statistics for lock quality
    phase_error_stats: Stats<f32>,
    freq_stats: Stats<f32>,
    lock_quality_weights: LockQualityWeights,

    // Pre-allocated buffer for filtering
    filter_buffer: Vec<f32>,
}

impl DpllNorthTracker {
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        let min_samples = (config.min_interval_ms / 1000.0 * sample_rate) as usize;

        // Initial frequency estimate from config
        let initial_freq = config.dpll.initial_frequency_hz;
        let omega = 2.0 * PI * initial_freq / sample_rate;

        // PLL gains - calculated from natural frequency and damping ratio
        let wn = 2.0 * PI * config.dpll.natural_frequency_hz / sample_rate;
        let zeta = config.dpll.damping_ratio;
        let kp = 2.0 * zeta * wn;
        let ki = wn * wn;

        // Calculate frequency limits in radians/sample
        let min_omega = 2.0 * PI * config.dpll.frequency_min_hz / sample_rate;
        let max_omega = 2.0 * PI * config.dpll.frequency_max_hz / sample_rate;

        let highpass = FirHighpass::new(
            config.highpass_cutoff,
            sample_rate,
            config.fir_highpass_taps,
            config.highpass_transition_hz,
        )?;

        let threshold_crossing_offset =
            highpass.threshold_crossing_offset(config.threshold, config.expected_pulse_amplitude);

        Ok(Self {
            highpass,
            peak_detector: PeakDetector::new(config.threshold, min_samples),
            threshold_crossing_offset,
            phase: 0.0,
            frequency: omega,
            kp,
            ki,
            min_omega,
            max_omega,
            sample_counter: 0,
            sample_rate,
            phase_error_stats: Stats::new(),
            freq_stats: Stats::new(),
            lock_quality_weights: config.lock_quality_weights,
            filter_buffer: Vec::new(),
        })
    }

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        self.filter_buffer.resize(buffer.len(), 0.0);
        self.filter_buffer.copy_from_slice(buffer);
        self.highpass.process_buffer(&mut self.filter_buffer);

        let peaks = self.peak_detector.find_all_peaks(&self.filter_buffer);

        // Total delay compensation: group_delay + threshold_crossing_offset
        let group_delay = self.highpass.group_delay_samples() as f32;
        let total_delay = (group_delay + self.threshold_crossing_offset).round() as usize;

        let mut ticks = Vec::with_capacity(peaks.len());
        let two_pi = 2.0 * PI;

        let mut last_sample_idx = 0;
        for &peak_idx in &peaks {
            // Advance PLL phase from last_sample_idx to peak_idx
            let samples_to_advance = peak_idx - last_sample_idx;
            self.phase += self.frequency * samples_to_advance as f32;
            // Wrap phase efficiently
            if self.phase >= two_pi {
                self.phase -= (self.phase / two_pi).floor() * two_pi;
            }

            let global_sample = self.sample_counter + peak_idx;

            // Phase error: how far are we from expected zero phase?
            // When we detect a tick, we expect phase to be near 0
            let mut phase_error = -self.phase;

            // Wrap phase error to [-π, π]
            if phase_error > PI {
                phase_error -= two_pi;
            } else if phase_error < -PI {
                phase_error += two_pi;
            }

            // Track phase error for variance calculation
            self.phase_error_stats.update(phase_error);

            // Update frequency and phase with PI controller
            self.frequency += self.ki * phase_error;
            self.phase += self.kp * phase_error;

            // Clamp frequency to configured range
            self.frequency = self.frequency.clamp(self.min_omega, self.max_omega);

            // Track frequency for stability calculation
            self.freq_stats.update(self.frequency);

            // Wrap phase after correction
            if self.phase >= two_pi {
                self.phase -= two_pi;
            } else if self.phase < 0.0 {
                self.phase += two_pi;
            }

            // Calculate period in samples from current frequency estimate
            let period = two_pi / self.frequency;

            // Compensate for filter delay: the filtered output at this sample
            // corresponds to an input pulse that occurred total_delay samples earlier.
            // For bearing calculation, tick defines north (phase=0).
            // The DPLL's internal phase tracks jitter, but the reference is the tick itself.
            ticks.push(NorthTick {
                sample_index: global_sample.saturating_sub(total_delay),
                period: Some(period),
                lock_quality: self.lock_quality(),
                phase: 0.0,
                frequency: self.frequency,
            });

            last_sample_idx = peak_idx + 1;
        }

        // Advance phase for remaining samples after the last peak
        if last_sample_idx < buffer.len() {
            let remaining = buffer.len() - last_sample_idx;
            self.phase += self.frequency * remaining as f32;
            if self.phase >= two_pi {
                self.phase -= (self.phase / two_pi).floor() * two_pi;
            }
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
        if self.phase_error_stats.count < 2 {
            None
        } else {
            let std_dev = self.phase_error_stats.std_dev;
            Some(std_dev * std_dev)
        }
    }

    pub fn lock_quality(&self) -> Option<f32> {
        if self.phase_error_stats.count < 2 || self.freq_stats.count < 2 {
            return None;
        }

        // Phase error std dev in radians - lower is better
        // A well-locked PLL should have phase error < 0.1 rad (~6 degrees)
        let phase_std = self.phase_error_stats.std_dev.abs();
        let phase_score = (1.0 - phase_std / PI).clamp(0.0, 1.0);

        // Frequency stability - lower variance relative to mean is better
        let freq_cv = if self.freq_stats.mean.abs() > FREQUENCY_EPSILON {
            (self.freq_stats.std_dev / self.freq_stats.mean).abs()
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
    use crate::config::NorthTickConfig;

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
}
