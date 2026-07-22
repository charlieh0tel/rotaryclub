use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crossbeam_channel::Receiver;
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
    capture: AudioCapture,
    reported_dropped_samples: u64,
}

impl DeviceSource {
    pub fn new(config: &AudioConfig, device_name: Option<&str>) -> Result<Self> {
        let (tx, rx) = crossbeam_channel::bounded(CAPTURE_CHANNEL_DEPTH);
        let capture = AudioCapture::new(config, tx, device_name)?;
        Ok(Self {
            rx,
            sample_rate: config.sample_rate,
            capture,
            reported_dropped_samples: 0,
        })
    }
}

impl AudioSource for DeviceSource {
    fn next_buffer(&mut self) -> Result<Option<Vec<f32>>> {
        match self.rx.recv() {
            Ok(Ok(data)) => Ok(Some(data)),
            Ok(Err(e)) => Err(RdfError::AudioStream(e.to_string())),
            Err(_) => Ok(None),
        }
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn take_dropped_frames(&mut self) -> usize {
        let dropped = self.capture.dropped_samples();
        let delta = dropped - self.reported_dropped_samples;
        if delta > 0 {
            log::warn!(
                "Audio capture dropped {} sample(s), {} total (processing too slow)",
                delta,
                dropped
            );
            self.reported_dropped_samples = dropped;
        }
        (delta / 2) as usize
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

        Ok(Self {
            reader,
            chunk_size,
            sample_rate: spec.sample_rate,
            sample_format: spec.sample_format,
            int_full_scale: 2.0_f32.powi(spec.bits_per_sample as i32 - 1),
        })
    }
}

impl AudioSource for WavFileSource {
    fn next_buffer(&mut self) -> Result<Option<Vec<f32>>> {
        let mut chunk = Vec::with_capacity(self.chunk_size);
        match self.sample_format {
            hound::SampleFormat::Float => {
                for sample in self.reader.samples::<f32>().take(self.chunk_size) {
                    chunk.push(sample?);
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
