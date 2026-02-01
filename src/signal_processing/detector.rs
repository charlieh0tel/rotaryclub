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

    /// Reset detector state
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.last_sample = 0.0;
    }
}

/// Peak detector for north tick pulse detection
///
/// Detects peaks (rising-edge threshold crossings) in a signal with
/// configurable threshold and minimum spacing between peaks.
///
/// The detector triggers when the signal rises above the threshold and
/// enforces a minimum interval between detections to reject spurious
/// triggers from noise or ringing.
pub struct PeakDetector {
    threshold: f32,
    min_samples_between_peaks: usize,
    samples_since_peak: usize,
    last_sample: f32,
    above_threshold: bool,
}

impl PeakDetector {
    /// Create a new peak detector
    ///
    /// # Arguments
    /// * `threshold` - Amplitude threshold for peak detection (0-1 range)
    /// * `min_interval_samples` - Minimum samples between detected peaks
    pub fn new(threshold: f32, min_interval_samples: usize) -> Self {
        Self {
            threshold,
            min_samples_between_peaks: min_interval_samples,
            samples_since_peak: min_interval_samples, // Allow immediate first peak
            last_sample: 0.0,
            above_threshold: false,
        }
    }

    /// Detect a peak in the next sample
    ///
    /// Returns `true` if a rising-edge threshold crossing is detected and
    /// sufficient time has elapsed since the last peak.
    ///
    /// # Arguments
    /// * `sample` - The next audio sample to process
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

    /// Find all peaks in a buffer
    ///
    /// Returns a vector of sample indices where peaks are detected.
    ///
    /// # Arguments
    /// * `buffer` - Audio samples to process
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
    #[allow(dead_code)]
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
