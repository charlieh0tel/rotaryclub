use crate::config::AgcConfig;
use crate::constants::MIN_RMS_THRESHOLD;

/// Automatic Gain Control (AGC)
///
/// Dynamically adjusts signal amplitude to maintain a target RMS level,
/// compensating for variations in input signal strength. Essential for
/// consistent bearing calculations across varying signal conditions.
///
/// Uses separate attack and release time constants for smooth gain adjustment:
/// - Attack: how quickly gain increases for weak signals
/// - Release: how quickly gain decreases for strong signals
///
/// Gain is clamped to configured min/max bounds to prevent extreme
/// amplification or attenuation.
pub struct AutomaticGainControl {
    target_rms: f32,
    attack_coeff: f32,
    release_coeff: f32,
    current_gain: f32,
    min_gain: f32,
    max_gain: f32,
    rms_accumulator: f32,
    sample_count: usize,
    window_size: usize,
}

impl AutomaticGainControl {
    /// Create a new AGC processor
    ///
    /// # Arguments
    /// * `config` - AGC configuration parameters
    /// * `sample_rate` - Audio sample rate in Hz
    pub fn new(config: &AgcConfig, sample_rate: f32) -> Self {
        let window_size = (sample_rate * config.measurement_window_ms / 1000.0) as usize;
        let attack_coeff =
            Self::time_constant_to_coeff(config.attack_time_ms, config.measurement_window_ms);
        let release_coeff =
            Self::time_constant_to_coeff(config.release_time_ms, config.measurement_window_ms);

        Self {
            target_rms: config.target_rms,
            attack_coeff,
            release_coeff,
            current_gain: 1.0,
            min_gain: config.min_gain,
            max_gain: config.max_gain,
            rms_accumulator: 0.0,
            sample_count: 0,
            window_size,
        }
    }

    fn time_constant_to_coeff(time_constant_ms: f32, window_ms: f32) -> f32 {
        (-window_ms / time_constant_ms).exp()
    }

    /// Process a single audio sample through the AGC
    ///
    /// Accumulates RMS measurements over a window and adjusts gain as needed.
    ///
    /// # Arguments
    /// * `sample` - Input audio sample
    ///
    /// # Returns
    /// Gain-adjusted output sample
    pub fn process(&mut self, sample: f32) -> f32 {
        self.rms_accumulator += sample * sample;
        self.sample_count += 1;

        if self.sample_count >= self.window_size {
            let rms = (self.rms_accumulator / self.window_size as f32).sqrt();
            self.rms_accumulator = 0.0;
            self.sample_count = 0;

            if rms > MIN_RMS_THRESHOLD {
                let desired_gain = self.target_rms / rms;
                let coeff = if desired_gain < self.current_gain {
                    self.attack_coeff
                } else {
                    self.release_coeff
                };

                self.current_gain = coeff * self.current_gain + (1.0 - coeff) * desired_gain;
                self.current_gain = self.current_gain.clamp(self.min_gain, self.max_gain);
            }
        }

        sample * self.current_gain
    }

