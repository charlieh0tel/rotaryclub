use crate::config::{NorthPulseEstimator, NorthTickConfig};
use crate::constants::FREQUENCY_EPSILON;
use crate::error::Result;
use crate::rdf::NorthTick;
use crate::signal_processing::{FirHighpass, PeakDetector, db_to_amplitude};
use std::f32::consts::PI;

use super::north_ref_common::{
    QuietChannelWatch, centroid_half_width, derive_delay_compensation, derive_peak_timing,
    estimate_fraction, highpass_taps, preprocess_north_buffer, retain_tail, split_effective_time,
    validate_north_tick_config,
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
    /// Smoothed squared deviation of the measured interval from its own
    /// average, in samples squared. See `reference_phase_variance`.
    period_variance: Option<f32>,
    /// Intervals measured so far, so a variance is not reported off one.
    intervals_seen: usize,
    sample_counter: usize,
    sample_rate: f32,
    filter_buffer: Vec<f32>,
    // Filtered samples preceding filter_buffer, for estimator windows that
    // straddle a buffer boundary.
    filter_tail: Vec<f32>,
    quiet_watch: QuietChannelWatch,
    threshold: f32,
}

impl SimpleNorthTracker {
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        validate_north_tick_config(config, sample_rate)?;

        let min_samples = (config.min_interval_ms / 1000.0 * sample_rate) as usize;
        let gain = db_to_amplitude(config.gain_db);

        let highpass = FirHighpass::new(
            config.highpass_cutoff,
            sample_rate,
            highpass_taps(config, sample_rate),
            config.highpass_transition_hz,
        )?;

        let effective_pulse_amplitude = (config.expected_pulse_amplitude * gain).max(f32::EPSILON);
        let nominal_period_samples = sample_rate / config.dpll.initial_frequency_hz.max(1.0);
        let centroid_half_width =
            centroid_half_width(config.estimator, sample_rate, nominal_period_samples);
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
            peak_detector: {
                let mut detector = PeakDetector::with_peak_search_window(
                    config.threshold,
                    min_samples,
                    peak_timing.peak_search_window_samples,
                );
                // The estimator reads samples on both sides of the peak, and
                // the peak can land at the end of the search window, so the
                // detector must hold a crossing until that context exists.
                detector.set_trailing_context(centroid_half_width);
                detector
            },
            pulse_reference_offset: peak_timing.pulse_reference_offset,
            estimator: config.estimator,
            centroid_half_width,
            // A peak whose search window straddled a buffer boundary is
            // reported at a negative index in the next buffer. That index
            // reaches back by the search window plus the trailing context the
            // detector now waits for, and the estimator then reads a further
            // half-width before it.
            filter_tail_len: 2 * centroid_half_width + peak_timing.peak_search_window_samples,
            nominal_period_samples,
            last_tick_sample: None,
            last_tick_fraction: 0.0,
            samples_per_rotation: None,
            period_variance: None,
            intervals_seen: 0,
            sample_counter: 0,
            sample_rate,
            filter_buffer: Vec::new(),
            filter_tail: Vec::new(),
            quiet_watch: QuietChannelWatch::new(sample_rate),
            threshold: config.threshold,
        })
    }

    /// This tracker's own timing scatter, in radians squared of rotation
    /// phase, or None before it has enough intervals to say.
    ///
    /// There is no oscillator here to compare a detection against, but the
    /// intervals between detections carry the same information: if each tick
    /// has timing variance v, the interval between two of them has variance
    /// 2v, so half the interval scatter is the per-tick scatter. Converting
    /// samples squared to radians squared is the square of one rotation's
    /// worth of phase per sample.
    ///
    /// This is what the bearing uncertainty needs from a reference and it is
    /// two fields and an exponential average to produce. Reporting nothing --
    /// which this used to -- is not free: an unknown reference suppresses the
    /// uncertainty figure entirely and with it any confidence built on it.
    fn reference_phase_variance(&self) -> Option<f32> {
        // One interval is a number, not a spread.
        if self.intervals_seen < 2 {
            return None;
        }
        let interval_variance = self.period_variance?;
        let period = self.samples_per_rotation?;
        if !interval_variance.is_finite() || !period.is_finite() || period <= f32::EPSILON {
            return None;
        }
        let radians_per_sample = 2.0 * PI / period;
        Some(interval_variance / 2.0 * radians_per_sample * radians_per_sample)
    }

    /// Advance the sample clock over lost samples so tick indices after a
    /// capture gap stay on the real timeline.
    pub fn advance_samples(&mut self, samples: usize) {
        self.sample_counter += samples;
        self.peak_detector.reset_continuity();
        self.filter_tail.clear();
        // The delay line holds pre-gap audio that no longer adjoins what
        // follows it.
        self.highpass.reset();
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
            phase_variance: self.reference_phase_variance(),
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

        let strongest = peaks.iter().fold(0.0f32, |acc, (_, amp)| acc.max(*amp));
        self.quiet_watch
            .note_detections(peaks.len(), strongest, self.threshold);
        self.quiet_watch.advance(buffer.len(), self.threshold);

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

                // How far this interval sits from the average, before the
                // average absorbs it. Smoothed the same way the period is,
                // which is all the statistic this tracker needs to say how
                // much its own timing scatters.
                if let Some(mean) = self.samples_per_rotation {
                    let deviation = period - mean;
                    let squared = deviation * deviation;
                    self.period_variance = Some(
                        self.period_variance
                            .map(|prev| {
                                (1.0 - PERIOD_SMOOTHING_FACTOR) * prev
                                    + PERIOD_SMOOTHING_FACTOR * squared
                            })
                            .unwrap_or(squared),
                    );
                    self.intervals_seen = self.intervals_seen.saturating_add(1);
                }

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
                phase_variance: self.reference_phase_variance(),
                fractional_sample_offset,
                phase: 0.0, // By definition, tick = north = 0 radians
                frequency,
            });

            self.last_tick_sample = Some(compensated_sample);
            // Relative to compensated_sample, not to the reported index: when
            // the estimate reanchors onto a neighbouring sample the two differ
            // by one, and the next period measurement would inherit that. The
            // difference is taken in integers because these are absolute
            // sample counts -- past 2^24 an f32 cannot represent them to
            // within a sample, and subtracting after the conversion loses the
            // very quantity being measured.
            self.last_tick_fraction = fractional_sample_offset
                + (reported_sample as i64 - compensated_sample as i64) as f32;
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

    /// Samples since a north pulse was last detected.
    pub fn samples_since_detection(&self) -> usize {
        self.quiet_watch.samples_since_detection()
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
