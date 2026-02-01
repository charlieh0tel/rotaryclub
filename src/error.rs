use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum RdfError {
    #[error("Audio device error: {0}")]
    AudioDevice(String),

    #[error("Audio stream error: {0}")]
    AudioStream(String),

    #[error("Filter design failed: {0}")]
    FilterDesign(String),

    #[error("No north tick detected for {0:.1}ms")]
    NoNorthTick(f32),

    #[error("Insufficient data: need {needed} samples, have {available}")]
    InsufficientData { needed: usize, available: usize },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Phase calculation failed: {0}")]
    PhaseError(String),
}

pub type Result<T> = std::result::Result<T, RdfError>;
