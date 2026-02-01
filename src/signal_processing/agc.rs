use crate::config::AgcConfig;

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
/// Gain is clamped to [0.1, 10.0] to prevent extreme amplification or
/// attenuation.
pub struct AutomaticGainControl {
    target_rms: f32,
    attack_coeff: f32,
    release_coeff: f32,
    current_gain: f32,
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
    pub fn new(config: &AgcConfig, sample_rate: u32) -> Self {
        let attack_coeff = Self::time_constant_to_coeff(config.attack_time_ms, sample_rate);
        let release_coeff = Self::time_constant_to_coeff(config.release_time_ms, sample_rate);
        let window_size = (sample_rate as f32 * config.measurement_window_ms / 1000.0) as usize;

        Self {
            target_rms: config.target_rms,
            attack_coeff,
            release_coeff,
            current_gain: 1.0,
            rms_accumulator: 0.0,
            sample_count: 0,
            window_size,
        }
    }

    fn time_constant_to_coeff(time_ms: f32, sample_rate: u32) -> f32 {
        let time_samples = (time_ms / 1000.0) * sample_rate as f32;
        (-1.0 / time_samples).exp()
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

            if rms > 1e-6 {
                let desired_gain = self.target_rms / rms;
                let coeff = if desired_gain > self.current_gain {
                    self.attack_coeff
                } else {
                    self.release_coeff
                };

                self.current_gain = coeff * self.current_gain + (1.0 - coeff) * desired_gain;
                self.current_gain = self.current_gain.clamp(0.1, 10.0);
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
    /// Returns the current gain multiplier (0.1 to 10.0 range).
    #[allow(dead_code)]
    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agc_amplifies_weak_signal() {
        let config = AgcConfig {
            target_rms: 0.5,
            attack_time_ms: 10.0,
            release_time_ms: 100.0,
            measurement_window_ms: 10.0,
        };

        let mut agc = AutomaticGainControl::new(&config, 48000);

        let weak_signal: Vec<f32> = (0..48000)
            .map(|i| 0.1 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin())
            .collect();

        let mut output = weak_signal.clone();
        agc.process_buffer(&mut output);

        let input_rms =
            (weak_signal.iter().map(|x| x * x).sum::<f32>() / weak_signal.len() as f32).sqrt();
        let last_quarter = &output[output.len() * 3 / 4..];
        let output_rms =
            (last_quarter.iter().map(|x| x * x).sum::<f32>() / last_quarter.len() as f32).sqrt();

        assert!(
            output_rms > input_rms * 2.0,
            "AGC should amplify weak signal"
        );
        assert!(
            (output_rms - config.target_rms).abs() < (input_rms - config.target_rms).abs(),
            "Output should be closer to target than input"
        );
    }

    #[test]
    fn test_agc_reduces_strong_signal() {
        let config = AgcConfig {
            target_rms: 0.5,
            attack_time_ms: 10.0,
            release_time_ms: 100.0,
            measurement_window_ms: 10.0,
        };

        let mut agc = AutomaticGainControl::new(&config, 48000);

        let strong_signal: Vec<f32> = (0..48000)
            .map(|i| 0.9 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin())
            .collect();

        let mut output = strong_signal.clone();
        agc.process_buffer(&mut output);

        let input_rms =
            (strong_signal.iter().map(|x| x * x).sum::<f32>() / strong_signal.len() as f32).sqrt();
        let last_quarter = &output[output.len() * 3 / 4..];
        let output_rms =
            (last_quarter.iter().map(|x| x * x).sum::<f32>() / last_quarter.len() as f32).sqrt();

        assert!(output_rms < input_rms, "AGC should reduce strong signal");
        assert!(
            (output_rms - config.target_rms).abs() < (input_rms - config.target_rms).abs(),
            "Output should be closer to target than input"
        );
    }

    #[test]
    fn test_agc_gain_clamping() {
        let config = AgcConfig {
            target_rms: 0.5,
            attack_time_ms: 1.0,
            release_time_ms: 10.0,
            measurement_window_ms: 1.0,
        };

        let mut agc = AutomaticGainControl::new(&config, 48000);

        let silent_signal = vec![0.001; 48000];
        let mut output = silent_signal.clone();
        agc.process_buffer(&mut output);

        assert!(agc.current_gain() <= 10.0);
    }
}