    /// Process an entire buffer of audio samples in-place
    ///
    /// Applies AGC to each sample in the buffer, replacing the original
    /// values with gain-adjusted output.
    ///
    /// # Arguments
    /// * `buffer` - Audio samples to process
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process(*sample);
        }
    }

    /// Get the current gain factor
    ///
    /// Returns the current gain multiplier (clamped to configured min/max range).
    #[allow(dead_code)]
    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tone(amplitude: f32, freq_hz: f32, sample_rate: f32, num_samples: usize) -> Vec<f32> {
        (0..num_samples)
            .map(|i| {
                amplitude * (2.0 * std::f32::consts::PI * freq_hz * i as f32 / sample_rate).sin()
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn default_config() -> AgcConfig {
        AgcConfig {
            target_rms: 0.5,
            attack_time_ms: 10.0,
            release_time_ms: 100.0,
            measurement_window_ms: 10.0,
            min_gain: 0.1,
            max_gain: 10.0,
        }
    }

    #[test]
    fn test_agc_converges_weak_signal() {
        let config = default_config();
        let mut agc = AutomaticGainControl::new(&config, 48000.0);

        let mut signal = make_tone(0.1, 1000.0, 48000.0, 48000);
        agc.process_buffer(&mut signal);

        let output_rms = rms(&signal[signal.len() * 3 / 4..]);
        let error = (output_rms - config.target_rms).abs() / config.target_rms;
        assert!(
            error < 0.15,
            "After 1s, weak signal RMS should be within 15% of target: got {output_rms:.3}, target {}, error {:.0}%",
            config.target_rms,
            error * 100.0
        );
    }

    #[test]
    fn test_agc_converges_strong_signal() {
        let config = default_config();
        let mut agc = AutomaticGainControl::new(&config, 48000.0);

        let mut signal = make_tone(0.9, 1000.0, 48000.0, 48000);
        agc.process_buffer(&mut signal);

        let output_rms = rms(&signal[signal.len() * 3 / 4..]);
        let error = (output_rms - config.target_rms).abs() / config.target_rms;
        assert!(
            error < 0.15,
            "After 1s, strong signal RMS should be within 15% of target: got {output_rms:.3}, target {}, error {:.0}%",
            config.target_rms,
            error * 100.0
        );
    }

    #[test]
    fn test_agc_gain_clamping() {
        let config = AgcConfig {
            attack_time_ms: 1.0,
            release_time_ms: 10.0,
            measurement_window_ms: 1.0,
            ..default_config()
        };

        let mut agc = AutomaticGainControl::new(&config, 48000.0);

        let mut signal = vec![0.001; 48000];
        agc.process_buffer(&mut signal);

        assert!(agc.current_gain() <= config.max_gain);
    }

    #[test]
    fn test_agc_attack_faster_than_release() {
        let config = default_config();
        let sample_rate = 48000.0;
        let samples_500ms = 24000;

        // Measure attack: loud signal drives gain down
        let mut agc = AutomaticGainControl::new(&config, sample_rate);
        let mut loud = make_tone(0.9, 1000.0, sample_rate, samples_500ms);
        agc.process_buffer(&mut loud);
        let gain_after_attack = agc.current_gain();

        // Measure release: quiet signal drives gain up
        let mut agc = AutomaticGainControl::new(&config, sample_rate);
        let mut quiet = make_tone(0.1, 1000.0, sample_rate, samples_500ms);
        agc.process_buffer(&mut quiet);
        let gain_after_release = agc.current_gain();

        // Attack (gain decrease for loud signal) should converge faster
        // than release (gain increase for quiet signal).
        let attack_target = config.target_rms / (0.9 / 2.0_f32.sqrt());
        let release_target = config.target_rms / (0.1 / 2.0_f32.sqrt());

        let attack_convergence =
            1.0 - (gain_after_attack - attack_target).abs() / (1.0 - attack_target).abs();
        let release_convergence =
            1.0 - (gain_after_release - release_target).abs() / (1.0 - release_target).abs();

        assert!(
            attack_convergence > release_convergence,
            "Attack should converge faster than release: attack {attack_convergence:.2} vs release {release_convergence:.2}"
        );
    }

    #[test]
    fn test_agc_loud_after_quiet_recovers_fast() {
        let config = default_config();
        let sample_rate = 48000.0;

        let mut agc = AutomaticGainControl::new(&config, sample_rate);

        // 0.5s of quiet signal — gain ramps up
        let mut quiet = make_tone(0.05, 1000.0, sample_rate, 24000);
        agc.process_buffer(&mut quiet);
        let gain_after_quiet = agc.current_gain();
        assert!(
            gain_after_quiet > 3.0,
            "Gain should be high after quiet period: {gain_after_quiet:.2}"
        );

        // 100ms of loud signal — gain should come back down quickly (attack)
        let mut loud = make_tone(0.9, 1000.0, sample_rate, 4800);
        agc.process_buffer(&mut loud);
        let gain_after_loud = agc.current_gain();

        assert!(
            gain_after_loud < 1.5,
            "Gain should recover within 100ms after loud signal arrives: {gain_after_loud:.2}"
        );
    }
}
