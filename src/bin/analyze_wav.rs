use clap::Parser;
use rolling_stats::Stats;
use serde::Serialize;
use std::path::PathBuf;

use rotaryclub::audio::{AudioRingBuffer, AudioSource, WavFileSource};
use rotaryclub::config::{
    BearingMethod, ChannelRole, NorthTrackingMode, RdfConfig, RotationFrequency,
};
use rotaryclub::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthReferenceTracker, NorthTracker,
    ZeroCrossingBearingCalculator,
};

#[derive(Parser, Debug)]
#[command(name = "analyze_wav")]
#[command(about = "Analyze WAV files for pseudo-Doppler RDF statistics", long_about = None)]
struct Args {
    /// WAV files to analyze
    #[arg(required = true)]
    files: Vec<PathBuf>,

    /// Output format: text, csv, json
    #[arg(short = 'f', long, value_enum, default_value = "text")]
    format: OutputFormat,

    /// Swap left/right channels
    #[arg(short = 's', long)]
    swap_channels: bool,

    /// Increase output verbosity
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// North tracking mode: simple, dpll
    #[arg(short = 'n', long, value_enum, default_value = "dpll")]
    north_mode: NorthTrackingMode,

    /// Bearing calculation method: correlation, zero-crossing
    #[arg(short = 'm', long, value_enum, default_value = "correlation")]
    method: BearingMethod,

    /// Rotation frequency (e.g., "1602.564", "624us")
    #[arg(long)]
    rotation: Option<RotationFrequency>,

    /// Bandpass filter lower cutoff in Hz
    #[arg(long, default_value = "1350")]
    bandpass_low: f32,

    /// Bandpass filter upper cutoff in Hz
    #[arg(long, default_value = "1850")]
    bandpass_high: f32,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Csv,
    Json,
}

#[derive(Debug, Clone, Serialize)]
struct StatsSummary {
    count: usize,
    mean: f32,
    std_dev: f32,
    min: f32,
    max: f32,
}

