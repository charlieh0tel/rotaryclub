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
use rotaryclub::signal_processing::DcRemover;

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

    /// Skip bearing calculation and output
    #[arg(long)]
    no_bearing: bool,

    /// Auto-trim to stable rotation region (based on lock quality)
    #[arg(long)]
    auto_trim: bool,

    /// Lock quality threshold for auto-trim (0.0-1.0)
    #[arg(long, default_value = "0.5")]
    trim_threshold: f32,

    /// Remove DC offset from audio
    #[arg(long)]
    remove_dc: bool,

    /// Dump audio to WAV file (stereo: left=doppler, right=north_tick)
    #[arg(long)]
    dump_audio: Option<PathBuf>,

    /// North tick input gain multiplier (default: 1.0)
    #[arg(long, default_value = "1.0")]
    north_tick_gain: f32,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Csv,
    Json,
}

#[derive(Debug, Clone, Copy)]
struct TrimOptions {
    lock_threshold: f32,
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
    raw_period_us: Option<StatsSummary>,
    dpll_period_us: Option<StatsSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trimmed_range: Option<TrimmedRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TrimmedRange {
    total_ticks: usize,
    used_ticks: usize,
    start_tick: usize,
    end_tick: usize,
    dropouts: usize,
    dropout_positions: Vec<f32>,
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

    config.north_tick.gain = args.north_tick_gain;

    if args.swap_channels {
        config.audio.doppler_channel = ChannelRole::Right;
        config.audio.north_tick_channel = ChannelRole::Left;
    }

    let trim_opts = if args.auto_trim {
        Some(TrimOptions {
            lock_threshold: args.trim_threshold,
        })
    } else {
        None
    };

    let results: Vec<FileAnalysis> = args
        .files
        .iter()
        .map(|path| {
            analyze_file(
                path,
                &config,
                args.no_bearing,
                trim_opts.as_ref(),
                args.remove_dc,
                args.dump_audio.as_deref(),
            )
        })
        .collect();

    match args.format {
        OutputFormat::Text => print_text(&results, &config),
        OutputFormat::Csv => print_csv(&results),
        OutputFormat::Json => print_json(&results)?,
    }

    Ok(())
}

fn analyze_file(
    path: &PathBuf,
    config: &RdfConfig,
    no_bearing: bool,
    trim_opts: Option<&TrimOptions>,
    remove_dc: bool,
    dump_audio: Option<&std::path::Path>,
) -> FileAnalysis {
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    match analyze_file_impl(path, config, no_bearing, trim_opts, remove_dc, dump_audio) {
        Ok(analysis) => analysis,
        Err(e) => FileAnalysis {
            filename,
            rotation_freq: None,
            lock_quality: None,
            phase_error_variance: None,
            bearing: None,
            sample_count: 0,
            raw_period_us: None,
            dpll_period_us: None,
            trimmed_range: None,
            error: Some(e.to_string()),
        },
    }
}

struct CollectedTick {
    sample_index: usize,
    lock_quality: Option<f32>,
    period: Option<f32>,
    frequency: Option<f32>,
    bearing: Option<f32>,
    phase_error_variance: Option<f32>,
}

struct StableRegion {
    start: usize,
    end: usize,
    dropouts: usize,
    dropout_positions: Vec<usize>,
}

fn find_stable_region(ticks: &[CollectedTick], threshold: f32) -> StableRegion {
    if ticks.is_empty() {
        return StableRegion {
            start: 0,
            end: 0,
            dropouts: 0,
            dropout_positions: Vec::new(),
        };
    }

    let mut start = 0;
    let mut end = ticks.len();

    // Find first tick with lock quality above threshold
    for (i, tick) in ticks.iter().enumerate() {
        if tick.lock_quality.unwrap_or(0.0) >= threshold {
            start = i;
            break;
        }
    }

    // Find last tick with lock quality above threshold
    for (i, tick) in ticks.iter().enumerate().rev() {
        if tick.lock_quality.unwrap_or(0.0) >= threshold {
            end = i + 1;
            break;
        }
    }

    // Ensure valid range
    if start >= end {
        return StableRegion {
            start: 0,
            end: ticks.len(),
            dropouts: 0,
            dropout_positions: Vec::new(),
        };
    }

    // Count dropouts within the stable region and record positions
    let mut dropouts = 0;
    let mut dropout_positions = Vec::new();
    let mut was_locked = true;
    for (i, tick) in ticks[start..end].iter().enumerate() {
        let is_locked = tick.lock_quality.unwrap_or(0.0) >= threshold;
        if was_locked && !is_locked {
            dropouts += 1;
            dropout_positions.push(start + i);
        }
        was_locked = is_locked;
    }

    StableRegion {
        start,
        end,
        dropouts,
        dropout_positions,
    }
}

fn analyze_file_impl(
    path: &PathBuf,
    config: &RdfConfig,
    no_bearing: bool,
    trim_opts: Option<&TrimOptions>,
    remove_dc: bool,
    dump_audio: Option<&std::path::Path>,
) -> anyhow::Result<FileAnalysis> {
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    let chunk_size = config.audio.buffer_size * 2;
    let mut source: Box<dyn AudioSource> = Box::new(WavFileSource::new(path, chunk_size)?);
    let sample_rate = config.audio.sample_rate as f32;

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;

    let mut bearing_calc: Option<Box<dyn BearingCalculator>> = if no_bearing {
        None
    } else {
        Some(match config.doppler.method {
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
        })
    };

    let mut ring_buffer = AudioRingBuffer::new();
    let mut collected_ticks: Vec<CollectedTick> = Vec::new();

    let mut dc_remover_doppler = DcRemover::with_cutoff(sample_rate, 1.0);
    let mut dc_remover_north = DcRemover::with_cutoff(sample_rate, 1.0);

    let mut dump_samples: Vec<f32> = Vec::new();

    loop {
        let Some(audio_data) = source.next_buffer()? else {
            break;
        };

        ring_buffer.push_interleaved(&audio_data);

        let samples = ring_buffer.latest(audio_data.len() / 2);
        let stereo_pairs: Vec<(f32, f32)> = samples.iter().map(|s| (s.left, s.right)).collect();
        let (mut doppler, mut north_tick) = config.audio.split_channels(&stereo_pairs);

        if remove_dc {
            dc_remover_doppler.process(&mut doppler);
            dc_remover_north.process(&mut north_tick);
        }

        let north_ticks = north_tracker.process_buffer(&north_tick);

        if let Some(ref mut calc) = bearing_calc {
            calc.preprocess(&doppler);
        }

        // Dump filtered signals (after bandpass/highpass filters)
        if dump_audio.is_some() {
            let filtered_doppler = bearing_calc
                .as_ref()
                .map(|c| c.filtered_buffer())
                .unwrap_or(&doppler);
            let filtered_north = north_tracker.filtered_buffer();
            for (&d, &n) in filtered_doppler.iter().zip(filtered_north.iter()) {
                dump_samples.push(d);
                dump_samples.push(n);
            }
        }

        for tick in &north_ticks {
            let bearing = if let Some(ref mut calc) = bearing_calc {
                calc.process_tick(tick).map(|b| b.bearing_degrees)
            } else {
                None
            };

            collected_ticks.push(CollectedTick {
                sample_index: tick.sample_index,
                lock_quality: tick.lock_quality,
                period: tick.period,
                frequency: north_tracker.rotation_frequency(),
                bearing,
                phase_error_variance: north_tracker.phase_error_variance(),
            });
        }

        if let Some(ref mut calc) = bearing_calc {
            calc.advance_buffer();
        }
    }

    // Determine range to analyze
    let total_ticks = collected_ticks.len();
    let (start, end, trimmed_range) = if let Some(opts) = trim_opts {
        let region = find_stable_region(&collected_ticks, opts.lock_threshold);
        let dropout_positions: Vec<f32> = region
            .dropout_positions
            .iter()
            .map(|&pos| 100.0 * pos as f32 / total_ticks as f32)
            .collect();
        (
            region.start,
            region.end,
            Some(TrimmedRange {
                total_ticks,
                used_ticks: region.end - region.start,
                start_tick: region.start,
                end_tick: region.end,
                dropouts: region.dropouts,
                dropout_positions,
            }),
        )
    } else {
        (0, total_ticks, None)
    };

    // Compute statistics on the selected range
    let mut rotation_stats: Stats<f32> = Stats::new();
    let mut lock_quality_stats: Stats<f32> = Stats::new();
    let mut bearing_stats: Stats<f32> = Stats::new();
    let mut raw_period_stats: Stats<f32> = Stats::new();
    let mut dpll_period_stats: Stats<f32> = Stats::new();
    let mut last_phase_error_variance: Option<f32> = None;
    let mut last_sample_index: Option<usize> = None;

    for tick in &collected_ticks[start..end] {
        if let Some(last_sample) = last_sample_index {
            let raw_interval = (tick.sample_index - last_sample) as f32;
            raw_period_stats.update(raw_interval);
        }
        last_sample_index = Some(tick.sample_index);

        if let Some(period) = tick.period {
            dpll_period_stats.update(period);
        }
        if let Some(freq) = tick.frequency {
            rotation_stats.update(freq);
        }
        if let Some(lq) = tick.lock_quality {
            lock_quality_stats.update(lq);
        }
        if let Some(bearing) = tick.bearing {
            bearing_stats.update(bearing);
        }
        last_phase_error_variance = tick.phase_error_variance;
    }

    let scale = 1_000_000.0 / sample_rate;

    let raw_period_us = StatsSummary::from_stats(&raw_period_stats).map(|s| StatsSummary {
        count: s.count,
        mean: s.mean * scale,
        std_dev: s.std_dev * scale,
        min: s.min * scale,
        max: s.max * scale,
    });

    let dpll_period_us = StatsSummary::from_stats(&dpll_period_stats).map(|s| StatsSummary {
        count: s.count,
        mean: s.mean * scale,
        std_dev: s.std_dev * scale,
        min: s.min * scale,
        max: s.max * scale,
    });

    if let Some(dump_dir) = dump_audio {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "output".to_string());
        let dump_path = dump_dir.join(format!("{}_split.wav", stem));
        eprintln!(
            "Writing {} samples to {}",
            dump_samples.len() / 2,
            dump_path.display()
        );
        rotaryclub::save_wav(
            dump_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid path"))?,
            &dump_samples,
            config.audio.sample_rate,
        )?;
    }

