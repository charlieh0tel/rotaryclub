pub use crate::config::ConfidenceConfig;
pub use crate::constants::MIN_POWER_THRESHOLD;
use crate::error::{RdfError, Result};

use super::NorthTick;

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

/// Reject a confidence configuration that cannot produce a meaningful score.
///
/// Every one of these fails silently rather than loudly. A NaN half point
/// slips past a `<= EPSILON` guard, because no comparison with NaN is ever
/// true, and yields a NaN confidence that the JSON formatter emits as a bare
/// `NaN` -- which is not valid JSON, so it breaks the consumer rather than the
/// producer. A zero or negative half point scores every bearing zero, so a
/// perfect fix reads as worthless and the GUI needle goes black. A NaN
/// `min_signal_strength` makes the reported signal verdict always false, since
/// no comparison with NaN is ever true, so the output would claim there was
/// never a signal.
pub(super) fn validate_confidence_config(config: &ConfidenceConfig) -> Result<()> {
    if !config.half_confidence_deg.is_finite() || config.half_confidence_deg <= 0.0 {
        return Err(RdfError::Config(format!(
            "bearing.confidence.half_confidence_deg is {}, must be a finite number greater \
             than 0; it is the uncertainty at which confidence reads one half",
            config.half_confidence_deg
        )));
    }
    if let Some(min) = config.min_signal_strength
        && (!min.is_finite() || !(0.0..=1.0).contains(&min))
    {
        return Err(RdfError::Config(format!(
            "bearing.confidence.min_signal_strength is {min}, must be a finite number within \
             [0, 1]; it is the signal strength at or above which a bearing is reported as \
             having a signal behind it"
        )));
    }
    Ok(())
}

/// One-sigma bearing uncertainty, in degrees, from the signal-to-noise ratio
/// and the reference the bearing was measured against.
///
/// For a real tone in additive noise at a signal-to-noise power ratio r,
/// averaged over n independent looks, the phase estimate scatters by
/// 1 / sqrt(r n). That is the whole of the doppler term, and everything
/// hangs on n being counted right: a look is one decorrelation interval of
/// the in-band noise, fs / 2B samples for a filter of noise-equivalent
/// bandwidth B, because a real bandpass passes +-f and its noise
/// decorrelates over the two-sided width.
///
/// An earlier version counted fs / B intervals and compensated by dropping
/// the factor of two from the complex-exponential form of the variance --
/// two errors of root two that cancelled at the formula and did not cancel
/// in what the formula meant. Counted properly against the filter's
/// measured bandwidth, the variance is 1/(r n) with no adjustment, and the
/// stated figure matches known-truth scatter without either fudge.
///
/// This used to be built from the spread of the per-rotation phase estimates
/// instead, and that understated the bearing scatter everywhere -- measured
/// against a standard deviation the ratio ran 0.53 to 0.91 and centred on
/// 0.69, against 1.08 for this. The reason is that the spread cannot see the
/// error that matters. The doppler passband is 500 Hz, so in-band noise
/// decorrelates over about 96 samples, and a buffer holds only a handful of
/// independent realisations: the interference is close to one coherent
/// perturbation that shifts every window together, invisible to a spread
/// taken within the buffer, and varying between buffers so that it lands in
/// the bearing scatter. It is not a bias -- the perturbation is shared but not
/// constant, so the mean error stays small, which is what made it look like a
/// bias question and not one.
///
/// An unknown reference gives no answer at all rather than a confident one.
/// `ConfidenceMetrics::score` maps that to zero confidence: the absence of a
/// claim, not a claim of a bad bearing.
pub(super) fn bearing_uncertainty_deg(
    snr_db: f32,
    independent_looks: f32,
    north_tick: &NorthTick,
) -> Option<f32> {
    let reference = north_tick
        .reference_variance
        .filter(|v| v.is_finite() && *v >= 0.0)?;
    if !snr_db.is_finite() || !independent_looks.is_finite() {
        return None;
    }
    let snr = 10.0f32.powf(snr_db / 10.0);
    if snr <= f32::EPSILON {
        return None;
    }
    let looks = independent_looks.max(1.0);
    // The small-error variance, valid while the estimate stays near the
    // true phase.
    let linear_variance = 1.0 / (snr * looks);
    // Threshold correction. Below a few tens of snr x looks the estimator
    // leaves the linear regime: the phase estimate is sometimes captured by
    // noise outright, and those outliers land anywhere on the circle. The
    // small-error formula cannot see them -- once the look accounting above
    // was made honest (two inflations that previously papered over this
    // regime are gone), it claimed 22 degrees against 36 of actual scatter
    // at the worst recorded channel condition.
    //
    // The outlier probability is an exponential in snr x looks, calibrated
    // at two measured points rather than derived: 0.11 at snr x looks 4.6
    // and 0.08 at 6.7, from the known-truth scatter at the two conditions
    // where the linear formula underclaims. A Rife-Boorstyn union bound was
    // tried first and overshoots threefold here, because the correlation is
    // not a discrete bin search. The form dies fast enough to leave the
    // moderate conditions untouched: 0.009 by snr x looks of 20, under 1e-3
    // by 35, so everything the linear formula already handled is unchanged.
    let uniform_variance = (std::f32::consts::PI * std::f32::consts::PI) / 3.0;
    let outlier_probability = (0.21 * (-(snr * looks) / 6.6).exp()).min(0.5);
    let doppler_variance =
        (1.0 - outlier_probability) * linear_variance + outlier_probability * uniform_variance;
    Some((doppler_variance + reference).sqrt().to_degrees())
}

