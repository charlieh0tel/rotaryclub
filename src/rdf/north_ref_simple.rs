use crate::config::NorthTickConfig;
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{HighpassFilter, PeakDetector};

pub struct SimpleNorthTracker {
    highpass: HighpassFilter,
    peak_detector: PeakDetector,
    last_tick_sample: Option<usize>,
    samples_per_rotation: Option<f32>,
    sample_counter: usize,
    sample_rate: f32,
}

impl SimpleNorthTracker {
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        let min_samples = (config.min_interval_ms / 1000.0 * sample_rate) as usize;

        Ok(Self {
            highpass: HighpassFilter::new(
                config.highpass_cutoff,
                sample_rate,
                config.filter_order,
            )?,
            peak_detector: PeakDetector::new(config.threshold, min_samples),
            last_tick_sample: None,
            samples_per_rotation: None,
            sample_counter: 0,
            sample_rate,
        })
    }

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        let mut filtered = buffer.to_vec();
        self.highpass.process_buffer(&mut filtered);

        let peaks = self.peak_detector.find_all_peaks(&filtered);

        let mut ticks = Vec::new();

        for peak_idx in peaks {
            let global_sample = self.sample_counter + peak_idx;

            // Update rotation period estimate with exponential averaging
            if let Some(last) = self.last_tick_sample {
                let period = (global_sample - last) as f32;

                self.samples_per_rotation = Some(
                    self.samples_per_rotation
                        .map(|prev| 0.9 * prev + 0.1 * period)
                        .unwrap_or(period),
                );
            }

            ticks.push(NorthTick {
                sample_index: global_sample,
                period: self.samples_per_rotation,
            });

            self.last_tick_sample = Some(global_sample);
        }

        self.sample_counter += buffer.len();
        ticks
    }

    #[allow(dead_code)]
    pub fn rotation_period(&self) -> Option<f32> {
        self.samples_per_rotation
    }

    pub fn rotation_frequency(&self) -> Option<f32> {
        self.samples_per_rotation
            .map(|period| self.sample_rate / period)
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.last_tick_sample = None;
        self.samples_per_rotation = None;
        self.sample_counter = 0;
        self.peak_detector.reset();
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

        let mut signal = vec![0.0; 500];
        signal[50] = 0.8;
        signal[146] = 0.8;
        signal[242] = 0.8;

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
