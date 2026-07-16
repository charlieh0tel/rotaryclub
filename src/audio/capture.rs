use crate::config::AudioConfig;
use crate::error::{RdfError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, TrySendError};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Message from the capture callbacks: a chunk of interleaved samples, or a
/// fatal stream error that ends capture.
pub type AudioMessage = std::result::Result<Vec<f32>, cpal::StreamError>;

pub struct AudioCapture {
    stream: cpal::Stream,
    dropped_chunks: Arc<AtomicU64>,
}

impl AudioCapture {
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

        // Configure stereo input
        let stream_config = cpal::StreamConfig {
            channels: config.channels,
            sample_rate: config.sample_rate,
            buffer_size: cpal::BufferSize::Fixed(config.buffer_size as u32),
        };

        // Build input stream with callback
        let dropped_chunks = Arc::new(AtomicU64::new(0));
        let dropped_chunks_cb = Arc::clone(&dropped_chunks);
        let error_tx = tx.clone();
        let stream = device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // This runs on the real-time audio thread: never block.
                    // If the consumer lags and the channel fills, drop the
                    // chunk and account for it instead of stalling the driver.
                    match tx.try_send(Ok(data.to_vec())) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            dropped_chunks_cb.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            log::warn!("Audio receiver dropped");
                        }
                    }
                },
                move |err| {
                    // Forward the error so a consumer blocked in recv() wakes
                    // up instead of hanging after the stream dies. Not the RT
                    // data callback, so a briefly blocking send is fine.
                    log::error!("Audio stream error: {}", err);
                    let _ = error_tx.send(Err(err));
                },
                None,
            )
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
            dropped_chunks,
        })
    }

    /// Total audio chunks dropped because the consumer could not keep up.
    pub fn dropped_chunks(&self) -> u64 {
        self.dropped_chunks.load(Ordering::Relaxed)
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        let _ = self.stream.pause();
    }
}
