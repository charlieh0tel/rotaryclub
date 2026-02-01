use rotaryclub::signal_processing::{BandpassFilter, HighpassFilter};
use std::f32::consts::PI;

fn main() -> anyhow::Result<()> {
    println!("=== Filter Frequency Response Test ===\n");

    let sample_rate = 48000.0;

    // Test bandpass filter (400-600 Hz for Doppler tone)
    println!("Bandpass Filter (400-600 Hz, Order 4):");
    println!(
        "{:<10} {:<15} {:<15}",
        "Freq (Hz)", "Attenuation (dB)", "Status"
    );
    println!("{}", "-".repeat(45));

    let mut bandpass = BandpassFilter::new(400.0, 600.0, sample_rate, 4)?;

    for freq in [
        100.0, 200.0, 300.0, 400.0, 450.0, 500.0, 550.0, 600.0, 700.0, 800.0, 1000.0,
    ] {
        let attenuation = test_frequency(&mut bandpass, freq, sample_rate);
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

    println!("\nHighpass Filter (2000 Hz, Order 2):");
    println!(
        "{:<10} {:<15} {:<15}",
        "Freq (Hz)", "Attenuation (dB)", "Status"
    );
    println!("{}", "-".repeat(45));

    let mut highpass = HighpassFilter::new(2000.0, sample_rate, 2)?;

    for freq in [
        100.0, 500.0, 1000.0, 1500.0, 2000.0, 3000.0, 5000.0, 10000.0,
    ] {
        let attenuation = test_frequency(&mut highpass, freq, sample_rate);
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

    println!("\nFilter test complete.");

    Ok(())
}

fn test_frequency<F>(filter: &mut F, freq: f32, sample_rate: f32) -> f32
where
    F: Filter,
{
    // Generate sine wave at test frequency
    let num_samples = 4800;
    let input: Vec<f32> = (0..num_samples)
        .map(|i| (2.0 * PI * freq * i as f32 / sample_rate).sin())
        .collect();

    let mut output = input.clone();
    filter.process_buffer(&mut output);

    // Calculate RMS of input and output (skip initial transient)
    let skip = 1000;
    let input_rms: f32 =
        input.iter().skip(skip).map(|x| x * x).sum::<f32>().sqrt() / (input.len() - skip) as f32;

    let output_rms: f32 =
        output.iter().skip(skip).map(|x| x * x).sum::<f32>().sqrt() / (output.len() - skip) as f32;

    // Calculate attenuation in dB
    20.0 * (output_rms / input_rms).log10()
}

trait Filter {
    fn process_buffer(&mut self, buffer: &mut [f32]);
}

impl Filter for BandpassFilter {
    fn process_buffer(&mut self, buffer: &mut [f32]) {
        BandpassFilter::process_buffer(self, buffer);
    }
}

impl Filter for HighpassFilter {
    fn process_buffer(&mut self, buffer: &mut [f32]) {
        HighpassFilter::process_buffer(self, buffer);
    }
}
