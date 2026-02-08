use rotaryclub::config::RdfConfig;
use rotaryclub::processing::RdfProcessor;
use rotaryclub::simulation::{circular_mean_degrees, generate_test_signal};

fn main() -> anyhow::Result<()> {
    println!("=== Synthetic RDF Signal Test ===\n");

    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate;
    let rotation_hz = config.doppler.expected_freq;

    // Test multiple bearings
    let test_bearings = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

    println!("Testing bearing calculation with synthetic signals...\n");
    println!(
        "{:<15} {:<15} {:<15} {:<15}",
        "Expected (°)", "Measured (°)", "Error (°)", "Status"
    );
    println!("{}", "-".repeat(65));

    for &expected_bearing in &test_bearings {
        let signal = generate_test_signal(0.5, sample_rate, rotation_hz, expected_bearing);

        let mut processor = RdfProcessor::new(&config, false, true)?;
        let results = processor.process_signal(&signal);
        let measurements: Vec<f32> = results
            .iter()
            .filter_map(|r| r.bearing.map(|b| b.bearing_degrees))
            .collect();

        let avg = if measurements.len() > 5 {
            circular_mean_degrees(&measurements[3..])
        } else {
            circular_mean_degrees(&measurements)
        };

        if let Some(avg) = avg {
            let mut err = (avg - expected_bearing).abs();
            if err > 180.0 {
                err = 360.0 - err;
            }
            let status = if err < 10.0 { "PASS" } else { "FAIL" };
            println!(
                "{:<15.1} {:<15.1} {:<15.1} {:<15}",
                expected_bearing, avg, err, status
            );

            if err > 10.0 {
                println!("  WARNING: Large error detected!");
            }
        } else {
            println!(
                "{:<15.1} {:<15} {:<15} {:<15}",
                expected_bearing, "N/A", "N/A", "FAIL (no measurement)"
            );
        }
    }

    println!("\nTest complete.");

    Ok(())
}
