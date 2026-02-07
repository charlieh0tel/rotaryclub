/// Stateful DC offset remover using a single-pole IIR high-pass filter.
pub struct DcRemover {
    dc_estimate: f32,
    alpha: f32,
}

impl DcRemover {
    /// Create a new DC remover with the given smoothing factor.
    /// Alpha should be small (e.g., 0.0001) for slow adaptation.
    pub fn new(alpha: f32) -> Self {
        Self {
            dc_estimate: 0.0,
            alpha,
        }
    }

    /// Create a DC remover with a specified cutoff frequency.
    /// Frequencies below cutoff_hz will be attenuated.
    pub fn with_cutoff(sample_rate: f32, cutoff_hz: f32) -> Self {
        let alpha = (2.0 * std::f32::consts::PI * cutoff_hz / sample_rate).min(1.0);
        Self::new(alpha)
    }

    /// Process samples in-place, removing DC offset.
    pub fn process(&mut self, samples: &mut [f32]) {
        for sample in samples.iter_mut() {
            self.dc_estimate += self.alpha * (*sample - self.dc_estimate);
            *sample -= self.dc_estimate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dc_remover_converges() {
        let mut remover = DcRemover::new(0.01);
        let dc_offset = 5.0;

        // Process many samples with constant DC offset
        for _ in 0..1000 {
            let mut samples = vec![dc_offset; 100];
            remover.process(&mut samples);
        }

        // After convergence, output should be near zero
        let mut samples = vec![dc_offset; 10];
        remover.process(&mut samples);
        for sample in &samples {
            assert!(sample.abs() < 0.1, "Expected near zero, got {}", sample);
        }
    }

    #[test]
    fn test_dc_remover_preserves_ac() {
        let mut remover = DcRemover::with_cutoff(48000.0, 1.0);

        // Generate a 1000 Hz sine wave with DC offset
        let dc_offset = 2.0;
        let freq = 1000.0;
        let sample_rate = 48000.0;

        // Let the filter settle
        for _ in 0..100 {
            let mut samples: Vec<f32> = (0..480)
                .map(|i| {
                    dc_offset + (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin()
                })
                .collect();
            remover.process(&mut samples);
        }

        // Check that AC component is preserved (amplitude should be close to 1.0)
        let mut samples: Vec<f32> = (0..480)
            .map(|i| dc_offset + (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate).sin())
            .collect();
        remover.process(&mut samples);

        let max = samples.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let min = samples.iter().cloned().fold(f32::INFINITY, f32::min);
        let amplitude = (max - min) / 2.0;

        assert!(
            (amplitude - 1.0).abs() < 0.1,
            "AC amplitude should be ~1.0, got {}",
            amplitude
        );
    }

    #[test]
    fn test_dc_remover_empty() {
        let mut remover = DcRemover::new(0.01);
        let mut samples: Vec<f32> = vec![];
        remover.process(&mut samples);
        assert!(samples.is_empty());
    }
}
