use std::f32::consts::PI;

pub const NORTH_TICK_PULSE_WIDTH_RADIANS: f32 = 0.2;
pub const NORTH_TICK_AMPLITUDE: f32 = 0.8;

/// Half-width, in samples, of the synthesized north pulse.
const NORTH_TICK_HALF_WIDTH_SAMPLES: i64 = 12;

/// A band-limited impulse at a fractional sample position.
///
/// The hardware pulse is ~20 us, well under one sample at 48 kHz, so what an
/// anti-aliased converter records is a band-limited impulse centred on the
/// arrival time rather than a sample that happens to be non-zero. Gating on
/// rotation phase instead would light the first sample at or after the
/// rotation boundary -- ceil(epoch) -- which lags the truth by half a sample
/// on average and shows up downstream as a fixed 6 degrees of bearing.
fn add_north_pulse(channel: &mut [f32], epoch: f64, amplitude: f32) {
    let center = epoch.round() as i64;
    for n in (center - NORTH_TICK_HALF_WIDTH_SAMPLES)..=(center + NORTH_TICK_HALF_WIDTH_SAMPLES) {
        if n < 0 || n as usize >= channel.len() {
            continue;
        }
        let x = n as f64 - epoch;
        let value = if x.abs() < f64::EPSILON {
            1.0
        } else {
            let px = std::f64::consts::PI * x;
            let window = px / NORTH_TICK_HALF_WIDTH_SAMPLES as f64;
            (px.sin() / px) * (window.sin() / window)
        };
        channel[n as usize] += amplitude * value as f32;
    }
}

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
    let samples_per_rotation = sample_rate as f64 / rotation_hz as f64;

    let mut north = vec![0.0f32; num_samples];
    let mut rotation = 0i64;
    loop {
        let epoch = rotation as f64 * samples_per_rotation;
        if epoch >= num_samples as f64 {
            break;
        }
        add_north_pulse(&mut north, epoch, NORTH_TICK_AMPLITUDE);
        rotation += 1;
    }

    let mut samples = Vec::with_capacity(num_samples * 2);
    for (i, &north_tick) in north.iter().enumerate() {
        let t = i as f32 / sample_rate as f32;
        let bearing_radians = bearing_fn(t).to_radians();
        let doppler_phase = rotation_hz * t * 2.0 * PI - bearing_radians;
        samples.push(doppler_phase.sin());
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
