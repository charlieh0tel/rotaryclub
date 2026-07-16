use crate::constants::INTERPOLATION_EPSILON;

/// Zero-crossing detector with hysteresis
///
/// Detects rising-edge zero crossings (negative to positive transitions) in
/// an audio signal with configurable hysteresis to reject noise.
///
/// The detector only triggers when the signal transitions from below
/// `-hysteresis` to above `+hysteresis`, providing noise immunity for
/// noisy signals near zero.
pub struct ZeroCrossingDetector {
    hysteresis: f32,
    armed: bool,
    // Carried across find_all_crossings calls so a crossing that straddles a
    // buffer boundary is interpolated instead of snapped to index 0.
    prev_sample: Option<f32>,
    // Crossing detected but not yet confirmed by the hysteresis threshold,
    // expressed relative to the current buffer (negative once carried over).
    pending_crossing: Option<f32>,
}

impl ZeroCrossingDetector {
    /// Create a new zero-crossing detector
    ///
    /// # Arguments
    /// * `hysteresis` - Hysteresis threshold (typically 0.01-0.1)
    pub fn new(hysteresis: f32) -> Self {
        Self {
            hysteresis,
            armed: false,
            prev_sample: None,
            pending_crossing: None,
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
        if sample < -self.hysteresis {
            self.armed = true;
        }
        if self.armed && sample > self.hysteresis {
            self.armed = false;
            return true;
        }
        false
    }

    /// Find all zero crossings in a buffer with sub-sample interpolation
    ///
    /// Returns interpolated sample positions where rising-edge crossings occur.
    ///
    /// # Arguments
    /// * `buffer` - Audio samples to process
    pub fn find_all_crossings(&mut self, buffer: &[f32]) -> Vec<f32> {
        let mut crossings = Vec::new();
        if buffer.is_empty() {
            return crossings;
        }

        let mut prev_sample = self.prev_sample;
        let mut pending_crossing = self.pending_crossing.take();

        for (i, &sample) in buffer.iter().enumerate() {
            if sample < -self.hysteresis {
                self.armed = true;
                pending_crossing = None;
            }

            if self.armed
                && pending_crossing.is_none()
                && sample > 0.0
                && let Some(prev) = prev_sample.filter(|&p| p <= 0.0)
            {
                // A crossing straddling the buffer boundary interpolates
                // against the previous buffer's last sample, yielding a
                // small negative position (before this buffer's start).
                let denominator = sample - prev;
                let crossing = if denominator.abs() > INTERPOLATION_EPSILON {
                    let fraction = sample / denominator;
                    i as f32 - fraction
                } else {
                    i as f32
                };
                pending_crossing = Some(crossing);
            }

            if self.armed && sample > self.hysteresis {
                crossings.push(pending_crossing.unwrap_or(i as f32));
                self.armed = false;
                pending_crossing = None;
            }

            prev_sample = Some(sample);
        }

        self.prev_sample = prev_sample;
        // Re-express an unconfirmed crossing relative to the next buffer.
        self.pending_crossing = pending_crossing.map(|c| c - buffer.len() as f32);

        crossings
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

        let signal = vec![-0.05, 0.05, -0.05, 0.05, -0.5, 0.5];

        let crossings = detector.find_all_crossings(&signal);

        assert_eq!(crossings.len(), 1);
        let expected = 5.0 - 0.5 / (0.5 - (-0.5));
        assert!((crossings[0] - expected).abs() < 0.01);
    }

    #[test]
    fn test_zero_crossing_interpolation() {
        let mut detector = ZeroCrossingDetector::new(0.01);

        let signal = vec![-0.3, -0.1, 0.2, 0.4];

        let crossings = detector.find_all_crossings(&signal);

        assert_eq!(crossings.len(), 1);
        let expected = 2.0 - 0.2 / (0.2 - (-0.1));
        assert!(
            (crossings[0] - expected).abs() < 0.001,
            "Expected {}, got {}",
            expected,
            crossings[0]
        );
    }

    #[test]
    fn test_zero_crossing_first_sample_arms_detector() {
        let mut detector = ZeroCrossingDetector::new(0.1);

        let signal = vec![-0.5, 0.5];
        let crossings = detector.find_all_crossings(&signal);

        assert_eq!(crossings.len(), 1);
        let expected = 1.0 - 0.5 / (0.5 - (-0.5));
        assert!((crossings[0] - expected).abs() < 0.01);
    }

    #[test]
    fn test_zero_crossings_identical_when_split_across_buffers() {
        // Regression test: crossings straddling a buffer boundary used to be
        // reported at index 0 of the new buffer instead of interpolated
        // against the previous buffer's last sample.
        let signal: Vec<f32> = (0..200).map(|i| (i as f32 * 0.1).sin()).collect();

        let mut whole_detector = ZeroCrossingDetector::new(0.01);
        let whole = whole_detector.find_all_crossings(&signal);
        assert!(!whole.is_empty());

        // Split right after sample 62, mid-crossing (sin goes -0.083 -> 0.017).
        for split_at in [1usize, 30, 63, 100, 199] {
            let mut split_detector = ZeroCrossingDetector::new(0.01);
            let mut split_crossings = Vec::new();
            for (start, chunk) in [(0, &signal[..split_at]), (split_at, &signal[split_at..])] {
                for c in split_detector.find_all_crossings(chunk) {
                    split_crossings.push(start as f32 + c);
                }
            }
            assert_eq!(
                whole.len(),
                split_crossings.len(),
                "crossing count differs for split at {}",
                split_at
            );
            for (w, s) in whole.iter().zip(&split_crossings) {
                assert!(
                    (w - s).abs() < 1e-3,
                    "split at {}: whole {} vs split {}",
                    split_at,
                    w,
                    s
                );
            }
        }
    }

    #[test]
    fn test_zero_crossing_interpolation_uses_zero_crossing_not_threshold_crossing() {
        let mut detector = ZeroCrossingDetector::new(0.1);

        // Zero crossing happens between samples 0 and 1, but hysteresis threshold is
        // only exceeded at sample 2.
        let signal = vec![-0.3, 0.05, 0.2, 0.4];
        let crossings = detector.find_all_crossings(&signal);

        assert_eq!(crossings.len(), 1);
        let expected = 1.0 - 0.05 / (0.05 - (-0.3));
        assert!(
            (crossings[0] - expected).abs() < 0.01,
            "Expected {}, got {}",
            expected,
            crossings[0]
        );
    }
}
