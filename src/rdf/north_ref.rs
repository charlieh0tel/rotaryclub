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
    /// Variance of this reference's own timing, in radians squared of
    /// rotation phase, or None if the tracker cannot estimate it.
    ///
    /// A bearing is measured against this tick, so whatever the tick's timing
    /// scatters by, the bearing scatters by too. Phase agreement within the
    /// Doppler tone cannot see this: a reference that is late by the same
    /// amount every rotation moves every bearing equally and leaves the tone
    /// looking perfectly coherent.
    ///
    /// The DPLL reports the scatter of its detections here, deliberately not
    /// reduced by the averaging the loop performs on top of them. That
    /// reduction is real -- while the phase correction runs, the reported time
    /// is pulled onto an oscillator estimate resting on the loop's whole
    /// memory, 755 ticks at the shipped bandwidth, so the reported tick
    /// scatters some twenty-seven times less than this figure says. Dividing
    /// by it was tried and measured, and it makes the number worse where it
    /// matters: as the signal degrades the tick's error stops being scatter
    /// and becomes a displacement the loop follows, invisible from inside
    /// because the oscillator agrees with the detections dragging it. Taking
    /// the reduction turned a figure that brackets the true error into one
    /// that understates it sixteenfold exactly when a bearing is worthless.
    ///
    /// The simple tracker has no oscillator, and derives the same quantity
    /// from the scatter of the intervals between its detections.
    pub phase_variance: Option<f32>,
    /// Timing variance of the tick actually emitted, in radians squared --
    /// the quantity a bearing's uncertainty needs from its reference.
    ///
    /// For the simple tracker this is the same as `phase_variance`: what it
    /// emits is what it detected. For the DPLL the two differ by the loop's
    /// whole memory -- the emitted tick rests on an oscillator averaging
    /// hundreds of detections, so charging a bearing with raw detection
    /// scatter overstated the reference by a factor around twenty-six and
    /// capped confidence at 0.74 on a perfect signal. The DPLL derives this
    /// the same way the simple tracker derives its figure, from the interval
    /// scatter of the ticks it emits, plus the square of the systematic
    /// phase offset its own statistics currently track (the lag while it
    /// follows a rate change).
    ///
    /// What no internal statistic can carry: a displacement the loop follows
    /// perfectly, a detection bias moving slowly enough that the oscillator
    /// agrees with it. That is the same blindness the doppler term has to a
    /// reflection, and it lives in the same row of METRICS.md's table.
    pub reference_variance: Option<f32>,
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
    /// Advance the tracker's sample clock over samples that were lost
    /// (e.g. capture chunks dropped under overload) without processing
    /// audio, so subsequent tick indices stay on the real timeline.
    fn advance_samples(&mut self, samples: usize);

    /// Emit any tick still pending at end-of-stream (a crossing whose
    /// peak-search window had not completed when the last buffer ended).
    fn finish(&mut self) -> Vec<NorthTick>;
    fn rotation_frequency(&self) -> Option<f32>;
    #[allow(dead_code)]
    fn lock_quality(&self) -> Option<f32>;
    fn phase_error_variance(&self) -> Option<f32>;

    /// Samples since a north pulse was last detected.
    ///
    /// Grows without bound while the channel is silent, which is the one
    /// failure that reports nothing on its own: below the detection
    /// threshold there are no ticks, so no bearings, and no metric that
    /// would show a problem.
    fn samples_since_detection(&self) -> usize;
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
    Simple(Box<SimpleNorthTracker>),
    Dpll(Box<DpllNorthTracker>),
}

impl NorthReferenceTracker {
    /// Create a new north reference tracker
    ///
    /// # Arguments
    /// * `config` - North tick detection configuration
    /// * `sample_rate` - Audio sample rate in Hz
    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        match config.mode {
            crate::config::NorthTrackingMode::Simple => Ok(Self::Simple(Box::new(
                SimpleNorthTracker::new(config, sample_rate)?,
            ))),
            crate::config::NorthTrackingMode::Dpll => Ok(Self::Dpll(Box::new(
                DpllNorthTracker::new(config, sample_rate)?,
            ))),
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

    fn advance_samples(&mut self, samples: usize) {
        match self {
            Self::Simple(tracker) => tracker.advance_samples(samples),
            Self::Dpll(tracker) => tracker.advance_samples(samples),
        }
    }

    fn finish(&mut self) -> Vec<NorthTick> {
        match self {
            Self::Simple(tracker) => tracker.finish(),
            Self::Dpll(tracker) => tracker.finish(),
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

    fn samples_since_detection(&self) -> usize {
        match self {
            Self::Simple(tracker) => tracker.samples_since_detection(),
            Self::Dpll(tracker) => tracker.samples_since_detection(),
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
