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
    fn test_moving_average() {
        let mut ma = MovingAverage::new(3);

        assert!((ma.add(1.0) - 1.0).abs() < 0.01);
        assert!((ma.add(2.0) - 1.5).abs() < 0.01);
        assert!((ma.add(3.0) - 2.0).abs() < 0.01);
        assert!((ma.add(4.0) - 3.0).abs() < 0.01); // (2+3+4)/3
        assert!((ma.add(5.0) - 4.0).abs() < 0.01); // (3+4+5)/3
    }
}
