use crate::config::NorthTickConfig;
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{FirHighpass, PeakDetector};
use std::f32::consts::PI;

const PERIOD_SMOOTHING_FACTOR: f32 = 0.1;

pub struct SimpleNorthTracker {
    gain: f32,
    highpass: FirHighpass,
    peak_detector: PeakDetector,
    threshold_crossing_offset: f32,
    last_tick_sample: Option<usize>,
    samples_per_rotation: Option<f32>,
    sample_counter: usize,
    sample_rate: f32,
    filter_buffer: Vec<f32>,
}

impl SimpleNorthTracker {
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        let min_samples = (config.min_interval_ms / 1000.0 * sample_rate) as usize;

        let highpass = FirHighpass::new(
            config.highpass_cutoff,
            sample_rate,
            config.fir_highpass_taps,
            config.highpass_transition_hz,
        )?;

        let threshold_crossing_offset =
            highpass.threshold_crossing_offset(config.threshold, config.expected_pulse_amplitude);

        Ok(Self {
            gain: 10.0_f32.powf(config.gain_db / 20.0),
            highpass,
            peak_detector: PeakDetector::new(config.threshold, min_samples),
            threshold_crossing_offset,
            last_tick_sample: None,
            samples_per_rotation: None,
            sample_counter: 0,
            sample_rate,
            filter_buffer: Vec::new(),
        })
    }

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        self.filter_buffer.resize(buffer.len(), 0.0);
        self.filter_buffer.copy_from_slice(buffer);
        if self.gain != 1.0 {
            for s in self.filter_buffer.iter_mut() {
                *s *= self.gain;
            }
        }
        self.highpass.process_buffer(&mut self.filter_buffer);

        let peaks = self.peak_detector.find_all_peaks(&self.filter_buffer);

        // Total delay compensation: group_delay + threshold_crossing_offset
        let group_delay = self.highpass.group_delay_samples() as f32;
        let total_delay = (group_delay + self.threshold_crossing_offset).round() as usize;

        let mut ticks = Vec::new();

        for (peak_idx, _amplitude) in peaks {
            // Compensate for FIR filter delay: the filtered output at peak_idx
            // corresponds to an input pulse that occurred total_delay samples earlier.
            let global_sample = self
                .sample_counter
                .saturating_add(peak_idx)
                .saturating_sub(total_delay);

            // Update rotation period estimate with exponential averaging
            if let Some(last) = self.last_tick_sample {
                let period = (global_sample - last) as f32;

                self.samples_per_rotation = Some(
                    self.samples_per_rotation
                        .map(|prev| {
                            (1.0 - PERIOD_SMOOTHING_FACTOR) * prev
                                + PERIOD_SMOOTHING_FACTOR * period
                        })
                        .unwrap_or(period),
                );
            }

            // Calculate frequency from period estimate
            let frequency = self
                .samples_per_rotation
                .map(|p| 2.0 * PI / p)
                .unwrap_or(0.0);

            ticks.push(NorthTick {
                sample_index: global_sample,
                period: self.samples_per_rotation,
                lock_quality: self.lock_quality(),
                phase: 0.0, // By definition, tick = north = 0 radians
                frequency,
            });

            self.last_tick_sample = Some(global_sample);
        }

        self.sample_counter += buffer.len();
        ticks
    }

    pub fn rotation_frequency(&self) -> Option<f32> {
        self.samples_per_rotation
            .map(|period| self.sample_rate / period)
    }

    pub fn lock_quality(&self) -> Option<f32> {
        None
    }

    pub fn phase_error_variance(&self) -> Option<f32> {
        None
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
    fn test_simple_north_tick_detection() {
        let config = NorthTickConfig::default();
        let sample_rate = 48000.0;
        let mut tracker = SimpleNorthTracker::new(&config, sample_rate).unwrap();

        // Generate signal with pulses - need longer buffer for FIR transient
        let mut signal = vec![0.0; 1000];
        signal[100] = 0.8;
        signal[196] = 0.8;
        signal[292] = 0.8;
        signal[388] = 0.8;

        let ticks = tracker.process_buffer(&signal);

        assert!(ticks.len() >= 2, "Should detect at least 2 ticks");

        if let Some(freq) = tracker.rotation_frequency() {
            assert!(
                (freq - 500.0).abs() < 50.0,
                "Rotation frequency {} should be close to 500 Hz",
                freq
            );
        }
    }
}
