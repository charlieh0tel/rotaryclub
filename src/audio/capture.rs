use crate::config::AudioConfig;
use crate::error::{RdfError, Result};
use audio_thread_priority::RtPriorityHandle;
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

pub struct AudioCapture {
    stream: cpal::Stream,
    _rt_handle: Option<RtPriorityHandle>,
    dropped_chunks: Arc<AtomicU64>,
}

impl AudioCapture {
    pub fn new(
        config: &AudioConfig,
        tx: Sender<Vec<f32>>,
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
        let stream = device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // This runs on the real-time audio thread: never block.
                    // If the consumer lags and the channel fills, drop the
                    // chunk and account for it instead of stalling the driver.
                    match tx.try_send(data.to_vec()) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            dropped_chunks_cb.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            log::warn!("Audio receiver dropped");
                        }
                    }
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            )
            .map_err(|e| RdfError::AudioStream(format!("{}", e)))?;

        // Attempt to promote to real-time priority
        let rt_handle = audio_thread_priority::promote_current_thread_to_real_time(
            config.buffer_size as u32,
            config.sample_rate,
        );

        let rt_handle = match rt_handle {
            Ok(handle) => Some(handle),
            Err(e) => {
                log::warn!("Could not set real-time priority: {}", e);
                None
            }
        };

        stream
            .play()
            .map_err(|e| RdfError::AudioStream(format!("{}", e)))?;

        Ok(Self {
            stream,
            _rt_handle: rt_handle,
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
