use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crossbeam_channel::Receiver;
use hound::WavReader;

use super::AudioCapture;
use super::capture::AudioMessage;
use crate::config::AudioConfig;

pub trait AudioSource: Send {
    fn next_buffer(&mut self) -> anyhow::Result<Option<Vec<f32>>>;
    fn sample_rate(&self) -> u32;
}

// ~700 ms of slack at the default 1024-sample buffers / 48 kHz before the
// capture callback starts dropping chunks.
const CAPTURE_CHANNEL_DEPTH: usize = 32;

pub struct DeviceSource {
    rx: Receiver<AudioMessage>,
    sample_rate: u32,
    capture: AudioCapture,
    reported_drops: u64,
}

impl DeviceSource {
    pub fn new(config: &AudioConfig, device_name: Option<&str>) -> anyhow::Result<Self> {
        let (tx, rx) = crossbeam_channel::bounded(CAPTURE_CHANNEL_DEPTH);
        let capture = AudioCapture::new(config, tx, device_name)?;
        Ok(Self {
            rx,
            sample_rate: config.sample_rate,
            capture,
            reported_drops: 0,
        })
    }
}

impl AudioSource for DeviceSource {
    fn next_buffer(&mut self) -> anyhow::Result<Option<Vec<f32>>> {
        let dropped = self.capture.dropped_chunks();
        if dropped > self.reported_drops {
            log::warn!(
                "Audio capture dropped {} chunk(s), {} total (processing too slow)",
                dropped - self.reported_drops,
                dropped
            );
            self.reported_drops = dropped;
        }
        match self.rx.recv() {
            Ok(Ok(data)) => Ok(Some(data)),
            Ok(Err(e)) => Err(anyhow::anyhow!("Audio stream error: {}", e)),
            Err(_) => Ok(None),
        }
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

pub struct WavFileSource {
    samples: Vec<f32>,
    position: usize,
    chunk_size: usize,
    sample_rate: u32,
}

impl WavFileSource {
    pub fn new<P: AsRef<Path>>(path: P, chunk_size: usize) -> anyhow::Result<Self> {
        let reader = WavReader::open(path.as_ref())?;
        let spec = reader.spec();

        if spec.channels != 2 {
            anyhow::bail!("Expected stereo WAV file, got {} channels", spec.channels);
        }

        let sample_rate = spec.sample_rate;
        let samples = Self::read_samples(reader, &spec)?;

        Ok(Self {
            samples,
            position: 0,
            chunk_size,
            sample_rate,
        })
    }

    fn read_samples(
        mut reader: WavReader<BufReader<File>>,
        spec: &hound::WavSpec,
    ) -> anyhow::Result<Vec<f32>> {
        let samples = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
            hound::SampleFormat::Int => {
                // Compute in f32: 2_i32.pow(31) overflows for 32-bit PCM.
                let max_val = 2.0_f32.powi(spec.bits_per_sample as i32 - 1);
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / max_val))
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        Ok(samples)
    }
}

impl AudioSource for WavFileSource {
    fn next_buffer(&mut self) -> anyhow::Result<Option<Vec<f32>>> {
        if self.position >= self.samples.len() {
            return Ok(None);
        }

        let end = (self.position + self.chunk_size).min(self.samples.len());
        let chunk = self.samples[self.position..end].to_vec();
        self.position = end;

        Ok(Some(chunk))
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
