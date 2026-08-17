use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError};
use hound::WavReader;

use super::AudioCapture;
use super::capture::AudioMessage;
use crate::config::AudioConfig;
use crate::error::{RdfError, Result};

pub trait AudioSource: Send {
    fn next_buffer(&mut self) -> Result<Option<Vec<f32>>>;
    fn sample_rate(&self) -> u32;

    /// Frames (per-channel samples) lost since the last call, e.g. capture
    /// chunks dropped under overload. Callers advance the DSP clock by this
    /// amount so timestamps stay on the real timeline.
    fn take_dropped_frames(&mut self) -> usize {
        0
    }
}

// ~700 ms of slack at the default 1024-sample buffers / 48 kHz before the
// capture callback starts dropping chunks.
const CAPTURE_CHANNEL_DEPTH: usize = 32;

pub struct DeviceSource {
    rx: Receiver<AudioMessage>,
    sample_rate: u32,
    _capture: AudioCapture,
    // Gap (frames) that preceded the buffer returned by the last
    // next_buffer call, awaiting collection by take_dropped_frames.
    pending_gap_frames: usize,
    total_dropped_frames: u64,
    shutdown: Option<Arc<AtomicBool>>,
}

impl DeviceSource {
    pub fn new(config: &AudioConfig, device_name: Option<&str>) -> Result<Self> {
        let (tx, rx) = crossbeam_channel::bounded(CAPTURE_CHANNEL_DEPTH);
        let capture = AudioCapture::new(config, tx, device_name)?;
        Ok(Self {
            rx,
            // The negotiated rate, not the configured one: the DSP chain is
            // built from what this reports, so it must be what the stream
            // actually delivers.
            sample_rate: capture.sample_rate(),
            _capture: capture,
            pending_gap_frames: 0,
            total_dropped_frames: 0,
            shutdown: None,
        })
    }

    /// Observe a shutdown flag while waiting for audio, so a caller's stop
    /// request ends the blocking wait even if the device goes silent
    /// without raising a stream error.
    pub fn set_shutdown_flag(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown = Some(flag);
    }
}

impl AudioSource for DeviceSource {
    fn next_buffer(&mut self) -> Result<Option<Vec<f32>>> {
        // Wait in bounded slices so the shutdown flag is observed even when
        // a silently-dead device delivers neither data nor a stream error.
        loop {
            if let Some(flag) = &self.shutdown
                && flag.load(Ordering::Relaxed)
            {
                return Ok(None);
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(chunk)) => {
                    if chunk.gap_before_frames > 0 {
                        self.pending_gap_frames = chunk.gap_before_frames;
                        self.total_dropped_frames += chunk.gap_before_frames as u64;
                        log::warn!(
                            "Audio capture dropped {} frame(s), {} total (processing too slow)",
                            chunk.gap_before_frames,
                            self.total_dropped_frames
                        );
                    }
                    return Ok(Some(chunk.samples));
                }
                Ok(Err(e)) => return Err(RdfError::AudioStream(e.to_string())),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
            }
        }
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn take_dropped_frames(&mut self) -> usize {
        std::mem::take(&mut self.pending_gap_frames)
    }
}

/// Streams samples from a WAV file one chunk per `next_buffer` call, so
/// arbitrarily long recordings never load into memory at once.
pub struct WavFileSource {
    reader: WavReader<BufReader<File>>,
    chunk_size: usize,
    sample_rate: u32,
    sample_format: hound::SampleFormat,
    // Full-scale divisor for integer PCM; computed in f32 because
    // 2_i32.pow(31) overflows for 32-bit samples.
    int_full_scale: f32,
}

impl WavFileSource {
    pub fn new<P: AsRef<Path>>(path: P, chunk_size: usize) -> Result<Self> {
        let reader = WavReader::open(path.as_ref())?;
        let spec = reader.spec();

        if spec.channels != 2 {
            return Err(RdfError::UnsupportedWav(format!(
                "expected stereo, got {} channels",
                spec.channels
            )));
        }

        // Buffers are interleaved stereo, so an odd chunk size splits a frame
        // across reads: the dropped half-frame swaps every later sample's
        // channel and the stream is silently misinterpreted from there on.
        if chunk_size == 0 || !chunk_size.is_multiple_of(2) {
            return Err(RdfError::Config(format!(
                "chunk_size is {chunk_size}, must be a positive even number of                  interleaved samples (whole stereo frames)"
            )));
        }

        Ok(Self {
            reader,
            chunk_size,
            sample_rate: spec.sample_rate,
            sample_format: spec.sample_format,
            int_full_scale: 2.0_f32.powi(spec.bits_per_sample as i32 - 1),
        })
    }
}

