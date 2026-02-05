use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Normal};
use std::f32::consts::PI;

#[derive(Clone, Debug, Default)]
pub struct NoiseConfig {
    pub seed: Option<u64>,
    pub additive: Option<AdditiveNoiseConfig>,
    pub fading: Option<FadingConfig>,
    pub multipath: Option<MultipathConfig>,
    pub doubling: Option<DoublingConfig>,
    pub impulse: Option<ImpulseNoiseConfig>,
    pub frequency_drift: Option<FrequencyDriftConfig>,
}

#[derive(Clone, Debug)]
pub struct AdditiveNoiseConfig {
    pub snr_db: f32,
}

#[derive(Clone, Debug)]
pub enum FadingType {
    Rayleigh,
    Rician { k_factor: f32 },
}

#[derive(Clone, Debug)]
pub struct FadingConfig {
    pub fading_type: FadingType,
    pub doppler_spread_hz: f32,
}

#[derive(Clone, Debug)]
pub struct MultipathComponent {
    pub delay_samples: usize,
    pub amplitude: f32,
    pub phase_offset: f32,
}

#[derive(Clone, Debug)]
pub struct MultipathConfig {
    pub components: Vec<MultipathComponent>,
}

#[derive(Clone, Debug)]
pub struct DoublingConfig {
    pub second_bearing_degrees: f32,
    pub amplitude_ratio: f32,
}

#[derive(Clone, Debug)]
pub struct ImpulseNoiseConfig {
    pub rate_hz: f32,
    pub amplitude: f32,
    pub duration_samples: usize,
}

#[derive(Clone, Debug)]
pub struct FrequencyDriftConfig {
    pub max_deviation_hz: f32,
    pub drift_rate_hz_per_sec: f32,
}

fn create_rng(seed: Option<u64>) -> ChaCha8Rng {
    match seed {
        Some(s) => ChaCha8Rng::seed_from_u64(s),
        None => ChaCha8Rng::from_entropy(),
    }
}

fn signal_power(signal: &[f32]) -> f32 {
    if signal.is_empty() {
        return 0.0;
    }
    signal.iter().map(|&x| x * x).sum::<f32>() / signal.len() as f32
}

fn apply_additive_noise(signal: &mut [f32], config: &AdditiveNoiseConfig, rng: &mut ChaCha8Rng) {
    let sig_power = signal_power(signal);
    if sig_power == 0.0 {
        return;
    }

    let snr_linear = 10.0_f32.powf(config.snr_db / 10.0);
    let noise_power = sig_power / snr_linear;
    let noise_std = noise_power.sqrt();

    let normal = Normal::new(0.0, noise_std as f64).unwrap();

    for sample in signal.iter_mut() {
        *sample += normal.sample(rng) as f32;
    }
}

fn apply_fading(signal: &mut [f32], config: &FadingConfig, sample_rate: f32, rng: &mut ChaCha8Rng) {
    let n = signal.len();
    if n == 0 {
        return;
    }

    let normal = Normal::new(0.0, 1.0).unwrap();
    let fd = config.doppler_spread_hz;

    let mut fading_envelope = vec![1.0f32; n];

    if fd > 0.0 {
        let num_sinusoids = 16;
        let mut real_part = vec![0.0f32; n];
        let mut imag_part = vec![0.0f32; n];

        for _ in 0..num_sinusoids {
            let theta: f32 = rng.r#gen::<f32>() * 2.0 * PI;
            let freq = fd * theta.cos();
            let phi: f32 = rng.r#gen::<f32>() * 2.0 * PI;

            for (i, (real, imag)) in real_part.iter_mut().zip(imag_part.iter_mut()).enumerate() {
                let t = i as f32 / sample_rate;
                let phase = 2.0 * PI * freq * t + phi;
                *real += phase.cos();
                *imag += phase.sin();
            }
        }

        let scale = 1.0 / (num_sinusoids as f32).sqrt();
        for i in 0..n {
            real_part[i] *= scale;
            imag_part[i] *= scale;
        }

        match &config.fading_type {
            FadingType::Rayleigh => {
                for i in 0..n {
                    fading_envelope[i] =
                        (real_part[i] * real_part[i] + imag_part[i] * imag_part[i]).sqrt();
                }
            }
            FadingType::Rician { k_factor } => {
                let k = *k_factor;
                let los_amplitude = (k / (k + 1.0)).sqrt();
                let scatter_amplitude = (1.0 / (k + 1.0)).sqrt();

                for i in 0..n {
                    let real_total = los_amplitude + scatter_amplitude * real_part[i];
                    let imag_total = scatter_amplitude * imag_part[i];
                    fading_envelope[i] = (real_total * real_total + imag_total * imag_total).sqrt();
                }
            }
        }
    } else {
        match &config.fading_type {
            FadingType::Rayleigh => {
                let x: f32 = normal.sample(rng) as f32;
                let y: f32 = normal.sample(rng) as f32;
                let env = (x * x + y * y).sqrt();
                for val in fading_envelope.iter_mut() {
                    *val = env;
                }
            }
            FadingType::Rician { k_factor } => {
                let k = *k_factor;
                let los = (k / (k + 1.0)).sqrt();
                let scatter = (1.0 / (k + 1.0)).sqrt();
                let x: f32 = normal.sample(rng) as f32;
                let y: f32 = normal.sample(rng) as f32;
                let real = los + scatter * x;
                let imag = scatter * y;
                let env = (real * real + imag * imag).sqrt();
                for val in fading_envelope.iter_mut() {
                    *val = env;
                }
            }
        }
    }

    for (sample, &env) in signal.iter_mut().zip(fading_envelope.iter()) {
        *sample *= env;
    }
}

