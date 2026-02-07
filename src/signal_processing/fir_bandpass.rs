use crate::constants::{MAX_NORMALIZED_FREQ, MIN_NORMALIZED_FREQ};
use crate::error::{RdfError, Result};
use crate::signal_processing::{Filter, FirFilterCore};
use pm_remez::{BandSetting, constant, pm_parameters, pm_remez};

/// FIR bandpass filter with linear phase response
///
/// Uses the Parks-McClellan (Remez) algorithm to design an optimal equiripple
/// FIR filter. Linear phase ensures all frequency components are delayed equally,
/// preserving waveform shape and accurate phase measurements.
pub struct FirBandpass {
    core: FirFilterCore,
}

impl FirBandpass {
    /// Create a new FIR bandpass filter
    ///
    /// # Arguments
    /// * `low_hz` - Lower cutoff frequency in Hz
    /// * `high_hz` - Upper cutoff frequency in Hz
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `num_taps` - Number of filter taps (must be odd for Type I linear phase)
    /// * `transition_hz` - Transition bandwidth in Hz
    ///
    /// # Errors
    /// Returns `RdfError::FilterDesign` if filter parameters are invalid
    pub fn new(
        low_hz: f32,
        high_hz: f32,
        sample_rate: f32,
        num_taps: usize,
        transition_hz: f32,
    ) -> Result<Self> {
        let num_taps = if num_taps.is_multiple_of(2) {
            num_taps + 1
        } else {
            num_taps
        };

        let normalize = |hz: f32| (hz / sample_rate) as f64;

        let trans_norm = (transition_hz / sample_rate) as f64;

        let stop1_end = normalize(low_hz) - trans_norm;
        let pass_start = normalize(low_hz);
        let pass_end = normalize(high_hz);
        let stop2_start = normalize(high_hz) + trans_norm;

        let stop1_end = stop1_end.max(MIN_NORMALIZED_FREQ);
        let stop2_start = stop2_start.min(MAX_NORMALIZED_FREQ);

        if pass_start <= stop1_end || pass_end >= stop2_start {
            return Err(RdfError::FilterDesign(format!(
                "Invalid filter frequencies: low={}, high={}, sample_rate={}, transition={}",
                low_hz, high_hz, sample_rate, transition_hz
            )));
        }

        let bands = [
            BandSetting::new(0.0, stop1_end, constant(0.0))
                .map_err(|e| RdfError::FilterDesign(format!("Lower stopband: {:?}", e)))?,
            BandSetting::new(pass_start, pass_end, constant(1.0))
                .map_err(|e| RdfError::FilterDesign(format!("Passband: {:?}", e)))?,
            BandSetting::new(stop2_start, 0.5, constant(0.0))
                .map_err(|e| RdfError::FilterDesign(format!("Upper stopband: {:?}", e)))?,
        ];

        let params = pm_parameters(num_taps, &bands)
            .map_err(|e| RdfError::FilterDesign(format!("PM parameters: {:?}", e)))?;

        let design =
            pm_remez(&params).map_err(|e| RdfError::FilterDesign(format!("PM Remez: {:?}", e)))?;

        Ok(Self {
            core: FirFilterCore::new(design.impulse_response),
        })
    }

    /// Process a single audio sample through the filter
    pub fn process(&mut self, sample: f32) -> f32 {
        self.core.process(sample)
    }

    /// Process an entire buffer of audio samples in-place
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        self.core.process_buffer(buffer)
    }

    /// Get the number of taps (filter length)
    #[allow(dead_code)]
    pub fn num_taps(&self) -> usize {
        self.core.num_taps()
    }

    /// Get the group delay in samples (half the filter length for linear phase)
    pub fn group_delay_samples(&self) -> usize {
        self.core.group_delay_samples()
    }
}

impl Filter for FirBandpass {
    fn process(&mut self, sample: f32) -> f32 {
        FirBandpass::process(self, sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn test_fir_bandpass_design() {
        let filter = FirBandpass::new(1500.0, 1700.0, 48000.0, 127, 100.0);
        assert!(filter.is_ok());
        let filter = filter.unwrap();
        assert_eq!(filter.num_taps(), 127);
    }

    #[test]
    fn test_fir_bandpass_passes_center_frequency() {
        let mut filter = FirBandpass::new(400.0, 600.0, 48000.0, 127, 100.0).unwrap();

        let input: Vec<f32> = (0..4800)
            .map(|i| (2.0 * PI * 500.0 * i as f32 / 48000.0).sin())
            .collect();

        let mut output = input.clone();
        filter.process_buffer(&mut output);

        let input_rms: f32 = (input.iter().skip(1000).map(|x| x * x).sum::<f32>()
            / (input.len() - 1000) as f32)
            .sqrt();
        let output_rms: f32 = (output.iter().skip(1000).map(|x| x * x).sum::<f32>()
            / (output.len() - 1000) as f32)
            .sqrt();

        let attenuation_db = 20.0 * (output_rms / input_rms).log10();
        assert!(
            attenuation_db > -3.0,
            "Center frequency too attenuated: {} dB",
            attenuation_db
        );
    }

    #[test]
    fn test_fir_bandpass_attenuates_out_of_band() {
        let mut filter = FirBandpass::new(400.0, 600.0, 48000.0, 255, 100.0).unwrap();

        let input: Vec<f32> = (0..4800)
            .map(|i| (2.0 * PI * 100.0 * i as f32 / 48000.0).sin())
            .collect();

        let mut output = input.clone();
        filter.process_buffer(&mut output);

        let input_rms: f32 = (input.iter().skip(1000).map(|x| x * x).sum::<f32>()
            / (input.len() - 1000) as f32)
            .sqrt();
        let output_rms: f32 = (output.iter().skip(1000).map(|x| x * x).sum::<f32>()
            / (output.len() - 1000) as f32)
            .sqrt();

        let attenuation_db = 20.0 * (output_rms / input_rms).log10();
        assert!(
            attenuation_db < -20.0,
            "Out-of-band frequency not attenuated enough: {} dB",
            attenuation_db
        );
    }
}
