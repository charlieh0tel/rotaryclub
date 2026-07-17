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
    let print_interval = Duration::from_millis(100);
    let mut last_print = std::time::Instant::now();

    // Drain every buffer as it arrives (a lagging consumer forces the
    // capture side to drop chunks) and throttle only the printing,
    // accumulating RMS over each print interval.
    let mut left_sum = 0.0f32;
    let mut right_sum = 0.0f32;
    let mut count = 0usize;

    loop {
        match audio_rx.recv_timeout(print_interval) {
            Ok(Ok(data)) => {
                for chunk in data.chunks_exact(2) {
                    left_sum += chunk[0] * chunk[0];
                    right_sum += chunk[1] * chunk[1];
                    count += 1;
                }
            }
            Ok(Err(e)) => {
                eprintln!("Audio stream error: {}", e);
                break Ok(());
            }
            Err(_) => {}
        }

        if count > 0 && last_print.elapsed() >= print_interval {
            let left_rms = (left_sum / count as f32).sqrt();
            let right_rms = (right_sum / count as f32).sqrt();
            let elapsed = start.elapsed().as_secs_f32();
            println!("{:<10.2} {:<10.4} {:<10.4}", elapsed, left_rms, right_rms);
            left_sum = 0.0;
            right_sum = 0.0;
            count = 0;
            last_print = std::time::Instant::now();
        }
    }
}
