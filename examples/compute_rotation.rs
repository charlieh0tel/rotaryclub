use hound::WavReader;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <wav_file>", args[0]);
        std::process::exit(1);
    }

    let filename = &args[1];
    let mut reader = WavReader::open(filename)?;
    let spec = reader.spec();

    println!("=== Rotation Interval Analysis ===");
    println!("File: {}", filename);
    println!("Sample rate: {} Hz\n", spec.sample_rate);

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

    // Extract RIGHT channel (north tick)
    let mut right: Vec<f32> = Vec::new();
    for chunk in samples.chunks_exact(2) {
        right.push(chunk[1]);
    }

    println!("Analyzing RIGHT channel (north tick)...\n");

    // Find peaks using threshold
    let threshold = 0.2;
    let peaks = find_peaks_with_spacing(&right, threshold, spec.sample_rate);

    if peaks.len() < 2 {
        println!("Error: Found {} peaks, need at least 2", peaks.len());
        return Ok(());
    }

    println!("Found {} peaks above threshold {}", peaks.len(), threshold);

    // Calculate intervals between consecutive peaks
    let mut intervals: Vec<f32> = peaks
        .windows(2)
        .map(|w| (w[1] - w[0]) as f32 / spec.sample_rate as f32)
        .collect();

    // Remove outliers (keep intervals within 1.5-2.5ms for ~500Hz rotation)
    intervals.retain(|&x| x > 0.0015 && x < 0.0025);

    if intervals.is_empty() {
        println!("Error: No valid intervals found");
        return Ok(());
    }

    // Statistics
    let avg_interval = intervals.iter().sum::<f32>() / intervals.len() as f32;
    let min_interval = intervals.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let max_interval = intervals.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));

    // Compute rotation frequency
    let rotation_hz = 1.0 / avg_interval;

    println!("\nRotation Interval Statistics:");
    println!("  Valid intervals: {}", intervals.len());
    println!(
        "  Average interval: {:.4} ms ({:.4} seconds)",
        avg_interval * 1000.0,
        avg_interval
    );
    println!("  Min interval: {:.4} ms", min_interval * 1000.0);
    println!("  Max interval: {:.4} ms", max_interval * 1000.0);
    println!("  Range: {:.4} ms", (max_interval - min_interval) * 1000.0);

    println!("\nRotation Frequency:");
    println!("  {:.2} Hz", rotation_hz);

    println!("\nRecommended Configuration:");
    println!("  expected_freq: {:.1}", rotation_hz);
    println!("  bandpass_low: {:.1}", (rotation_hz - 100.0).max(50.0));
    println!("  bandpass_high: {:.1}", rotation_hz + 100.0);
    println!("  min_interval_ms: {:.2}", avg_interval * 1000.0 * 0.9);

    Ok(())
}

fn find_peaks_with_spacing(signal: &[f32], threshold: f32, sample_rate: u32) -> Vec<usize> {
    let mut peaks = Vec::new();
    let mut was_below = true;
    let min_spacing = (sample_rate as f32 * 0.0015) as usize; // 1.5ms minimum
    let mut last_peak = 0;

    for (i, &sample) in signal.iter().enumerate() {
        if sample > threshold && was_below && (i - last_peak) >= min_spacing {
            peaks.push(i);
            last_peak = i;
            was_below = false;
        } else if sample < threshold * 0.5 {
            was_below = true;
        }
    }

    peaks
}
