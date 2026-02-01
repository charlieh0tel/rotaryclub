use clap::Parser;
use rolling_stats::Stats;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod audio;
mod config;
mod error;
mod output;
mod rdf;
mod signal_processing;

use audio::{AudioRingBuffer, AudioSource, DeviceSource, WavFileSource};
use config::{BearingMethod, ChannelRole, NorthTrackingMode, RdfConfig};
use output::{BearingOutput, Formatter, OutputFormat, create_formatter};
use rdf::{
    CorrelationBearingCalculator, NorthReferenceTracker, NorthTick, ZeroCrossingBearingCalculator,
};

#[derive(Parser, Debug)]
#[command(name = "rotaryclub")]
#[command(about = "Pseudo Doppler Radio Direction Finding", long_about = None)]
struct Args {
    /// Bearing calculation method
    #[arg(short = 'm', long, value_enum, default_value = "correlation")]
    method: BearingMethodArg,

    /// North tick tracking mode
    #[arg(short = 'n', long, value_enum, default_value = "dpll")]
    north_mode: NorthModeArg,

    /// Swap left/right channels
    #[arg(short = 's', long)]
    swap_channels: bool,

    /// Output rate in Hz
    #[arg(short = 'r', long, default_value = "10.0")]
    output_rate: f32,

    /// North reference offset in degrees (added to all bearings)
    #[arg(short = 'o', long, default_value = "0.0")]
    north_offset: f32,

    /// Increase output verbosity (-v for debug, -vv for trace)
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Output format
    #[arg(short = 'f', long, value_enum, default_value = "text")]
    format: OutputFormat,

