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
    pub coherence: f32,
    pub signal_strength: f32,
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
