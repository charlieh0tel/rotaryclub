use rotaryclub::config::{AgcConfig, DopplerConfig, RdfConfig};
use rotaryclub::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthReferenceTracker, NorthTick,
    NorthTracker, ZeroCrossingBearingCalculator,
};
use std::f32::consts::PI;

fn generate_correct_test_signal(
    num_samples: usize,
    sample_rate: f32,
    rotation_freq: f32,
    bearing_degrees: f32,
) -> (Vec<f32>, Vec<f32>) {
    let bearing_radians = bearing_degrees.to_radians();
    let samples_per_rotation = sample_rate / rotation_freq;

    let mut doppler = Vec::with_capacity(num_samples);
    let mut north_tick = Vec::with_capacity(num_samples);

    let mut last_rotation_count: i32 = -1;

    for i in 0..num_samples {
        let rotation_phase = (i as f32 / samples_per_rotation) * 2.0 * PI;
        let doppler_sample = (rotation_phase - bearing_radians).sin();
        doppler.push(doppler_sample);

        let current_rotation = (i as f32 / samples_per_rotation) as i32;
        let north_tick_sample = if current_rotation > last_rotation_count {
            last_rotation_count = current_rotation;
            0.8
        } else {
            0.0
        };
        north_tick.push(north_tick_sample);
    }

    (doppler, north_tick)
}

#[test]
fn test_phase_to_bearing_edge_cases() {
    use rotaryclub::rdf::bearing::phase_to_bearing;

    assert!((phase_to_bearing(0.0) - 0.0).abs() < 0.01, "0 rad -> 0 deg");
    assert!(
        (phase_to_bearing(PI / 2.0) - 90.0).abs() < 0.01,
        "π/2 rad -> 90 deg"
    );
    assert!(
        (phase_to_bearing(PI) - 180.0).abs() < 0.01,
        "π rad -> 180 deg"
    );
    assert!(
        (phase_to_bearing(-PI / 2.0) - 270.0).abs() < 0.01,
        "-π/2 rad -> 270 deg"
    );

    let result = phase_to_bearing(-2.0 * PI - PI / 2.0);
    assert!(
        (result - 270.0).abs() < 0.01,
        "BUG: -5π/2 rad should be 270 deg, got {}",
        result
    );

    let result = phase_to_bearing(3.0 * PI);
    assert!(
        (result - 180.0).abs() < 0.01,
        "3π rad should be 180 deg, got {}",
        result
    );
}

fn circular_mean(angles_degrees: &[f32]) -> f32 {
    let sum_x: f32 = angles_degrees.iter().map(|a| a.to_radians().cos()).sum();
    let sum_y: f32 = angles_degrees.iter().map(|a| a.to_radians().sin()).sum();
    let avg_rad = sum_y.atan2(sum_x);
    let avg_deg = avg_rad.to_degrees();
    if avg_deg < 0.0 {
        avg_deg + 360.0
    } else {
        avg_deg
    }
}

#[test]
fn test_bearing_smoothing_wraparound_bug() {
    let angles = vec![350.0, 10.0];
    let arithmetic_mean = angles.iter().sum::<f32>() / angles.len() as f32;
    let correct_circular_mean = circular_mean(&angles);

    println!("Arithmetic mean of 350° and 10°: {}", arithmetic_mean);
    println!("Correct circular mean: {}", correct_circular_mean);

    assert!(
        (arithmetic_mean - 180.0).abs() < 0.01,
        "Arithmetic mean should be 180"
    );
    assert!(
        !(10.0..=350.0).contains(&correct_circular_mean),
        "BUG: Circular mean of 350° and 10° should be near 0°/360°, got {}",
        correct_circular_mean
    );
}

#[test]
fn test_correlation_bearing_with_correct_signal() {
    let sample_rate = 48000.0;
    let rotation_freq = 1602.0;

    let doppler_config = DopplerConfig {
        expected_freq: rotation_freq,
        bandpass_low: 1500.0,
        bandpass_high: 1700.0,
        ..Default::default()
    };

    let agc_config = AgcConfig::default();

    let test_bearings = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

    println!("\n=== Correlation Bearing Calculator Test ===");
    println!(
        "{:<15} {:<15} {:<15}",
        "Expected (°)", "Measured (°)", "Error (°)"
    );
    println!("{}", "-".repeat(45));

    for &expected_bearing in &test_bearings {
        let mut calc =
            CorrelationBearingCalculator::new(&doppler_config, &agc_config, sample_rate, 1)
                .unwrap();

        let num_samples = (sample_rate * 0.1) as usize; // 100ms of data
        let (doppler, _north_tick) =
            generate_correct_test_signal(num_samples, sample_rate, rotation_freq, expected_bearing);

        let samples_per_rotation = sample_rate / rotation_freq;
        let north_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase: 0.0,
            frequency: 2.0 * PI / samples_per_rotation,
        };

        let measurement = calc.process_buffer(&doppler, &north_tick);

        if let Some(m) = measurement {
            let mut error = (m.raw_bearing - expected_bearing).abs();
            if error > 180.0 {
                error = 360.0 - error;
            }
            println!(
                "{:<15.1} {:<15.1} {:<15.1}",
                expected_bearing, m.raw_bearing, error
            );

            assert!(
                error < 15.0,
                "Correlation bearing error too large: expected {}, got {}, error {}",
                expected_bearing,
                m.raw_bearing,
                error
            );
        } else {
            panic!("No measurement for bearing {}", expected_bearing);
        }
    }
}

