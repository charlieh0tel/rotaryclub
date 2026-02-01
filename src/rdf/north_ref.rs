use crate::config::NorthTickConfig;
use crate::error::Result;
use crate::rdf::north_ref_dpll::DpllNorthTracker;
use crate::rdf::north_ref_simple::SimpleNorthTracker;

#[derive(Debug, Clone, Copy)]
pub struct NorthTick {
    pub sample_index: usize,
    pub period: Option<f32>,
}

pub enum NorthReferenceTracker {
    Simple(SimpleNorthTracker),
    Dpll(DpllNorthTracker),
}

impl NorthReferenceTracker {
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

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        match self {
            Self::Simple(tracker) => tracker.process_buffer(buffer),
            Self::Dpll(tracker) => tracker.process_buffer(buffer),
        }
    }

    #[allow(dead_code)]
    pub fn rotation_period(&self) -> Option<f32> {
        match self {
            Self::Simple(tracker) => tracker.rotation_period(),
            Self::Dpll(tracker) => tracker.rotation_period(),
        }
    }

    pub fn rotation_frequency(&self) -> Option<f32> {
        match self {
            Self::Simple(tracker) => tracker.rotation_frequency(),
            Self::Dpll(tracker) => tracker.rotation_frequency(),
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        match self {
            Self::Simple(tracker) => tracker.reset(),
            Self::Dpll(tracker) => tracker.reset(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{NorthTickConfig, NorthTrackingMode};

    #[test]
    fn test_simple_tracker() {
        let mut config = NorthTickConfig::default();
        config.mode = NorthTrackingMode::Simple;
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
        let mut config = NorthTickConfig::default();
        config.mode = NorthTrackingMode::Dpll;
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
