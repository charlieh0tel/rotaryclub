use chrono::Utc;
use clap::Parser;
use crossbeam_channel::bounded;
use std::time::{Duration, Instant};

mod audio;
mod config;
mod error;
mod rdf;
mod signal_processing;

use audio::{AudioCapture, AudioRingBuffer};
use config::{BearingMethod, ChannelRole, NorthTrackingMode, RdfConfig};
use rdf::{
    BearingMeasurement, CorrelationBearingCalculator, NorthReferenceTracker, NorthTick,
    ZeroCrossingBearingCalculator,
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
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
    Csv,
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
    fn process_buffer(&mut self, buffer: &[f32], tick: &NorthTick) -> Option<BearingMeasurement> {
        match self {
            BearingCalculator::ZeroCrossing(calc) => calc.process_buffer(buffer, tick),
            BearingCalculator::Correlation(calc) => calc.process_buffer(buffer, tick),
        }
    }
}

fn iso8601_timestamp() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

struct BearingOutput {
    bearing: f32,
    raw: f32,
    confidence: f32,
    snr_db: f32,
    coherence: f32,
    signal_strength: f32,
}

fn format_text(output: &BearingOutput, verbose: bool) -> String {
    if verbose {
        format!(
            "Bearing: {:>6.1}° (raw: {:>6.1}°) conf: {:.2} [SNR: {:>5.1} dB, coh: {:.2}, str: {:.2}]",
            output.bearing,
            output.raw,
            output.confidence,
            output.snr_db,
            output.coherence,
            output.signal_strength
        )
    } else {
        format!(
            "Bearing: {:>6.1}° (raw: {:>6.1}°) confidence: {:.2}",
            output.bearing, output.raw, output.confidence
        )
    }
}

fn format_json(output: &BearingOutput) -> String {
    format!(
        r#"{{"ts":"{}","bearing":{:.1},"raw":{:.1},"confidence":{:.2},"snr_db":{:.1},"coherence":{:.2},"signal_strength":{:.2}}}"#,
        iso8601_timestamp(),
        output.bearing,
        output.raw,
        output.confidence,
        output.snr_db,
        output.coherence,
        output.signal_strength
    )
}

fn format_csv(output: &BearingOutput) -> String {
    format!(
        "{},{:.1},{:.1},{:.2},{:.1},{:.2},{:.2}",
        iso8601_timestamp(),
        output.bearing,
        output.raw,
        output.confidence,
        output.snr_db,
        output.coherence,
        output.signal_strength
    )
}

fn csv_header() -> &'static str {
    "ts,bearing,raw,confidence,snr_db,coherence,signal_strength"
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

    let use_stderr_banner = matches!(args.format, OutputFormat::Json | OutputFormat::Csv);

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

    let (audio_tx, audio_rx) = bounded(10);

    banner!("Starting audio capture...");
    let _capture = AudioCapture::new(&config.audio, audio_tx)?;

    banner!("Audio capture started. Processing...");
    if !use_stderr_banner {
        println!();
    }

    if matches!(args.format, OutputFormat::Csv) {
        println!("{}", csv_header());
    }

    run_processing_loop(audio_rx, config, args.verbose, args.format)?;

    Ok(())
}

fn run_processing_loop(
    audio_rx: crossbeam_channel::Receiver<Vec<f32>>,
    config: RdfConfig,
    verbose: u8,
    format: OutputFormat,
) -> anyhow::Result<()> {
    let sample_rate = config.audio.sample_rate as f32;

    // Initialize processing components
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

    loop {
        // Receive audio data (blocking)
        let audio_data = match audio_rx.recv() {
            Ok(data) => data,
            Err(_) => {
                eprintln!("Audio stream closed");
                break;
            }
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
            }
        }

        if let Some(ref tick) = last_north_tick {
            if let Some(bearing) = bearing_calc.process_buffer(&doppler, tick) {
                // Throttle output
                if last_output.elapsed() >= output_interval {
                    // Apply north offset
                    let mut adjusted_bearing =
                        bearing.bearing_degrees + config.bearing.north_offset_degrees;
                    let mut adjusted_raw =
                        bearing.raw_bearing + config.bearing.north_offset_degrees;

                    // Normalize to [0, 360)
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

                    let line = match format {
                        OutputFormat::Text => format_text(&output, verbose >= 1),
                        OutputFormat::Json => format_json(&output),
                        OutputFormat::Csv => format_csv(&output),
                    };
                    println!("{}", line);
                    last_output = Instant::now();
                }
            }
        } else {
            // Only print warning occasionally to avoid spam
            if last_output.elapsed() >= Duration::from_secs(2) {
                log::warn!("Waiting for north tick...");
                last_output = Instant::now();
            }
        }
    }

    Ok(())
}
