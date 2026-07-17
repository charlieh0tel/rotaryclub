pub mod audio;
pub mod config;
pub mod constants;
pub mod error;
pub mod processing;
pub mod rdf;
pub mod signal_processing;
pub mod stats;
pub mod wav;

#[cfg(feature = "simulation")]
pub mod simulation;

pub use config::RdfConfig;
pub use error::{RdfError, Result};
pub use processing::RdfProcessor;
pub use stats::{CircularStats, CircularSummary};
pub use wav::{WavStreamWriter, save_wav};
