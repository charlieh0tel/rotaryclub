use crate::config::NorthTickConfig;
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{FirHighpass, PeakDetector};
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
        })
    }

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        let mut filtered = buffer.to_vec();
        self.highpass.process_buffer(&mut filtered);

        let peaks = self.peak_detector.find_all_peaks(&filtered);

        // Total delay compensation: group_delay + threshold_crossing_offset
        let group_delay = self.highpass.group_delay_samples() as f32;
        let total_delay = (group_delay + self.threshold_crossing_offset).round() as usize;

        let mut ticks = Vec::new();

        for (i, &_sample) in buffer.iter().enumerate() {
            let global_sample = self.sample_counter + i;

            // Update PLL phase
            self.phase += self.frequency;
            if self.phase >= 2.0 * PI {
                self.phase -= 2.0 * PI;
            }

            // Check if we detected a peak at this sample
            if peaks.contains(&i) {
                // Phase error: how far are we from expected zero phase?
                // When we detect a tick, we expect phase to be near 0
                let mut phase_error = -self.phase;

                // Wrap phase error to [-π, π]
                while phase_error > PI {
                    phase_error -= 2.0 * PI;
                }
                while phase_error < -PI {
                    phase_error += 2.0 * PI;
                }

                // Update frequency and phase with PI controller
                self.frequency += self.ki * phase_error;
                self.phase += self.kp * phase_error;

                // Clamp frequency to configured range
                self.frequency = self.frequency.clamp(self.min_omega, self.max_omega);

                // Wrap phase after correction
                if self.phase >= 2.0 * PI {
                    self.phase -= 2.0 * PI;
                } else if self.phase < 0.0 {
                    self.phase += 2.0 * PI;
                }

                // Calculate period in samples from current frequency estimate
                let period = 2.0 * PI / self.frequency;

                // Compensate for filter delay: the filtered output at this sample
                // corresponds to an input pulse that occurred total_delay samples earlier.
                ticks.push(NorthTick {
                    sample_index: global_sample.saturating_sub(total_delay),
                    period: Some(period),
                });
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
