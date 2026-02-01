/// Bearing measurement result
#[derive(Debug, Clone, Copy)]
pub struct BearingMeasurement {
    pub bearing_degrees: f32,
    pub raw_bearing: f32,
    pub confidence: f32,
    #[allow(dead_code)]
    pub timestamp_samples: usize,
}
