use crossbeam_channel::bounded;
use rotaryclub::audio::AudioCapture;
use rotaryclub::config::RdfConfig;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    println!("=== Audio Loopback Test ===");
    println!("This example captures audio and displays RMS levels for each channel.");
    println!("Press Ctrl+C to stop.\n");

    let config = RdfConfig::default();

    println!("Configuration:");
    println!("  Sample rate: {} Hz", config.audio.sample_rate);
    println!("  Buffer size: {} samples", config.audio.buffer_size);
    println!("  Channels: {}", config.audio.channels);
    println!("  Doppler channel: {:?}", config.audio.doppler_channel);
    println!(
        "  North tick channel: {:?}\n",
        config.audio.north_tick_channel
    );

    // Create channel for audio data
    let (audio_tx, audio_rx) = bounded(10);

    // Start audio capture
    println!("Starting audio capture...\n");
    let _capture = AudioCapture::new(&config.audio, audio_tx, None)?;

    println!("Capturing audio... (Ctrl+C to stop)");
    println!("{:<10} {:<10} {:<10}", "Time (s)", "Left RMS", "Right RMS");
    println!("{}", "-".repeat(35));

    let start = std::time::Instant::now();

    loop {
        let data = match audio_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(data) => data,
            Err(_) => continue,
        };

        // Calculate RMS for each channel
        let mut left_sum = 0.0;
        let mut right_sum = 0.0;
        let mut count = 0;

        for chunk in data.chunks_exact(2) {
            left_sum += chunk[0] * chunk[0];
            right_sum += chunk[1] * chunk[1];
            count += 1;
        }

        let left_rms = if count > 0 {
            (left_sum / count as f32).sqrt()
        } else {
            0.0
        };

        let right_rms = if count > 0 {
            (right_sum / count as f32).sqrt()
        } else {
            0.0
        };

        let elapsed = start.elapsed().as_secs_f32();

        println!("{:<10.2} {:<10.4} {:<10.4}", elapsed, left_rms, right_rms);

        std::thread::sleep(Duration::from_millis(100));
    }
}
