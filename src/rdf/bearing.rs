use super::NorthTick;

pub const MIN_POWER_THRESHOLD: f32 = 1e-10;
const SNR_NORMALIZATION_DB: f32 = 20.0;
const SNR_WEIGHT: f32 = 0.4;
const COHERENCE_WEIGHT: f32 = 0.4;
const SIGNAL_STRENGTH_WEIGHT: f32 = 0.2;

pub trait BearingCalculator {
    fn process_buffer(
        &mut self,
        doppler_buffer: &[f32],
        north_tick: &NorthTick,
    ) -> Option<BearingMeasurement>;
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

/// Detailed confidence metrics for bearing measurements
#[derive(Debug, Clone, Copy, Default)]
pub struct ConfidenceMetrics {
    /// Signal-to-noise ratio in dB
    pub snr_db: f32,
    /// Phase stability across the buffer (0-1, higher is more stable)
    pub coherence: f32,
    /// Normalized signal power (0-1)
    pub signal_strength: f32,
}

impl ConfidenceMetrics {
    /// Calculate combined confidence score from metrics
    pub fn combined_score(&self) -> f32 {
        let snr_score = (self.snr_db / SNR_NORMALIZATION_DB).clamp(0.0, 1.0);
        SNR_WEIGHT * snr_score
            + COHERENCE_WEIGHT * self.coherence
            + SIGNAL_STRENGTH_WEIGHT * self.signal_strength
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
        let metrics = ConfidenceMetrics::default();
        assert_eq!(metrics.snr_db, 0.0);
        assert_eq!(metrics.coherence, 0.0);
        assert_eq!(metrics.signal_strength, 0.0);
        assert_eq!(metrics.combined_score(), 0.0);
    }

    #[test]
    fn test_confidence_metrics_combined_score() {
        let metrics = ConfidenceMetrics {
            snr_db: 20.0,
            coherence: 1.0,
            signal_strength: 1.0,
        };
        let score = metrics.combined_score();
        assert!((score - 1.0).abs() < 0.001);

        let metrics = ConfidenceMetrics {
            snr_db: 10.0,
            coherence: 0.5,
            signal_strength: 0.5,
        };
        let score = metrics.combined_score();
        let expected = SNR_WEIGHT * 0.5 + COHERENCE_WEIGHT * 0.5 + SIGNAL_STRENGTH_WEIGHT * 0.5;
        assert!((score - expected).abs() < 0.001);
    }

    #[test]
    fn test_confidence_metrics_snr_clamping() {
        let metrics = ConfidenceMetrics {
            snr_db: 40.0,
            coherence: 0.0,
            signal_strength: 0.0,
        };
        let score = metrics.combined_score();
        assert!((score - 0.4).abs() < 0.001);

        let metrics = ConfidenceMetrics {
            snr_db: -10.0,
            coherence: 0.0,
            signal_strength: 0.0,
        };
        let score = metrics.combined_score();
        assert_eq!(score, 0.0);
    }
}
