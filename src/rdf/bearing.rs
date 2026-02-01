/// Bearing measurement result
///
/// Contains a bearing angle measurement with smoothing and confidence metrics.
#[derive(Debug, Clone, Copy)]
pub struct BearingMeasurement {
    /// Smoothed bearing angle in degrees (0-360)
    pub bearing_degrees: f32,
    /// Raw (unsmoothed) bearing angle in degrees (0-360)
    pub raw_bearing: f32,
    /// Confidence metric (0-1 range, higher is better)
    pub confidence: f32,
    /// Sample timestamp
    #[allow(dead_code)]
    pub timestamp_samples: usize,
}