impl StatsSummary {
    fn from_stats(stats: &Stats<f32>) -> Option<Self> {
        if stats.count == 0 {
            return None;
        }
        Some(Self {
            count: stats.count,
            mean: stats.mean,
            std_dev: stats.std_dev,
            min: stats.min,
            max: stats.max,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct FileAnalysis {
    filename: String,
    rotation_freq: Option<StatsSummary>,
    lock_quality: Option<StatsSummary>,
    phase_error_variance: Option<f32>,
    bearing: Option<StatsSummary>,
    sample_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let log_level = match args.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    let mut config = RdfConfig::default();
    config.doppler.method = args.method;
    config.north_tick.mode = args.north_mode;
    config.doppler.bandpass_low = args.bandpass_low;
    config.doppler.bandpass_high = args.bandpass_high;

    if let Some(rotation) = args.rotation {
        let hz = rotation.as_hz();
        config.doppler.expected_freq = hz;
        config.north_tick.dpll.initial_frequency_hz = hz;
    }

    if args.swap_channels {
        config.audio.doppler_channel = ChannelRole::Right;
        config.audio.north_tick_channel = ChannelRole::Left;
    }

    let results: Vec<FileAnalysis> = args
        .files
        .iter()
        .map(|path| analyze_file(path, &config))
        .collect();

    match args.format {
        OutputFormat::Text => print_text(&results, &config),
        OutputFormat::Csv => print_csv(&results),
        OutputFormat::Json => print_json(&results)?,
    }

    Ok(())
}

fn analyze_file(path: &PathBuf, config: &RdfConfig) -> FileAnalysis {
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    match analyze_file_impl(path, config) {
        Ok(analysis) => analysis,
        Err(e) => FileAnalysis {
            filename,
            rotation_freq: None,
            lock_quality: None,
            phase_error_variance: None,
            bearing: None,
            sample_count: 0,
            error: Some(e.to_string()),
        },
    }
}

fn analyze_file_impl(path: &PathBuf, config: &RdfConfig) -> anyhow::Result<FileAnalysis> {
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    let chunk_size = config.audio.buffer_size * 2;
    let mut source: Box<dyn AudioSource> = Box::new(WavFileSource::new(path, chunk_size)?);
    let sample_rate = config.audio.sample_rate as f32;

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;

    let mut bearing_calc: Box<dyn BearingCalculator> = match config.doppler.method {
        BearingMethod::ZeroCrossing => Box::new(ZeroCrossingBearingCalculator::new(
            &config.doppler,
            &config.agc,
            sample_rate,
            config.bearing.smoothing_window,
        )?),
        BearingMethod::Correlation => Box::new(CorrelationBearingCalculator::new(
            &config.doppler,
            &config.agc,
            sample_rate,
            config.bearing.smoothing_window,
        )?),
    };

    let mut ring_buffer = AudioRingBuffer::new();

    let mut rotation_stats: Stats<f32> = Stats::new();
    let mut lock_quality_stats: Stats<f32> = Stats::new();
    let mut bearing_stats: Stats<f32> = Stats::new();
    let mut last_phase_error_variance: Option<f32> = None;

    loop {
        let Some(audio_data) = source.next_buffer()? else {
            break;
        };

        ring_buffer.push_interleaved(&audio_data);

        let samples = ring_buffer.latest(audio_data.len() / 2);
        let stereo_pairs: Vec<(f32, f32)> = samples.iter().map(|s| (s.left, s.right)).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo_pairs);

        let north_ticks = north_tracker.process_buffer(&north_tick);

        // Preprocess once for all ticks in this buffer
        bearing_calc.preprocess(&doppler);

        for tick in &north_ticks {
            if let Some(freq) = north_tracker.rotation_frequency() {
                rotation_stats.update(freq);
            }
            if let Some(lq) = tick.lock_quality {
                lock_quality_stats.update(lq);
            }
            last_phase_error_variance = north_tracker.phase_error_variance();

            if let Some(bearing) = bearing_calc.process_tick(tick) {
                bearing_stats.update(bearing.bearing_degrees);
            }
        }

        // Advance counter after processing all ticks
        bearing_calc.advance_buffer();
    }

    Ok(FileAnalysis {
        filename,
        rotation_freq: StatsSummary::from_stats(&rotation_stats),
        lock_quality: StatsSummary::from_stats(&lock_quality_stats),
        phase_error_variance: last_phase_error_variance,
        bearing: StatsSummary::from_stats(&bearing_stats),
        sample_count: rotation_stats.count,
        error: None,
    })
}

fn print_text(results: &[FileAnalysis], config: &RdfConfig) {
    eprintln!(
        "Channels: Doppler={:?}, NorthTick={:?}",
        config.audio.doppler_channel, config.audio.north_tick_channel
    );
    eprintln!();

    println!(
        "{:<50} {:>12} {:>8} {:>10} {:>10} {:>8}",
        "File", "Rotation", "Std", "LockQual", "PhaseVar", "Samples"
    );
    println!("{}", "-".repeat(100));

    for result in results {
        if let Some(ref err) = result.error {
            println!("{:<50} ERROR: {}", result.filename, err);
            continue;
        }

        let rotation_mean = result
            .rotation_freq
            .as_ref()
            .map(|s| format!("{:.4}", s.mean))
            .unwrap_or_else(|| "-".to_string());
        let rotation_std = result
            .rotation_freq
            .as_ref()
            .map(|s| format!("{:.4}", s.std_dev))
            .unwrap_or_else(|| "-".to_string());
        let lock_qual = result
            .lock_quality
            .as_ref()
            .map(|s| format!("{:.2}", s.mean))
            .unwrap_or_else(|| "-".to_string());
        let phase_var = result
            .phase_error_variance
            .map(|v| format!("{:.6}", v))
            .unwrap_or_else(|| "-".to_string());

        println!(
            "{:<50} {:>12} {:>8} {:>10} {:>10} {:>8}",
            result.filename, rotation_mean, rotation_std, lock_qual, phase_var, result.sample_count
        );
    }

    for result in results {
        if result.error.is_some() {
            continue;
        }
        if let Some(ref bearing) = result.bearing {
            eprintln!();
            eprintln!("Bearing statistics for {}:", result.filename);
            eprintln!("  Mean: {:.1}°", bearing.mean);
            eprintln!("  Std dev: {:.1}°", bearing.std_dev);
            eprintln!("  Min: {:.1}°", bearing.min);
            eprintln!("  Max: {:.1}°", bearing.max);
            eprintln!("  Range: {:.1}°", bearing.max - bearing.min);
        }
    }
}

fn print_csv(results: &[FileAnalysis]) {
    println!(
        "filename,rotation_mean,rotation_std,lock_quality,phase_error_variance,bearing_mean,bearing_std,sample_count,error"
    );
    for result in results {
        let rotation_mean = result
            .rotation_freq
            .as_ref()
            .map(|s| format!("{:.6}", s.mean))
            .unwrap_or_default();
        let rotation_std = result
            .rotation_freq
            .as_ref()
            .map(|s| format!("{:.6}", s.std_dev))
            .unwrap_or_default();
        let lock_qual = result
            .lock_quality
            .as_ref()
            .map(|s| format!("{:.4}", s.mean))
            .unwrap_or_default();
        let phase_var = result
            .phase_error_variance
            .map(|v| format!("{:.8}", v))
            .unwrap_or_default();
        let bearing_mean = result
            .bearing
            .as_ref()
            .map(|s| format!("{:.2}", s.mean))
            .unwrap_or_default();
        let bearing_std = result
            .bearing
            .as_ref()
            .map(|s| format!("{:.2}", s.std_dev))
            .unwrap_or_default();
        let error = result.error.as_deref().unwrap_or("");

        println!(
            "{},{},{},{},{},{},{},{},{}",
            result.filename,
            rotation_mean,
            rotation_std,
            lock_qual,
            phase_var,
            bearing_mean,
            bearing_std,
            result.sample_count,
            error
        );
    }
}

fn print_json(results: &[FileAnalysis]) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(results)?;
    println!("{}", json);
    Ok(())
}
