use crate::config::NorthTickConfig;
use crate::error::Result;
use crate::rdf::north_ref_dpll::DpllNorthTracker;
use crate::rdf::north_ref_simple::SimpleNorthTracker;

/// North reference tick event
///
/// Represents a detected north timing pulse with its sample position and
/// estimated rotation period. Includes optional sub-sample timing correction
/// and DPLL phase/frequency state for reference signal generation.
#[derive(Debug, Clone, Copy)]
pub struct NorthTick {
    /// Global sample index where the tick was detected
    pub sample_index: usize,
    /// Estimated rotation period in samples (None if not yet established)
    pub period: Option<f32>,
    /// DPLL lock quality (0-1, higher is better lock)
    pub lock_quality: Option<f32>,
    /// Fractional timing offset (samples) relative to `sample_index`.
    /// Positive means the effective tick time is after `sample_index`.
    pub fractional_sample_offset: f32,
    /// Reference phase offset at the effective tick time (radians, 0 = north).
    /// For north-anchored bearing calculation this is typically 0.
    pub phase: f32,
    /// DPLL frequency estimate (radians/sample)
    pub frequency: f32,
}

pub trait NorthTracker {
    fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick>;
    fn rotation_frequency(&self) -> Option<f32>;
    #[allow(dead_code)]
    fn lock_quality(&self) -> Option<f32>;
    fn phase_error_variance(&self) -> Option<f32>;
    /// Get the filtered buffer (after highpass) from the last process_buffer call
    fn filtered_buffer(&self) -> &[f32];
}

/// North reference tracker
///
/// Detects and tracks north timing reference pulses from the antenna array.
/// Provides rotation frequency estimates for bearing calculations.
///
/// # Example
/// ```no_run
/// use rotaryclub::config::RdfConfig;
/// use rotaryclub::rdf::{NorthReferenceTracker, NorthTracker};
///
/// let config = RdfConfig::default();
/// let sample_rate = 48000.0;
/// let mut tracker = NorthReferenceTracker::new(&config.north_tick, sample_rate)?;
///
/// // Process audio buffer
/// let audio_samples = vec![0.0; 1024];
/// let ticks = tracker.process_buffer(&audio_samples);
/// if let Some(freq) = tracker.rotation_frequency() {
///     println!("Rotation: {:.1} Hz", freq);
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub enum NorthReferenceTracker {
    Simple(SimpleNorthTracker),
    Dpll(DpllNorthTracker),
}

impl NorthReferenceTracker {
    /// Create a new north reference tracker
    ///
    /// # Arguments
    /// * `config` - North tick detection configuration
    /// * `sample_rate` - Audio sample rate in Hz
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        match config.mode {
            crate::config::NorthTrackingMode::Simple => {
                Ok(Self::Simple(SimpleNorthTracker::new(config, sample_rate)?))
            }
            crate::config::NorthTrackingMode::Dpll => {
                Ok(Self::Dpll(DpllNorthTracker::new(config, sample_rate)?))
            }
        }
    }
}

impl NorthTracker for NorthReferenceTracker {
    fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        match self {
            Self::Simple(tracker) => tracker.process_buffer(buffer),
            Self::Dpll(tracker) => tracker.process_buffer(buffer),
        }
    }

    fn rotation_frequency(&self) -> Option<f32> {
        match self {
            Self::Simple(tracker) => tracker.rotation_frequency(),
            Self::Dpll(tracker) => tracker.rotation_frequency(),
        }
    }

    fn lock_quality(&self) -> Option<f32> {
        match self {
            Self::Simple(tracker) => tracker.lock_quality(),
            Self::Dpll(tracker) => tracker.lock_quality(),
        }
    }

    fn phase_error_variance(&self) -> Option<f32> {
        match self {
            Self::Simple(tracker) => tracker.phase_error_variance(),
            Self::Dpll(tracker) => tracker.phase_error_variance(),
        }
    }

    fn filtered_buffer(&self) -> &[f32] {
        match self {
            Self::Simple(tracker) => tracker.filtered_buffer(),
            Self::Dpll(tracker) => tracker.filtered_buffer(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{NorthTickConfig, NorthTrackingMode};

    #[test]
    fn test_simple_tracker() {
        let config = NorthTickConfig {
            mode: NorthTrackingMode::Simple,
            ..Default::default()
        };
        let sample_rate = 48000.0;
        let mut tracker = NorthReferenceTracker::new(&config, sample_rate).unwrap();

        let mut signal = vec![0.0; 500];
        signal[50] = 0.8;
        signal[146] = 0.8;
        signal[242] = 0.8;

        let ticks = tracker.process_buffer(&signal);
        assert!(ticks.len() >= 2, "Simple tracker should detect ticks");
    }

    #[test]
    fn test_dpll_tracker() {
        let config = NorthTickConfig {
            mode: NorthTrackingMode::Dpll,
            ..Default::default()
        };
        let sample_rate = 48000.0;
        let mut tracker = NorthReferenceTracker::new(&config, sample_rate).unwrap();

        let mut signal = vec![0.0; 500];
        signal[50] = 0.8;
        signal[146] = 0.8;
        signal[242] = 0.8;

        let ticks = tracker.process_buffer(&signal);
        assert!(ticks.len() >= 2, "DPLL tracker should detect ticks");
    }
}
