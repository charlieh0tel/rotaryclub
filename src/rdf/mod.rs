pub mod bearing;
mod bearing_calculator_base;
mod bearing_correlation;
mod bearing_zero_crossing;
pub mod north_ref;
mod north_ref_common;
mod north_ref_dpll;
mod north_ref_simple;

pub use bearing::{BearingCalculator, BearingMeasurement, ConfidenceMetrics};
pub use bearing_correlation::CorrelationBearingCalculator;
pub use bearing_zero_crossing::ZeroCrossingBearingCalculator;
pub use north_ref::{NorthReferenceTracker, NorthTick, NorthTracker};
