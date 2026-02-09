use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crossbeam_channel::Receiver;
use hound::WavReader;

use super::AudioCapture;
use crate::config::AudioConfig;

#[allow(dead_code)]
pub trait AudioSource: Send {
    fn next_buffer(&mut self) -> anyhow::Result<Option<Vec<f32>>>;
    fn sample_rate(&self) -> u32;
}

pub struct DeviceSource {
    rx: Receiver<Vec<f32>>,
    #[allow(dead_code)]
    sample_rate: u32,
    _capture: AudioCapture,
}

impl DeviceSource {
    pub fn new(config: &AudioConfig, device_name: Option<&str>) -> anyhow::Result<Self> {
        let (tx, rx) = crossbeam_channel::bounded(10);
        let capture = AudioCapture::new(config, tx, device_name)?;
        Ok(Self {
            rx,
            sample_rate: config.sample_rate,
            _capture: capture,
        })
    }
}

impl AudioSource for DeviceSource {
    fn next_buffer(&mut self) -> anyhow::Result<Option<Vec<f32>>> {
        match self.rx.recv() {
            Ok(data) => Ok(Some(data)),
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
    #[allow(dead_code)]
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
                let max_val = 2_i32.pow(spec.bits_per_sample as u32 - 1) as f32;
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
