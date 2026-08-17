use clap::Parser;
use rolling_stats::Stats;
use std::path::PathBuf;

mod output;

use output::{BearingOutput, Formatter, OutputFormat, create_formatter};
use rotaryclub::audio::{AudioSource, DeviceSource, WavFileSource, list_input_devices};
use rotaryclub::config::{
    BearingMethod, ChannelRole, NorthPulseEstimator, NorthTrackingMode, RdfConfig,
    RotationFrequency,
};
use rotaryclub::processing::RdfProcessor;
use rotaryclub::stats::CircularStats;

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

    /// North pulse sub-sample estimator
    #[arg(long, value_enum, default_value = "energy-centroid")]
    north_estimator: NorthPulseEstimator,

    /// Rotation frequency (e.g., "1602", "1602hz", "624us")
    #[arg(long)]
    rotation: Option<RotationFrequency>,

    /// Swap left/right channels
    #[arg(short = 's', long)]
    swap_channels: bool,

    /// Output rate in Hz
    #[arg(short = 'r', long, default_value = "10.0", value_parser = rotaryclub::cli::parse_output_rate)]
    output_rate: f32,

    /// North reference offset in degrees (added to all bearings)
    #[arg(short = 'o', long, default_value = "0.0", value_parser = rotaryclub::cli::parse_north_offset,
          allow_negative_numbers = true)]
    north_offset: f32,

    /// Increase output verbosity (-v for info, -vv for debug, -vvv for trace)
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
    config.north_tick.estimator = args.north_estimator;
    config.bearing.output_rate_hz = args.output_rate;
    config.bearing.north_offset_degrees = args.north_offset;

    if let Some(rotation) = args.rotation {
        config.apply_rotation(rotation);
    }

    config.north_tick.gain_db = args.north_tick_gain;

    if args.swap_channels {
        config.audio.doppler_channel = ChannelRole::Right;
        config.audio.north_tick_channel = ChannelRole::Left;
    }

    // The dump writer truncates its path on creation, so pointing it at the
    // input would destroy the recording being read -- the reader is already
    // open, and the data is gone before the first buffer arrives. Compared by
    // identity rather than by string, so a symlink or a relative spelling of
    // the same file is caught too.
    if let (Some(input), Some(dump)) = (&args.input, &args.dump_audio)
        && rotaryclub::cli::same_file(input, dump)
    {
        anyhow::bail!(
            "--dump-audio points at the input file {}; writing would destroy it",
            input.display()
        );
    }

    // Validate the whole configuration before any input is opened. The
    // real processor is built later against the source's actual sample
    // rate; this early pass costs one throwaway construction and stops a
    // config error from lighting the microphone first -- the stream used to
    // start capturing and then die on the same error moments later.
    rotaryclub::RdfProcessor::new(&config, args.remove_dc, true).map(drop)?;

    let (source, is_file_input): (Box<dyn AudioSource>, bool) = match &args.input {
        Some(path) => {
            eprintln!("Loading WAV file: {}", path.display());
            let chunk_size = config.audio.buffer_size * 2;
            (Box::new(WavFileSource::new(path, chunk_size)?), true)
        }
        None => {
            eprintln!("Starting audio capture...");
            (
                Box::new(DeviceSource::new(&config.audio, args.device.as_deref())?),
                false,
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
    eprintln!(
        "North tick threshold: {} of the expected pulse",
        config.north_tick.resolved_threshold_fraction()
    );
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
        is_file_input,
        args.remove_dc,
        args.dump_audio.as_deref(),
    )?;

    if args.input.is_some()
        && let Some(bearing) = stats.bearing_stats.summary()
    {
        eprintln!();
        eprintln!("Bearing statistics:");
        eprintln!("  Measurements: {}", bearing.count);
        eprintln!("  Mean: {:.1}°", bearing.mean);
        eprintln!("  Std dev: {:.1}°", bearing.std_dev);
        eprintln!("  Min: {:.1}°", bearing.min);
        eprintln!("  Max: {:.1}°", bearing.max);
        eprintln!("  Range: {:.1}°", bearing.range);
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
    bearing_stats: CircularStats,
    rotation_stats: Stats<f32>,
}

fn run_processing_loop(
    mut source: Box<dyn AudioSource>,
    config: RdfConfig,
    formatter: Box<dyn Formatter>,
    is_file_input: bool,
    remove_dc: bool,
    dump_audio: Option<&std::path::Path>,
) -> anyhow::Result<ProcessingStats> {
    let mut processor = RdfProcessor::new(&config, remove_dc, true)?;

    // Output throttling runs in signal time (sample frames), not wall
    // clock: on live capture the two agree, and on file input a wall clock
    // would let a faster-than-real-time decode emit every rotation's tick
    // regardless of the configured rate.
    let output_interval_frames =
        config.audio.sample_rate as f64 / config.bearing.output_rate_hz as f64;
    let mut next_output_frame: f64 = 0.0;

    let mut bearing_stats = CircularStats::new();
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

    // Emit a bearing line for one tick, honoring the output-rate throttle
    // unless forced (used for the terminal end-of-stream tick).
    let emit_tick = |result: &rotaryclub::processing::TickResult,
                     phase_error_variance: Option<f32>,
                     bearing_stats: &mut CircularStats,
                     next_output_frame: &mut f64,
                     force: bool| {
        if let Some(ref bearing) = result.bearing {
            let adjusted_bearing =
                (bearing.bearing_degrees + config.bearing.north_offset_degrees).rem_euclid(360.0);
            // Only for file input, where the summary is printed at the
            // end. Live mode ran this Vec unbounded -- about 864,000
            // entries a day at the default rate -- for a summary nothing
            // ever read. Fed before the throttle so the summary covers
            // every tick, not just the emitted lines.
            if is_file_input {
                bearing_stats.update(adjusted_bearing);
            }
            if !force && (result.north_tick.sample_index as f64) < *next_output_frame {
                return;
            }
            let adjusted_raw =
                (bearing.raw_bearing + config.bearing.north_offset_degrees).rem_euclid(360.0);
            let output = BearingOutput {
                bearing: adjusted_bearing,
                raw: adjusted_raw,
                confidence: bearing.confidence,
                snr_db: bearing.metrics.snr_db,
                bearing_uncertainty_deg: bearing.metrics.bearing_uncertainty_deg,
                signal_strength: bearing.metrics.signal_strength,
                signal_present: bearing.signal_present,
                tone_peak: bearing.metrics.tone_peak,
                resultant_length: bearing.metrics.resultant_length,
                lock_quality: result.north_tick.lock_quality,
                phase_error_variance,
            };
            // Empty means the formatter has no honest encoding for this
            // record (KN5R with a non-finite bearing); print nothing rather
            // than a blank line in a fixed-width stream.
            let line = formatter.format(&output);
            if !line.is_empty() {
                println!("{line}");
            }
            *next_output_frame = result.north_tick.sample_index as f64 + output_interval_frames;
        }
    };

    // Ctrl-C requests a clean exit from the loop rather than killing the
    // process: the default SIGINT termination skipped the dump writer's
    // finalize, leaving buffered samples unwritten and the WAV header's
    // size fields stale.
    let interrupted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let interrupted = std::sync::Arc::clone(&interrupted);
        // A second Ctrl-C falls back to the default handler via the flag
        // check below being moot -- ctrlc keeps the handler installed, so
        // pressing it again while a slow source blocks still only sets the
        // flag; the source read itself is not interruptible.
        let _ = ctrlc::set_handler(move || {
            interrupted.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }

    while let Some(audio_data) = source.next_buffer()? {
        if interrupted.load(std::sync::atomic::Ordering::SeqCst) {
            eprintln!("Interrupted; finalizing...");
            break;
        }
        processor.advance_samples(source.take_dropped_frames());

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

        let phase_error_variance = processor.phase_error_variance();
        for result in &tick_results {
            emit_tick(
                result,
                phase_error_variance,
                &mut bearing_stats,
                &mut next_output_frame,
                false,
            );
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

    // Emit any tick whose search window was still pending at end-of-stream.
    let final_ticks = processor.finish();
    if let Some(freq) = processor.rotation_frequency()
        && !final_ticks.is_empty()
    {
        rotation_stats.update(freq);
    }
    let phase_error_variance = processor.phase_error_variance();
    for result in &final_ticks {
        emit_tick(
            result,
            phase_error_variance,
            &mut bearing_stats,
            &mut next_output_frame,
            true,
        );
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
