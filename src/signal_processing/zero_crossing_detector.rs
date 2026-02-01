/// Zero-crossing detector with hysteresis
///
/// Detects rising-edge zero crossings (negative to positive transitions) in
/// an audio signal with configurable hysteresis to reject noise.
///
/// The detector only triggers when the signal transitions from below
/// `-hysteresis` to above `+hysteresis`, providing noise immunity for
/// noisy signals near zero.
pub struct ZeroCrossingDetector {
    last_sample: f32,
    hysteresis: f32,
}

impl ZeroCrossingDetector {
    /// Create a new zero-crossing detector
    ///
    /// # Arguments
    /// * `hysteresis` - Hysteresis threshold (typically 0.01-0.1)
    pub fn new(hysteresis: f32) -> Self {
        Self {
            last_sample: 0.0,
            hysteresis,
        }
    }

    /// Detect a zero crossing in the next sample
    ///
    /// Returns `true` if a rising-edge crossing is detected (transition from
    /// negative to positive). The crossing must exceed the hysteresis threshold
    /// on both sides to trigger.
    ///
    /// # Arguments
    /// * `sample` - The next audio sample to process
    pub fn detect_crossing(&mut self, sample: f32) -> bool {
        let crossing = self.last_sample < -self.hysteresis && sample > self.hysteresis;
        self.last_sample = sample;
        crossing
    }

    /// Find all zero crossings in a buffer
    ///
    /// Returns a vector of sample indices where rising-edge crossings occur.
    ///
    /// # Arguments
    /// * `buffer` - Audio samples to process
    pub fn find_all_crossings(&mut self, buffer: &[f32]) -> Vec<usize> {
        buffer
            .iter()
            .enumerate()
            .filter_map(|(i, &sample)| {
                if self.detect_crossing(sample) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_crossing_detection() {
        let mut detector = ZeroCrossingDetector::new(0.01);

        // Generate sine wave with clear zero crossings
        let signal: Vec<f32> = (0..200).map(|i| (i as f32 * 0.1).sin()).collect();

        let crossings = detector.find_all_crossings(&signal);

        // Signal goes from 0 to 20 radians ≈ 3.2 periods, expect ~3 rising crossings
        assert!(
            crossings.len() >= 2 && crossings.len() <= 4,
            "Expected 2-4 crossings, found {}",
            crossings.len()
        );
    }

    #[test]
    fn test_zero_crossing_hysteresis() {
        let mut detector = ZeroCrossingDetector::new(0.1);

        // Small oscillations around zero should not trigger
        let signal = vec![-0.05, 0.05, -0.05, 0.05, -0.5, 0.5];

        let crossings = detector.find_all_crossings(&signal);

        // Only the last crossing should be detected (exceeds hysteresis)
        assert_eq!(crossings.len(), 1);
        assert_eq!(crossings[0], 5);
    }
}
