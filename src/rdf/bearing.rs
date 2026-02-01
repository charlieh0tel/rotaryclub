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
        let snr_score = (self.snr_db / 20.0).clamp(0.0, 1.0);
        0.4 * snr_score + 0.4 * self.coherence + 0.2 * self.signal_strength
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
    /// Sample timestamp
    #[allow(dead_code)]
    pub timestamp_samples: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let expected = 0.4 * 0.5 + 0.4 * 0.5 + 0.2 * 0.5;
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
