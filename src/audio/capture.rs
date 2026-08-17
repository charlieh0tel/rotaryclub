use crate::config::AudioConfig;
use crate::error::{RdfError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, TrySendError};

pub fn list_input_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|e| RdfError::AudioDevice(format!("Failed to enumerate devices: {}", e)))?;
    let mut names = Vec::new();
    for device in devices {
        if let Ok(desc) = device.description() {
            names.push(desc.name().to_string());
        }
    }
    Ok(names)
}

/// A buffer of interleaved samples, tagged with the number of frames
/// (per-channel samples) dropped immediately before it because the consumer
/// could not keep up. The gap travels with the chunk so the consumer can
/// advance the DSP clock at the exact point in the stream where audio was
/// lost, rather than at dequeue time (up to a full queue later).
pub struct AudioChunk {
    pub gap_before_frames: usize,
    pub samples: Vec<f32>,
}

/// Message from the capture callbacks: a gap-tagged chunk, or a fatal stream
/// error that ends capture.
pub type AudioMessage = std::result::Result<AudioChunk, cpal::StreamError>;

pub struct AudioCapture {
    stream: cpal::Stream,
    sample_rate: u32,
}

/// Render a device's supported input configs for an error message, so a
/// refusal tells the user what the device would have accepted.
fn describe_configs(device: &cpal::Device) -> String {
    let Ok(ranges) = device.supported_input_configs() else {
        return "unavailable".to_string();
    };
    let described: Vec<String> = ranges
        .map(|r| {
            let (lo, hi) = (r.min_sample_rate(), r.max_sample_rate());
            if lo == hi {
                format!("{} ch {} @ {} Hz", r.channels(), r.sample_format(), lo)
            } else {
                format!(
                    "{} ch {} @ {}-{} Hz",
                    r.channels(),
                    r.sample_format(),
                    lo,
                    hi
                )
            }
        })
        .collect();
    if described.is_empty() {
        "none".to_string()
    } else {
        described.join(", ")
    }
}

impl AudioCapture {
    /// The rate the stream actually runs at, which the DSP chain must be
    /// built against; it may differ from the configured rate when the
    /// device cannot do that rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn new(
        config: &AudioConfig,
        tx: Sender<AudioMessage>,
        device_name: Option<&str>,
    ) -> Result<Self> {
        let host = cpal::default_host();

        let device = if let Some(name) = device_name {
            let mut found = None;
            let devices = host.input_devices().map_err(|e| {
                RdfError::AudioDevice(format!("Failed to enumerate devices: {}", e))
            })?;
            for d in devices {
                if let Ok(desc) = d.description()
                    && desc.name().to_lowercase().contains(&name.to_lowercase())
                {
                    found = Some(d);
                    break;
                }
            }
            found.ok_or_else(|| {
                RdfError::AudioDevice(format!("No input device matching '{}'", name))
            })?
        } else {
            host.default_input_device()
                .ok_or_else(|| RdfError::AudioDevice("No input device found".into()))?
        };

        match device.description() {
            Ok(desc) => log::info!("Input device: {:?}", desc),
            Err(_) => log::info!("Input device: Unknown"),
        }

        // Negotiate the stream format instead of demanding f32 at the
        // configured rate: a device that only does i16, or only 44.1 kHz,
        // is perfectly usable -- the DSP chain is built against whatever
        // rate the source reports. Prefer f32 at the configured rate, then
        // f32 at the nearest supported rate, then the same for i16.
        let pick = |format: cpal::SampleFormat| -> Option<u32> {
            let ranges = device.supported_input_configs().ok()?;
            ranges
                .filter(|r| r.channels() == config.channels && r.sample_format() == format)
                .map(|r| {
                    config
                        .sample_rate
                        .clamp(r.min_sample_rate(), r.max_sample_rate())
                })
                .min_by_key(|&rate| rate.abs_diff(config.sample_rate))
        };
        let (sample_format, sample_rate) = pick(cpal::SampleFormat::F32)
            .map(|rate| (cpal::SampleFormat::F32, rate))
            .or_else(|| pick(cpal::SampleFormat::I16).map(|rate| (cpal::SampleFormat::I16, rate)))
            .ok_or_else(|| {
                RdfError::AudioDevice(format!(
                    "No {}-channel f32 or i16 input config on this device; \
                     it supports: {}",
                    config.channels,
                    describe_configs(&device)
                ))
            })?;
        if sample_rate != config.sample_rate {
            log::warn!(
                "Device cannot capture at {} Hz; using {} Hz",
                config.sample_rate,
                sample_rate
            );
        }

        let stream_config = cpal::StreamConfig {
            channels: config.channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Fixed(config.buffer_size as u32),
        };

        // Build input stream with callback
        let error_tx = tx.clone();
        // Frames dropped since the last successful send, carried forward on
        // the next chunk that gets through. Lives in the FnMut callback, so
        // no shared atomic is needed.
        //
        // This runs on the real-time audio thread: never block. If the
        // consumer lags and the channel fills, drop the chunk and account
        // for it instead of stalling the driver.
        let mut pending_gap_frames: usize = 0;
        let mut send_chunk = move |samples: Vec<f32>| {
            let frames = samples.len() / 2;
            let chunk = AudioChunk {
                gap_before_frames: pending_gap_frames,
                samples,
            };
            match tx.try_send(Ok(chunk)) {
                Ok(()) => pending_gap_frames = 0,
                Err(TrySendError::Full(_)) => {
                    // This chunk is lost too; keep the earlier pending gap
                    // and add these frames (data is interleaved stereo).
                    pending_gap_frames += frames;
                }
                Err(TrySendError::Disconnected(_)) => {
                    log::warn!("Audio receiver dropped");
                }
            }
        };
        let error_callback = move |err| {
            // Forward the error so a consumer blocked in recv() wakes
            // up instead of hanging after the stream dies. Not the RT
            // data callback, so a briefly blocking send is fine.
            log::error!("Audio stream error: {}", err);
            let _ = error_tx.send(Err(err));
        };
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Non-finite samples are zeroed for the same reason the
                    // WAV source zeroes them: a driver glitch should read as
                    // a dropout, not poison the filters and the output. The
                    // map is branch-light and allocation-free beyond the
                    // to_vec the chunk needs anyway.
                    send_chunk(
                        data.iter()
                            .map(|&s| if s.is_finite() { s } else { 0.0 })
                            .collect(),
                    );
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    // Same normalization as the WAV source's integer path:
                    // full scale is the type's magnitude, and an integer
                    // sample cannot be non-finite.
                    send_chunk(data.iter().map(|&s| s as f32 / 32768.0).collect());
                },
                error_callback,
                None,
            ),
            _ => unreachable!("pick() only returns F32 or I16"),
        }
        .map_err(|e| RdfError::AudioStream(format!("{}", e)))?;

        // Real-time priority for the callback thread comes from cpal's
        // audio_thread_priority feature (enabled in Cargo.toml), which
        // promotes the stream thread from inside cpal. Promoting from here
        // would only boost the caller's (setup) thread.
        stream
            .play()
            .map_err(|e| RdfError::AudioStream(format!("{}", e)))?;

        Ok(Self {
            stream,
            sample_rate,
        })
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        let _ = self.stream.pause();
    }
}
