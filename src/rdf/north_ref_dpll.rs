use crate::config::{LockQualityWeights, NorthPulseEstimator, NorthTickConfig};
use crate::constants::FREQUENCY_EPSILON;
use crate::error::{RdfError, Result};
use crate::rdf::NorthTick;
use crate::signal_processing::{FirHighpass, NorthPulseAgc, PeakDetector, db_to_amplitude};
use std::collections::VecDeque;
use std::f32::consts::PI;

use super::north_ref_common::{
    QuietChannelWatch, centroid_half_width, derive_delay_compensation, derive_peak_timing,
    estimate_fraction, highpass_taps, preprocess_north_buffer, retain_tail, split_effective_time,
    validate_north_tick_config,
};

const MIN_TICK_SPACING_FRACTION: f32 = 0.75;
/// Ceiling on the loop's timing correction, in samples.
///
/// The correction exists to recover the sub-sample part of the tick time that
/// a whole-sample peak index cannot express, so it is sized to exactly what
/// quantization can produce: half a sample. A larger disagreement between
/// oscillator and detector is not quantization -- the loop is lagging a rate
/// change, or the detection was spurious -- and the detected position is then
/// the better answer.
///
/// Measured at a 1 Hz loop bandwidth, before the default widened to 2: this
/// bound leaves steady-state error untouched (0.007 samples either way) while
/// halving the worst acquisition error, because during acquisition the
/// correction saturates and whatever it saturates at goes straight into the
/// bearing.
const MAX_PHASE_TIMING_CORRECTION_SAMPLES: f32 = 0.5;
/// The reported fraction is re-anchored onto the nearest sample, so it can
/// never point past a neighbouring sample.
#[cfg(test)]
const MAX_TOTAL_FRACTIONAL_OFFSET_SAMPLES: f32 = 0.5;
const MIN_PHASE_CORRECTION_SAMPLES: usize = 16;
/// Ticks of history, and phase-error spread, before the oscillator is treated
/// as locked well enough to overrule a detection or to predict without one.
const MIN_LOCKED_SAMPLES: usize = 64;
const MAX_LOCKED_PHASE_STD_RAD: f32 = 0.35;
const MAX_TIMING_GATE_FRACTION: f32 = 0.25;
/// Narrowest the timing gate may become, in samples.
///
/// A floor keeps the gate from collapsing onto a tracker that happens to be
/// momentarily quiet. It should not be doing the work the spread term does:
/// with a whole-sample peak index the spread alone opens the gate to about
/// 0.87 samples, so a floor near a full sample only overrides the estimator
/// that reports better than that.
const MIN_TIMING_GATE_SAMPLES: f32 = 0.25;
/// Consecutive gate rejections that mean the tracker, not the signal, is
/// wrong. Rejected detections never reach the statistics the gate is built
/// from, so without this a tracker whose rotation estimate has gone stale
/// would reject every real pulse forever.
const MAX_CONSECUTIVE_REJECTIONS: usize = 8;
/// Timing error, in samples, that coasting is allowed to accumulate before
/// the tracker stops predicting.
///
/// Holdover integrates the rate estimate, so error grows as the coast length
/// times the fractional error in that estimate. Bounding the error rather
/// than the duration lets a settled tracker coast far longer than a freshly
/// acquired one, which is the behaviour wanted: the duration that is safe is
/// a property of how well the rate is known, not a constant.
const MAX_COAST_TIMING_ERROR_SAMPLES: f32 = 0.5;

/// Rotations after a rejected detection during which coasting stays
/// suppressed. Long enough to break the reject-predict-reject feedback loop,
/// short enough that a genuine dropout is still coasted through.
const REJECTION_COAST_HOLDOFF_ROTATIONS: f32 = 2.0;
const LOCK_STATS_WINDOW_TICKS: usize = 128;

struct RollingWindowStats {
    window: VecDeque<f32>,
    max_len: usize,
    sum: f64,
    sum_sq: f64,
}

impl RollingWindowStats {
    fn new(max_len: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(max_len),
            max_len,
            sum: 0.0,
            sum_sq: 0.0,
        }
    }

    fn update(&mut self, value: f32) {
        if self.window.len() == self.max_len
            && let Some(old) = self.window.pop_front()
        {
            let old = old as f64;
            self.sum -= old;
            self.sum_sq -= old * old;
        }

        self.window.push_back(value);
        let v = value as f64;
        self.sum += v;
        self.sum_sq += v * v;
    }

    fn clear(&mut self) {
        self.window.clear();
        self.sum = 0.0;
        self.sum_sq = 0.0;
    }

    fn count(&self) -> usize {
        self.window.len()
    }

    fn mean(&self) -> Option<f32> {
        let n = self.window.len();
        if n == 0 {
            None
        } else {
            Some((self.sum / n as f64) as f32)
        }
    }

    fn variance(&self) -> Option<f32> {
        let n = self.window.len();
        if n < 2 {
            return None;
        }
        let n_f64 = n as f64;
        let mean = self.sum / n_f64;
        let var = (self.sum_sq / n_f64) - mean * mean;
        Some(var.max(0.0) as f32)
    }

    fn std_dev(&self) -> Option<f32> {
        self.variance().map(f32::sqrt)
    }
}

pub struct DpllNorthTracker {
    gain: f32,
    /// Slow gain on top of `gain`, driving the detected pulse peak towards
    /// what the configuration says to expect. None when disabled.
    agc: Option<NorthPulseAgc>,
    highpass: FirHighpass,
    peak_detector: PeakDetector,
    pulse_reference_offset: f32,
    estimator: NorthPulseEstimator,
    centroid_half_width: usize,
    filter_tail_len: usize,
    /// How far back of the current position a detection can still surface,
    /// because the detector defers a crossing until its search window and
    /// trailing context have both been seen.
    detector_deferral_samples: usize,
    last_tick_sample: Option<usize>,

    // PLL state
    phase: f32,     // Current phase estimate (radians, 0-2π)
    frequency: f32, // Frequency estimate (radians/sample)

    // PLL parameters
    kp: f32, // Proportional gain
    ki: f32, // Integral gain

    // Frequency limits (radians/sample)
    min_omega: f32,
    max_omega: f32,

