use std::f32::consts::PI;

/// Calculate instantaneous phase from zero-crossing
#[allow(dead_code)]
pub fn phase_from_crossing(crossing_sample: usize, samples_per_rotation: f32) -> f32 {
    let phase_radians = (crossing_sample as f32 / samples_per_rotation) * 2.0 * PI;
    phase_radians % (2.0 * PI)
}

/// Convert phase offset to bearing angle (0-360 degrees)
pub fn phase_to_bearing(phase_radians: f32) -> f32 {
    let degrees = phase_radians.to_degrees();
    // Normalize to 0-360
    if degrees < 0.0 {
        degrees + 360.0
    } else {
        degrees % 360.0
    }
}

/// Calculate phase difference with wrap-around handling
#[allow(dead_code)]
pub fn phase_difference(phase1: f32, phase2: f32) -> f32 {
    let diff = phase1 - phase2;
    // Wrap to [-PI, PI]
    if diff > PI {
        diff - 2.0 * PI
    } else if diff < -PI {
        diff + 2.0 * PI
    } else {
        diff
    }
}

/// Simple moving average filter
pub struct MovingAverage {
    buffer: Vec<f32>,
    index: usize,
    filled: bool,
}

impl MovingAverage {
    pub fn new(window_size: usize) -> Self {
        Self {
            buffer: vec![0.0; window_size],
            index: 0,
            filled: false,
        }
    }

    pub fn add(&mut self, value: f32) -> f32 {
        self.buffer[self.index] = value;
        self.index = (self.index + 1) % self.buffer.len();

        if self.index == 0 {
            self.filled = true;
        }

        self.average()
    }

    pub fn average(&self) -> f32 {
        let sum: f32 = self.buffer.iter().sum();
        let count = if self.filled {
            self.buffer.len()
        } else {
            self.index.max(1)
        };
        sum / count as f32
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
        self.filled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_to_bearing() {
        assert!((phase_to_bearing(0.0) - 0.0).abs() < 0.01);
        assert!((phase_to_bearing(PI / 2.0) - 90.0).abs() < 0.01);
        assert!((phase_to_bearing(PI) - 180.0).abs() < 0.01);
        assert!((phase_to_bearing(3.0 * PI / 2.0) - 270.0).abs() < 0.01);
    }

    #[test]
    fn test_phase_difference() {
        // Normal case
        assert!((phase_difference(1.0, 0.5) - 0.5).abs() < 0.01);

        // Wrap around positive
        assert!((phase_difference(0.1, 6.0) - (0.1 - 6.0 + 2.0 * PI)).abs() < 0.01);

        // Wrap around negative
        assert!((phase_difference(6.0, 0.1) - (6.0 - 0.1 - 2.0 * PI)).abs() < 0.01);
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

    #[test]
    fn test_phase_from_crossing() {
        let samples_per_rotation = 96.0; // 48kHz / 500Hz

        // Quarter rotation
        let phase = phase_from_crossing(24, samples_per_rotation);
        assert!((phase - PI / 2.0).abs() < 0.01);

        // Half rotation
        let phase = phase_from_crossing(48, samples_per_rotation);
        assert!((phase - PI).abs() < 0.01);
    }
}
