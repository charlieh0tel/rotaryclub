use crate::config::{NorthPulseEstimator, NorthTickConfig};
use crate::constants::FREQUENCY_EPSILON;
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{FirHighpass, PeakDetector, db_to_amplitude};
use std::f32::consts::PI;

use super::north_ref_common::{
    centroid_half_width, derive_delay_compensation, derive_peak_timing, estimate_fraction,
    preprocess_north_buffer, retain_tail, split_effective_time,
};

const PERIOD_SMOOTHING_FACTOR: f32 = 0.1;
const MIN_TICK_SPACING_FRACTION: f32 = 0.75;

pub struct SimpleNorthTracker {
    gain: f32,
    highpass: FirHighpass,
    peak_detector: PeakDetector,
    pulse_reference_offset: f32,
    estimator: NorthPulseEstimator,
    centroid_half_width: usize,
    filter_tail_len: usize,
    nominal_period_samples: f32,
    last_tick_sample: Option<usize>,
    /// Sub-sample position of the last tick, so the period estimate is not
    /// requantized to whole samples on every rotation.
    last_tick_fraction: f32,
    samples_per_rotation: Option<f32>,
    sample_counter: usize,
    sample_rate: f32,
    filter_buffer: Vec<f32>,
    // Filtered samples preceding filter_buffer, for estimator windows that
    // straddle a buffer boundary.
    filter_tail: Vec<f32>,
}

impl SimpleNorthTracker {
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        let min_samples = (config.min_interval_ms / 1000.0 * sample_rate) as usize;
        let gain = db_to_amplitude(config.gain_db);

        let highpass = FirHighpass::new(
            config.highpass_cutoff,
            sample_rate,
            config.fir_highpass_taps,
            config.highpass_transition_hz,
        )?;

        let effective_pulse_amplitude = (config.expected_pulse_amplitude * gain).max(f32::EPSILON);
        let centroid_half_width = centroid_half_width(sample_rate);
        let peak_timing = derive_peak_timing(
            &highpass,
            config.threshold,
            effective_pulse_amplitude,
            config.estimator,
            centroid_half_width,
        );
        let nominal_period_samples = if config.dpll.initial_frequency_hz > FREQUENCY_EPSILON {
            sample_rate / config.dpll.initial_frequency_hz
        } else {
            min_samples as f32
        };

        Ok(Self {
            gain,
            highpass,
            peak_detector: PeakDetector::with_peak_search_window(
                config.threshold,
                min_samples,
                peak_timing.peak_search_window_samples,
            ),
            pulse_reference_offset: peak_timing.pulse_reference_offset,
            estimator: config.estimator,
            centroid_half_width,
            // A peak whose search window straddled a buffer boundary is
            // reported at a negative index in the next buffer, so the tail
            // must reach back past the search window as well as the
            // estimator's own half-width.
            filter_tail_len: centroid_half_width + peak_timing.peak_search_window_samples,
            nominal_period_samples,
            last_tick_sample: None,
            last_tick_fraction: 0.0,
            samples_per_rotation: None,
            sample_counter: 0,
            sample_rate,
            filter_buffer: Vec::new(),
            filter_tail: Vec::new(),
        })
    }

    /// Advance the sample clock over lost samples so tick indices after a
    /// capture gap stay on the real timeline.
    pub fn advance_samples(&mut self, samples: usize) {
        self.sample_counter += samples;
        self.peak_detector.reset_continuity();
        self.filter_tail.clear();
        // Don't measure the first post-gap interval across the gap: it would
        // fold the whole gap into the period EMA and yank the estimate.
        self.last_tick_sample = None;
    }

    /// Emit a final tick for a crossing whose search window was still
    /// pending at end-of-stream.
    pub fn finish(&mut self) -> Vec<NorthTick> {
        let Some((rel, _amp)) = self.peak_detector.flush() else {
            return Vec::new();
        };
        let delay = derive_delay_compensation(&self.highpass, self.pulse_reference_offset);
        let global_sample = (self.sample_counter as isize + rel).max(0) as usize;
        let compensated_sample = global_sample.saturating_sub(delay.delay_samples);
        if let Some(last) = self.last_tick_sample {
            let period_reference = self
                .samples_per_rotation
                .unwrap_or(self.nominal_period_samples);
            let min_spacing = period_reference * MIN_TICK_SPACING_FRACTION;
            if (compensated_sample.saturating_sub(last) as f32) < min_spacing {
                return Vec::new();
            }
        }
        let frequency = self
            .samples_per_rotation
            .map(|p| 2.0 * PI / p)
            .unwrap_or(0.0);
        let (reported_sample, fractional_sample_offset) =
            split_effective_time(compensated_sample, delay.fractional_sample_offset);
        self.last_tick_sample = Some(compensated_sample);
        vec![NorthTick {
            sample_index: reported_sample,
            period: self.samples_per_rotation,
            lock_quality: self.lock_quality(),
            fractional_sample_offset,
            phase: 0.0,
            frequency,
        }]
    }

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        preprocess_north_buffer(
            &mut self.filter_buffer,
            buffer,
            self.gain,
            &mut self.highpass,
        );

        let peaks = self.peak_detector.find_all_peaks(&self.filter_buffer);

        let delay = derive_delay_compensation(&self.highpass, self.pulse_reference_offset);

        let mut ticks = Vec::with_capacity(peaks.len());

        for (peak_idx, _amplitude) in peaks {
            let estimator_fraction = estimate_fraction(
                &self.filter_tail,
                &self.filter_buffer,
                peak_idx,
                self.estimator,
                self.centroid_half_width,
            );
            // Compensate for FIR filter delay: the filtered output at peak_idx
            // corresponds to an input pulse that occurred earlier by the
            // configured delay compensation. peak_idx can be slightly
            // negative when its search window completed across a buffer
            // boundary.
            let global_sample = (self.sample_counter as isize + peak_idx).max(0) as usize;
            let compensated_sample = global_sample.saturating_sub(delay.delay_samples);

            if let Some(last) = self.last_tick_sample {
                let period_reference = self
                    .samples_per_rotation
                    .unwrap_or(self.nominal_period_samples);
                let min_spacing = period_reference * MIN_TICK_SPACING_FRACTION;
                let delta = compensated_sample.saturating_sub(last) as f32;
                if delta < min_spacing {
                    continue;
                }
            }

            let (reported_sample, fractional_sample_offset) = split_effective_time(
                compensated_sample,
                delay.fractional_sample_offset + estimator_fraction,
            );

            // Update rotation period estimate with exponential averaging
            if let Some(last) = self.last_tick_sample {
                let period = (compensated_sample - last) as f32 + fractional_sample_offset
                    - self.last_tick_fraction;

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
                sample_index: reported_sample,
                period: self.samples_per_rotation,
                lock_quality: self.lock_quality(),
                fractional_sample_offset,
                phase: 0.0, // By definition, tick = north = 0 radians
                frequency,
            });

            self.last_tick_sample = Some(compensated_sample);
            // Relative to compensated_sample, not to the reported index: when
            // the estimate reanchors onto a neighbouring sample the two differ
            // by one, and the next period measurement would inherit that.
            self.last_tick_fraction =
                fractional_sample_offset + (reported_sample as f32 - compensated_sample as f32);
        }

        retain_tail(
            &mut self.filter_tail,
            &self.filter_buffer,
            self.filter_tail_len,
        );

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