/// Detailed confidence metrics for bearing measurements
#[derive(Debug, Clone, Copy, Default)]
pub struct ConfidenceMetrics {
    /// Signal-to-noise ratio in dB
    pub snr_db: f32,
    /// Normalized signal power (0-1)
    pub signal_strength: f32,
    /// Largest positive sample of the filtered Doppler signal in this buffer,
    /// in full-scale units.
    ///
    /// One of two quantities carried for the KN5R "C" sentence, kept here in
    /// the units they are measured in rather than in the units that sentence
    /// wants. KR6DD's engine defines its third field as the running maximum
    /// of the FIR output over a batch section, that output scaled to plus or
    /// minus one, sent as thousandths; the thousandths belong to the wire
    /// format, so the scaling happens in the formatter and this stays a
    /// fraction of full scale.
    pub tone_peak: f32,
    /// Mean resultant length of the Doppler phase, from 0 to 1.
    ///
    /// The other KN5R quantity, and the one the "C" sentence calls magnitude:
    /// 1 when every look agrees on the angle, 0 when they are scattered. It
    /// is a coherence rather than a level, which is what made normalised
    /// signal strength the wrong thing to send in its place -- a strong tone
    /// pointing inconsistently reads high on that and low on this.
    ///
    /// Derived from the signal-to-noise ratio rather than from per-crossing
    /// vectors, which this pipeline no longer keeps: phase scattered with
    /// standard deviation sigma has resultant length exp(-sigma^2 / 2), and a
    /// single look at a signal-to-noise power ratio r scatters by 1/sqrt(r).
    ///
    /// Reported for every output, not just that one. It says something the
    /// other metrics do not -- whether the looks agree with each other, as
    /// against how strong they were or how uncertain the answer is.
    pub resultant_length: f32,
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
    /// is invisible to it. A mistimed north tick is exactly such a
    /// displacement -- it moves every estimate equally and leaves the spread
    /// untouched -- which is why the reference contributes its own term rather
    /// than being inferred from the scatter.
    ///
    /// Unlike `coherence` it is still a claim that can be checked, and
    /// `tests/bearing_uncertainty_test.rs` checks it: it must grow as the
    /// signal degrades, and it must not read lower than the scatter it
    /// describes.
    pub bearing_uncertainty_deg: Option<f32>,
}

