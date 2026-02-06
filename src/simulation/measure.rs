use std::f32::consts::PI;

use crate::config::RdfConfig;
use crate::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthReferenceTracker, NorthTick,
    NorthTracker, ZeroCrossingBearingCalculator,
};

use super::{NoiseConfig, apply_noise, generate_test_signal};

#[derive(Debug, Clone, Default)]
pub struct BearingMeasurement {
    pub zc_bearing: Option<f32>,
    pub corr_bearing: Option<f32>,
}

pub fn angle_error(measured: f32, expected: f32) -> f32 {
    let mut e = measured - expected;
    if e > 180.0 {
        e -= 360.0;
    } else if e < -180.0 {
        e += 360.0;
    }
    e
}

pub fn measure_bearing(signal: &[f32], config: &RdfConfig) -> BearingMeasurement {
    let sample_rate = config.audio.sample_rate as f32;

    let mut north_tracker = match NorthReferenceTracker::new(&config.north_tick, sample_rate) {
        Ok(t) => t,
        Err(_) => return BearingMeasurement::default(),
    };
    let mut zc_calc =
        match ZeroCrossingBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3) {
            Ok(c) => c,
            Err(_) => return BearingMeasurement::default(),
        };
    let mut corr_calc =
        match CorrelationBearingCalculator::new(&config.doppler, &config.agc, sample_rate, 3) {
            Ok(c) => c,
            Err(_) => return BearingMeasurement::default(),
        };

    let chunk_size = config.audio.buffer_size * 2;
    let mut zc_measurements = Vec::new();
    let mut corr_measurements = Vec::new();
    let mut last_tick: Option<NorthTick> = None;

    for chunk in signal.chunks(chunk_size) {
        let stereo: Vec<(f32, f32)> = chunk.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo);

        if let Some(ref tick) = last_tick {
            if let Some(bearing) = zc_calc.process_buffer(&doppler, tick) {
                zc_measurements.push(bearing.bearing_degrees);
            }
            if let Some(bearing) = corr_calc.process_buffer(&doppler, tick) {
                corr_measurements.push(bearing.bearing_degrees);
            }
        } else {
            let dummy_tick = NorthTick {
                sample_index: 0,
                period: Some(30.0),
                lock_quality: None,
                phase: 0.0,
                frequency: 2.0 * PI / 30.0,
            };
            zc_calc.process_buffer(&doppler, &dummy_tick);
            corr_calc.process_buffer(&doppler, &dummy_tick);
        }

        let ticks = north_tracker.process_buffer(&north_tick);
        if let Some(tick) = ticks.last() {
            last_tick = Some(*tick);
        }
    }

    let zc_bearing = if zc_measurements.len() > 5 {
        Some(zc_measurements.iter().skip(3).sum::<f32>() / (zc_measurements.len() - 3) as f32)
    } else if !zc_measurements.is_empty() {
        Some(zc_measurements.iter().sum::<f32>() / zc_measurements.len() as f32)
    } else {
        None
    };

    let corr_bearing = if corr_measurements.len() > 5 {
        Some(corr_measurements.iter().skip(3).sum::<f32>() / (corr_measurements.len() - 3) as f32)
    } else if !corr_measurements.is_empty() {
        Some(corr_measurements.iter().sum::<f32>() / corr_measurements.len() as f32)
    } else {
        None
    };

    BearingMeasurement {
        zc_bearing,
        corr_bearing,
    }
}

#[derive(Debug, Clone, Default)]
pub struct ErrorStats {
    pub zc_max_error: f32,
    pub corr_max_error: f32,
}

pub fn measure_error_across_bearings(
    noise_config: &NoiseConfig,
    rdf_config: &RdfConfig,
    test_bearings: &[f32],
) -> ErrorStats {
    let sample_rate = rdf_config.audio.sample_rate;
    let rotation_hz = rdf_config.doppler.expected_freq;

    let mut zc_errors = Vec::new();
    let mut corr_errors = Vec::new();

    for &bearing in test_bearings {
        let signal = generate_test_signal(0.5, sample_rate, rotation_hz, bearing);

        let doppler: Vec<f32> = signal.iter().step_by(2).copied().collect();
        let north_tick: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();

        let noisy_doppler = apply_noise(&doppler, noise_config, sample_rate as f32, rotation_hz);

        let mut noisy_signal = Vec::with_capacity(signal.len());
        for (d, n) in noisy_doppler.iter().zip(north_tick.iter()) {
            noisy_signal.push(*d);
            noisy_signal.push(*n);
        }

        let measurement = measure_bearing(&noisy_signal, rdf_config);

        if let Some(zc) = measurement.zc_bearing {
            zc_errors.push(angle_error(zc, bearing).abs());
        }
        if let Some(corr) = measurement.corr_bearing {
            corr_errors.push(angle_error(corr, bearing).abs());
        }
    }

    ErrorStats {
        zc_max_error: zc_errors.iter().fold(0.0f32, |a, &b| a.max(b)),
        corr_max_error: corr_errors.iter().fold(0.0f32, |a, &b| a.max(b)),
    }
}
