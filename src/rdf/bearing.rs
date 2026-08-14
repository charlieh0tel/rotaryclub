pub use crate::config::ConfidenceWeights;
pub use crate::constants::MIN_POWER_THRESHOLD;

use super::NorthTick;
use std::f32::consts::PI;

/// Circular variance of a phase spread uniformly over the full turn.
///
/// Coherence is scored against this, so a measurement whose phases carry no
/// information about a common bearing scores zero rather than something that
/// depends on how the spread happened to fall.
pub(super) const MAX_PHASE_VARIANCE: f32 = PI * PI / 3.0;

/// Mean direction of a set of phases, taken as vectors so that the wrap at
/// the turn does not pull the answer towards zero.
pub(super) fn circular_mean_phase(phases: &[f32]) -> f32 {
    let (sum_sin, sum_cos) = phases
        .iter()
        .fold((0.0_f32, 0.0_f32), |(acc_sin, acc_cos), &p| {
            (acc_sin + p.sin(), acc_cos + p.cos())
        });
    sum_sin.atan2(sum_cos)
}

/// Signed difference between two phases, in (-PI, PI].
pub(super) fn wrap_phase_diff(phase: f32, reference: f32) -> f32 {
    let diff = (phase - reference).rem_euclid(2.0 * PI);
    if diff > PI { diff - 2.0 * PI } else { diff }
}

pub trait BearingCalculator {
    /// Preprocess the doppler buffer (AGC + bandpass filter).
    /// Call this once per audio buffer before processing multiple ticks.
    fn preprocess(&mut self, doppler_buffer: &[f32]);

    /// Process a single north tick using the preprocessed buffer.
    /// Must call `preprocess` first.
    fn process_tick(&mut self, north_tick: &NorthTick) -> Option<BearingMeasurement>;

    /// Advance the sample counter after processing all ticks for a buffer.
    /// Call this once after all `process_tick` calls for a preprocessed buffer.
    fn advance_buffer(&mut self);

    /// Advance the sample clock over lost samples (no audio processed).
    fn advance_samples(&mut self, samples: usize);

    /// Get the filtered buffer (after AGC + bandpass) from the last preprocess call
    fn filtered_buffer(&self) -> &[f32];

    /// Convenience method that preprocesses and processes in one call.
    /// Use `preprocess` + `process_tick` + `advance_buffer` when processing multiple ticks per buffer.
    fn process_buffer(
        &mut self,
        doppler_buffer: &[f32],
        north_tick: &NorthTick,
    ) -> Option<BearingMeasurement> {
        self.preprocess(doppler_buffer);
        let result = self.process_tick(north_tick);
        self.advance_buffer();
        result
    }
}

/// Convert phase angle to bearing in degrees
///
/// Converts a phase angle in radians to a bearing angle in degrees,
/// normalized to the range 0-360°.
///
/// # Arguments
/// * `phase_radians` - Phase angle in radians
///
/// # Returns
/// Bearing angle in degrees (0-360)
pub fn phase_to_bearing(phase_radians: f32) -> f32 {
    let degrees = phase_radians.to_degrees();
    // Normalize to 0-360 using rem_euclid for proper modular arithmetic
    degrees.rem_euclid(360.0)
}

/// One-sigma bearing uncertainty, in degrees, from the spread of the
/// individual phase estimates and the reference they were measured against.
///
/// `phase_variance` is the spread of the `count` estimates that were averaged
/// into this bearing. It is deliberately not reduced by the root of that
/// count. Averaging would earn that reduction only if the estimates were
/// independent, and they are not: every one of them is measured against the
/// same north tick, through the same filter state, at the same AGC gain, so
/// whatever those contribute lands on all of them together. Taking the
/// reduction anyway makes the zero-crossing method claim 1.26 degrees where
/// it is 1.95 degrees out.
///
/// The reference contributes whole for the same reason, more obviously: an
/// error in the tick displaces every estimate equally.
pub(super) fn bearing_uncertainty_deg(
    phase_variance: f32,
    count: usize,
    north_tick: &NorthTick,
) -> Option<f32> {
    if count == 0 || !phase_variance.is_finite() || phase_variance < 0.0 {
        return None;
    }
    let reference_variance = north_tick
        .phase_variance
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0);
    let variance = phase_variance + reference_variance;
    variance.sqrt().to_degrees().into()
}

