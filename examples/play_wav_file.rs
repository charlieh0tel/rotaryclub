use hound::WavReader;
use rotaryclub::config::RdfConfig;
use rotaryclub::rdf::{ZeroCrossingBearingCalculator, NorthReferenceTracker};
use std::env;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} <wav_file>", args[0]);
        eprintln!("\nExample:");
        eprintln!("  cargo run --example play_wav_file test_bearing_045.wav");
        eprintln!("\nGenerate test files with:");
        eprintln!("  cargo run --example generate_test_wav");
        std::process::exit(1);
    }

    let filename = &args[1];

    println!("=== WAV File RDF Test ===");
    println!("File: {}\n", filename);

    // Open WAV file
    let mut reader = WavReader::open(filename)?;
    let spec = reader.spec();

    println!("WAV file info:");
    println!("  Sample rate: {} Hz", spec.sample_rate);
    println!("  Channels: {}", spec.channels);
    println!("  Bits per sample: {}", spec.bits_per_sample);
    println!(
        "  Duration: {:.2}s\n",
        reader.duration() as f32 / spec.sample_rate as f32
    );

    if spec.channels != 2 {
        eprintln!("Error: WAV file must be stereo (2 channels)");
        std::process::exit(1);
    }

    // Read all samples
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let max_val = 2_i32.pow(spec.bits_per_sample as u32 - 1) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max_val))
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    println!(
        "Read {} samples ({:.2}s)\n",
        samples.len(),
        samples.len() as f32 / 2.0 / spec.sample_rate as f32
    );

    // Initialize RDF configuration
    let mut config = RdfConfig::default();
    config.audio.sample_rate = spec.sample_rate;

    println!("RDF Configuration:");
    println!("  Doppler channel: {:?}", config.audio.doppler_channel);
    println!(
        "  North tick channel: {:?}",
        config.audio.north_tick_channel
    );
    println!(
        "  Doppler bandpass: {}-{} Hz",
        config.doppler.bandpass_low, config.doppler.bandpass_high
    );
    println!("  Expected rotation: {} Hz\n", config.doppler.expected_freq);

    // Process the signal
    println!("Processing...\n");

    let sample_rate = spec.sample_rate as f32;
    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;
    let mut bearing_calc = ZeroCrossingBearingCalculator::new(
        &config.doppler,
        &config.agc,
        sample_rate,
        config.bearing.smoothing_window,
    )?;

    let chunk_size = config.audio.buffer_size * 2; // stereo samples
    let output_interval = Duration::from_secs_f32(1.0 / config.bearing.output_rate_hz);
    let mut last_output = Instant::now();

    println!(
        "{:<10} {:<15} {:<15} {:<10}",
        "Time (s)", "Bearing (°)", "Raw Bearing (°)", "Confidence"
    );
    println!("{}", "-".repeat(55));

    let mut sample_count = 0;
    let mut bearing_measurements = Vec::new();

    for chunk in samples.chunks(chunk_size) {
        // Convert to stereo pairs
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();

        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        // Process north tick
        let ticks = north_tracker.process_buffer(&north_tick);

        // Process doppler with each tick
        for tick in ticks {
            if let Some(bearing) = bearing_calc.process_buffer(&doppler, &tick) {
                let timestamp = sample_count as f32 / sample_rate;

                // Store for statistics
                bearing_measurements.push(bearing.bearing_degrees);

                // Throttled output
                if last_output.elapsed() >= output_interval {
                    println!(
                        "{:<10.2} {:<15.1} {:<15.1} {:<10.2}",
                        timestamp, bearing.bearing_degrees, bearing.raw_bearing, bearing.confidence
                    );
                    last_output = Instant::now();
                }
            }
        }

        sample_count += chunk.len() / 2;
    }

    // Print statistics
    if !bearing_measurements.is_empty() {
        println!("\n{}", "=".repeat(55));
        println!("Statistics:");

        let avg = bearing_measurements.iter().sum::<f32>() / bearing_measurements.len() as f32;

        let variance = bearing_measurements
            .iter()
            .map(|x| (x - avg).powi(2))
            .sum::<f32>()
            / bearing_measurements.len() as f32;
        let std_dev = variance.sqrt();

        let min = bearing_measurements
            .iter()
            .fold(f32::INFINITY, |a, &b| a.min(b));
        let max = bearing_measurements
            .iter()
            .fold(f32::NEG_INFINITY, |a, &b| a.max(b));

        println!("  Measurements: {}", bearing_measurements.len());
        println!("  Average bearing: {:.1}°", avg);
        println!("  Std deviation: {:.1}°", std_dev);
        println!("  Min: {:.1}°", min);
        println!("  Max: {:.1}°", max);
        println!("  Range: {:.1}°", max - min);

        // Check rotation frequency
        if let Some(freq) = north_tracker.rotation_frequency() {
            println!("\nDetected rotation frequency: {:.1} Hz", freq);
        }
    } else {
        println!("\nNo bearing measurements obtained!");
        println!("Check that:");
        println!("  - North tick channel has pulses");
        println!("  - Doppler channel has signal");
        println!("  - Channel assignment is correct");
    }

    Ok(())
}
