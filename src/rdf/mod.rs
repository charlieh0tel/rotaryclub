pub mod bearing;
pub mod north_ref;
mod north_ref_dpll;
mod north_ref_simple;

pub use bearing::BearingCalculator;
pub use north_ref::{NorthReferenceTracker, NorthTick};