    Ok(FileAnalysis {
        filename,
        rotation_freq: StatsSummary::from_stats(&rotation_stats),
        lock_quality: StatsSummary::from_stats(&lock_quality_stats),
        phase_error_variance: last_phase_error_variance,
        bearing: StatsSummary::from_stats(&bearing_stats),
        sample_count: rotation_stats.count,
        raw_period_us,
        dpll_period_us,
        trimmed_range,
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
        "{:<60} {:>12} {:>8} {:>10} {:>10} {:>8}",
        "File", "Rotation", "Std", "LockQual", "PhaseVar", "Samples"
    );
    println!("{}", "-".repeat(113));

    for result in results {
        if let Some(ref err) = result.error {
            println!("{:<60} ERROR: {}", result.filename, err);
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
            "{:<60} {:>12} {:>8} {:>10} {:>10} {:>8}",
            result.filename, rotation_mean, rotation_std, lock_qual, phase_var, result.sample_count
        );
    }

    for result in results {
        if result.error.is_some() {
            continue;
        }

        if result.raw_period_us.is_some() || result.dpll_period_us.is_some() {
            eprintln!();
            eprintln!("Rotation timing for {}:", result.filename);
            if let Some(ref raw) = result.raw_period_us {
                eprintln!(
                    "  Raw period:  {:.2} ± {:.2} μs (jitter)",
                    raw.mean, raw.std_dev
                );
            }
            if let Some(ref dpll) = result.dpll_period_us {
                eprintln!("  DPLL period: {:.2} ± {:.2} μs", dpll.mean, dpll.std_dev);
            }
            if let Some(ref trim) = result.trimmed_range {
                let start_pct = 100.0 * trim.start_tick as f32 / trim.total_ticks as f32;
                let end_pct = 100.0 * trim.end_tick as f32 / trim.total_ticks as f32;
                eprintln!(
                    "  Lock region: {:.1}% - {:.1}% ({} of {} ticks, {} dropouts)",
                    start_pct, end_pct, trim.used_ticks, trim.total_ticks, trim.dropouts
                );
                if trim.dropouts > 0 {
                    let positions: Vec<String> = trim
                        .dropout_positions
                        .iter()
                        .map(|p| format!("{:.1}%", p))
                        .collect();
                    eprintln!("  Dropout positions: {}", positions.join(", "));
                }
            }
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
        "filename,rotation_mean,rotation_std,lock_quality,phase_error_variance,bearing_mean,bearing_std,raw_period_us,raw_jitter_us,dpll_period_us,dpll_jitter_us,sample_count,error"
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
        let raw_period = result
            .raw_period_us
            .as_ref()
            .map(|s| format!("{:.2}", s.mean))
            .unwrap_or_default();
        let raw_jitter = result
            .raw_period_us
            .as_ref()
            .map(|s| format!("{:.2}", s.std_dev))
            .unwrap_or_default();
        let dpll_period = result
            .dpll_period_us
            .as_ref()
            .map(|s| format!("{:.2}", s.mean))
            .unwrap_or_default();
        let dpll_jitter = result
            .dpll_period_us
            .as_ref()
            .map(|s| format!("{:.2}", s.std_dev))
            .unwrap_or_default();
        let error = result.error.as_deref().unwrap_or("");

        println!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}",
            result.filename,
            rotation_mean,
            rotation_std,
            lock_qual,
            phase_var,
            bearing_mean,
            bearing_std,
            raw_period,
            raw_jitter,
            dpll_period,
            dpll_jitter,
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
