use clap::Parser;
use rolling_stats::Stats;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod output;

use output::{BearingOutput, Formatter, OutputFormat, create_formatter};
use rotaryclub::audio::{AudioRingBuffer, AudioSource, DeviceSource, WavFileSource};
use rotaryclub::config::{
    BearingMethod, ChannelRole, NorthTrackingMode, RdfConfig, RotationFrequency,
};
use rotaryclub::rdf::{
    BearingCalculator, CorrelationBearingCalculator, NorthReferenceTracker, NorthTick,
    NorthTracker, ZeroCrossingBearingCalculator,
};
use rotaryclub::signal_processing::DcRemover;

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

    /// Remove DC offset from audio
    #[arg(long)]
    remove_dc: bool,

    /// Dump audio to WAV file (stereo: left=doppler, right=north_tick)
    #[arg(long)]
    dump_audio: Option<PathBuf>,

    /// North tick input gain in dB (default: 0)
    #[arg(long, default_value = "0")]
    north_tick_gain: f32,
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

    let (source, throttle_output): (Box<dyn AudioSource>, bool) = match &args.input {
        Some(path) => {
            eprintln!("Loading WAV file: {}", path.display());
            let chunk_size = config.audio.buffer_size * 2;
            (Box::new(WavFileSource::new(path, chunk_size)?), false)
        }
        None => {
            eprintln!("Starting audio capture...");
            (Box::new(DeviceSource::new(&config.audio)?), true)
        }
    };

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
    let mut last_output = Instant::now();
    let output_interval = Duration::from_secs_f32(1.0 / config.bearing.output_rate_hz);

    let mut last_north_tick: Option<NorthTick> = None;
    let mut bearing_stats: Stats<f32> = Stats::new();
    let mut rotation_stats: Stats<f32> = Stats::new();

    let mut dc_remover_doppler = DcRemover::with_cutoff(sample_rate, 1.0);
    let mut dc_remover_north = DcRemover::with_cutoff(sample_rate, 1.0);

    // Collects raw audio for --dump-audio (use analyze_wav for filtered output)
    let mut dump_samples: Vec<f32> = Vec::new();

    loop {
        let Some(audio_data) = source.next_buffer()? else {
            break;
        };

        if dump_audio.is_some() {
            dump_samples.extend_from_slice(&audio_data);
        }

        ring_buffer.push_interleaved(&audio_data);

        let samples = ring_buffer.latest(audio_data.len() / 2);
        let stereo_pairs: Vec<(f32, f32)> = samples.iter().map(|s| (s.left, s.right)).collect();
        let (mut doppler, mut north_tick) = config.audio.split_channels(&stereo_pairs);

        if remove_dc {
            dc_remover_doppler.process(&mut doppler);
            dc_remover_north.process(&mut north_tick);
        }

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
                    lock_quality: tick.lock_quality,
                    phase_error_variance: north_tracker.phase_error_variance(),
                };

                bearing_stats.update(adjusted_bearing);
                println!("{}", formatter.format(&output));
                last_output = Instant::now();
            }
        }

        if last_north_tick.is_none()
            && throttle_output
            && last_output.elapsed()
                >= Duration::from_secs_f32(config.bearing.north_tick_warning_timeout_secs)
        {
            log::warn!("Waiting for north tick...");
            last_output = Instant::now();
        }
    }

    if let Some(path) = dump_audio {
        eprintln!(
            "Writing {} samples to {}",
            dump_samples.len() / 2,
            path.display()
        );
        rotaryclub::save_wav(
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid path"))?,
            &dump_samples,
            config.audio.sample_rate,
        )?;
    }

    Ok(ProcessingStats {
        bearing_stats,
        rotation_stats,
    })
}
