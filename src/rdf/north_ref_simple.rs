use crate::config::NorthTickConfig;
use crate::constants::FREQUENCY_EPSILON;
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{FirHighpass, PeakDetector};
use std::f32::consts::PI;

const PERIOD_SMOOTHING_FACTOR: f32 = 0.1;
const MIN_TICK_SPACING_FRACTION: f32 = 0.75;

pub struct SimpleNorthTracker {
    gain: f32,
    highpass: FirHighpass,
    peak_detector: PeakDetector,
    threshold_crossing_offset: f32,
    nominal_period_samples: f32,
    last_tick_sample: Option<usize>,
    samples_per_rotation: Option<f32>,
    sample_counter: usize,
    sample_rate: f32,
    filter_buffer: Vec<f32>,
}

impl SimpleNorthTracker {
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        let min_samples = (config.min_interval_ms / 1000.0 * sample_rate) as usize;
        let gain = 10.0_f32.powf(config.gain_db / 20.0);

        let highpass = FirHighpass::new(
            config.highpass_cutoff,
            sample_rate,
            config.fir_highpass_taps,
            config.highpass_transition_hz,
        )?;

        let effective_pulse_amplitude = (config.expected_pulse_amplitude * gain).max(f32::EPSILON);
        let threshold_crossing_offset =
            highpass.threshold_crossing_offset(config.threshold, effective_pulse_amplitude);
        let nominal_period_samples = if config.dpll.initial_frequency_hz > FREQUENCY_EPSILON {
            sample_rate / config.dpll.initial_frequency_hz
        } else {
            min_samples as f32
        };

        Ok(Self {
            gain,
            highpass,
            peak_detector: PeakDetector::new(config.threshold, min_samples),
            threshold_crossing_offset,
            nominal_period_samples,
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

            if let Some(last) = self.last_tick_sample {
                let period_reference = self
                    .samples_per_rotation
                    .unwrap_or(self.nominal_period_samples);
                let min_spacing = period_reference * MIN_TICK_SPACING_FRACTION;
                let delta = global_sample.saturating_sub(last) as f32;
                if delta < min_spacing {
                    continue;
                }
            }

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

    #[test]
    fn test_simple_north_tick_delay_compensation_with_gain() {
        let sample_rate = 48000.0;
        let config = NorthTickConfig {
            gain_db: 20.0,
            dpll: crate::config::DpllConfig {
                initial_frequency_hz: 480.0,
                natural_frequency_hz: 10.0,
                damping_ratio: 0.707,
                frequency_min_hz: 300.0,
                frequency_max_hz: 800.0,
            },
            ..Default::default()
        };
        let mut tracker = SimpleNorthTracker::new(&config, sample_rate).unwrap();

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
}
