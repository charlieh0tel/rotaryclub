use hound::{WavSpec, WavWriter};
use std::f32::consts::PI;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Generate Test WAV Files ===\n");

    let sample_rate = 48000;
    let rotation_hz = 500.0;
    let doppler_hz = 500.0;
    let duration = 5.0; // 5 seconds

    let test_bearings = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

    for &bearing in &test_bearings {
        let filename = format!("test_bearing_{:03.0}.wav", bearing);
        println!("Generating: {} (bearing: {}°)", filename, bearing);

        let signal = generate_test_signal(duration, sample_rate, rotation_hz, doppler_hz, bearing);
        save_wav(&filename, &signal, sample_rate)?;
    }

    println!("\nGenerated {} test files.", test_bearings.len());
    println!("You can now test with: cargo run --example play_wav_file <filename>");

    Ok(())
}

fn generate_test_signal(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    doppler_tone_hz: f32,
    bearing_degrees: f32,
) -> Vec<f32> {
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    let mut samples = Vec::with_capacity(num_samples * 2);

    let bearing_radians = bearing_degrees.to_radians();
    let samples_per_rotation = sample_rate as f32 / rotation_hz;

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;

        // Left channel: Doppler tone with phase shift based on bearing
        let rotation_phase = (i as f32 / samples_per_rotation) * 2.0 * PI;
        let phase_offset = rotation_phase + bearing_radians;
        let doppler = (doppler_tone_hz * t * 2.0 * PI + phase_offset).sin();

        // Right channel: North tick pulse (sharp pulse at rotation start)
        let tick_phase = rotation_phase % (2.0 * PI);
        let north_tick = if tick_phase < 0.05 { 0.8 } else { 0.0 };

        samples.push(doppler);
        samples.push(north_tick);
    }

    samples
}

fn save_wav(
    filename: &str,
    samples: &[f32],
    sample_rate: u32,
) -> Result<(), Box<dyn std::error::Error>> {
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
    println!(
        "  Wrote: {} ({:.1}s, {} Hz)",
        filename,
        samples.len() as f32 / 2.0 / sample_rate as f32,
        sample_rate
    );

    Ok(())
}
