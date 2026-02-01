use std::f32::consts::PI;

/// Calculate instantaneous phase from a zero-crossing sample index
///
/// Converts a sample position within a rotation period to a phase angle.
///
/// # Arguments
/// * `crossing_sample` - Sample index of the zero-crossing
/// * `samples_per_rotation` - Number of samples in one complete rotation
///
/// # Returns
/// Phase angle in radians (0 to 2π)
#[allow(dead_code)]
pub fn phase_from_crossing(crossing_sample: usize, samples_per_rotation: f32) -> f32 {
    let phase_radians = (crossing_sample as f32 / samples_per_rotation) * 2.0 * PI;
    phase_radians % (2.0 * PI)
}

/// Convert phase angle to bearing in degrees
///
/// Converts a phase angle in radians to a bearing angle in degrees,
/// normalized to the range 0-360°.
///
/// # Arguments
/// * `phase_radians` - Phase angle in radians
///
/// # Returns
/// Bearing angle in degrees (0-360)
pub fn phase_to_bearing(phase_radians: f32) -> f32 {
    let degrees = phase_radians.to_degrees();
    // Normalize to 0-360
    if degrees < 0.0 {
        degrees + 360.0
    } else {
        degrees % 360.0
    }
}

/// Calculate the difference between two phase angles with wrap-around
///
/// Computes the shortest angular difference between two phase angles,
/// handling wrap-around at 2π boundaries.
///
/// # Arguments
/// * `phase1` - First phase angle in radians
/// * `phase2` - Second phase angle in radians
///
/// # Returns
/// Phase difference in radians, wrapped to [-π, π]
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
