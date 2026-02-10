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
    peak_search_window_samples: usize,
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
        Self::with_peak_search_window(threshold, min_interval_samples, min_interval_samples)
    }

    /// Create a peak detector with an explicit peak search window.
    ///
    /// `peak_search_window_samples` controls how far after a threshold crossing
    /// the detector searches for the local peak index.
    pub fn with_peak_search_window(
        threshold: f32,
        min_interval_samples: usize,
        peak_search_window_samples: usize,
    ) -> Self {
        Self {
            threshold,
            min_samples_between_peaks: min_interval_samples,
            peak_search_window_samples: peak_search_window_samples.max(1),
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
    /// Returns a vector of (sample_index, peak_amplitude) pairs.
    /// The index and amplitude correspond to the maximum positive value in a
    /// window after the threshold crossing.
    ///
    /// # Arguments
    /// * `buffer` - Audio samples to process
    pub fn find_all_peaks(&mut self, buffer: &[f32]) -> Vec<(usize, f32)> {
        let mut peaks = Vec::new();
        for (i, &sample) in buffer.iter().enumerate() {
            if self.detect_peak(sample) {
                let window_end = (i + self.peak_search_window_samples).min(buffer.len());
                let mut peak_idx = i;
                let mut peak_amp = sample;
                for (rel_idx, &candidate) in buffer[i..window_end].iter().enumerate() {
                    if candidate > peak_amp {
                        peak_amp = candidate;
                        peak_idx = i + rel_idx;
                    }
                }
                peaks.push((peak_idx, peak_amp));
            }
        }
        peaks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peak_detection() {
        let mut detector = PeakDetector::new(0.5, 10);

        let mut signal = vec![0.0; 100];
        signal[20] = 0.8; // Peak above threshold
        signal[25] = 0.9; // Too close, should be rejected
        signal[50] = 0.7; // Valid peak

        let peaks = detector.find_all_peaks(&signal);

        assert_eq!(peaks.len(), 2);
        assert_eq!(peaks[0].0, 25);
        assert!((peaks[0].1 - 0.9).abs() < 0.01); // max in window includes sample[25]
        assert_eq!(peaks[1].0, 50);
        assert!((peaks[1].1 - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_peak_detector_threshold() {
        let mut detector = PeakDetector::new(0.5, 5);

        let signal = vec![0.3, 0.4, 0.6, 0.7, 0.4, 0.2, 0.3, 0.4, 0.8, 0.3];

        let peaks = detector.find_all_peaks(&signal);

        // Should detect rising edges crossing threshold
        assert_eq!(peaks.len(), 2);
        assert_eq!(peaks[0].0, 3); // Peak near first rising edge
        assert_eq!(peaks[1].0, 8); // Rising edge 0.4 -> 0.8 (after min_interval)
    }
}
