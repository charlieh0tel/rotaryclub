mod csv;
mod json;
mod kn5r;
mod text;

use chrono::Utc;

pub use self::csv::CsvFormatter;
pub use self::json::JsonFormatter;
pub use self::kn5r::Kn5rFormatter;
pub use self::text::TextFormatter;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Kn5r,
    Json,
    Csv,
}

pub struct BearingOutput {
    pub bearing: f32,
    pub raw: f32,
    pub confidence: f32,
    pub snr_db: f32,
    /// One-sigma bearing uncertainty in degrees, or None where it could not
    /// be estimated.
    pub bearing_uncertainty_deg: Option<f32>,
    pub signal_strength: f32,
    /// Largest positive sample of the filtered Doppler signal, in full-scale
    /// units. Unscaled: the KN5R sentence wants thousandths and does that
    /// conversion itself.
    pub tone_peak: f32,
    /// Mean resultant length of the Doppler phase, 0 to 1. Whether the looks
    /// agreed with each other, which is neither how strong they were nor how
    /// uncertain the answer is. Unscaled, as above.
    pub resultant_length: f32,
    pub lock_quality: Option<f32>,
    pub phase_error_variance: Option<f32>,
}

pub trait Formatter: Send {
    fn format(&self, output: &BearingOutput) -> String;

    fn header(&self) -> Option<&'static str> {
        None
    }
}

pub fn create_formatter(format: OutputFormat, verbose: bool) -> Box<dyn Formatter> {
    match format {
        OutputFormat::Text => Box::new(TextFormatter::new(verbose)),
        OutputFormat::Kn5r => Box::new(Kn5rFormatter),
        OutputFormat::Json => Box::new(JsonFormatter),
        OutputFormat::Csv => Box::new(CsvFormatter),
    }
}

pub fn iso8601_timestamp() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn timestamp_millis() -> u64 {
    Utc::now().timestamp_millis() as u64
}
