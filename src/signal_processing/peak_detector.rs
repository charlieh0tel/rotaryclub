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
    crossing_indices: Vec<usize>,
    window_max_indices: Vec<usize>,
    suffix_max_indices: Vec<usize>,
    deque_indices: Vec<usize>,
    pending_peak: Option<PendingPeak>,
}

/// A threshold crossing whose peak-search window spans a buffer boundary;
/// the local-max search completes in the next buffer instead of being
/// truncated (which emitted early, never-revised peak positions).
struct PendingPeak {
    /// Window samples still to scan in the next buffer.
    remaining: usize,
    /// Best amplitude seen so far.
    amp: f32,
    /// Best position, relative to the NEXT buffer's start (negative while
    /// the best sample lies in an earlier buffer).
    rel: isize,
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
            crossing_indices: Vec::new(),
            window_max_indices: Vec::new(),
            suffix_max_indices: Vec::new(),
            deque_indices: Vec::new(),
            pending_peak: None,
        }
    }

    fn precompute_window_max_indices(&mut self, buffer: &[f32]) {
        let n = buffer.len();
        self.window_max_indices.resize(n, 0);
        if n == 0 {
            return;
        }

        let w = self.peak_search_window_samples.max(1).min(n);

        self.suffix_max_indices.resize(n, 0);
        self.suffix_max_indices[n - 1] = n - 1;
        for i in (0..(n - 1)).rev() {
            let next = self.suffix_max_indices[i + 1];
            self.suffix_max_indices[i] = if buffer[i] >= buffer[next] { i } else { next };
        }

        self.deque_indices.clear();
        let mut head = 0usize;

        for i in 0..n {
            let min_idx = i.saturating_add(1).saturating_sub(w);
            while head < self.deque_indices.len() && self.deque_indices[head] < min_idx {
                head += 1;
            }

            while self.deque_indices.len() > head {
                let back = *self
                    .deque_indices
                    .last()
                    .expect("deque should be non-empty when len > head");
                if buffer[back] < buffer[i] {
                    self.deque_indices.pop();
                } else {
                    break;
                }
            }
            self.deque_indices.push(i);

            if i + 1 >= w {
                let start = i + 1 - w;
                self.window_max_indices[start] = self.deque_indices[head];
            }
        }

        let full_window_limit = n.saturating_sub(w);
        for start in (full_window_limit + 1)..n {
            self.window_max_indices[start] = self.suffix_max_indices[start];
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
    /// Returns a vector of (sample_index, peak_amplitude) pairs. The index
    /// and amplitude correspond to the maximum positive value in a window
    /// after the threshold crossing. When a crossing lands near the end of
    /// a buffer its window completes in the NEXT call, and the peak is then
    /// reported with an index relative to that call's buffer — negative
    /// (bounded by the window length) if the maximum lay in the earlier
    /// buffer.
    ///
    /// # Arguments
    /// * `buffer` - Audio samples to process
    pub fn find_all_peaks(&mut self, buffer: &[f32]) -> Vec<(isize, f32)> {
        let n = buffer.len();
        let w = self.peak_search_window_samples.max(1);
        let mut peaks: Vec<(isize, f32)> = Vec::new();

        // Complete a window that spanned the previous buffer boundary.
        if let Some(mut pending) = self.pending_peak.take() {
            for (i, &candidate) in buffer[..pending.remaining.min(n)].iter().enumerate() {
                if candidate > pending.amp {
                    pending.amp = candidate;
                    pending.rel = i as isize;
                }
            }
            if pending.remaining > n {
                pending.remaining -= n;
                pending.rel -= n as isize;
                self.pending_peak = Some(pending);
            } else {
                peaks.push((pending.rel, pending.amp));
            }
        }

        self.crossing_indices.clear();
        for (i, &sample) in buffer.iter().enumerate() {
            if self.detect_peak(sample) {
                self.crossing_indices.push(i);
            }
        }

        // A crossing whose window extends past the buffer is deferred; the
        // detector's dead time guarantees at most one such crossing.
        if let Some(&start) = self.crossing_indices.last()
            && start + w > n
        {
            self.crossing_indices.pop();
            let mut amp = buffer[start];
            let mut rel = start as isize;
            for (i, &candidate) in buffer[start..].iter().enumerate() {
                if candidate > amp {
                    amp = candidate;
                    rel = (start + i) as isize;
                }
            }
            self.pending_peak = Some(PendingPeak {
                remaining: start + w - n,
                amp,
                rel: rel - n as isize,
            });
        }

        if self.crossing_indices.is_empty() {
            return peaks;
        }

        let estimated_rescan_work = self.crossing_indices.len().saturating_mul(w);
        if estimated_rescan_work <= n {
            for &start in &self.crossing_indices {
                let end = (start + w).min(n);
                let mut peak_idx = start;
                let mut peak_amp = buffer[start];
                for (rel_idx, &candidate) in buffer[start..end].iter().enumerate() {
                    if candidate > peak_amp {
                        peak_amp = candidate;
                        peak_idx = start + rel_idx;
                    }
                }
                peaks.push((peak_idx as isize, peak_amp));
            }
            return peaks;
        }

        self.precompute_window_max_indices(buffer);
        peaks.extend(self.crossing_indices.iter().copied().map(|start| {
            let peak_idx = self.window_max_indices[start];
            (peak_idx as isize, buffer[peak_idx])
        }));
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
    fn test_peak_window_completes_across_buffers() {
        // Regression test: a search window clipped at the buffer end used to
        // emit the truncated maximum immediately and never revise it.
        let mut whole = PeakDetector::with_peak_search_window(0.5, 10, 4);
        let mut split = PeakDetector::with_peak_search_window(0.5, 10, 4);

        // Crossing at 18, true maximum at 21 — beyond a boundary at 20.
        let mut signal = [0.0f32; 40];
        signal[18] = 0.6;
        signal[19] = 0.7;
        signal[20] = 0.8;
        signal[21] = 0.9;

        let whole_peaks = whole.find_all_peaks(&signal);
        assert_eq!(whole_peaks, vec![(21, 0.9)]);

        let first = split.find_all_peaks(&signal[..20]);
        assert!(first.is_empty(), "window must defer, got {:?}", first);
        let second = split.find_all_peaks(&signal[20..]);
        // Relative to the second buffer: global 21 = boundary 20 + 1.
        assert_eq!(second, vec![(1, 0.9)]);
    }

    #[test]
    fn test_peak_window_deferred_peak_can_resolve_into_previous_buffer() {
        let mut split = PeakDetector::with_peak_search_window(0.5, 10, 4);

        // Crossing at 17, true maximum at 18; window [17, 21) spans the
        // boundary at 20 but the maximum stays in the first buffer.
        let mut signal = [0.0f32; 40];
        signal[17] = 0.6;
        signal[18] = 0.9;
        signal[19] = 0.7;

        assert!(split.find_all_peaks(&signal[..20]).is_empty());
        let second = split.find_all_peaks(&signal[20..]);
        // Relative to the second buffer: global 18 = boundary 20 - 2.
        assert_eq!(second, vec![(-2, 0.9)]);
    }

    #[test]
    fn test_peak_detector_threshold() {
        let mut detector = PeakDetector::new(0.5, 5);

        let signal = vec![0.3, 0.4, 0.6, 0.7, 0.4, 0.2, 0.3, 0.4, 0.8, 0.3];

        let peaks = detector.find_all_peaks(&signal);

        // The first rising edge resolves within this buffer; the second
        // (index 8) has a search window extending past the end, so it is
        // deferred until the window completes in the next call.
        assert_eq!(peaks, vec![(3, 0.7)]);

        let continuation = vec![0.1, 0.0, 0.0, 0.0, 0.0];
        let peaks = detector.find_all_peaks(&continuation);
        // Relative to the continuation buffer: global 8 = boundary 10 - 2.
        assert_eq!(peaks, vec![(-2, 0.8)]);
    }
}
