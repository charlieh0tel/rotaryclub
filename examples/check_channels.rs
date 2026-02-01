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

    println!("=== Channel Signal Check ===\n");

    // Sample first 0.1 seconds
    let sample_count = (spec.sample_rate as f32 * 0.1) as usize;

    let mut left: Vec<f32> = Vec::new();
    let mut right: Vec<f32> = Vec::new();

    for chunk in samples[..sample_count.min(samples.len() / 2) * 2].chunks_exact(2) {
        left.push(chunk[0]);
        right.push(chunk[1]);
    }

    println!("First 100ms sample (showing every 100th sample):");
    println!("\n{:<10} {:<15} {:<15}", "Time(ms)", "LEFT", "RIGHT");
    println!("{}", "-".repeat(40));

    for i in (0..left.len()).step_by(100) {
        let time_ms = i as f32 / spec.sample_rate as f32 * 1000.0;
        println!("{:<10.2} {:<15.4} {:<15.4}", time_ms, left[i], right[i]);
    }

    // Check for periodic pulses (north tick characteristics)
    println!("\n\nPulse pattern analysis (first 1 second):");
    let one_sec = spec.sample_rate as usize;

    let left_peaks = find_peaks(&left[..one_sec.min(left.len())], 0.5);
    let right_peaks = find_peaks(&right[..one_sec.min(right.len())], 0.3);

    println!("\nLEFT channel peaks (>0.5): {}", left_peaks.len());
    if !left_peaks.is_empty() {
        println!(
            "  Intervals between peaks (ms): {:?}",
            left_peaks
                .windows(2)
                .take(10)
                .map(|w| (w[1] - w[0]) as f32 / spec.sample_rate as f32 * 1000.0)
                .collect::<Vec<_>>()
        );
    }

    println!("\nRIGHT channel peaks (>0.3): {}", right_peaks.len());
    if !right_peaks.is_empty() {
        println!(
            "  Intervals between peaks (ms): {:?}",
            right_peaks
                .windows(2)
                .take(10)
                .map(|w| (w[1] - w[0]) as f32 / spec.sample_rate as f32 * 1000.0)
                .collect::<Vec<_>>()
        );
    }

    println!("\n\nInterpretation:");

    // North tick at 534 Hz = ~1.87ms intervals
    // Doppler tone = continuous ~534 Hz oscillation

    let left_avg_interval = if left_peaks.len() > 1 {
        let intervals: Vec<f32> = left_peaks
            .windows(2)
            .map(|w| (w[1] - w[0]) as f32 / spec.sample_rate as f32 * 1000.0)
            .collect();
        intervals.iter().sum::<f32>() / intervals.len() as f32
    } else {
        0.0
    };

    let right_avg_interval = if right_peaks.len() > 1 {
        let intervals: Vec<f32> = right_peaks
            .windows(2)
            .map(|w| (w[1] - w[0]) as f32 / spec.sample_rate as f32 * 1000.0)
            .collect();
        intervals.iter().sum::<f32>() / intervals.len() as f32
    } else {
        0.0
    };

    println!("LEFT avg peak interval: {:.2}ms", left_avg_interval);
    println!("RIGHT avg peak interval: {:.2}ms", right_avg_interval);

    if left_avg_interval > 1.5 && left_avg_interval < 2.5 && left_peaks.len() > 400 {
        println!("\n→ LEFT looks like NORTH TICK (regular ~1.87ms pulses)");
        println!("→ RIGHT should be DOPPLER TONE");
        println!("\n✗ Channels are SWAPPED from current config!");
        println!("  Change to: Doppler=Right, NorthTick=Left");
    } else if right_avg_interval > 1.5 && right_avg_interval < 2.5 && right_peaks.len() > 400 {
        println!("\n→ RIGHT looks like NORTH TICK (regular ~1.87ms pulses)");
        println!("→ LEFT should be DOPPLER TONE");
        println!("\n✓ Current config appears CORRECT");
    } else {
        println!("\n? Signal pattern unclear, both channels complex");
    }

    Ok(())
}

fn find_peaks(signal: &[f32], threshold: f32) -> Vec<usize> {
    let mut peaks = Vec::new();
    let mut was_below = true;

    for (i, &sample) in signal.iter().enumerate() {
        if sample > threshold && was_below {
            peaks.push(i);
            was_below = false;
        } else if sample < threshold / 2.0 {
            was_below = true;
        }
    }

    peaks
}
