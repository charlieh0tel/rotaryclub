use crate::constants::{MAX_NORMALIZED_FREQ, MIN_NORMALIZED_FREQ};
use crate::error::{RdfError, Result};
use crate::signal_processing::{Filter, FirFilterCore};
use pm_remez::{BandSetting, constant, pm_parameters, pm_remez};

/// FIR highpass filter with linear phase response
///
/// Uses the Parks-McClellan (Remez) algorithm to design an optimal equiripple
/// FIR filter. Linear phase ensures predictable group delay for accurate
/// north tick timing.
pub struct FirHighpass {
    core: FirFilterCore,
}

impl FirHighpass {
    /// Create a new FIR highpass filter
    ///
    /// # Arguments
    /// * `cutoff_hz` - Cutoff frequency in Hz
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `num_taps` - Number of filter taps (must be odd for Type I linear phase)
    /// * `transition_hz` - Transition bandwidth in Hz
    ///
    /// # Errors
    /// Returns `RdfError::FilterDesign` if filter parameters are invalid
    pub fn new(
        cutoff_hz: f32,
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

        let stop_end = normalize(cutoff_hz) - trans_norm;
        let pass_start = normalize(cutoff_hz);

        let stop_end = stop_end.max(MIN_NORMALIZED_FREQ);
        let pass_start = pass_start.min(MAX_NORMALIZED_FREQ - trans_norm);

        if pass_start <= stop_end {
            return Err(RdfError::FilterDesign(format!(
                "Invalid filter frequencies: cutoff={}, sample_rate={}, transition={}",
                cutoff_hz, sample_rate, transition_hz
            )));
        }

        let bands = [
            BandSetting::new(0.0, stop_end, constant(0.0))
                .map_err(|e| RdfError::FilterDesign(format!("Stopband: {:?}", e)))?,
            BandSetting::new(pass_start, 0.5, constant(1.0))
                .map_err(|e| RdfError::FilterDesign(format!("Passband: {:?}", e)))?,
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

    /// Compute the threshold crossing offset for pulse detection
    ///
    /// When a pulse of given amplitude passes through this filter, the peak detector
    /// triggers when the filtered output first exceeds the threshold. This method
    /// returns the offset from group_delay to the first integer sample above threshold.
    ///
    /// Returns the offset in samples. For most filters this will be 0 or close to 0
    /// since the impulse response peak (at group_delay) typically exceeds the threshold.
    pub fn threshold_crossing_offset(&self, threshold: f32, pulse_amplitude: f32) -> f32 {
        let scaled_threshold = (threshold / pulse_amplitude) as f64;
        let group_delay = self.core.group_delay_samples();

        // Find the first integer sample where the impulse response exceeds the threshold.
        // This matches the peak detector behavior (triggers at integer samples).
        for (i, &tap) in self.core.taps().iter().enumerate() {
            if tap > scaled_threshold {
                return i as f32 - group_delay as f32;
            }
        }

        // Fallback: threshold never exceeded (shouldn't happen with valid parameters)
        0.0
    }

    /// Compute the filtered impulse peak offset for pulse detection
    ///
    /// Returns the sample offset from group delay to the maximum positive
    /// impulse-response tap. This matches peak-index timing when the detector
    /// reports local positive maxima instead of threshold crossings.
    pub fn peak_offset(&self) -> f32 {
        let group_delay = self.core.group_delay_samples();
        let peak_idx = self
            .core
            .taps()
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(i, _)| i)
            .unwrap_or(group_delay);
        peak_idx as f32 - group_delay as f32
    }
}

impl Filter for FirHighpass {
    fn process(&mut self, sample: f32) -> f32 {
        FirHighpass::process(self, sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn test_fir_highpass_design() {
        let filter = FirHighpass::new(5000.0, 48000.0, 63, 500.0);
        assert!(filter.is_ok());
        let filter = filter.unwrap();
        assert_eq!(filter.num_taps(), 63);
        assert_eq!(filter.group_delay_samples(), 31);
    }

    #[test]
    fn test_fir_highpass_passes_high_frequency() {
        let mut filter = FirHighpass::new(2000.0, 48000.0, 127, 500.0).unwrap();

        let input: Vec<f32> = (0..4800)
            .map(|i| (2.0 * PI * 10000.0 * i as f32 / 48000.0).sin())
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
            "High frequency too attenuated: {} dB",
            attenuation_db
        );
    }

    #[test]
    fn test_fir_highpass_attenuates_low_frequency() {
        let mut filter = FirHighpass::new(5000.0, 48000.0, 127, 500.0).unwrap();

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
            attenuation_db < -20.0,
            "Low frequency not attenuated enough: {} dB",
            attenuation_db
        );
    }

    #[test]
    fn test_fir_highpass_group_delay() {
        let filter = FirHighpass::new(5000.0, 48000.0, 63, 500.0).unwrap();
        assert_eq!(filter.group_delay_samples(), 31);

        let filter = FirHighpass::new(5000.0, 48000.0, 127, 500.0).unwrap();
        assert_eq!(filter.group_delay_samples(), 63);
    }

    #[test]
    fn test_threshold_crossing_offset() {
        let filter = FirHighpass::new(5000.0, 48000.0, 63, 500.0).unwrap();

        // For the 5000Hz highpass at 48kHz with default threshold/amplitude,
        // the impulse response has its first above-threshold tap at the center,
        // so the offset should be 0.
        let offset = filter.threshold_crossing_offset(0.15, 0.8);
        assert_eq!(
            offset, 0.0,
            "Offset should be 0 when threshold is exceeded at center tap"
        );

        // Total delay should equal group delay when offset is 0
        let total_delay = filter.group_delay_samples() as f32 + offset;
        assert_eq!(total_delay, 31.0);
    }

    #[test]
    fn test_peak_offset() {
        let filter = FirHighpass::new(5000.0, 48000.0, 63, 500.0).unwrap();
        let offset = filter.peak_offset();
        assert!(
            offset.abs() <= 1.0,
            "Peak offset should be near group delay, got {}",
            offset
        );
    }

    #[test]
    fn test_fir_highpass_impulse_response_timing() {
        let mut filter = FirHighpass::new(5000.0, 48000.0, 63, 500.0).unwrap();
        let group_delay = filter.group_delay_samples();

        let mut samples = vec![0.0f32; 200];
        let impulse_idx = 50;
        samples[impulse_idx] = 0.8;

        filter.process_buffer(&mut samples);

        let threshold = 0.15;
        let first_crossing = samples
            .iter()
            .enumerate()
            .find(|&(_, &s)| s > threshold)
            .map(|(i, _)| i);

        let peak_idx = samples
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap();

        eprintln!("Impulse at sample {}", impulse_idx);
        eprintln!("Group delay: {}", group_delay);
        eprintln!("Expected peak at: {}", impulse_idx + group_delay);
        eprintln!("Actual peak at: {}", peak_idx);
        if let Some(fc) = first_crossing {
            eprintln!("First threshold crossing at: {}", fc);
            eprintln!("Crossing delay from impulse: {}", fc - impulse_idx);
        }

        assert_eq!(
            peak_idx,
            impulse_idx + group_delay,
            "Peak should be at impulse + group_delay"
        );
    }
}
