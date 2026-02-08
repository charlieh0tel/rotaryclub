use rotaryclub::RdfProcessor;
use rotaryclub::config::{BearingMethod, RdfConfig};
use rotaryclub::simulation::{
    FadingType, MultipathComponent, NoiseConfig, angle_error, apply_noise, circular_mean_degrees,
    generate_test_signal,
};

const NUM_TRIALS: usize = 10;
const TEST_BEARINGS: [f32; 7] = [45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

fn mean(data: &[f32]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    data.iter().sum::<f32>() / data.len() as f32
}

fn std_dev(data: &[f32]) -> f32 {
    if data.len() < 2 {
        return 0.0;
    }
    let m = mean(data);
    let variance = data.iter().map(|x| (x - m).powi(2)).sum::<f32>() / (data.len() - 1) as f32;
    variance.sqrt()
}

fn run_trial(
    config: &RdfConfig,
    sample_rate: u32,
    rotation_hz: f32,
    bearing: f32,
    noise_config: &NoiseConfig,
) -> (Option<f32>, Option<f32>) {
    let signal = generate_test_signal(0.5, sample_rate, rotation_hz, bearing);
    let doppler: Vec<f32> = signal.iter().step_by(2).copied().collect();
    let north_tick: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();

    let noisy_doppler = apply_noise(&doppler, noise_config, sample_rate as f32, rotation_hz);

    let mut noisy_signal = Vec::with_capacity(signal.len());
    for (d, n) in noisy_doppler.iter().zip(north_tick.iter()) {
        noisy_signal.push(*d);
        noisy_signal.push(*n);
    }

    let mut zc_config = config.clone();
    zc_config.doppler.method = BearingMethod::ZeroCrossing;
    let mut corr_config = config.clone();
    corr_config.doppler.method = BearingMethod::Correlation;

    let mut zc_processor = match RdfProcessor::new(&zc_config, false, true) {
        Ok(p) => p,
        Err(_) => return (None, None),
    };
    let mut corr_processor = match RdfProcessor::new(&corr_config, false, true) {
        Ok(p) => p,
        Err(_) => return (None, None),
    };

    let zc_results = zc_processor.process_signal(&noisy_signal);
    let corr_results = corr_processor.process_signal(&noisy_signal);

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

    let zc_error = zc_bearing.map(|z| angle_error(z, bearing).abs());
    let corr_error = corr_bearing.map(|c| angle_error(c, bearing).abs());

    (zc_error, corr_error)
}

fn run_sweep<F>(
    config: &RdfConfig,
    sample_rate: u32,
    rotation_hz: f32,
    noise_type: &str,
    params: impl Iterator<Item = (f32, F)>,
) where
    F: Fn(u64) -> NoiseConfig,
{
    for (param_value, make_noise_config) in params {
        let mut zc_errors = Vec::new();
        let mut corr_errors = Vec::new();

        for trial in 0..NUM_TRIALS {
            let base_seed = (trial * 1000) as u64;

            for &bearing in &TEST_BEARINGS {
                let seed = base_seed + bearing as u64;
                let noise_config = make_noise_config(seed);

                let (zc, corr) =
                    run_trial(config, sample_rate, rotation_hz, bearing, &noise_config);

                if let Some(e) = zc {
                    zc_errors.push(e);
                }
                if let Some(e) = corr {
                    corr_errors.push(e);
                }
            }
        }

        let zc_mean = mean(&zc_errors);
        let zc_std = std_dev(&zc_errors);
        let corr_mean = mean(&corr_errors);
        let corr_std = std_dev(&corr_errors);

        println!(
            "{},{:.1},{:.2},{:.2},{:.2},{:.2}",
            noise_type, param_value, zc_mean, zc_std, corr_mean, corr_std
        );
    }
}

fn main() {
    println!("noise_type,parameter,zc_mean,zc_std,corr_mean,corr_std");

    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate;
    let rotation_hz = config.doppler.expected_freq;

    // SNR sweep
    run_sweep(
        &config,
        sample_rate,
        rotation_hz,
        "awgn",
        (0..=40).step_by(2).map(|snr_db| {
            let snr = snr_db as f32;
            (snr, move |seed| {
                NoiseConfig::default().with_seed(seed).with_awgn(snr)
            })
        }),
    );

    // Fading doppler spread sweep
    run_sweep(
        &config,
        sample_rate,
        rotation_hz,
        "fading",
        (0..=20).map(|spread_idx| {
            let spread = spread_idx as f32;
            (spread, move |seed| {
                let mut nc = NoiseConfig::default().with_seed(seed).with_awgn(25.0);
                if spread > 0.0 {
                    nc = nc.with_fading(FadingType::Rayleigh, spread);
                }
                nc
            })
        }),
    );

    // Multipath delay sweep
    let samples_per_rotation = (sample_rate as f32 / rotation_hz) as usize;
    run_sweep(
        &config,
        sample_rate,
        rotation_hz,
        "multipath",
        (0..=50).step_by(2).map(|delay_pct| {
            let delay = (samples_per_rotation * delay_pct) / 100;
            (delay_pct as f32, move |seed| {
                let mut nc = NoiseConfig::default().with_seed(seed).with_awgn(25.0);
                if delay > 0 {
                    nc = nc.with_multipath(vec![MultipathComponent {
                        delay_samples: delay,
                        amplitude: 0.3,
                        phase_offset: 0.0,
                    }]);
                }
                nc
            })
        }),
    );

    // Impulse noise rate sweep
    run_sweep(
        &config,
        sample_rate,
        rotation_hz,
        "impulse",
        (0..=200).step_by(10).map(|rate| {
            let rate_hz = rate as f32;
            (rate_hz, move |seed| {
                let mut nc = NoiseConfig::default().with_seed(seed).with_awgn(25.0);
                if rate_hz > 0.0 {
                    nc = nc.with_impulse(rate_hz, 2.0, 5);
                }
                nc
            })
        }),
    );
}
