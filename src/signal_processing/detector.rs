/// Zero-crossing detector with hysteresis
pub struct ZeroCrossingDetector {
    last_sample: f32,
    hysteresis: f32,
}

impl ZeroCrossingDetector {
    pub fn new(hysteresis: f32) -> Self {
        Self {
            last_sample: 0.0,
            hysteresis,
        }
    }

    /// Detect zero crossing, returns true if rising edge crossing detected
    /// Crossing = transition from negative to positive (rising edge)
    pub fn detect_crossing(&mut self, sample: f32) -> bool {
        let crossing = self.last_sample < -self.hysteresis && sample > self.hysteresis;
        self.last_sample = sample;
        crossing
    }

    /// Find all zero crossings in buffer, return sample indices
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

    /// Reset detector state
    pub fn reset(&mut self) {
        self.last_sample = 0.0;
    }
}

/// Peak detector for north tick pulse
pub struct PeakDetector {
    threshold: f32,
    min_samples_between_peaks: usize,
    samples_since_peak: usize,
    last_sample: f32,
    above_threshold: bool,
}

impl PeakDetector {
    pub fn new(threshold: f32, min_interval_samples: usize) -> Self {
        Self {
            threshold,
            min_samples_between_peaks: min_interval_samples,
            samples_since_peak: min_interval_samples, // Allow immediate first peak
            last_sample: 0.0,
            above_threshold: false,
        }
    }

    /// Detect peak above threshold (rising edge detection)
    pub fn detect_peak(&mut self, sample: f32) -> bool {
        self.samples_since_peak += 1;

        // Detect rising edge crossing threshold
        let crossed_threshold = !self.above_threshold
            && self.last_sample <= self.threshold
            && sample > self.threshold
            && self.samples_since_peak >= self.min_samples_between_peaks;

        // Track whether we're above threshold
        self.above_threshold = sample > self.threshold;
        self.last_sample = sample;

        if crossed_threshold {
            self.samples_since_peak = 0;
        }

        crossed_threshold
    }

    /// Find all peaks in buffer, return sample indices
    pub fn find_all_peaks(&mut self, buffer: &[f32]) -> Vec<usize> {
        buffer
            .iter()
            .enumerate()
            .filter_map(|(i, &sample)| {
                if self.detect_peak(sample) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Reset detector state
    pub fn reset(&mut self) {
        self.samples_since_peak = self.min_samples_between_peaks;
        self.last_sample = 0.0;
        self.above_threshold = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

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
    fn test_peak_detection() {
        let mut detector = PeakDetector::new(0.5, 10);

        let mut signal = vec![0.0; 100];
        signal[20] = 0.8; // Peak above threshold
        signal[25] = 0.9; // Too close, should be rejected
        signal[50] = 0.7; // Valid peak

        let peaks = detector.find_all_peaks(&signal);

        assert_eq!(peaks.len(), 2);
        assert_eq!(peaks[0], 20);
        assert_eq!(peaks[1], 50);
    }

    #[test]
    fn test_peak_detector_threshold() {
        let mut detector = PeakDetector::new(0.5, 5);

        let signal = vec![0.3, 0.4, 0.6, 0.7, 0.4, 0.2, 0.3, 0.4, 0.8, 0.3];

        let peaks = detector.find_all_peaks(&signal);

        // Should detect rising edges crossing threshold
        assert_eq!(peaks.len(), 2);
        assert_eq!(peaks[0], 2); // Rising edge 0.4 -> 0.6
        assert_eq!(peaks[1], 8); // Rising edge 0.4 -> 0.8 (after min_interval)
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