/// Detailed confidence metrics for bearing measurements
#[derive(Debug, Clone, Copy, Default)]
pub struct ConfidenceMetrics {
    /// Signal-to-noise ratio in dB
    pub snr_db: f32,
    /// Phase stability across the buffer (0-1, higher is more stable)
    pub coherence: f32,
    /// Normalized signal power (0-1)
    pub signal_strength: f32,
    /// Estimated one-sigma uncertainty of this bearing, in degrees, or None
    /// where it cannot be estimated.
    ///
    /// Two things move a bearing off the truth and this is meant to carry
    /// both. The Doppler phase scatters from rotation to rotation, and
    /// averaging N rotations divides that scatter by the root of N. The north
    /// reference has a timing error of its own, and that one does not average
    /// away: the bearing is measured against the tick, so the tick's error is
    /// the bearing's error, whole.
    ///
    /// This is precision, not accuracy, and cannot be otherwise: it is built
    /// from how much the estimates disagree, so a displacement they all share
    /// is invisible to it. The zero-crossing method's error is almost entirely
    /// such a displacement, growing to six degrees of offset under noise, and
    /// no spread-derived figure will ever see it.
    ///
    /// Unlike `coherence` it is still a claim that can be checked, and
    /// `tests/bearing_uncertainty_test.rs` checks it: it must grow as the
    /// signal degrades, and it must not read lower than the scatter it
    /// describes.
    pub bearing_uncertainty_deg: Option<f32>,
}

impl ConfidenceMetrics {
    /// Calculate combined confidence score from metrics using provided weights
    pub fn combined_score(&self, weights: &ConfidenceWeights) -> f32 {
        let snr_score = (self.snr_db / weights.snr_normalization_db).clamp(0.0, 1.0);
        weights.snr_weight * snr_score
            + weights.coherence_weight * self.coherence
            + weights.signal_strength_weight * self.signal_strength
    }
}

/// Bearing measurement result
///
/// Contains a bearing angle measurement with smoothing and confidence metrics.
#[derive(Debug, Clone, Copy)]
pub struct BearingMeasurement {
    /// Smoothed bearing angle in degrees (0-360)
    pub bearing_degrees: f32,
    /// Raw (unsmoothed) bearing angle in degrees (0-360)
    pub raw_bearing: f32,
    /// Combined confidence metric (0-1 range, higher is better)
    pub confidence: f32,
    /// Detailed confidence metrics breakdown
    pub metrics: ConfidenceMetrics,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn test_phase_to_bearing() {
        assert!((phase_to_bearing(0.0) - 0.0).abs() < 0.01);
        assert!((phase_to_bearing(PI / 2.0) - 90.0).abs() < 0.01);
        assert!((phase_to_bearing(PI) - 180.0).abs() < 0.01);
        assert!((phase_to_bearing(3.0 * PI / 2.0) - 270.0).abs() < 0.01);
    }

    #[test]
    fn test_confidence_metrics_default() {
        let weights = ConfidenceWeights::default();
        let metrics = ConfidenceMetrics::default();
        assert_eq!(metrics.snr_db, 0.0);
        assert_eq!(metrics.coherence, 0.0);
        assert_eq!(metrics.signal_strength, 0.0);
        assert_eq!(metrics.combined_score(&weights), 0.0);
    }

    #[test]
    fn test_confidence_metrics_combined_score() {
        let weights = ConfidenceWeights::default();
        let metrics = ConfidenceMetrics {
            snr_db: 20.0,
            coherence: 1.0,
            signal_strength: 1.0,
            bearing_uncertainty_deg: None,
        };
        let score = metrics.combined_score(&weights);
        assert!((score - 1.0).abs() < 0.001);

        let metrics = ConfidenceMetrics {
            snr_db: 10.0,
            coherence: 0.5,
            signal_strength: 0.5,
            bearing_uncertainty_deg: None,
        };
        let score = metrics.combined_score(&weights);
        let expected = weights.snr_weight * 0.5
            + weights.coherence_weight * 0.5
            + weights.signal_strength_weight * 0.5;
        assert!((score - expected).abs() < 0.001);
    }

    #[test]
    fn test_confidence_metrics_snr_clamping() {
        let weights = ConfidenceWeights::default();
        let metrics = ConfidenceMetrics {
            snr_db: 40.0,
            coherence: 0.0,
            signal_strength: 0.0,
            bearing_uncertainty_deg: None,
        };
        let score = metrics.combined_score(&weights);
        assert!((score - 0.4).abs() < 0.001);

        let metrics = ConfidenceMetrics {
            snr_db: -10.0,
            coherence: 0.0,
            signal_strength: 0.0,
            bearing_uncertainty_deg: None,
        };
        let score = metrics.combined_score(&weights);
        assert_eq!(score, 0.0);
    }
}
