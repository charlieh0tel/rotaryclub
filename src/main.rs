use clap::Parser;
use rolling_stats::Stats;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod output;

use output::{BearingOutput, Formatter, OutputFormat, create_formatter};
use rotaryclub::audio::{AudioSource, DeviceSource, WavFileSource, list_input_devices};
use rotaryclub::config::{
    BearingMethod, ChannelRole, NorthTrackingMode, RdfConfig, RotationFrequency,
};
use rotaryclub::processing::RdfProcessor;

#[derive(Parser, Debug)]
#[command(name = "rotaryclub")]
#[command(about = "Pseudo Doppler Radio Direction Finding", long_about = None)]
struct Args {
    /// Bearing calculation method
    #[arg(short = 'm', long, value_enum, default_value = "correlation")]
    method: BearingMethod,

    /// North tick tracking mode
    #[arg(short = 'n', long, value_enum, default_value = "dpll")]
    north_mode: NorthTrackingMode,

    /// Rotation frequency (e.g., "1602", "1602hz", "624us")
    #[arg(long)]
    rotation: Option<RotationFrequency>,

    /// Swap left/right channels
    #[arg(short = 's', long)]
    swap_channels: bool,

    /// Output rate in Hz
    #[arg(short = 'r', long, default_value = "10.0", value_parser = parse_output_rate)]
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

    /// Remove DC offset from audio
    #[arg(long)]
    remove_dc: bool,

    /// Dump audio to WAV file (stereo: left=doppler, right=north_tick)
    #[arg(long)]
    dump_audio: Option<PathBuf>,

    /// North tick input gain in dB (default: 0)
    #[arg(long, default_value = "0")]
    north_tick_gain: f32,

    /// Select input device by substring match
    #[arg(long)]
    device: Option<String>,

    /// List available input devices and exit
    #[arg(long)]
    list_devices: bool,
}