/// Mean resultant length of phase scattered by a given signal-to-noise ratio.
///
/// See `ConfidenceMetrics::resultant_length`. Zero for a ratio that is not
/// finite or not positive, since no coherence can be claimed from it.
pub(super) fn resultant_length_from_snr(snr_db: f32) -> f32 {
    if !snr_db.is_finite() {
        return 0.0;
    }
    let snr = 10.0f32.powf(snr_db / 10.0);
    if snr <= f32::EPSILON {
        return 0.0;
    }
    (-1.0 / (2.0 * snr)).exp()
}

impl ConfidenceMetrics {
    /// Confidence in this bearing, from 0 to 1.
    ///
    /// A ratio rather than a subtraction, so the score keeps resolution over
    /// the whole range the uncertainty covers instead of bottoming out. At
    /// the configured half-confidence uncertainty it reads 0.5; at twice that
    /// 0.2, at ten times it 0.01. Measured across a noise sweep it runs from
    /// 0.97 on a clean signal to 0.01 on a bearing forty degrees out, where
    /// the weighted sum it replaced ran from 0.9999 to 0.76.
    ///
    /// An unknown uncertainty scores zero. It is not a claim of a bad
    /// bearing, it is the absence of a claim, and treating that as confident
    /// is how a confidence score becomes dangerous.
    pub fn score(&self, config: &ConfidenceConfig) -> f32 {
        let Some(sigma) = self
            .bearing_uncertainty_deg
            .filter(|s| s.is_finite() && *s >= 0.0)
        else {
            return 0.0;
        };
        if config.half_confidence_deg <= f32::EPSILON {
            return 0.0;
        }
        let ratio = sigma / config.half_confidence_deg;
        1.0 / (1.0 + ratio * ratio)
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
    /// Whether the signal strength reached the threshold for this method.
    ///
    /// A reported verdict, not a filter: the measurement is produced either
    /// way and a consumer is free to disagree with it. It exists because the
    /// quantity that decides it is not the same quantity in both methods and
    /// sits on a different scale in each, so a consumer applying one rule to
    /// `signal_strength` across both would be wrong in one of them.
    pub signal_present: bool,
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

    fn metrics_with(uncertainty: Option<f32>) -> ConfidenceMetrics {
        ConfidenceMetrics {
            tone_peak: 0.0,
            resultant_length: 0.0,
            snr_db: 20.0,
            signal_strength: 1.0,
            bearing_uncertainty_deg: uncertainty,
        }
    }

    #[test]
    fn test_unknown_uncertainty_scores_zero() {
        // Not a claim that the bearing is bad, but the absence of a claim.
        // The other metrics are set high on purpose: none of them may rescue
        // a measurement whose uncertainty could not be estimated.
        let config = ConfidenceConfig::default();
        assert_eq!(metrics_with(None).score(&config), 0.0);
        assert_eq!(ConfidenceMetrics::default().score(&config), 0.0);
    }

    #[test]
    fn test_half_confidence_lands_at_the_configured_uncertainty() {
        let config = ConfidenceConfig::default();
        let score = metrics_with(Some(config.half_confidence_deg)).score(&config);
        assert!(
            (score - 0.5).abs() < 0.001,
            "Confidence should read one half at the configured uncertainty, got {score}"
        );
    }

    #[test]
    fn test_confidence_keeps_resolution_across_the_range() {
        let config = ConfidenceConfig::default();
        let half = config.half_confidence_deg;

        // The failure this replaced: a score that floors near 0.59 however
        // bad the bearing gets, because two of its three terms never moved.
        let ruined = metrics_with(Some(half * 10.0)).score(&config);
        assert!(
            ruined < 0.02,
            "A bearing ten times worse than the half point should score near \
             zero, got {ruined}"
        );

        let mut previous = 1.0f32;
        for sigma in [0.1f32, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0] {
            let score = metrics_with(Some(sigma)).score(&config);
            assert!(
                score < previous,
                "Confidence must fall as uncertainty grows: {sigma} degrees \
                 scored {score} against {previous} for the step before it"
            );
            previous = score;
        }
    }
}
