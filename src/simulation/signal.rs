use std::f32::consts::PI;

pub const NORTH_TICK_PULSE_WIDTH_RADIANS: f32 = 0.2;
pub const NORTH_TICK_AMPLITUDE: f32 = 0.8;

/// Generate synthetic RDF test signal with fixed bearing
/// Returns interleaved stereo samples [L, R, L, R, ...]
/// Left = Doppler tone, Right = North tick
pub fn generate_test_signal(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    bearing_degrees: f32,
) -> Vec<f32> {
    generate_test_signal_with_bearing_fn(duration_secs, sample_rate, rotation_hz, |_| {
        bearing_degrees
    })
}

/// Generate synthetic RDF test signal with time-varying bearing
/// The bearing_fn takes time in seconds and returns bearing in degrees
pub fn generate_test_signal_with_bearing_fn<F>(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    bearing_fn: F,
) -> Vec<f32>
where
    F: Fn(f32) -> f32,
{
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    let mut samples = Vec::with_capacity(num_samples * 2);

    let samples_per_rotation = sample_rate as f32 / rotation_hz;

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;

        let bearing_radians = bearing_fn(t).to_radians();

        let rotation_phase = (i as f32 / samples_per_rotation) * 2.0 * PI;

        let doppler_phase = rotation_hz * t * 2.0 * PI - bearing_radians;
        let doppler = doppler_phase.sin();

        let tick_phase = rotation_phase % (2.0 * PI);
        let north_tick = if tick_phase < NORTH_TICK_PULSE_WIDTH_RADIANS {
            NORTH_TICK_AMPLITUDE
        } else {
            0.0
        };

        samples.push(doppler);
        samples.push(north_tick);
    }

    samples
}

/// Generate a pure Doppler signal for a given bearing (no north tick)
pub fn generate_doppler_signal_for_bearing(
    num_samples: usize,
    sample_rate: f32,
    rotation_hz: f32,
    bearing_degrees: f32,
) -> Vec<f32> {
    let bearing_radians = bearing_degrees.to_radians();
    let omega = 2.0 * PI * rotation_hz;

    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate;
            (omega * t - bearing_radians).sin()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_signal_length() {
        let signal = generate_test_signal(1.0, 48000, 500.0, 0.0);
        assert_eq!(signal.len(), 48000 * 2);
    }

    #[test]
    fn test_generate_signal_interleaved() {
        let signal = generate_test_signal(0.01, 48000, 500.0, 0.0);

        assert_eq!(signal.len() % 2, 0);

        let left: Vec<f32> = signal.iter().step_by(2).copied().collect();
        let left_rms: f32 = (left.iter().map(|x| x * x).sum::<f32>() / left.len() as f32).sqrt();
        assert!(
            left_rms > 0.1,
            "Left channel should contain signal, got RMS {}",
            left_rms
        );

        let right: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();
        let right_max = right.iter().fold(0.0f32, |a, &b| a.max(b));
        assert!(
            right_max > NORTH_TICK_AMPLITUDE * 0.5,
            "Right channel should have tick pulses"
        );
    }

    #[test]
    fn test_generate_multiple_bearings() {
        for bearing in [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
            let signal = generate_test_signal(0.1, 48000, 500.0, bearing);
            assert_eq!(signal.len(), 4800 * 2);
        }
    }
}
