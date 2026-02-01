use crate::config::NorthTickConfig;
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{HighpassFilter, PeakDetector};
use std::f32::consts::PI;

pub struct DpllNorthTracker {
    highpass: HighpassFilter,
    peak_detector: PeakDetector,

    // PLL state
    phase: f32,     // Current phase estimate (radians, 0-2π)
    frequency: f32, // Frequency estimate (radians/sample)

    // PLL parameters
    kp: f32, // Proportional gain
    ki: f32, // Integral gain

    sample_counter: usize,
    sample_rate: f32,
}

impl DpllNorthTracker {
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        let min_samples = (config.min_interval_ms / 1000.0 * sample_rate) as usize;

        // Initial frequency estimate: assume 1602 Hz rotation
        let initial_freq = 1602.0;
        let omega = 2.0 * PI * initial_freq / sample_rate;

        // PLL gains - tune these for tracking performance
        // Natural frequency around 10 Hz, damping ratio 0.707
        let wn = 2.0 * PI * 10.0 / sample_rate; // Natural frequency
        let zeta = 0.707; // Damping ratio
        let kp = 2.0 * zeta * wn;
        let ki = wn * wn;

        Ok(Self {
            highpass: HighpassFilter::new(
                config.highpass_cutoff,
                sample_rate,
                config.filter_order,
            )?,
            peak_detector: PeakDetector::new(config.threshold, min_samples),
            phase: 0.0,
            frequency: omega,
            kp,
            ki,
            sample_counter: 0,
            sample_rate,
        })
    }

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        let mut filtered = buffer.to_vec();
        self.highpass.process_buffer(&mut filtered);

        let peaks = self.peak_detector.find_all_peaks(&filtered);

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

                // Clamp frequency to reasonable range (1400-1800 Hz)
                let min_omega = 2.0 * PI * 1400.0 / self.sample_rate;
                let max_omega = 2.0 * PI * 1800.0 / self.sample_rate;
                self.frequency = self.frequency.clamp(min_omega, max_omega);

                // Wrap phase after correction
                if self.phase >= 2.0 * PI {
                    self.phase -= 2.0 * PI;
                } else if self.phase < 0.0 {
                    self.phase += 2.0 * PI;
                }

                // Calculate period in samples from current frequency estimate
                let period = 2.0 * PI / self.frequency;

                ticks.push(NorthTick {
                    sample_index: global_sample,
                    period: Some(period),
                });
            }
        }

        self.sample_counter += buffer.len();
        ticks
    }

    #[allow(dead_code)]
    pub fn rotation_period(&self) -> Option<f32> {
        if self.frequency > 0.0 {
            Some(2.0 * PI / self.frequency)
        } else {
            None
        }
    }

    pub fn rotation_frequency(&self) -> Option<f32> {
        if self.frequency > 0.0 {
            Some(self.frequency * self.sample_rate / (2.0 * PI))
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.phase = 0.0;
        let initial_freq = 1602.0;
        self.frequency = 2.0 * PI * initial_freq / self.sample_rate;
        self.sample_counter = 0;
        self.peak_detector.reset();
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

        for _ in 0..20 {
            let mut signal = vec![0.0; samples_per_pulse];
            signal[5] = 0.8; // Pulse near start

            let ticks = tracker.process_buffer(&signal);
            if !ticks.is_empty() {
                ticks_detected += ticks.len();
            }
        }

        assert!(ticks_detected >= 15, "Should detect most ticks");

        if let Some(freq) = tracker.rotation_frequency() {
            assert!(
                (freq - 1602.0).abs() < 50.0,
                "Rotation frequency {} should be close to 1602 Hz",
                freq
            );
        }
    }
}
