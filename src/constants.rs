//! Numeric constants for signal processing stability
//!
//! These constants define thresholds and epsilon values used throughout
//! the signal processing pipeline to ensure numerical stability.

/// Epsilon for preventing division by zero in interpolation calculations.
/// Used when computing sub-sample positions (e.g., zero-crossing interpolation).
pub const INTERPOLATION_EPSILON: f32 = 1e-10;

/// Epsilon for frequency/period comparisons to avoid division by near-zero values.
/// Used in DPLL and bearing calculations when normalizing by frequency.
pub const FREQUENCY_EPSILON: f32 = 1e-10;

/// Minimum signal power threshold for confidence calculations.
/// Signals with power below this are considered too weak for reliable measurement.
pub const MIN_POWER_THRESHOLD: f32 = 1e-10;

/// Minimum RMS threshold for AGC operation.
/// Signals with RMS below this are considered silent; AGC holds gain constant.
pub const MIN_RMS_THRESHOLD: f32 = 1e-6;
