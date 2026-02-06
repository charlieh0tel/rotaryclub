use anyhow::{Context, Result};
use clap::Parser;
use rotaryclub::save_wav;
use rotaryclub::simulation::{
    AdditiveNoiseConfig, FadingConfig, FadingType, ImpulseNoiseConfig, MultipathComponent,
    MultipathConfig, NoiseConfig, generate_noisy_test_signal,
};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "generate_wav")]
#[command(about = "Generate synthetic WAV files with configurable noise for RDF testing")]
struct Args {
    /// TOML noise configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Output directory
    #[arg(short, long, default_value = "data/synthetic")]
    output_dir: PathBuf,

    /// Bearings: comma-separated (e.g., "45,90,180") or range (e.g., "0-360:15")
    #[arg(short, long, default_value = "0-360:15")]
    bearings: String,

    /// Number of trials per bearing
    #[arg(short, long, default_value_t = 10)]
    trials: u32,

    /// Base seed for reproducibility
    #[arg(short, long)]
    seed: Option<u64>,

    /// Signal duration in seconds
    #[arg(short, long, default_value_t = 0.5)]
    duration: f32,

    /// Sample rate in Hz
    #[arg(long, default_value_t = 48000)]
    sample_rate: u32,

    /// Rotation frequency in Hz
    #[arg(long, default_value_t = 1602.564)]
    rotation_hz: f32,

    /// Output filename prefix
    #[arg(long, default_value = "synth")]
    prefix: String,

    /// Generate manifest.json
    #[arg(long)]
    manifest: bool,

    /// AWGN SNR in dB (CLI override)
    #[arg(long)]
    snr: Option<f32>,

    /// Rayleigh fading Doppler spread in Hz (CLI override)
    #[arg(long)]
    fading_hz: Option<f32>,

    /// Impulse noise rate in Hz (CLI override)
    #[arg(long)]
    impulse_rate: Option<f32>,
}

#[derive(Debug, Deserialize, Default)]
struct TomlConfig {
    awgn: Option<AwgnSection>,
    fading: Option<FadingSection>,
    multipath: Option<Vec<MultipathSection>>,
    impulse: Option<ImpulseSection>,
}

#[derive(Debug, Deserialize)]
struct AwgnSection {
    snr_db: f32,
}

#[derive(Debug, Deserialize)]
struct FadingSection {
    #[serde(rename = "type")]
    fading_type: String,
    doppler_spread_hz: f32,
    k_factor: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct MultipathSection {
    delay_samples: usize,
    amplitude: f32,
    phase_offset: f32,
}

#[derive(Debug, Deserialize)]
struct ImpulseSection {
    rate_hz: f32,
    amplitude: f32,
    duration_samples: usize,
}

#[derive(Debug, serde::Serialize)]
struct ManifestEntry {
    file: String,
    bearing: f32,
    trial: u32,
    seed: u64,
}

#[derive(Debug, serde::Serialize)]
struct Manifest {
    sample_rate: u32,
    rotation_hz: f32,
    duration: f32,
    files: Vec<ManifestEntry>,
}

fn parse_bearings(s: &str) -> Result<Vec<f32>> {
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid range format. Use 'start-end:step'");
        }
        let step: f32 = parts[1].parse().context("Invalid step value")?;
        let range_parts: Vec<&str> = parts[0].split('-').collect();
        if range_parts.len() != 2 {
            anyhow::bail!("Invalid range format. Use 'start-end:step'");
        }
        let start: f32 = range_parts[0].parse().context("Invalid start value")?;
        let end: f32 = range_parts[1].parse().context("Invalid end value")?;

        let mut bearings = Vec::new();
        let mut b = start;
        while b <= end {
            bearings.push(b);
            b += step;
        }
        Ok(bearings)
    } else {
        s.split(',')
            .map(|p| p.trim().parse::<f32>().context("Invalid bearing value"))
            .collect()
    }
}

fn load_toml_config(path: &PathBuf) -> Result<TomlConfig> {
    let content = fs::read_to_string(path).context("Failed to read config file")?;
    toml::from_str(&content).context("Failed to parse config file")
}