    sample_counter: usize,
    sample_rate: f32,

    // Rolling statistics for lock quality
    phase_error_stats: RollingWindowStats,
    freq_stats: RollingWindowStats,
    lock_quality_weights: LockQualityWeights,

    // Pre-allocated buffer for filtering
    filter_buffer: Vec<f32>,
    // Filtered samples preceding filter_buffer, for estimator windows that
    // straddle a buffer boundary.
    filter_tail: Vec<f32>,
    quiet_watch: QuietChannelWatch,
    threshold: f32,
    /// Where a pulse was last accepted, for coasting and lock state.
    last_measured_sample: Option<usize>,
    /// Where a detection was last rejected, so coasting can stand aside while
    /// detections are being disputed without standing aside forever.
    last_rejection_sample: Option<usize>,
    /// Sub-sample part of the last emitted tick, so coasting advances by the
    /// true period instead of accumulating a rounding error every rotation.
    last_tick_fraction: f32,
    consecutive_rejections: usize,
    max_coast_samples: usize,
    gate_sigma: f32,
}

impl DpllNorthTracker {
    #[inline]
    fn wrap_phase(phase: f32) -> f32 {
        phase.rem_euclid(2.0 * PI)
    }

    #[inline]
    fn wrap_phase_error(phase_error: f32) -> f32 {
        (phase_error + PI).rem_euclid(2.0 * PI) - PI
    }

    /// Whether the oscillator has seen enough ticks for its phase to be a
    /// better estimate of the tick time than the detected peak index.
    ///
    /// This is deliberately a count and not a phase-error dispersion test.
    /// A dispersion threshold chatters on and off near its limit, and since
    /// the correction is the whole sub-sample part of the answer, chatter
    /// puts a step of that size straight into the reported tick time.
    #[inline]
    fn stable_enough_for_phase_correction(&self) -> bool {
        self.phase_error_stats.count() >= MIN_PHASE_CORRECTION_SAMPLES
    }

    /// Whether the oscillator is tracking well enough to be trusted over the
    /// detector: to reject a detection that disagrees with it, or to predict
    /// a tick where no detection happened.
    ///
    /// A tick count alone is not evidence of that. It says only that pulses
    /// arrived, not that the loop followed them -- a rotation starting away
    /// from the configured initial frequency, or moving faster than the loop
    /// can track, produces plenty of ticks while the oscillator sits at the
    /// wrong rate. Both callers here would then do damage rather than good,
    /// gating out valid pulses or coasting at a stale rate, so they also
    /// require the phase error to have settled.
    ///
    /// Phase correction deliberately does not use this predicate: it is
    /// harmless when the loop is wrong, and a dispersion threshold chatters
    /// near its limit, which would put a step into the reported tick time.
    fn locked(&self) -> bool {
        if self.phase_error_stats.count() < MIN_LOCKED_SAMPLES {
            return false;
        }
        self.phase_error_stats
            .std_dev()
            .is_some_and(|std_dev| std_dev.is_finite() && std_dev <= MAX_LOCKED_PHASE_STD_RAD)
    }

