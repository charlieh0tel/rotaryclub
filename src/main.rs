use crossbeam_channel::bounded;
use std::time::{Duration, Instant};

mod audio;
mod config;
mod error;
mod rdf;
mod signal_processing;

use audio::{AudioCapture, AudioRingBuffer};
use config::RdfConfig;
use rdf::{BearingCalculator, NorthReferenceTracker};

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let config = RdfConfig::default();

    println!("=== Rotary Club - Pseudo Doppler RDF ===");
    println!("Sample rate: {} Hz", config.audio.sample_rate);
    println!("Expected rotation: {} Hz", config.doppler.expected_freq);
    println!(
        "Doppler bandpass: {}-{} Hz",
        config.doppler.bandpass_low, config.doppler.bandpass_high
    );
    println!("North tick threshold: {}", config.north_tick.threshold);
    println!("Output rate: {} Hz", config.bearing.output_rate_hz);
    println!(
        "Channel assignment: Doppler={:?}, North tick={:?}",
        config.audio.doppler_channel, config.audio.north_tick_channel
    );
    println!();

    let (audio_tx, audio_rx) = bounded(10);

    println!("Starting audio capture...");
    let _capture = AudioCapture::new(&config.audio, audio_tx)?;

    println!("Audio capture started. Processing...\n");

    run_processing_loop(audio_rx, config)?;

    Ok(())
}

fn run_processing_loop(
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    config: RdfConfig,
) -> anyhow::Result<()> {
    let sample_rate = config.audio.sample_rate as f32;

    // Initialize processing components
    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;

    let mut bearing_calc = BearingCalculator::new(
        &config.doppler,
        sample_rate,
        config.bearing.smoothing_window,
    )?;

    let mut ring_buffer = AudioRingBuffer::new();
    let mut last_output = Instant::now();
    let output_interval = Duration::from_secs_f32(1.0 / config.bearing.output_rate_hz);

    let mut last_north_tick: Option<rdf::NorthTick> = None;

    loop {
        // Receive audio data (blocking)
        let audio_data = match audio_rx.recv() {
            Ok(data) => data,
            Err(_) => {
                eprintln!("Audio stream closed");
                break;
            }
        };

        ring_buffer.push_interleaved(&audio_data);

        let samples = ring_buffer.latest(audio_data.len() / 2);
        let stereo_pairs: Vec<(f32, f32)> = samples.iter().map(|s| (s.left, s.right)).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo_pairs);

        let north_ticks = north_tracker.process_buffer(&north_tick);

        if let Some(tick) = north_ticks.last() {
            last_north_tick = Some(*tick);

            if let Some(freq) = north_tracker.rotation_frequency() {
                log::debug!("Rotation detected: {:.1} Hz", freq);
            }
        }

        if let Some(ref tick) = last_north_tick {
            if let Some(bearing) = bearing_calc.process_buffer(&doppler, tick) {
                // Throttle output
                if last_output.elapsed() >= output_interval {
                    println!(
                        "Bearing: {:>6.1}° (raw: {:>6.1}°) confidence: {:.2}",
                        bearing.bearing_degrees, bearing.raw_bearing, bearing.confidence
                    );
                    last_output = Instant::now();
                }
            }
        } else {
            // Only print warning occasionally to avoid spam
            if last_output.elapsed() >= Duration::from_secs(2) {
                log::warn!("Waiting for north tick...");
                last_output = Instant::now();
            }
        }
    }

    Ok(())
}