fn apply_multipath(signal: &mut [f32], config: &MultipathConfig) {
    if config.components.is_empty() {
        return;
    }

    let original = signal.to_vec();

    for component in &config.components {
        let delay = component.delay_samples;
        let amp = component.amplitude;
        let phase = component.phase_offset;

        for (i, s) in signal.iter_mut().enumerate().skip(delay) {
            let original_idx = i - delay;
            *s += amp * original[original_idx] * phase.cos();
        }
    }
}

fn apply_impulse_noise(
    signal: &mut [f32],
    config: &ImpulseNoiseConfig,
    sample_rate: f32,
    rng: &mut ChaCha8Rng,
) {
    let n = signal.len();
    if n == 0 || config.rate_hz <= 0.0 {
        return;
    }

    let avg_samples_between_impulses = sample_rate / config.rate_hz;

    let mut pos = 0usize;
    loop {
        let interval = (rng.r#gen::<f32>() * 2.0 * avg_samples_between_impulses) as usize;
        pos += interval.max(1);

        if pos >= n {
            break;
        }

        let sign = if rng.r#gen::<bool>() { 1.0 } else { -1.0 };
        let end = (pos + config.duration_samples).min(n);

        for sample in signal[pos..end].iter_mut() {
            *sample += sign * config.amplitude;
        }
    }
}

pub fn generate_doppler_signal_for_bearing(
    num_samples: usize,
    sample_rate: f32,
    rotation_hz: f32,
    bearing_degrees: f32,
) -> Vec<f32> {
    let bearing_radians = bearing_degrees.to_radians();
    let omega = 2.0 * PI * rotation_hz;

    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate;
            (omega * t - bearing_radians).sin()
        })
        .collect()
}

fn apply_doubling(signal: &mut [f32], config: &DoublingConfig, sample_rate: f32, rotation_hz: f32) {
    let n = signal.len();
    let second_signal = generate_doppler_signal_for_bearing(
        n,
        sample_rate,
        rotation_hz,
        config.second_bearing_degrees,
    );

    for (sample, &second) in signal.iter_mut().zip(second_signal.iter()) {
        *sample += config.amplitude_ratio * second;
    }
}

fn apply_frequency_drift(
    signal: &mut [f32],
    config: &FrequencyDriftConfig,
    sample_rate: f32,
    rotation_hz: f32,
) {
    let n = signal.len();
    if n == 0 {
        return;
    }

    let max_phase_error_radians = 2.0 * PI * config.max_deviation_hz / rotation_hz;
    let modulation_rate = config.drift_rate_hz_per_sec;

    for (i, s) in signal.iter_mut().enumerate() {
        let t = i as f32 / sample_rate;
        let phase_error = max_phase_error_radians * (2.0 * PI * modulation_rate * t).sin();
        let amplitude_reduction = 1.0 - 0.3 * (phase_error / max_phase_error_radians).abs();
        *s *= amplitude_reduction;
    }
}

pub fn apply_noise(
    clean_signal: &[f32],
    config: &NoiseConfig,
    sample_rate: f32,
    rotation_hz: f32,
) -> Vec<f32> {
    let mut signal = clean_signal.to_vec();
    let mut rng = create_rng(config.seed);

    if let Some(ref drift_config) = config.frequency_drift {
        apply_frequency_drift(&mut signal, drift_config, sample_rate, rotation_hz);
    }

    if let Some(ref doubling_config) = config.doubling {
        apply_doubling(&mut signal, doubling_config, sample_rate, rotation_hz);
    }

    if let Some(ref multipath_config) = config.multipath {
        apply_multipath(&mut signal, multipath_config);
    }

    if let Some(ref fading_config) = config.fading {
        apply_fading(&mut signal, fading_config, sample_rate, &mut rng);
    }

    if let Some(ref additive_config) = config.additive {
        apply_additive_noise(&mut signal, additive_config, &mut rng);
    }

    if let Some(ref impulse_config) = config.impulse {
        apply_impulse_noise(&mut signal, impulse_config, sample_rate, &mut rng);
    }

    signal
}