fn parse_output_rate(s: &str) -> Result<f32, String> {
    let rate: f32 = s.parse().map_err(|_| format!("invalid number: {s}"))?;
    if rate.is_finite() && rate > 0.0 {
        Ok(rate)
    } else {
        Err("output rate must be a positive, finite number of Hz".to_string())
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.list_devices {
        let devices = list_input_devices()?;
        if devices.is_empty() {
            eprintln!("No input devices found.");
        } else {
            for name in &devices {
                println!("{}", name);
            }
        }
        return Ok(());
    }

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
    config.doppler.method = args.method;
    config.north_tick.mode = args.north_mode;
    config.bearing.output_rate_hz = args.output_rate;
    config.bearing.north_offset_degrees = args.north_offset;

    if let Some(rotation) = args.rotation {
        let hz = rotation.as_hz();
        config.doppler.expected_freq = hz;
        config.north_tick.dpll.initial_frequency_hz = hz;
    }

    config.north_tick.gain_db = args.north_tick_gain;

    if args.swap_channels {
        config.audio.doppler_channel = ChannelRole::Right;
        config.audio.north_tick_channel = ChannelRole::Left;
    }

    let (source, throttle_output): (Box<dyn AudioSource>, bool) = match &args.input {
        Some(path) => {
            eprintln!("Loading WAV file: {}", path.display());
            let chunk_size = config.audio.buffer_size * 2;
            (Box::new(WavFileSource::new(path, chunk_size)?), false)
        }
        None => {
            eprintln!("Starting audio capture...");
            (
                Box::new(DeviceSource::new(&config.audio, args.device.as_deref())?),
                true,
            )
        }
    };

    // The DSP chain (filters, DPLL bounds, period scaling) is built from the
    // configured rate, so it must match the source's actual rate.
    config.audio.sample_rate = source.sample_rate();

    eprintln!("=== Rotary Club - Pseudo Doppler RDF ===");
    eprintln!("Sample rate: {} Hz", config.audio.sample_rate);
    eprintln!(
        "Nominal rotation: {} Hz (actual tracked by DPLL)",
        config.doppler.expected_freq
    );
    eprintln!(
        "Doppler bandpass: {}-{} Hz",
        config.doppler.bandpass_low, config.doppler.bandpass_high
    );
    eprintln!("North tick threshold: {}", config.north_tick.threshold);
    eprintln!("North tick tracking: {:?}", config.north_tick.mode);
    eprintln!("Bearing method: {:?}", config.doppler.method);
    eprintln!("Output rate: {} Hz", config.bearing.output_rate_hz);
    eprintln!(
        "Channel assignment: Doppler={:?}, North tick={:?}",
        config.audio.doppler_channel, config.audio.north_tick_channel
    );
    eprintln!();

    eprintln!("Processing...");

    let formatter = create_formatter(args.format, args.verbose >= 1);
    if let Some(header) = formatter.header() {
        println!("{}", header);
    }

    let stats = run_processing_loop(
        source,
        config,
        formatter,
        throttle_output,
        args.remove_dc,
        args.dump_audio.as_deref(),
    )?;

    if args.input.is_some() && stats.bearing_stats.count > 0 {
        eprintln!();
        eprintln!("Bearing statistics:");
        eprintln!("  Measurements: {}", stats.bearing_stats.count);
        eprintln!("  Mean: {:.1}°", stats.bearing_stats.mean);
        eprintln!("  Std dev: {:.1}°", stats.bearing_stats.std_dev);
        eprintln!("  Min: {:.1}°", stats.bearing_stats.min);
        eprintln!("  Max: {:.1}°", stats.bearing_stats.max);
        eprintln!(
            "  Range: {:.1}°",
            stats.bearing_stats.max - stats.bearing_stats.min
        );
    }

    if args.input.is_some() && stats.rotation_stats.count > 0 {
        eprintln!();
        eprintln!("Rotation statistics:");
        eprintln!("  Measurements: {}", stats.rotation_stats.count);
        eprintln!("  Mean: {:.1} Hz", stats.rotation_stats.mean);
        eprintln!("  Std dev: {:.3} Hz", stats.rotation_stats.std_dev);
        eprintln!("  Min: {:.1} Hz", stats.rotation_stats.min);
        eprintln!("  Max: {:.1} Hz", stats.rotation_stats.max);
        eprintln!(
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

// TODO: For device input, implement file rotation to avoid unbounded memory growth
// when using --dump-audio for long recordings.
fn run_processing_loop(
    mut source: Box<dyn AudioSource>,
    config: RdfConfig,
    formatter: Box<dyn Formatter>,
    throttle_output: bool,
    remove_dc: bool,
    dump_audio: Option<&std::path::Path>,
) -> anyhow::Result<ProcessingStats> {
    let mut processor = RdfProcessor::new(&config, remove_dc, true)?;

    let mut last_output = Instant::now();
    let output_interval = Duration::from_secs_f32(1.0 / config.bearing.output_rate_hz);

    let mut bearing_stats: Stats<f32> = Stats::new();
    let mut rotation_stats: Stats<f32> = Stats::new();

    // North-tick staleness tracking in signal time (sample frames), so it
    // also works for faster-than-real-time file input.
    let warning_interval_frames =
        (config.bearing.north_tick_warning_timeout_secs * config.audio.sample_rate as f32) as u64;
    let mut frames_processed: u64 = 0;
    let mut last_tick_frame: u64 = 0;
    let mut next_warning_frame = warning_interval_frames;

    // Streams raw audio to disk for --dump-audio (use analyze_wav for
    // filtered output); long recordings must not accumulate in memory.
    let mut dump_writer = dump_audio
        .map(|path| rotaryclub::WavStreamWriter::create(path, config.audio.sample_rate))
        .transpose()?;

    loop {
        let Some(audio_data) = source.next_buffer()? else {
            break;
        };

        if let Some(writer) = dump_writer.as_mut() {
            writer.write_samples(&audio_data)?;
        }

        let tick_results = processor.process_audio(&audio_data);

        if let Some(freq) = processor.rotation_frequency()
            && !tick_results.is_empty()
        {
            log::debug!("Rotation detected: {:.1} Hz", freq);
            rotation_stats.update(freq);
        }

        for result in &tick_results {
            if let Some(ref bearing) = result.bearing
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
                    lock_quality: result.north_tick.lock_quality,
                    phase_error_variance: processor.phase_error_variance(),
                };

                bearing_stats.update(adjusted_bearing);
                println!("{}", formatter.format(&output));
                last_output = Instant::now();
            }
        }

        // Warn on a missing north reference — both before first acquisition
        // and when the reference disappears mid-run.
        frames_processed += (audio_data.len() / 2) as u64;
        if !tick_results.is_empty() {
            last_tick_frame = frames_processed;
            next_warning_frame = frames_processed + warning_interval_frames;
        } else if frames_processed >= next_warning_frame {
            let silent_secs =
                (frames_processed - last_tick_frame) as f32 / config.audio.sample_rate as f32;
            if processor.last_north_tick().is_none() {
                log::warn!(
                    "Waiting for north tick... ({:.1} s without one)",
                    silent_secs
                );
            } else {
                log::warn!(
                    "No north tick for {:.1} s - check the north reference signal",
                    silent_secs
                );
            }
            next_warning_frame = frames_processed + warning_interval_frames;
        }
    }

    if let Some(writer) = dump_writer {
        eprintln!("Wrote {} sample frames of audio dump", writer.len() / 2);
        writer.finalize()?;
    }

    Ok(ProcessingStats {
        bearing_stats,
        rotation_stats,
    })
}
