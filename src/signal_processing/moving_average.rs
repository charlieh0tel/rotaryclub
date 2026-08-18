/// Simple moving average filter for signal smoothing
///
/// Computes the arithmetic mean of the last N values in a sliding window.
/// Used to smooth bearing measurements and reduce noise in the output.
///
/// The filter maintains a circular buffer and updates incrementally, making
/// it efficient for real-time processing.
pub struct MovingAverage {
    buffer: Vec<f32>,
    index: usize,
    filled: bool,
}

impl MovingAverage {
    /// Create a new moving average filter
    ///
    /// # Arguments
    /// * `window_size` - Number of samples to average (larger = smoother but slower response)
    pub fn new(window_size: usize) -> Self {
        Self {
            buffer: vec![0.0; window_size],
            index: 0,
            filled: false,
        }
    }

    /// Add a new value to the moving average and return the updated average
    ///
    /// Adds the value to the circular buffer and returns the current average
    /// of all values in the window.
    ///
    /// # Arguments
    /// * `value` - New value to add to the window
    ///
    /// # Returns
    /// Current moving average after adding the new value
    pub fn add(&mut self, value: f32) -> f32 {
        self.buffer[self.index] = value;
        self.index = (self.index + 1) % self.buffer.len();

        if self.index == 0 {
            self.filled = true;
        }

        self.average()
    }

    /// Overwrite the most recently added value and return the updated
    /// average. Falls back to `add` when the window is still empty.
    ///
    /// This is what lets a caller hold one window slot per independent
    /// observation while still tracking the newest measurement: revisions
    /// of the same observation replace their slot instead of consuming
    /// the window.
    pub fn replace_last(&mut self, value: f32) -> f32 {
        if !self.filled && self.index == 0 {
            return self.add(value);
        }
        let last = (self.index + self.buffer.len() - 1) % self.buffer.len();
        self.buffer[last] = value;
        self.average()
    }

    /// Get the current average without adding a new value
    ///
    /// Returns the mean of all values currently in the window.
    pub fn average(&self) -> f32 {
        let sum: f32 = self.buffer.iter().sum();
        let count = if self.filled {
            self.buffer.len()
        } else {
            self.index.max(1)
        };
        sum / count as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_last_revises_without_consuming_the_window() {
        let mut ma = MovingAverage::new(3);
        // Empty window: replace_last degrades to add.
        assert!((ma.replace_last(1.0) - 1.0).abs() < 0.01);
        // Revisions overwrite the newest slot; the older entry survives.
        assert!((ma.replace_last(3.0) - 3.0).abs() < 0.01);
        assert!((ma.add(9.0) - 6.0).abs() < 0.01); // (3+9)/2
        assert!((ma.replace_last(3.0) - 3.0).abs() < 0.01); // (3+3)/2
    }

    #[test]
    fn test_moving_average() {
        let mut ma = MovingAverage::new(3);

        assert!((ma.add(1.0) - 1.0).abs() < 0.01);
        assert!((ma.add(2.0) - 1.5).abs() < 0.01);
        assert!((ma.add(3.0) - 2.0).abs() < 0.01);
        assert!((ma.add(4.0) - 3.0).abs() < 0.01); // (2+3+4)/3
        assert!((ma.add(5.0) - 4.0).abs() < 0.01); // (3+4+5)/3
    }
}
