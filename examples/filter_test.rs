use rotaryclub::signal_processing::{Filter, FirBandpass, FirHighpass};
use std::f32::consts::PI;

fn main() -> anyhow::Result<()> {
    println!("=== Filter Frequency Response Test ===\n");

    let sample_rate = 48000.0;

    // Test FIR bandpass filter
    println!("FIR Bandpass Filter (400-600 Hz, 127 taps):");
    let mut fir_bandpass = FirBandpass::new(400.0, 600.0, sample_rate, 127, 100.0)?;
    test_bandpass(&mut fir_bandpass, sample_rate);

    // Test FIR highpass filter
    println!("\nFIR Highpass Filter (2000 Hz, 63 taps):");
    let mut highpass = FirHighpass::new(2000.0, sample_rate, 63, 500.0)?;
    test_highpass(&mut highpass, sample_rate);

    println!("\nFilter test complete.");
    Ok(())
}

fn test_bandpass<F: Filter>(filter: &mut F, sample_rate: f32) {
    println!(
        "{:<10} {:<15} {:<15}",
        "Freq (Hz)", "Attenuation (dB)", "Status"
    );
    println!("{}", "-".repeat(45));

    for freq in [
        100.0, 200.0, 300.0, 400.0, 450.0, 500.0, 550.0, 600.0, 700.0, 800.0, 1000.0,
    ] {
        let attenuation = test_frequency(filter, freq, sample_rate);
        let status = if (400.0..=600.0).contains(&freq) {
            if attenuation > -3.0 {
                "PASS"
            } else {
                "FAIL (too attenuated)"
            }
        } else if attenuation < -20.0 {
            "PASS"
        } else {
            "FAIL (not attenuated)"
        };
        println!("{:<10.1} {:<15.2} {:<15}", freq, attenuation, status);
    }
}

fn test_highpass<F: Filter>(filter: &mut F, sample_rate: f32) {
    println!(
        "{:<10} {:<15} {:<15}",
        "Freq (Hz)", "Attenuation (dB)", "Status"
    );
    println!("{}", "-".repeat(45));

    for freq in [
        100.0, 500.0, 1000.0, 1500.0, 2000.0, 3000.0, 5000.0, 10000.0,
    ] {
        let attenuation = test_frequency(filter, freq, sample_rate);
        let status = if freq < 2000.0 {
            if attenuation < -10.0 {
                "PASS"
            } else {
                "FAIL (not attenuated)"
            }
        } else if attenuation > -3.0 {
            "PASS"
        } else {
            "FAIL (too attenuated)"
        };
        println!("{:<10.1} {:<15.2} {:<15}", freq, attenuation, status);
    }
}

fn test_frequency<F: Filter>(filter: &mut F, freq: f32, sample_rate: f32) -> f32 {
    let num_samples = 4800;
    let input: Vec<f32> = (0..num_samples)
        .map(|i| (2.0 * PI * freq * i as f32 / sample_rate).sin())
        .collect();

    let mut output = input.clone();
    filter.process_buffer(&mut output);

    let skip = 1000;
    let input_rms: f32 =
        input.iter().skip(skip).map(|x| x * x).sum::<f32>().sqrt() / (input.len() - skip) as f32;
    let output_rms: f32 =
        output.iter().skip(skip).map(|x| x * x).sum::<f32>().sqrt() / (output.len() - skip) as f32;

    20.0 * (output_rms / input_rms).log10()
}
