use rotaryclub::config::RdfConfig;
use rotaryclub::test_utils::{
    FadingType, MultipathComponent, NoiseConfig, angle_error, apply_noise, generate_test_signal,
    measure_bearing,
};

fn run_snr_sweep() {
    println!("noise_type,parameter,zc_error,corr_error");

    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate;
    let rotation_hz = config.doppler.expected_freq;
    let test_bearings = [45.0_f32, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

    // SNR sweep
    for snr_db in (0..=40).step_by(2) {
        let snr = snr_db as f32;
        let mut zc_errors = Vec::new();
        let mut corr_errors = Vec::new();

        for &bearing in &test_bearings {
            let noise_config = NoiseConfig::default()
                .with_seed(42 + bearing as u64)
                .with_awgn(snr);

            let signal = generate_test_signal(0.5, sample_rate, rotation_hz, rotation_hz, bearing);
            let doppler: Vec<f32> = signal.iter().step_by(2).copied().collect();
            let north_tick: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();

            let noisy_doppler =
                apply_noise(&doppler, &noise_config, sample_rate as f32, rotation_hz);

            let mut noisy_signal = Vec::with_capacity(signal.len());
            for (d, n) in noisy_doppler.iter().zip(north_tick.iter()) {
                noisy_signal.push(*d);
                noisy_signal.push(*n);
            }

            let measurement = measure_bearing(&noisy_signal, &config);
            if let Some(z) = measurement.zc_bearing {
                zc_errors.push(angle_error(z, bearing).abs());
            }
            if let Some(c) = measurement.corr_bearing {
                corr_errors.push(angle_error(c, bearing).abs());
            }
        }

        let zc_max = zc_errors.iter().fold(0.0f32, |a, &b| a.max(b));
        let corr_max = corr_errors.iter().fold(0.0f32, |a, &b| a.max(b));
        println!("awgn,{},{:.2},{:.2}", snr, zc_max, corr_max);
    }

    // Fading doppler spread sweep
    for spread_idx in 0..=20 {
        let spread = spread_idx as f32;
        let mut zc_errors = Vec::new();
        let mut corr_errors = Vec::new();

        for &bearing in &test_bearings {
            let mut noise_config = NoiseConfig::default()
                .with_seed(42 + bearing as u64)
                .with_awgn(25.0);

            if spread > 0.0 {
                noise_config = noise_config.with_fading(FadingType::Rayleigh, spread);
            }

            let signal = generate_test_signal(0.5, sample_rate, rotation_hz, rotation_hz, bearing);
            let doppler: Vec<f32> = signal.iter().step_by(2).copied().collect();
            let north_tick: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();

            let noisy_doppler =
                apply_noise(&doppler, &noise_config, sample_rate as f32, rotation_hz);

            let mut noisy_signal = Vec::with_capacity(signal.len());
            for (d, n) in noisy_doppler.iter().zip(north_tick.iter()) {
                noisy_signal.push(*d);
                noisy_signal.push(*n);
            }

            let measurement = measure_bearing(&noisy_signal, &config);
            if let Some(z) = measurement.zc_bearing {
                zc_errors.push(angle_error(z, bearing).abs());
            }
            if let Some(c) = measurement.corr_bearing {
                corr_errors.push(angle_error(c, bearing).abs());
            }
        }

        let zc_max = zc_errors.iter().fold(0.0f32, |a, &b| a.max(b));
        let corr_max = corr_errors.iter().fold(0.0f32, |a, &b| a.max(b));
        println!("fading,{},{:.2},{:.2}", spread, zc_max, corr_max);
    }

    // Multipath delay sweep (as fraction of rotation period)
    let samples_per_rotation = (sample_rate as f32 / rotation_hz) as usize;
    for delay_pct in (0..=50).step_by(2) {
        let delay = (samples_per_rotation * delay_pct) / 100;
        let mut zc_errors = Vec::new();
        let mut corr_errors = Vec::new();

        for &bearing in &test_bearings {
            let mut noise_config = NoiseConfig::default()
                .with_seed(42 + bearing as u64)
                .with_awgn(25.0);

            if delay > 0 {
                noise_config = noise_config.with_multipath(vec![MultipathComponent {
                    delay_samples: delay,
                    amplitude: 0.3,
                    phase_offset: 0.0,
                }]);
            }

            let signal = generate_test_signal(0.5, sample_rate, rotation_hz, rotation_hz, bearing);
            let doppler: Vec<f32> = signal.iter().step_by(2).copied().collect();
            let north_tick: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();

            let noisy_doppler =
                apply_noise(&doppler, &noise_config, sample_rate as f32, rotation_hz);

            let mut noisy_signal = Vec::with_capacity(signal.len());
            for (d, n) in noisy_doppler.iter().zip(north_tick.iter()) {
                noisy_signal.push(*d);
                noisy_signal.push(*n);
            }

            let measurement = measure_bearing(&noisy_signal, &config);
            if let Some(z) = measurement.zc_bearing {
                zc_errors.push(angle_error(z, bearing).abs());
            }
            if let Some(c) = measurement.corr_bearing {
                corr_errors.push(angle_error(c, bearing).abs());
            }
        }

        let zc_max = zc_errors.iter().fold(0.0f32, |a, &b| a.max(b));
        let corr_max = corr_errors.iter().fold(0.0f32, |a, &b| a.max(b));
        println!("multipath,{},{:.2},{:.2}", delay_pct, zc_max, corr_max);
    }

    // Impulse noise rate sweep
    for rate in (0..=200).step_by(10) {
        let rate_hz = rate as f32;
        let mut zc_errors = Vec::new();
        let mut corr_errors = Vec::new();

        for &bearing in &test_bearings {
            let mut noise_config = NoiseConfig::default()
                .with_seed(42 + bearing as u64)
                .with_awgn(25.0);

            if rate_hz > 0.0 {
                noise_config = noise_config.with_impulse(rate_hz, 2.0, 5);
            }

            let signal = generate_test_signal(0.5, sample_rate, rotation_hz, rotation_hz, bearing);
            let doppler: Vec<f32> = signal.iter().step_by(2).copied().collect();
            let north_tick: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();

            let noisy_doppler =
                apply_noise(&doppler, &noise_config, sample_rate as f32, rotation_hz);

            let mut noisy_signal = Vec::with_capacity(signal.len());
            for (d, n) in noisy_doppler.iter().zip(north_tick.iter()) {
                noisy_signal.push(*d);
                noisy_signal.push(*n);
            }

            let measurement = measure_bearing(&noisy_signal, &config);
            if let Some(z) = measurement.zc_bearing {
                zc_errors.push(angle_error(z, bearing).abs());
            }
            if let Some(c) = measurement.corr_bearing {
                corr_errors.push(angle_error(c, bearing).abs());
            }
        }

        let zc_max = zc_errors.iter().fold(0.0f32, |a, &b| a.max(b));
        let corr_max = corr_errors.iter().fold(0.0f32, |a, &b| a.max(b));
        println!("impulse,{},{:.2},{:.2}", rate, zc_max, corr_max);
    }
}

fn main() {
    run_snr_sweep();
}