impl WavFileSource {
    /// Read an entire stereo WAV into interleaved f32 samples, returning
    /// them with the file's sample rate. Convenience for small diagnostic
    /// tools; long recordings should stream via `next_buffer` instead.
    pub fn read_all<P: AsRef<Path>>(path: P) -> Result<(Vec<f32>, u32)> {
        let mut source = Self::new(path, 65_536)?;
        let sample_rate = source.sample_rate();
        let mut samples = Vec::new();
        while let Some(chunk) = source.next_buffer()? {
            samples.extend(chunk);
        }
        Ok((samples, sample_rate))
    }
}

impl AudioSource for WavFileSource {
    fn next_buffer(&mut self) -> Result<Option<Vec<f32>>> {
        let mut chunk = Vec::with_capacity(self.chunk_size);
        match self.sample_format {
            hound::SampleFormat::Float => {
                // Any 32-bit pattern is a "valid" float, so a corrupted
                // capture can carry NaN or infinity here, and nothing
                // downstream rejects them: NaN fails every comparison, so it
                // slides through the correlation gates and reaches the output
                // formats, where JSON emits a bare NaN (not JSON) and the
                // KN5R cast saturates to a clean-looking bearing due north.
                // Zeroed instead, so a glitch burst looks like the dropout it
                // is and the pipeline handles it as one.
                for sample in self.reader.samples::<f32>().take(self.chunk_size) {
                    let sample = sample?;
                    chunk.push(if sample.is_finite() { sample } else { 0.0 });
                }
            }
            hound::SampleFormat::Int => {
                for sample in self.reader.samples::<i32>().take(self.chunk_size) {
                    chunk.push(sample? as f32 / self.int_full_scale);
                }
            }
        }

        if chunk.is_empty() {
            Ok(None)
        } else {
            Ok(Some(chunk))
        }
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav_file_source_streams_chunks_matching_whole_file() {
        let path = std::env::temp_dir().join("rotaryclub_test_stream.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        // 100 stereo frames of a known pattern; 200 samples does not divide
        // evenly by the chunk size of 16, so the final chunk is partial.
        let expected: Vec<f32> = (0..200).map(|i| (i as f32) / 200.0 - 0.5).collect();
        for &v in &expected {
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();

        let mut source = WavFileSource::new(&path, 16).unwrap();
        let mut streamed = Vec::new();
        let mut chunks = 0;
        while let Some(chunk) = source.next_buffer().unwrap() {
            assert!(chunk.len() <= 16);
            streamed.extend(chunk);
            chunks += 1;
        }
        std::fs::remove_file(&path).ok();

        assert_eq!(chunks, 200_usize.div_ceil(16));
        assert_eq!(streamed, expected);
    }

    #[test]
    fn test_wav_file_source_normalizes_32bit_pcm() {
        // Regression test: 2_i32.pow(31) overflowed i32 (debug panic,
        // release polarity inversion) when normalizing 32-bit PCM.
        let path = std::env::temp_dir().join("rotaryclub_test_pcm32.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        // One stereo frame at +half scale, one at -half scale.
        for v in [i32::MAX / 2, i32::MAX / 2, i32::MIN / 2, i32::MIN / 2] {
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();

        let mut source = WavFileSource::new(&path, 8).unwrap();
        let samples = source.next_buffer().unwrap().unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(samples.len(), 4);
        for (i, &s) in samples.iter().enumerate() {
            let expected = if i < 2 { 0.5 } else { -0.5 };
            assert!(
                (s - expected).abs() < 1e-3,
                "sample {} was {}, expected {}",
                i,
                s,
                expected
            );
        }
    }
}