    /// Input WAV file (default: live device capture)
    #[arg(short = 'i', long)]
    input: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum BearingMethodArg {
    Correlation,
    ZeroCrossing,
}

impl From<BearingMethodArg> for BearingMethod {
    fn from(arg: BearingMethodArg) -> Self {
        match arg {
            BearingMethodArg::Correlation => BearingMethod::Correlation,
            BearingMethodArg::ZeroCrossing => BearingMethod::ZeroCrossing,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum NorthModeArg {
    Dpll,
    Simple,
}

impl From<NorthModeArg> for NorthTrackingMode {
    fn from(arg: NorthModeArg) -> Self {
        match arg {
            NorthModeArg::Dpll => NorthTrackingMode::Dpll,
            NorthModeArg::Simple => NorthTrackingMode::Simple,
        }
    }
}

enum BearingCalculator {
    ZeroCrossing(ZeroCrossingBearingCalculator),
    Correlation(CorrelationBearingCalculator),
}

impl BearingCalculator {
    fn process_buffer(
        &mut self,
        buffer: &[f32],
        tick: &NorthTick,
    ) -> Option<rdf::BearingMeasurement> {
        match self {
            BearingCalculator::ZeroCrossing(calc) => calc.process_buffer(buffer, tick),
            BearingCalculator::Correlation(calc) => calc.process_buffer(buffer, tick),
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Configure logging based on verbosity
    let log_level = match args.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    // Apply CLI arguments to config
    let mut config = RdfConfig::default();
    config.doppler.method = args.method.into();
    config.north_tick.mode = args.north_mode.into();
    config.bearing.output_rate_hz = args.output_rate;
    config.bearing.north_offset_degrees = args.north_offset;

    if args.swap_channels {
        // Swap the channels
        config.audio.doppler_channel = ChannelRole::Right;
        config.audio.north_tick_channel = ChannelRole::Left;
    }

    let use_stderr_banner = !matches!(args.format, OutputFormat::Text);

    macro_rules! banner {
        ($($arg:tt)*) => {
            if use_stderr_banner {
                eprintln!($($arg)*);
            } else {
                println!($($arg)*);
            }
        };
    }

    banner!("=== Rotary Club - Pseudo Doppler RDF ===");
    banner!("Sample rate: {} Hz", config.audio.sample_rate);
    banner!("Expected rotation: {} Hz", config.doppler.expected_freq);
    banner!(
        "Doppler bandpass: {}-{} Hz",
        config.doppler.bandpass_low,
        config.doppler.bandpass_high
    );
    banner!("North tick threshold: {}", config.north_tick.threshold);
    banner!("North tick tracking: {:?}", config.north_tick.mode);
    banner!("Bearing method: {:?}", config.doppler.method);
    banner!("Output rate: {} Hz", config.bearing.output_rate_hz);
    banner!(
        "Channel assignment: Doppler={:?}, North tick={:?}",
        config.audio.doppler_channel,
        config.audio.north_tick_channel
    );
    banner!("");

    let (source, throttle_output): (Box<dyn AudioSource>, bool) = match &args.input {
        Some(path) => {
            banner!("Loading WAV file: {}", path.display());
            let chunk_size = config.audio.buffer_size * 2;
            (Box::new(WavFileSource::new(path, chunk_size)?), false)
        }
        None => {
            banner!("Starting audio capture...");
            (Box::new(DeviceSource::new(&config.audio)?), true)
        }
    };

    banner!("Processing...");
    if !use_stderr_banner {
        println!();
    }

    let formatter = create_formatter(args.format, args.verbose >= 1);
    if let Some(header) = formatter.header() {
        println!("{}", header);
    }

    let stats = run_processing_loop(source, config, formatter, throttle_output)?;

    if args.input.is_some() && stats.bearing_stats.count > 0 {
        banner!("");
        banner!("Bearing statistics:");
        banner!("  Measurements: {}", stats.bearing_stats.count);
        banner!("  Mean: {:.1}°", stats.bearing_stats.mean);
        banner!("  Std dev: {:.1}°", stats.bearing_stats.std_dev);
        banner!("  Min: {:.1}°", stats.bearing_stats.min);
        banner!("  Max: {:.1}°", stats.bearing_stats.max);
        banner!(
            "  Range: {:.1}°",
            stats.bearing_stats.max - stats.bearing_stats.min
        );
    }

    if args.input.is_some() && stats.rotation_stats.count > 0 {
        banner!("");
        banner!("Rotation statistics:");
        banner!("  Measurements: {}", stats.rotation_stats.count);
        banner!("  Mean: {:.1} Hz", stats.rotation_stats.mean);
        banner!("  Std dev: {:.3} Hz", stats.rotation_stats.std_dev);
        banner!("  Min: {:.1} Hz", stats.rotation_stats.min);
        banner!("  Max: {:.1} Hz", stats.rotation_stats.max);
        banner!(
            "  Range: {:.3} Hz",
            stats.rotation_stats.max - stats.rotation_stats.min
        );
    }

    Ok(())
}

struct ProcessingStats {
    bearing_stats: Stats<f32>,
    rotation_stats: Stats<f32>,
}

fn run_processing_loop(
    mut source: Box<dyn AudioSource>,
    config: RdfConfig,
    formatter: Box<dyn Formatter>,
    throttle_output: bool,
) -> anyhow::Result<ProcessingStats> {
    let sample_rate = config.audio.sample_rate as f32;

    let mut north_tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;

    let mut bearing_calc = match config.doppler.method {
        BearingMethod::ZeroCrossing => {
            BearingCalculator::ZeroCrossing(ZeroCrossingBearingCalculator::new(
                &config.doppler,
                &config.agc,
                sample_rate,
                config.bearing.smoothing_window,
            )?)
        }
        BearingMethod::Correlation => {
            BearingCalculator::Correlation(CorrelationBearingCalculator::new(
                &config.doppler,
                &config.agc,
                sample_rate,
                config.bearing.smoothing_window,
            )?)
        }
    };

    let mut ring_buffer = AudioRingBuffer::new();
    let mut last_output = Instant::now();
    let output_interval = Duration::from_secs_f32(1.0 / config.bearing.output_rate_hz);

    let mut last_north_tick: Option<rdf::NorthTick> = None;
    let mut bearing_stats: Stats<f32> = Stats::new();
    let mut rotation_stats: Stats<f32> = Stats::new();

    loop {
        let Some(audio_data) = source.next_buffer()? else {
            break;
        };

        ring_buffer.push_interleaved(&audio_data);

        let samples = ring_buffer.latest(audio_data.len() / 2);
        let stereo_pairs: Vec<(f32, f32)> = samples.iter().map(|s| (s.left, s.right)).collect();
        let (doppler, north_tick) = config.audio.split_channels(&stereo_pairs);

        let north_ticks = north_tracker.process_buffer(&north_tick);

        if let Some(tick) = north_ticks.last() {
            last_north_tick = Some(*tick);

            if let Some(freq) = north_tracker.rotation_frequency() {
                log::debug!("Rotation detected: {:.1} Hz", freq);
                rotation_stats.update(freq);
            }
        }

        let ticks_to_process: Vec<_> = if throttle_output {
            last_north_tick.iter().copied().collect()
        } else {
            north_ticks
        };

        for tick in &ticks_to_process {
            if let Some(bearing) = bearing_calc.process_buffer(&doppler, tick)
                && (!throttle_output || last_output.elapsed() >= output_interval)
            {
                let mut adjusted_bearing =
                    bearing.bearing_degrees + config.bearing.north_offset_degrees;
                let mut adjusted_raw = bearing.raw_bearing + config.bearing.north_offset_degrees;

                adjusted_bearing = adjusted_bearing.rem_euclid(360.0);
                adjusted_raw = adjusted_raw.rem_euclid(360.0);

                let output = BearingOutput {
                    bearing: adjusted_bearing,
                    raw: adjusted_raw,
                    confidence: bearing.confidence,
                    snr_db: bearing.metrics.snr_db,
                    coherence: bearing.metrics.coherence,
                    signal_strength: bearing.metrics.signal_strength,
                };

                bearing_stats.update(adjusted_bearing);
                println!("{}", formatter.format(&output));
                last_output = Instant::now();
            }
        }

        if last_north_tick.is_none()
            && throttle_output
            && last_output.elapsed() >= Duration::from_secs(2)
        {
            log::warn!("Waiting for north tick...");
            last_output = Instant::now();
        }
    }

    Ok(ProcessingStats {
        bearing_stats,
        rotation_stats,
    })
}
