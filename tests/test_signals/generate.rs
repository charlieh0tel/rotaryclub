use std::f32::consts::PI;

const NORTH_TICK_PULSE_WIDTH_RADIANS: f32 = 0.2;
const NORTH_TICK_AMPLITUDE: f32 = 0.8;

/// Generate synthetic RDF test signal with fixed bearing
/// Returns interleaved stereo samples [L, R, L, R, ...]
/// By default: Left = Doppler tone, Right = North tick
pub fn generate_test_signal(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    _doppler_tone_hz: f32,
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

        // Get bearing at this time
        let bearing_radians = bearing_fn(t).to_radians();

        // Calculate rotation phase for this sample
        let rotation_phase = (i as f32 / samples_per_rotation) * 2.0 * PI;

        // Left channel: Doppler tone at rotation frequency
        // The bearing determines the phase offset of the Doppler tone relative to north tick
        let doppler_phase = rotation_hz * t * 2.0 * PI - bearing_radians;
        let doppler = doppler_phase.sin();

        // Right channel: North tick pulse (sharp pulse at rotation start)
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

#[cfg(feature = "wav-export")]
pub fn save_wav(filename: &str, samples: &[f32], sample_rate: u32) -> std::io::Result<()> {
    use hound::{WavSpec, WavWriter};

    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut writer = WavWriter::create(filename, spec)?;

    for &sample in samples {
        writer.write_sample(sample)?;
    }

    writer.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_signal_length() {
        let signal = generate_test_signal(1.0, 48000, 500.0, 500.0, 0.0);
        // Should be 1 second * 48000 samples * 2 channels
        assert_eq!(signal.len(), 48000 * 2);
    }

    #[test]
    fn test_generate_signal_interleaved() {
        let signal = generate_test_signal(0.01, 48000, 500.0, 500.0, 0.0);

        // Check that we have interleaved stereo
        assert_eq!(signal.len() % 2, 0);

        // Left channel should be mostly non-zero (doppler tone)
        let left: Vec<f32> = signal.iter().step_by(2).copied().collect();
        let left_rms: f32 = (left.iter().map(|x| x * x).sum::<f32>() / left.len() as f32).sqrt();
        assert!(
            left_rms > 0.1,
            "Left channel should contain signal, got RMS {}",
            left_rms
        );

        // Right channel should have some pulses
        let right: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();
        let right_max = right.iter().fold(0.0f32, |a, &b| a.max(b));
        assert!(
            right_max > NORTH_TICK_AMPLITUDE * 0.5,
            "Right channel should have tick pulses"
        );
    }

    #[test]
    fn test_generate_multiple_bearings() {
        // Just verify no panics for various bearings
        for bearing in [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
            let signal = generate_test_signal(0.1, 48000, 500.0, 500.0, bearing);
            assert_eq!(signal.len(), 4800 * 2);
        }
    }
}