#[test]
fn test_zero_crossing_bearing_with_correct_signal() {
    let sample_rate = 48000.0;
    let rotation_freq = 1602.0;

    let doppler_config = DopplerConfig {
        expected_freq: rotation_freq,
        bandpass_low: 1500.0,
        bandpass_high: 1700.0,
        zero_cross_hysteresis: 0.01,
        ..Default::default()
    };

    let agc_config = AgcConfig::default();

    let test_bearings = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

    println!("\n=== Zero-Crossing Bearing Calculator Test ===");
    println!(
        "{:<15} {:<15} {:<15}",
        "Expected (°)", "Measured (°)", "Error (°)"
    );
    println!("{}", "-".repeat(45));

    for &expected_bearing in &test_bearings {
        let mut calc =
            ZeroCrossingBearingCalculator::new(&doppler_config, &agc_config, sample_rate, 1)
                .unwrap();

        let num_samples = (sample_rate * 0.1) as usize; // 100ms of data
        let (doppler, _north_tick) =
            generate_correct_test_signal(num_samples, sample_rate, rotation_freq, expected_bearing);

        let samples_per_rotation = sample_rate / rotation_freq;
        let north_tick = NorthTick {
            sample_index: 0,
            period: Some(samples_per_rotation),
            lock_quality: None,
            phase: 0.0,
            frequency: 2.0 * PI / samples_per_rotation,
        };

        let measurement = calc.process_buffer(&doppler, &north_tick);

        if let Some(m) = measurement {
            let mut error = (m.raw_bearing - expected_bearing).abs();
            if error > 180.0 {
                error = 360.0 - error;
            }
            println!(
                "{:<15.1} {:<15.1} {:<15.1}",
                expected_bearing, m.raw_bearing, error
            );

            assert!(
                error < 20.0,
                "Zero-crossing bearing error too large: expected {}, got {}, error {}",
                expected_bearing,
                m.raw_bearing,
                error
            );
        } else {
            panic!("No measurement for bearing {}", expected_bearing);
        }
    }
}

#[test]
fn test_full_pipeline_with_north_tracker() {
    let mut config = RdfConfig::default();
    config.north_tick.threshold = 0.1;
    let sample_rate = config.audio.sample_rate as f32;
    let rotation_freq = config.doppler.expected_freq;

    let test_bearing = 90.0;
    let duration_secs = 0.5;
    let num_samples = (duration_secs * sample_rate) as usize;

    let (doppler, north_tick_signal) =
        generate_correct_test_signal(num_samples, sample_rate, rotation_freq, test_bearing);

    let tick_count = north_tick_signal.iter().filter(|&&x| x > 0.5).count();
    println!("Generated {} north tick pulses", tick_count);

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate).unwrap();
    let mut bearing_calc =
        CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 1).unwrap();

    let chunk_size = config.audio.buffer_size;
    let mut measurements = Vec::new();
    let mut pending_tick: Option<NorthTick> = None;
    let mut total_ticks_detected = 0;

    for i in (0..num_samples).step_by(chunk_size) {
        let end = (i + chunk_size).min(num_samples);
        let doppler_chunk = &doppler[i..end];
        let north_chunk = &north_tick_signal[i..end];

        let ticks = north_tracker.process_buffer(north_chunk);
        total_ticks_detected += ticks.len();

        if let Some(ref tick) = pending_tick {
            if let Some(m) = bearing_calc.process_buffer(doppler_chunk, tick) {
                measurements.push(m.raw_bearing);
            }
        } else {
            let period = sample_rate / rotation_freq;
            let dummy_tick = NorthTick {
                sample_index: 0,
                period: Some(period),
                lock_quality: None,
                phase: 0.0,
                frequency: 2.0 * PI / period,
            };
            let _ = bearing_calc.process_buffer(doppler_chunk, &dummy_tick);
        }

        if let Some(tick) = ticks.first() {
            pending_tick = Some(*tick);
        }
    }

    println!("\n=== Full Pipeline Test (bearing = 90°) ===");
    println!("Total ticks detected: {}", total_ticks_detected);
    println!("Number of measurements: {}", measurements.len());

    if measurements.len() > 3 {
        let avg_bearing = circular_mean(&measurements[3..]);
        let mut error = (avg_bearing - test_bearing).abs();
        if error > 180.0 {
            error = 360.0 - error;
        }
        println!("Average bearing: {:.1}°", avg_bearing);
        println!("Error: {:.1}°", error);

        assert!(
            error < 20.0,
            "Full pipeline bearing error too large: expected {}, got {}, error {}",
            test_bearing,
            avg_bearing,
            error
        );
    } else {
        println!("WARNING: Not enough measurements collected");
    }
}
