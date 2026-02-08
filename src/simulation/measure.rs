use crate::config::{BearingMethod, RdfConfig};
use crate::processing::RdfProcessor;

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

pub fn circular_mean_degrees(angles: &[f32]) -> Option<f32> {
    if angles.is_empty() {
        return None;
    }
    let (sum_cos, sum_sin) = angles.iter().fold((0.0f32, 0.0f32), |(c, s), &a| {
        let r = a.to_radians();
        (c + r.cos(), s + r.sin())
    });
    Some(sum_sin.atan2(sum_cos).to_degrees().rem_euclid(360.0))
}

pub fn measure_bearing(signal: &[f32], config: &RdfConfig) -> BearingMeasurement {
    let mut zc_config = config.clone();
    zc_config.doppler.method = BearingMethod::ZeroCrossing;
    let mut corr_config = config.clone();
    corr_config.doppler.method = BearingMethod::Correlation;

    let mut zc_processor = match RdfProcessor::new(&zc_config, false, true) {
        Ok(p) => p,
        Err(_) => return BearingMeasurement::default(),
    };
    let mut corr_processor = match RdfProcessor::new(&corr_config, false, true) {
        Ok(p) => p,
        Err(_) => return BearingMeasurement::default(),
    };

    let zc_results = zc_processor.process_signal(signal);
    let corr_results = corr_processor.process_signal(signal);

    let zc_measurements: Vec<f32> = zc_results
        .iter()
        .filter_map(|r| r.bearing.map(|b| b.bearing_degrees))
        .collect();
    let corr_measurements: Vec<f32> = corr_results
        .iter()
        .filter_map(|r| r.bearing.map(|b| b.bearing_degrees))
        .collect();

    let zc_bearing = if zc_measurements.len() > 5 {
        circular_mean_degrees(&zc_measurements[3..])
    } else {
        circular_mean_degrees(&zc_measurements)
    };

    let corr_bearing = if corr_measurements.len() > 5 {
        circular_mean_degrees(&corr_measurements[3..])
    } else {
        circular_mean_degrees(&corr_measurements)
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