fn build_noise_config(toml: &TomlConfig, args: &Args, seed: u64) -> NoiseConfig {
    let mut config = NoiseConfig::default().with_seed(seed);

    if let Some(snr) = args.snr {
        config.additive = Some(AdditiveNoiseConfig { snr_db: snr });
    } else if let Some(ref awgn) = toml.awgn {
        config.additive = Some(AdditiveNoiseConfig {
            snr_db: awgn.snr_db,
        });
    }

    if let Some(fading_hz) = args.fading_hz {
        config.fading = Some(FadingConfig {
            fading_type: FadingType::Rayleigh,
            doppler_spread_hz: fading_hz,
        });
    } else if let Some(ref fading) = toml.fading {
        let fading_type = match fading.fading_type.to_lowercase().as_str() {
            "rician" => FadingType::Rician {
                k_factor: fading.k_factor.unwrap_or(4.0),
            },
            _ => FadingType::Rayleigh,
        };
        config.fading = Some(FadingConfig {
            fading_type,
            doppler_spread_hz: fading.doppler_spread_hz,
        });
    }

    if let Some(ref multipath) = toml.multipath
        && !multipath.is_empty()
    {
        config.multipath = Some(MultipathConfig {
            components: multipath
                .iter()
                .map(|m| MultipathComponent {
                    delay_samples: m.delay_samples,
                    amplitude: m.amplitude,
                    phase_offset: m.phase_offset,
                })
                .collect(),
        });
    }

    if let Some(impulse_rate) = args.impulse_rate {
        config.impulse = Some(ImpulseNoiseConfig {
            rate_hz: impulse_rate,
            amplitude: 2.0,
            duration_samples: 5,
        });
    } else if let Some(ref impulse) = toml.impulse {
        config.impulse = Some(ImpulseNoiseConfig {
            rate_hz: impulse.rate_hz,
            amplitude: impulse.amplitude,
            duration_samples: impulse.duration_samples,
        });
    }

    config
}

fn main() -> Result<()> {
    let args = Args::parse();

    fs::create_dir_all(&args.output_dir).context("Failed to create output directory")?;

    let toml_config = if let Some(ref config_path) = args.config {
        load_toml_config(config_path)?
    } else {
        TomlConfig::default()
    };

    let bearings = parse_bearings(&args.bearings)?;
    let base_seed = args.seed.unwrap_or(0);

    let mut manifest_entries = Vec::new();
    let total_files = bearings.len() * args.trials as usize;
    let mut file_count = 0;

    for &bearing in &bearings {
        for trial in 0..args.trials {
            let seed = base_seed + trial as u64 * 1000 + bearing as u64;
            let noise_config = build_noise_config(&toml_config, &args, seed);

            let signal = generate_noisy_test_signal(
                args.duration,
                args.sample_rate,
                args.rotation_hz,
                bearing,
                &noise_config,
            );

            let filename = format!("{}_b{:03}_t{:02}.wav", args.prefix, bearing as i32, trial);
            let filepath = args.output_dir.join(&filename);

            save_wav(filepath.to_str().unwrap(), &signal, args.sample_rate)
                .context("Failed to write WAV file")?;

            manifest_entries.push(ManifestEntry {
                file: filename,
                bearing,
                trial,
                seed,
            });

            file_count += 1;
            eprint!("\rGenerating: {}/{}", file_count, total_files);
        }
    }
    eprintln!();

    if args.manifest {
        let manifest = Manifest {
            sample_rate: args.sample_rate,
            rotation_hz: args.rotation_hz,
            duration: args.duration,
            files: manifest_entries,
        };
        let manifest_path = args.output_dir.join("manifest.json");
        let manifest_json =
            serde_json::to_string_pretty(&manifest).context("Failed to serialize manifest")?;
        fs::write(&manifest_path, manifest_json).context("Failed to write manifest")?;
        eprintln!("Manifest written to: {}", manifest_path.display());
    }

    eprintln!(
        "Generated {} files in {}",
        total_files,
        args.output_dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bearings_comma_separated() {
        let bearings = parse_bearings("45,90,180").unwrap();
        assert_eq!(bearings, vec![45.0, 90.0, 180.0]);
    }

    #[test]
    fn test_parse_bearings_range() {
        let bearings = parse_bearings("0-90:30").unwrap();
        assert_eq!(bearings, vec![0.0, 30.0, 60.0, 90.0]);
    }

    #[test]
    fn test_parse_bearings_range_full_circle() {
        let bearings = parse_bearings("0-360:90").unwrap();
        assert_eq!(bearings, vec![0.0, 90.0, 180.0, 270.0, 360.0]);
    }
}