pub fn generate_noisy_test_signal(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    bearing_degrees: f32,
    noise_config: &NoiseConfig,
) -> Vec<f32> {
    let clean = super::generate::generate_test_signal(
        duration_secs,
        sample_rate,
        rotation_hz,
        rotation_hz,
        bearing_degrees,
    );

    let doppler: Vec<f32> = clean.iter().step_by(2).copied().collect();
    let north_tick: Vec<f32> = clean.iter().skip(1).step_by(2).copied().collect();

    let noisy_doppler = apply_noise(&doppler, noise_config, sample_rate as f32, rotation_hz);

    let mut result = Vec::with_capacity(clean.len());
    for (d, n) in noisy_doppler.iter().zip(north_tick.iter()) {
        result.push(*d);
        result.push(*n);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_additive_noise_changes_signal() {
        let clean: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.1).sin()).collect();
        let config = NoiseConfig {
            seed: Some(42),
            additive: Some(AdditiveNoiseConfig { snr_db: 10.0 }),
            ..Default::default()
        };

        let noisy = apply_noise(&clean, &config, 48000.0, 500.0);

        assert_eq!(clean.len(), noisy.len());
        assert_ne!(clean, noisy);
    }

    #[test]
    fn test_seeded_rng_reproducibility() {
        let clean: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.1).sin()).collect();
        let config = NoiseConfig {
            seed: Some(12345),
            additive: Some(AdditiveNoiseConfig { snr_db: 20.0 }),
            ..Default::default()
        };

        let noisy1 = apply_noise(&clean, &config, 48000.0, 500.0);
        let noisy2 = apply_noise(&clean, &config, 48000.0, 500.0);

        assert_eq!(noisy1, noisy2);
    }

    #[test]
    fn test_fading_rayleigh() {
        let clean: Vec<f32> = (0..10000).map(|i| (i as f32 * 0.1).sin()).collect();
        let config = NoiseConfig {
            seed: Some(42),
            fading: Some(FadingConfig {
                fading_type: FadingType::Rayleigh,
                doppler_spread_hz: 10.0,
            }),
            ..Default::default()
        };

        let faded = apply_noise(&clean, &config, 48000.0, 500.0);

        assert_eq!(clean.len(), faded.len());
        let clean_power = signal_power(&clean);
        let faded_power = signal_power(&faded);
        assert!(faded_power > 0.0);
        assert!((faded_power - clean_power).abs() / clean_power < 2.0);
    }

    #[test]
    fn test_multipath_adds_delayed_copies() {
        let mut clean = vec![0.0f32; 100];
        clean[10] = 1.0;

        let config = NoiseConfig {
            multipath: Some(MultipathConfig {
                components: vec![MultipathComponent {
                    delay_samples: 5,
                    amplitude: 0.5,
                    phase_offset: 0.0,
                }],
            }),
            ..Default::default()
        };

        let result = apply_noise(&clean, &config, 48000.0, 500.0);

        assert!(result[10].abs() > 0.9);
        assert!(result[15].abs() > 0.4);
    }

    #[test]
    fn test_impulse_noise_adds_spikes() {
        let clean = vec![0.0f32; 10000];
        let config = NoiseConfig {
            seed: Some(42),
            impulse: Some(ImpulseNoiseConfig {
                rate_hz: 100.0,
                amplitude: 1.0,
                duration_samples: 5,
            }),
            ..Default::default()
        };

        let noisy = apply_noise(&clean, &config, 48000.0, 500.0);

        let spike_count = noisy.iter().filter(|&&x| x.abs() > 0.5).count();
        assert!(spike_count > 10);
        assert!(spike_count < 1000);
    }

    #[test]
    fn test_generate_noisy_test_signal() {
        let config = NoiseConfig {
            seed: Some(42),
            additive: Some(AdditiveNoiseConfig { snr_db: 20.0 }),
            ..Default::default()
        };

        let signal = generate_noisy_test_signal(0.1, 48000, 500.0, 45.0, &config);

        assert_eq!(signal.len(), 4800 * 2);
    }

    #[test]
    fn test_doubling_adds_second_bearing() {
        let sample_rate = 48000.0;
        let rotation_hz = 500.0;
        let num_samples = 1000;

        let primary =
            generate_doppler_signal_for_bearing(num_samples, sample_rate, rotation_hz, 0.0);

        let config = NoiseConfig {
            doubling: Some(DoublingConfig {
                second_bearing_degrees: 90.0,
                amplitude_ratio: 0.5,
            }),
            ..Default::default()
        };

        let result = apply_noise(&primary, &config, sample_rate, rotation_hz);

        assert_eq!(primary.len(), result.len());
        assert_ne!(primary, result);
    }

    #[test]
    fn test_combined_noise_effects() {
        let clean: Vec<f32> = (0..10000).map(|i| (i as f32 * 0.1).sin()).collect();

        let config = NoiseConfig {
            seed: Some(42),
            additive: Some(AdditiveNoiseConfig { snr_db: 20.0 }),
            fading: Some(FadingConfig {
                fading_type: FadingType::Rician { k_factor: 4.0 },
                doppler_spread_hz: 5.0,
            }),
            multipath: Some(MultipathConfig {
                components: vec![MultipathComponent {
                    delay_samples: 10,
                    amplitude: 0.3,
                    phase_offset: 0.5,
                }],
            }),
            ..Default::default()
        };

        let noisy = apply_noise(&clean, &config, 48000.0, 500.0);

        assert_eq!(clean.len(), noisy.len());
        assert_ne!(clean, noisy);
    }
}
