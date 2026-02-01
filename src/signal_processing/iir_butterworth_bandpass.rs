use crate::error::{RdfError, Result};
use iir_filters::filter::{DirectForm2Transposed, Filter};
use iir_filters::filter_design::{FilterType, butter};
use iir_filters::sos::zpk2sos;

/// Butterworth IIR bandpass filter for Doppler tone extraction
///
/// Implements a Butterworth bandpass filter using direct form II transposed
/// structure for efficient filtering of the Doppler tone around the antenna
/// rotation frequency (~1602 Hz).
///
/// The filter passes frequencies between `low_hz` and `high_hz` while
/// attenuating frequencies outside this range. Higher filter orders provide
/// steeper rolloff at the cost of slightly more processing.
pub struct IirButterworthBandpass {
    filter: DirectForm2Transposed,
}

impl IirButterworthBandpass {
    /// Create a new Butterworth bandpass filter
    ///
    /// # Arguments
    /// * `low_hz` - Lower cutoff frequency in Hz
    /// * `high_hz` - Upper cutoff frequency in Hz
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `order` - Filter order (higher = steeper rolloff, typically 4)
    ///
    /// # Errors
    /// Returns `RdfError::FilterDesign` if filter parameters are invalid
    pub fn new(low_hz: f32, high_hz: f32, sample_rate: f32, order: usize) -> Result<Self> {
        let zpk = butter(
            order as u32,
            FilterType::BandPass(low_hz as f64, high_hz as f64),
            sample_rate as f64,
        )
        .map_err(|e| RdfError::FilterDesign(format!("{:?}", e)))?;

        let sos = zpk2sos(&zpk, None).map_err(|e| RdfError::FilterDesign(format!("{:?}", e)))?;

        Ok(Self {
            filter: DirectForm2Transposed::new(&sos),
        })
    }

    /// Process a single audio sample through the filter
    ///
    /// Returns the filtered sample value.
    pub fn process(&mut self, sample: f32) -> f32 {
        self.filter.filter(sample as f64) as f32
    }

    /// Process an entire buffer of audio samples in-place
    ///
    /// Filters each sample in the buffer, replacing the original values
    /// with the filtered output.
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn test_butterworth_bandpass_design() {
        let filter = IirButterworthBandpass::new(400.0, 600.0, 48000.0, 4);
        assert!(filter.is_ok());
    }

    #[test]
    fn test_butterworth_bandpass_passes_center_frequency() {
        let mut filter = IirButterworthBandpass::new(400.0, 600.0, 48000.0, 4).unwrap();

        // Generate 500Hz sine wave (center of bandpass)
        let input: Vec<f32> = (0..4800)
            .map(|i| (2.0 * PI * 500.0 * i as f32 / 48000.0).sin())
            .collect();

        let mut output = input.clone();
        filter.process_buffer(&mut output);

        // Calculate RMS
        let input_rms: f32 = input.iter().skip(1000).map(|x| x * x).sum::<f32>().sqrt()
            / (input.len() - 1000) as f32;
        let output_rms: f32 = output.iter().skip(1000).map(|x| x * x).sum::<f32>().sqrt()
            / (output.len() - 1000) as f32;

        // Should pass with minimal attenuation (> -3dB)
        let attenuation_db = 20.0 * (output_rms / input_rms).log10();
        assert!(
            attenuation_db > -3.0,
            "Center frequency too attenuated: {} dB",
            attenuation_db
        );
    }
}