    pub fn new(config: &NorthTickConfig, sample_rate: f32) -> Result<Self> {
        if !sample_rate.is_finite() || sample_rate <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick sample_rate must be finite and > {}, got {}",
                FREQUENCY_EPSILON, sample_rate
            )));
        }

        validate_north_tick_config(config, sample_rate)?;

        let initial_freq = config.dpll.initial_frequency_hz;
        if !initial_freq.is_finite() || initial_freq <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.initial_frequency_hz must be finite and > {}, got {}",
                FREQUENCY_EPSILON, initial_freq
            )));
        }

        let natural_frequency_hz = config.dpll.natural_frequency_hz;
        if !natural_frequency_hz.is_finite() || natural_frequency_hz <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.natural_frequency_hz must be finite and > {}, got {}",
                FREQUENCY_EPSILON, natural_frequency_hz
            )));
        }

        let damping_ratio = config.dpll.damping_ratio;
        if !damping_ratio.is_finite() || damping_ratio < 0.0 {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.damping_ratio must be finite and >= 0, got {}",
                damping_ratio
            )));
        }

        let frequency_min_hz = config.dpll.frequency_min_hz;
        let frequency_max_hz = config.dpll.frequency_max_hz;
        if !frequency_min_hz.is_finite() || frequency_min_hz <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.frequency_min_hz must be finite and > {}, got {}",
                FREQUENCY_EPSILON, frequency_min_hz
            )));
        }
        if !frequency_max_hz.is_finite() || frequency_max_hz <= FREQUENCY_EPSILON {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.frequency_max_hz must be finite and > {}, got {}",
                FREQUENCY_EPSILON, frequency_max_hz
            )));
        }
        if frequency_min_hz >= frequency_max_hz {
            return Err(RdfError::Config(format!(
                "north_tick.dpll.frequency_min_hz ({}) must be < north_tick.dpll.frequency_max_hz ({})",
                frequency_min_hz, frequency_max_hz
            )));
        }

        // The detector dead time must be shorter than the period at
        // frequency_max_hz, or valid ticks at the top of the band are
        // silently rejected and the tracker sees an aliased half-rate
        // stream. Shortening the dead time instead measurably hurts
        // low-SNR detection, so a conflicting configuration is an error.
        // Compare in continuous time so the verdict is identical at every
        // sample rate (truncated sample counts flip near the boundary).
        let period_at_max_ms = 1000.0 / frequency_max_hz;
        if config.min_interval_ms >= period_at_max_ms {
            return Err(RdfError::Config(format!(
                "north_tick.min_interval_ms ({} ms) must be shorter than the period at \
                 dpll.frequency_max_hz ({} Hz = {:.3} ms); lower min_interval_ms or \
                 frequency_max_hz",
                config.min_interval_ms, frequency_max_hz, period_at_max_ms
            )));
        }
        let min_samples = (config.min_interval_ms / 1000.0 * sample_rate) as usize;
        let gain = db_to_amplitude(config.gain_db);

        // Initial frequency estimate from config
        let omega = 2.0 * PI * initial_freq / sample_rate;

        // PLL gains — the loop updates once per detected tick, not once per
        // sample. Normalize the natural frequency to the tick rate and scale
        // the integral gain by the expected update interval in samples.
        let tick_rate = initial_freq;
        let samples_per_tick = sample_rate / tick_rate;
        let wn = 2.0 * PI * config.dpll.natural_frequency_hz / tick_rate;
        let zeta = config.dpll.damping_ratio;
        let kp = 2.0 * zeta * wn;
        let ki = wn * wn / samples_per_tick;

        // Calculate frequency limits in radians/sample
        let min_omega = 2.0 * PI * config.dpll.frequency_min_hz / sample_rate;
        let max_omega = 2.0 * PI * config.dpll.frequency_max_hz / sample_rate;

        let highpass = FirHighpass::new(
            config.highpass_cutoff,
            sample_rate,
            highpass_taps(config, sample_rate),
            config.highpass_transition_hz,
        )?;

        // With the AGC running, the level reaching the filter is driven to
        // the expected amplitude rather than being whatever the receiver
        // delivers times a static gain, so that is what the peak search
        // window and the delay compensation should be derived from.
        let effective_pulse_amplitude = if config.agc.enabled {
            config.expected_pulse_amplitude.max(f32::EPSILON)
        } else {
            (config.expected_pulse_amplitude * gain).max(f32::EPSILON)
        };
        let nominal_period_samples = sample_rate / config.dpll.initial_frequency_hz.max(1.0);
        let centroid_half_width =
            centroid_half_width(config.estimator, sample_rate, nominal_period_samples);
        let peak_timing = derive_peak_timing(
            &highpass,
            config.threshold,
            effective_pulse_amplitude,
            config.estimator,
            centroid_half_width,
        );

        Ok(Self {
            gain,
            agc: config.agc.enabled.then(|| {
                NorthPulseAgc::new(
                    config.expected_pulse_amplitude * highpass.peak_response(),
                    config.agc.time_constant_secs,
                    config.dpll.initial_frequency_hz.max(1.0),
                    config.agc.min_gain,
                    config.agc.max_gain,
                )
            }),
            highpass,
            peak_detector: {
                let mut detector = PeakDetector::with_peak_search_window(
                    config.threshold,
                    min_samples,
                    peak_timing.peak_search_window_samples,
                );
                // The estimator reads samples on both sides of the peak, and
                // the peak can land at the end of the search window, so the
                // detector must hold a crossing until that context exists.
                detector.set_trailing_context(centroid_half_width);
                detector
            },
            pulse_reference_offset: peak_timing.pulse_reference_offset,
            estimator: config.estimator,
            centroid_half_width,
            // A peak whose search window straddled a buffer boundary is
            // reported at a negative index in the next buffer. That index
            // reaches back by the search window plus the trailing context the
            // detector now waits for, and the estimator then reads a further
            // half-width before it.
            filter_tail_len: 2 * centroid_half_width + peak_timing.peak_search_window_samples,
            detector_deferral_samples: centroid_half_width + peak_timing.peak_search_window_samples,
            last_tick_sample: None,
            phase: 0.0,
            frequency: omega,
            kp,
            ki,
            min_omega,
            max_omega,
            sample_counter: 0,
            sample_rate,
            phase_error_stats: RollingWindowStats::new(LOCK_STATS_WINDOW_TICKS),
            freq_stats: RollingWindowStats::new(LOCK_STATS_WINDOW_TICKS),
            lock_quality_weights: config.lock_quality_weights,
            filter_buffer: Vec::new(),
            filter_tail: Vec::new(),
            quiet_watch: QuietChannelWatch::new(sample_rate),
            threshold: config.threshold,
            last_measured_sample: None,
            last_rejection_sample: None,
            last_tick_fraction: 0.0,
            consecutive_rejections: 0,
            max_coast_samples: ((config.max_coast_ms / 1000.0 * sample_rate).max(0.0)) as usize,
            gate_sigma: config.gate_sigma.max(0.0),
        })
    }

    /// Coast over lost samples: the NCO keeps rotating at its tracked
    /// frequency and the sample clock advances, so tick indices after a
    /// capture gap stay on the real timeline.
    pub fn advance_samples(&mut self, samples: usize) {
        // Advance in f64 and reduce before narrowing: a long gap makes
        // frequency * samples large enough that an f32 product cannot
        // resolve where in the rotation the oscillator lands, and the loop
        // then rejects valid pulses on the far side of the gap.
        let advance =
            (self.frequency as f64 * samples as f64).rem_euclid(2.0 * std::f64::consts::PI);
        self.phase = Self::wrap_phase(self.phase + advance as f32);
        self.sample_counter += samples;
        // The delay line holds pre-gap audio that no longer adjoins what
        // follows it.
        self.highpass.reset();
        // A capture gap is time with no pulses, so it spends coasting budget
        // like any other dropout. Nothing was disputed across it, so a
        // rejection from before the gap must not hold coasting off after it.
        self.consecutive_rejections = 0;
        self.last_rejection_sample = None;
        self.peak_detector.reset_continuity();
        self.filter_tail.clear();
        // The next tick begins a fresh interval; the min-spacing guard must
        // not compare it against a pre-gap tick.
        self.last_tick_sample = None;
    }

    /// Emit a final tick for a crossing whose search window was still
    /// pending at end-of-stream. Uses the current locked frequency without
    /// running the PI update — nothing follows this tick.
    pub fn finish(&mut self) -> Vec<NorthTick> {
        let Some((rel, _amp)) = self.peak_detector.flush() else {
            return Vec::new();
        };
        let delay = derive_delay_compensation(&self.highpass, self.pulse_reference_offset);
        let global_sample = (self.sample_counter as isize + rel).max(0) as usize;
        let compensated_sample = global_sample.saturating_sub(delay.delay_samples);
        if let Some(last) = self.last_tick_sample {
            let min_spacing = (2.0 * PI / self.frequency) * MIN_TICK_SPACING_FRACTION;
            if (compensated_sample.saturating_sub(last) as f32) < min_spacing {
                return Vec::new();
            }
        }
        let (reported_sample, fractional_sample_offset) =
            split_effective_time(compensated_sample, delay.fractional_sample_offset);
        self.last_tick_sample = Some(compensated_sample);
        vec![NorthTick {
            sample_index: reported_sample,
            period: Some(2.0 * PI / self.frequency),
            lock_quality: self.lock_quality(),
            phase_variance: self.phase_error_stats.variance(),
            fractional_sample_offset,
            phase: 0.0,
            frequency: self.frequency,
        }]
    }

    /// Samples since a pulse was last accepted at the given position.
    fn coasted_samples(&self, at: usize) -> Option<usize> {
        self.last_measured_sample
            .map(|measured| at.saturating_sub(measured))
    }

    /// How far the tracker may coast before its own rate uncertainty puts
    /// more than `MAX_COAST_TIMING_ERROR_SAMPLES` into a predicted tick.
    ///
    /// The spread of the frequency estimate over the recent window stands in
    /// for that uncertainty. A tracker that has been following a steady
    /// rotation for a while has a tight spread and earns close to the
    /// configured maximum; one still settling has a loose spread and is held
    /// to a short prediction, or none.
    fn coast_budget_samples(&self) -> usize {
        if self.frequency <= FREQUENCY_EPSILON {
            return 0;
        }
        let Some(period) = Some(2.0 * PI / self.frequency).filter(|p| p.is_finite() && *p >= 1.0)
        else {
            return 0;
        };

        // Per-rotation timing discrepancy, from two directions.
        //
        // The scatter of the frequency estimate says how repeatable the rate
        // is. On its own that is not enough: a loop still converging on the
        // true rate is steadily wrong with very little scatter, and would be
        // granted a long prediction it cannot deliver. The mean phase error
        // catches exactly that case -- an oscillator sitting at an offset
        // from the pulses is one whose rate is wrong -- so the tighter of
        // the two governs.
        let spread = self
            .freq_stats
            .std_dev()
            .zip(self.freq_stats.mean())
            .filter(|(s, m)| s.is_finite() && *m > FREQUENCY_EPSILON)
            .map(|(s, m)| (s / m) * period)
            .unwrap_or(f32::INFINITY);
        // Only the part of the mean that stands out from its own noise counts.
        // The mean of a zero-mean scatter over N ticks is not zero but about
        // sigma/sqrt(N), and charging that against the budget would hold a
        // long-settled tracker to the same short prediction as a converging
        // one.
        let systematic = match (
            self.phase_error_stats.mean(),
            self.phase_error_stats.std_dev(),
        ) {
            (Some(mean), Some(std_dev)) if mean.is_finite() && std_dev.is_finite() => {
                let count = self.phase_error_stats.count().max(1) as f32;
                let noise_floor = 2.0 * std_dev / count.sqrt();
                ((mean.abs() - noise_floor).max(0.0) / self.frequency).abs()
            }
            _ => f32::INFINITY,
        };

        let per_rotation = spread.max(systematic);
        if per_rotation <= f32::EPSILON {
            return self.max_coast_samples;
        }

        let rotations = MAX_COAST_TIMING_ERROR_SAMPLES / per_rotation;
        if !rotations.is_finite() || rotations <= 0.0 {
            return 0;
        }
        self.max_coast_samples.min((rotations * period) as usize)
    }

    /// Whether the rotation estimate is recent enough to predict from.
    fn within_coast_budget(&self, at: usize) -> bool {
        let budget = self.coast_budget_samples();
        self.coasted_samples(at)
            .is_some_and(|coasted| coasted <= budget)
    }

    /// Confidence in a tick predicted rather than measured, falling to zero at
    /// the end of the coasting budget.
    /// Timing variance a predicted tick carries on top of the measured
    /// scatter, in radians squared of rotation phase.
    ///
    /// A coasted tick is not as good as a measured one and its error grows
    /// with every rotation predicted, which is the whole reason the budget
    /// exists. Reporting the last measured scatter unchanged -- as this did --
    /// left confidence flat across a dropout: a tick predicted a full second
    /// ago scored the same as one just detected, and `lock_quality`, the only
    /// field that did decay, is not in the confidence.
    ///
    /// The budget's contract is that accumulated error stays inside
    /// `MAX_COAST_TIMING_ERROR_SAMPLES`, so the fraction of the budget spent
    /// is the fraction of that error to expect.
    fn coast_drift_variance(&self, at: usize) -> f32 {
        let spent = 1.0 - self.coast_quality_scale(at);
        let drift_samples = MAX_COAST_TIMING_ERROR_SAMPLES * spent;
        let drift_radians = drift_samples * self.frequency;
        drift_radians * drift_radians
    }

    fn coast_quality_scale(&self, at: usize) -> f32 {
        let budget = self.coast_budget_samples();
        if budget == 0 {
            return 0.0;
        }
        let coasted = self.coasted_samples(at).unwrap_or(usize::MAX);
        1.0 - (coasted as f32 / budget as f32).clamp(0.0, 1.0)
    }

    /// Widest disagreement between a detection and the tracked rotation that
    /// is still treated as the same pulse, in samples.
    ///
    /// Scaled to the phase error the tracker is actually seeing, with a floor
    /// of one sample so quantization alone can never trip it and a ceiling of
    /// a quarter rotation so the gate cannot swallow a genuine half-rate
    /// stream.
    fn timing_gate_samples(&self, period_estimate: f32) -> Option<f32> {
        if !self.locked() || self.frequency <= FREQUENCY_EPSILON {
            return None;
        }
        let std_dev = self.phase_error_stats.std_dev()?;
        if !std_dev.is_finite() {
            return None;
        }
        let gate = (self.gate_sigma * std_dev / self.frequency).max(MIN_TIMING_GATE_SAMPLES);
        Some(gate.min(period_estimate * MAX_TIMING_GATE_FRACTION))
    }

    /// Emit ticks from the tracked rotation for rotations that produced no
    /// usable detection, up to `until_sample`.
    fn coast_to(&mut self, until_sample: usize, ticks: &mut Vec<NorthTick>) {
        // Predicting requires a rotation estimate worth predicting from.
        if !self.locked() || self.frequency <= FREQUENCY_EPSILON {
            return;
        }

        // Coasting covers rotations where no pulse arrived. A rejected
        // detection is not that: a pulse did arrive and the tracker distrusted
        // it. Predicting through a rejection closes a feedback loop -- the
        // prediction stands in for the measurement, so the loop gets no
        // correction, so its disagreement with the next detection grows, so
        // that one is rejected too. Measured across a rate step, disagreement
        // ran away from 1.5 to 7 samples over eight rotations this way.
        //
        // The hold is on recency, not on a flag: while detections keep being
        // rejected it keeps renewing and the loop stays broken, but a single
        // disputed pulse at the start of a real dropout must not switch
        // coasting off for the rest of it.
        if let Some(rejected_at) = self.last_rejection_sample {
            let holdoff = 2.0 * PI / self.frequency * REJECTION_COAST_HOLDOFF_ROTATIONS;
            if (until_sample.saturating_sub(rejected_at) as f32) < holdoff {
                return;
            }
        }
        // Coasting integrates the rotation rate over many rotations, so the
        // instantaneous estimate's tick-to-tick noise would accumulate into a
        // drift. The averaged estimate is what the loop actually knows.
        let frequency = self
            .freq_stats
            .mean()
            .filter(|f| f.is_finite() && *f > FREQUENCY_EPSILON)
            .unwrap_or(self.frequency);
        let period = 2.0 * PI / frequency;
        if !period.is_finite() || period < 1.0 {
            return;
        }

        // A predicted tick must not land where a real pulse could still be
        // detected: it would take the detection's place and push the real one
        // inside the dead-time guard.
        //
        // "Could still be detected" reaches further back than the current
        // position, because the detector holds a crossing until its search
        // window and trailing context have been seen and then reports it at a
        // position already passed. Reserving only the dead time would leave
        // room for a predicted tick to land inside the dead time of a
        // detection that has not surfaced yet.
        let reserved =
            (period * MIN_TICK_SPACING_FRACTION).round() as usize + self.detector_deferral_samples;

        while let Some(last) = self.last_tick_sample {
            // Advance a fractional epoch. Rounding the period on every
            // rotation instead would accumulate: the default 624 us rotation
            // is 29.952 samples, so a rounded step drifts the better part of
            // a sample every twenty rotations.
            let position = last as f64 + self.last_tick_fraction as f64 + period as f64;
            let next = position.round().max(0.0) as usize;
            if next + reserved > until_sample || !self.within_coast_budget(next) {
                break;
            }
            self.last_tick_sample = Some(next);
            self.last_tick_fraction = (position - next as f64) as f32;
            ticks.push(NorthTick {
                sample_index: next,
                period: Some(period),
                lock_quality: self
                    .lock_quality()
                    .map(|q| q * self.coast_quality_scale(next)),
                phase_variance: self
                    .phase_error_stats
                    .variance()
                    .map(|v| v + self.coast_drift_variance(next)),
                fractional_sample_offset: self.last_tick_fraction,
                phase: 0.0,
                frequency,
            });
        }
    }

    pub fn process_buffer(&mut self, buffer: &[f32]) -> Vec<NorthTick> {
        let gain = self.gain * self.agc.as_ref().map_or(1.0, |agc| agc.gain());
        preprocess_north_buffer(&mut self.filter_buffer, buffer, gain, &mut self.highpass);

        let peaks = self.peak_detector.find_all_peaks(&self.filter_buffer);

        // Adapt only on detections. A peak tracker given a silent channel
        // raises gain until the noise floor crosses the threshold and then
        // detects its own noise.
        // Before anything has been detected there is nothing better to go on.
        // Once there is, the gain learns only from detections the timing gate
        // accepted, further down: a detection the tracker does not believe is
        // not evidence about the pulse level, and above about a quarter RMS of
        // channel noise most detections are of that kind.
        if let Some(agc) = self.agc.as_mut()
            && peaks.is_empty()
        {
            agc.observe_undetected(&self.filter_buffer);
        }
        let strongest = peaks.iter().fold(0.0f32, |acc, (_, amp)| acc.max(*amp));
        self.quiet_watch
            .note_detections(peaks.len(), strongest, self.threshold);
        self.quiet_watch.advance(buffer.len(), self.threshold);

        let delay = derive_delay_compensation(&self.highpass, self.pulse_reference_offset);

        let mut ticks = Vec::with_capacity(peaks.len());

        // Peak indices are strictly increasing but a peak whose search
        // window completed in this buffer may sit at a small negative index
        // (in the previous buffer); the phase advance is then a rewind.
        let mut last_sample_idx: isize = 0;
        for &(peak_idx, amplitude) in &peaks {
            // Advance PLL phase from last_sample_idx to peak_idx
            let samples_to_advance = peak_idx - last_sample_idx;
            self.phase += self.frequency * samples_to_advance as f32;
            self.phase = Self::wrap_phase(self.phase);

            let estimator_fraction = estimate_fraction(
                &self.filter_tail,
                &self.filter_buffer,
                peak_idx,
                self.estimator,
                self.centroid_half_width,
            );

            let global_sample = (self.sample_counter as isize + peak_idx).max(0) as usize;
            let compensated_sample = global_sample.saturating_sub(delay.delay_samples);
            let period_estimate = 2.0 * PI / self.frequency;

            // Fill in rotations that produced no usable detection before
            // accounting for this one, so a dropout in the middle of a buffer
            // is coasted through rather than left as a gap.
            self.coast_to(compensated_sample, &mut ticks);

            if let Some(last) = self.last_tick_sample {
                let min_spacing = period_estimate * MIN_TICK_SPACING_FRACTION;
                let delta = compensated_sample.saturating_sub(last) as f32;
                if delta < min_spacing {
                    last_sample_idx = peak_idx;
                    continue;
                }
            }

            // Phase error: how far are we from expected zero phase?
            // The oscillator was advanced to the whole-sample peak index, so
            // the estimator's sub-sample offset is added here rather than
            // accumulated into the oscillator, which continues on the sample
            // grid.
            let phase_at_pulse = Self::wrap_phase(self.phase + self.frequency * estimator_fraction);
            let phase_error = Self::wrap_phase_error(-phase_at_pulse);

            // Reject a detection that disagrees with the tracked rotation by
            // more than the tracker's own timing spread. The gate stays
            // inactive until there is enough history to know that spread, so
            // the tracker can never lock itself out.
            //
            // What reaches this point is narrower than "interference": the
            // detector's dead time covers most of the rotation, so an impulse
            // arriving early is never detected at all. What the gate can act
            // on is a detection displaced from where the rotation says the
            // pulse belongs -- an interferer just ahead of a real pulse,
            // which also masks it, or a pulse whose leading edge noise moved
            // it. Rejecting those keeps the loop from being pulled, and
            // coasting covers the rotation they cost.
            if let Some(gate) = self.timing_gate_samples(period_estimate) {
                let systematic = self.phase_error_stats.mean().unwrap_or(0.0);
                let disagreement = (phase_error - systematic) / self.frequency;
                if disagreement.abs() > gate {
                    self.consecutive_rejections += 1;
                    self.last_rejection_sample = Some(compensated_sample);
                    if self.consecutive_rejections < MAX_CONSECUTIVE_REJECTIONS {
                        last_sample_idx = peak_idx;
                        continue;
                    }
                    // Everything is being rejected, so the rotation estimate
                    // is what is wrong. Drop the history the gate is built
                    // from and reacquire from this pulse.
                    self.phase_error_stats.clear();
                    self.freq_stats.clear();
                    self.last_measured_sample = None;
                }
            }
            self.consecutive_rejections = 0;
            self.last_rejection_sample = None;

            // The gate believes this one, so it is evidence about the pulse
            // level -- but only while the oscillator is locked. Above about a
            // quarter RMS of channel noise the loop does not lock, most
            // detections are noise, and an amplitude drawn from them is worse
            // than no adaptation at all: the gain converges on nonsense and
            // then freezes there.
            let locked = self.locked();
            if let Some(agc) = self.agc.as_mut()
                && locked
            {
                agc.observe(amplitude);
            }

            // Track phase error for variance calculation
            self.phase_error_stats.update(phase_error);

            // Convert phase error to a bounded fractional timing correction.
            // phase_error = -phase, so positive NCO phase at the peak means the
            // oscillator's zero crossing occurred phase/frequency samples earlier;
            // the correction must shift the tick earlier (negative), i.e.
            // phase_error/frequency = -phase/frequency.
            //
            // Only the part of the phase error that varies tick to tick comes
            // from quantizing the peak to a whole sample, and only that part
            // should move the reported time. A persistent component means the
            // oscillator is sitting at an offset from the pulses -- it is
            // lagging a rate change, or the rate is commensurate with the
            // sample clock so the quantization error never dithers. Applying
            // it would put the loop's own lag into the bearing, so the
            // running mean is removed first.
            let phase_timing_correction = if self.stable_enough_for_phase_correction()
                && self.frequency > FREQUENCY_EPSILON
            {
                let systematic = self.phase_error_stats.mean().unwrap_or(0.0);
                ((phase_error - systematic) / self.frequency).clamp(
                    -MAX_PHASE_TIMING_CORRECTION_SAMPLES,
                    MAX_PHASE_TIMING_CORRECTION_SAMPLES,
                )
            } else {
                0.0
            };

            // The detected peak index is quantized to whole samples; the
            // oscillator's estimate of the same event is not. Re-anchor the
            // reported index on the corrected time so the fraction stays
            // within half a sample and no sub-sample information is lost to
            // the split.
            let (reported_sample, fractional_sample_offset) = split_effective_time(
                compensated_sample,
                delay.fractional_sample_offset + estimator_fraction + phase_timing_correction,
            );

            // Update frequency and phase with PI controller
            self.frequency += self.ki * phase_error;
            self.phase += self.kp * phase_error;

            // Clamp frequency to configured range
            self.frequency = self.frequency.clamp(self.min_omega, self.max_omega);

            // Track frequency for stability calculation
            self.freq_stats.update(self.frequency);

            // Wrap phase after correction
            self.phase = Self::wrap_phase(self.phase);

            // Calculate period in samples from current frequency estimate
            let period = 2.0 * PI / self.frequency;

            // Dead time is a property of the detector, so it is measured
            // against the detected position rather than the corrected one.
            // Feeding the correction back here lets a large correction during
            // acquisition push the next valid tick inside the guard.
            //
            // For bearing calculation, the tick itself defines north reference (phase = 0).
            // Jitter is represented by sample_index timing; using absolute DPLL oscillator
            // phase here would introduce reference drift across rotations.
            self.last_tick_sample = Some(compensated_sample);
            self.last_measured_sample = Some(compensated_sample);
            // Differenced in integers: these are absolute sample counts, and
            // past 2^24 an f32 cannot represent them to within a sample, so
            // converting before subtracting loses the quantity being measured.
            self.last_tick_fraction = fractional_sample_offset
                + (reported_sample as i64 - compensated_sample as i64) as f32;
            ticks.push(NorthTick {
                sample_index: reported_sample,
                period: Some(period),
                lock_quality: self.lock_quality(),
                phase_variance: self.phase_error_stats.variance(),
                fractional_sample_offset,
                phase: 0.0,
                frequency: self.frequency,
            });

            last_sample_idx = peak_idx;
        }

        // Advance phase for remaining samples after the last peak
        if last_sample_idx < buffer.len() as isize {
            let remaining = buffer.len() as isize - last_sample_idx;
            self.phase += self.frequency * remaining as f32;
            self.phase = Self::wrap_phase(self.phase);
        }

        retain_tail(
            &mut self.filter_tail,
            &self.filter_buffer,
            self.filter_tail_len,
        );

        self.sample_counter += buffer.len();

        // Cover the tail of the buffer, where a dropout that started mid-way
        // through leaves rotations with no detection to trigger the fill.
        let buffer_end = self
            .sample_counter
            .saturating_sub(delay.delay_samples.max(1));
        self.coast_to(buffer_end, &mut ticks);

        ticks
    }

    pub fn rotation_frequency(&self) -> Option<f32> {
        if self.frequency > 0.0 {
            Some(self.frequency * self.sample_rate / (2.0 * PI))
        } else {
            None
        }
    }

    /// Samples since a north pulse was last detected.
    pub fn samples_since_detection(&self) -> usize {
        self.quiet_watch.samples_since_detection()
    }

    pub fn phase_error_variance(&self) -> Option<f32> {
        self.phase_error_stats.variance()
    }

    pub fn lock_quality(&self) -> Option<f32> {
        if self.phase_error_stats.count() < 2 || self.freq_stats.count() < 2 {
            return None;
        }

        // Phase error std dev in radians - lower is better
        // A well-locked PLL should have phase error < 0.1 rad (~6 degrees)
        let phase_std = self.phase_error_stats.std_dev()?.abs();
        let phase_score = (1.0 - phase_std / PI).clamp(0.0, 1.0);

        // Frequency stability - lower variance relative to mean is better
        let freq_mean = self.freq_stats.mean()?;
        let freq_std = self.freq_stats.std_dev()?;
        let freq_cv = if freq_mean.abs() > FREQUENCY_EPSILON {
            (freq_std / freq_mean).abs()
        } else {
            1.0
        };
        let freq_score = (1.0 - freq_cv * 100.0).clamp(0.0, 1.0);

        // Combined score using configured weights
        Some(
            self.lock_quality_weights.phase_weight * phase_score
                + self.lock_quality_weights.frequency_weight * freq_score,
        )
    }

    pub fn filtered_buffer(&self) -> &[f32] {
        &self.filter_buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NorthAgcConfig;
    use crate::config::{DpllConfig, NorthTickConfig};

    #[test]
    fn test_dpll_north_tick_detection() {
        let config = NorthTickConfig::default();
        let sample_rate = 48000.0;
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        // Generate signal with pulses at 1602 Hz (every 30 samples approx)
        let samples_per_pulse = (sample_rate / 1602.0) as usize;
        let mut ticks_detected = 0;

        for _ in 0..40 {
            let mut signal = vec![0.0; samples_per_pulse];
            signal[5] = 0.8; // Pulse near start

            let ticks = tracker.process_buffer(&signal);
            if !ticks.is_empty() {
                ticks_detected += ticks.len();
            }
        }

        // May detect fewer initially due to FIR transient
        assert!(
            ticks_detected >= 30,
            "Should detect most ticks with FIR filter"
        );

        if let Some(freq) = tracker.rotation_frequency() {
            assert!(
                (freq - 1602.0).abs() < 50.0,
                "Rotation frequency {} should be close to 1602 Hz",
                freq
            );
        }
    }

    #[test]
    fn test_dpll_north_tick_delay_compensation_with_gain() {
        let sample_rate = 48000.0;
        // The subject here is the static gain_db path, so the adaptive gain is
        // turned off rather than left to whatever the default is: with it on
        // the level reaching the filter is driven to the expected amplitude
        // and gain_db stops deciding anything.
        let config = NorthTickConfig {
            gain_db: 20.0,
            agc: NorthAgcConfig {
                enabled: false,
                ..Default::default()
            },
            dpll: DpllConfig {
                initial_frequency_hz: 480.0,
                natural_frequency_hz: 10.0,
                damping_ratio: 0.707,
                frequency_min_hz: 300.0,
                frequency_max_hz: 800.0,
            },
            ..Default::default()
        };
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        let pulse_positions = [100, 200, 300, 400, 500];
        let mut signal = vec![0.0f32; 1000];
        for &pos in &pulse_positions {
            signal[pos] = config.expected_pulse_amplitude;
        }

        let ticks = tracker.process_buffer(&signal);
        assert!(
            ticks.len() == pulse_positions.len(),
            "Expected {} ticks, got {}",
            pulse_positions.len(),
            ticks.len()
        );

        for tick in &ticks {
            let closest_pulse = pulse_positions
                .iter()
                .min_by_key(|&&p| (p as isize - tick.sample_index as isize).abs())
                .unwrap();
            let error = (*closest_pulse as isize - tick.sample_index as isize).abs();
            assert!(
                error <= 2,
                "Tick sample_index {} too far from expected pulse {}",
                tick.sample_index,
                closest_pulse
            );
        }
    }

    #[test]
    fn test_dpll_locks_to_true_frequency() {
        // Regression test for an off-by-one in phase advancement that made
        // the loop lock to sample_rate/(period-1) instead of sample_rate/period
        // (484.85 Hz instead of 480 Hz for a 100-sample period at 48 kHz).
        let sample_rate = 48_000.0;
        let config = NorthTickConfig {
            dpll: DpllConfig {
                initial_frequency_hz: 480.0,
                natural_frequency_hz: 10.0,
                damping_ratio: 0.707,
                frequency_min_hz: 300.0,
                frequency_max_hz: 800.0,
            },
            ..Default::default()
        };
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        // Pulses at exactly 480 Hz (period 100 samples), split across buffers
        // so cross-buffer phase accounting is exercised too.
        let period = 100;
        let buffer_len = 1024;
        let total_samples = 50_000;
        let mut signal = vec![0.0f32; total_samples];
        for idx in (50..total_samples).step_by(period) {
            signal[idx] = config.expected_pulse_amplitude;
        }
        for buffer in signal.chunks(buffer_len) {
            tracker.process_buffer(buffer);
        }

        let freq = tracker
            .rotation_frequency()
            .expect("tracker should be tracking a frequency");
        assert!(
            (freq - 480.0).abs() < 0.5,
            "DPLL locked to {} Hz, expected 480 Hz",
            freq
        );
    }

    #[test]
    fn test_dpll_rejects_dead_time_conflicting_with_frequency_max() {
        // 0.6 ms dead time (28 samples @ 48 kHz) supports at most ~1714 Hz;
        // pairing it with a higher frequency_max_hz used to silently alias
        // ticks to half rate instead of failing.
        let config = NorthTickConfig {
            dpll: DpllConfig {
                frequency_max_hz: 1_800.0,
                ..NorthTickConfig::default().dpll
            },
            ..Default::default()
        };
        let err = match DpllNorthTracker::new(&config, 48_000.0) {
            Err(e) => e,
            Ok(_) => panic!("expected a config error for 0.6 ms dead time at 1800 Hz max"),
        };
        assert!(
            err.to_string().contains("min_interval_ms"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_dpll_detects_ticks_at_frequency_max() {
        // Regression test: the default dead time and frequency_max_hz used
        // to contradict each other, silently rejecting every other tick at
        // the top of the configured band (aliased half-rate stream).
        let sample_rate = 48_000.0;
        let config = NorthTickConfig::default();
        let freq_max = config.dpll.frequency_max_hz;
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        let period = sample_rate as f64 / freq_max as f64;
        let total_samples = 48_000usize;
        let n_pulses = ((total_samples as f64 - 100.0) / period) as usize;
        let mut signal = vec![0.0f32; total_samples];
        for k in 0..n_pulses {
            signal[(50.0 + k as f64 * period).round() as usize] = config.expected_pulse_amplitude;
        }

        let mut ticks = 0;
        for buffer in signal.chunks(1024) {
            ticks += tracker.process_buffer(buffer).len();
        }
        assert!(
            ticks >= n_pulses * 9 / 10,
            "detected {} of {} ticks at frequency_max {} Hz",
            ticks,
            n_pulses,
            freq_max
        );
    }

    #[test]
    fn test_dpll_phase_correction_reduces_timing_error() {
        // Regression test for a sign inversion in the fractional timing
        // correction. Pulses at fractional period 30.4 samples (~1578.9 Hz
        // @ 48 kHz) land on quantized integer samples; the phase correction
        // should recover sub-sample timing. Measured steady-state RMS error:
        // correct sign 0.20 samples, correction disabled 0.28, inverted
        // sign 0.37 — the 0.25 bound fails both regressions.
        let sample_rate = 48_000.0;
        let config = NorthTickConfig {
            dpll: DpllConfig {
                initial_frequency_hz: 1_578.9,
                natural_frequency_hz: 15.0,
                damping_ratio: 0.707,
                frequency_min_hz: 1_400.0,
                frequency_max_hz: 1_650.0,
            },
            ..Default::default()
        };
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        let period = 30.4f64;
        let total_samples = 120_000usize;
        let n_pulses = ((total_samples as f64 - 100.0) / period) as usize;
        let true_times: Vec<f64> = (0..n_pulses).map(|k| 50.0 + k as f64 * period).collect();
        let mut signal = vec![0.0f32; total_samples];
        for t in &true_times {
            signal[t.round() as usize] = config.expected_pulse_amplitude;
        }

        let mut ticks = Vec::new();
        for buffer in signal.chunks(1024) {
            ticks.extend(tracker.process_buffer(buffer));
        }
        assert!(ticks.len() > 1000, "got {} ticks", ticks.len());

        // Steady state: last half of the run.
        let steady = &ticks[ticks.len() / 2..];
        let errors: Vec<f64> = steady
            .iter()
            .map(|t| {
                let measured = t.sample_index as f64 + t.fractional_sample_offset as f64;
                true_times
                    .iter()
                    .map(|&tt| measured - tt)
                    .min_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap())
                    .unwrap()
            })
            .collect();
        let mean = errors.iter().sum::<f64>() / errors.len() as f64;
        let rms_about_mean =
            (errors.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / errors.len() as f64).sqrt();
        let n_corrected = steady
            .iter()
            .filter(|t| t.fractional_sample_offset.abs() > 1e-6)
            .count();
        assert!(
            n_corrected > steady.len() / 2,
            "phase correction should be active in steady state ({} of {} ticks corrected)",
            n_corrected,
            steady.len()
        );
        assert!(
            rms_about_mean < 0.25,
            "steady-state RMS timing error {:.4} samples exceeds 0.25",
            rms_about_mean
        );
    }

    #[test]
    fn test_dpll_fractional_timing_correction_is_bounded() {
        let sample_rate = 48_000.0;
        let config = NorthTickConfig {
            dpll: DpllConfig {
                initial_frequency_hz: 1_602.0,
                natural_frequency_hz: 15.0,
                damping_ratio: 0.707,
                frequency_min_hz: 1_400.0,
                frequency_max_hz: 1_650.0,
            },
            ..Default::default()
        };
        let mut tracker = DpllNorthTracker::new(&config, sample_rate).unwrap();

        let nominal_period = (sample_rate / config.dpll.initial_frequency_hz).round() as isize;
        let mut signal = vec![0.0f32; 4096];
        for k in 0..110isize {
            let jitter = match k % 4 {
                0 => -1,
                1 => 0,
                2 => 1,
                _ => 0,
            };
            let idx = 60 + k * nominal_period + jitter;
            if idx >= 0 && (idx as usize) < signal.len() {
                signal[idx as usize] = config.expected_pulse_amplitude;
            }
        }

        let ticks = tracker.process_buffer(&signal);
        assert!(!ticks.is_empty(), "Expected at least one detected tick");

        for tick in ticks {
            assert!(
                tick.fractional_sample_offset.is_finite(),
                "fractional_sample_offset must be finite"
            );
            assert!(
                tick.fractional_sample_offset.abs() <= MAX_TOTAL_FRACTIONAL_OFFSET_SAMPLES + 1e-6,
                "fractional_sample_offset {} exceeds bound {}",
                tick.fractional_sample_offset,
                MAX_TOTAL_FRACTIONAL_OFFSET_SAMPLES
            );
        }
    }

    #[test]
    fn test_dpll_rejects_non_positive_initial_frequency() {
        let sample_rate = 48_000.0;
        let mut config = NorthTickConfig::default();
        config.dpll.initial_frequency_hz = 0.0;

        match DpllNorthTracker::new(&config, sample_rate) {
            Err(RdfError::Config(msg)) => {
                assert!(
                    msg.contains("initial_frequency_hz"),
                    "Unexpected message: {msg}"
                );
            }
            Err(err) => panic!("Expected configuration error, got {err}"),
            Ok(_) => panic!("Expected configuration error, got Ok"),
        }
    }

    #[test]
    fn test_dpll_rejects_invalid_frequency_bounds() {
        let sample_rate = 48_000.0;
        let mut config = NorthTickConfig::default();
        config.dpll.frequency_min_hz = 1800.0;
        config.dpll.frequency_max_hz = 1400.0;

        match DpllNorthTracker::new(&config, sample_rate) {
            Err(RdfError::Config(msg)) => {
                assert!(
                    msg.contains("frequency_min_hz") && msg.contains("frequency_max_hz"),
                    "Unexpected message: {msg}"
                );
            }
            Err(err) => panic!("Expected configuration error, got {err}"),
            Ok(_) => panic!("Expected configuration error, got Ok"),
        }
    }
}
