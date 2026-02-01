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

    println!("=== Channel Analysis ===");
    println!("File: {}", filename);
    println!("Sample rate: {} Hz", spec.sample_rate);
    println!("Channels: {}\n", spec.channels);

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

    // Analyze first 5 seconds
    let analyze_samples = (spec.sample_rate * 5).min(samples.len() as u32 / 2) as usize;

    let mut left: Vec<f32> = Vec::new();
    let mut right: Vec<f32> = Vec::new();

    for chunk in samples[..analyze_samples * 2].chunks_exact(2) {
        left.push(chunk[0]);
        right.push(chunk[1]);
    }

    println!(
        "Analyzing first {:.1} seconds...\n",
        analyze_samples as f32 / spec.sample_rate as f32
    );

    // Calculate RMS
    let left_rms = (left.iter().map(|x| x * x).sum::<f32>() / left.len() as f32).sqrt();
    let right_rms = (right.iter().map(|x| x * x).sum::<f32>() / right.len() as f32).sqrt();

    // Calculate peak
    let left_peak = left.iter().map(|x| x.abs()).fold(0.0f32, |a, b| a.max(b));
    let right_peak = right.iter().map(|x| x.abs()).fold(0.0f32, |a, b| a.max(b));

    // Count zero crossings (rough frequency check)
    let left_crossings = count_crossings(&left);
    let right_crossings = count_crossings(&right);

    // Count pulses (rapid changes above threshold)
    let left_pulses = count_pulses(&left, 0.3);
    let right_pulses = count_pulses(&right, 0.3);

    println!("LEFT Channel:");
    println!("  RMS: {:.4}", left_rms);
    println!("  Peak: {:.4}", left_peak);
    println!(
        "  Zero crossings: {} ({:.1} Hz apparent)",
        left_crossings,
        left_crossings as f32 / 5.0
    );
    println!("  Pulses detected: {}\n", left_pulses);

    println!("RIGHT Channel:");
    println!("  RMS: {:.4}", right_rms);
    println!("  Peak: {:.4}", right_peak);
    println!(
        "  Zero crossings: {} ({:.1} Hz apparent)",
        right_crossings,
        right_crossings as f32 / 5.0
    );
    println!("  Pulses detected: {}\n", right_pulses);

    println!("Analysis:");

    // Doppler tone should have:
    // - Higher RMS (continuous signal)
    // - Many zero crossings (~500-600 Hz)
    // - Fewer distinct pulses

    // North tick should have:
    // - Lower RMS (sparse pulses)
    // - Fewer zero crossings
    // - Clear pulse count (~500-600 pulses in 5 seconds)

    if left_crossings > right_crossings * 2 && left_rms > right_rms {
        println!(
            "  LEFT appears to be DOPPLER TONE (continuous ~{}Hz signal)",
            left_crossings / 5
        );
        println!("  RIGHT appears to be NORTH TICK (pulse train)");
        println!("\n  Current config: Doppler=Left, NorthTick=Right ✓ CORRECT");
    } else if right_crossings > left_crossings * 2 && right_rms > left_rms {
        println!(
            "  RIGHT appears to be DOPPLER TONE (continuous ~{}Hz signal)",
            right_crossings / 5
        );
        println!("  LEFT appears to be NORTH TICK (pulse train)");
        println!("\n  Current config: Doppler=Left, NorthTick=Right ✗ SWAPPED!");
        println!("  Change to: Doppler=Right, NorthTick=Left");
    } else {
        println!("  Unable to clearly identify channel roles");
        println!("  Both channels have similar characteristics");
    }

    Ok(())
}

fn count_crossings(signal: &[f32]) -> usize {
    signal
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] > 0.0)
        .count()
}

fn count_pulses(signal: &[f32], threshold: f32) -> usize {
    let mut count = 0;
    let mut in_pulse = false;

    for &sample in signal {
        if sample.abs() > threshold {
            if !in_pulse {
                count += 1;
                in_pulse = true;
            }
        } else {
            in_pulse = false;
        }
    }

    count
}
