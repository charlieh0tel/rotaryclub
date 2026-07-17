use hound::{WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

fn stereo_f32_spec(sample_rate: u32) -> WavSpec {
    WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    }
}

/// Incrementally writes a stereo f32 WAV file, so long recordings stream to
/// disk instead of accumulating in memory.
pub struct WavStreamWriter {
    writer: WavWriter<BufWriter<File>>,
}

impl WavStreamWriter {
    pub fn create<P: AsRef<Path>>(path: P, sample_rate: u32) -> Result<Self, hound::Error> {
        Ok(Self {
            writer: WavWriter::create(path, stereo_f32_spec(sample_rate))?,
        })
    }

    pub fn write_samples(&mut self, samples: &[f32]) -> Result<(), hound::Error> {
        for &sample in samples {
            self.writer.write_sample(sample)?;
        }
        Ok(())
    }

    /// Interleaved samples written so far.
    pub fn len(&self) -> u32 {
        self.writer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.writer.len() == 0
    }

    /// Write the final WAV header. Dropping without finalizing leaves a
    /// stale length header.
    pub fn finalize(self) -> Result<(), hound::Error> {
        self.writer.finalize()
    }
}

pub fn save_wav(filename: &str, samples: &[f32], sample_rate: u32) -> Result<(), hound::Error> {
    let mut writer = WavStreamWriter::create(filename, sample_rate)?;
    writer.write_samples(samples)?;
    writer.finalize()
}
