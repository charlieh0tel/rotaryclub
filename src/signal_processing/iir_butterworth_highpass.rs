use crate::error::{RdfError, Result};
use iir_filters::filter::{DirectForm2Transposed, Filter};
use iir_filters::filter_design::{FilterType, butter};
use iir_filters::sos::zpk2sos;

/// Butterworth IIR highpass filter for north tick extraction
///
/// Implements a Butterworth highpass filter to isolate the high-frequency
/// transients of north timing reference pulses (~20 µs width).
///
/// The filter passes frequencies above `cutoff_hz` while attenuating lower
/// frequencies. This is essential for detecting the sharp transients of
/// north tick pulses against background signals.
pub struct IirButterworthHighpass {
    filter: DirectForm2Transposed,
}

impl IirButterworthHighpass {
    /// Create a new Butterworth highpass filter
    ///
    /// # Arguments
    /// * `cutoff_hz` - Cutoff frequency in Hz (typically 5000+ for north ticks)
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `order` - Filter order (higher = steeper rolloff, typically 2)
    ///
    /// # Errors
    /// Returns `RdfError::FilterDesign` if filter parameters are invalid
    pub fn new(cutoff_hz: f32, sample_rate: f32, order: usize) -> Result<Self> {
        let zpk = butter(
            order as u32,
            FilterType::HighPass(cutoff_hz as f64),
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

    #[test]
    fn test_butterworth_highpass_design() {
        let filter = IirButterworthHighpass::new(2000.0, 48000.0, 2);
        assert!(filter.is_ok());
    }
}
