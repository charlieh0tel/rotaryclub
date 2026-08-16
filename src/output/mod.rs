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
    /// Whether the pipeline judged a signal to be present, by the threshold
    /// for the method in use. Reported rather than enforced: everything else
    /// in this struct is filled in either way.
    pub signal_present: bool,
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

#[cfg(test)]
mod non_finite_tests {
    use super::*;

    fn bad_output() -> BearingOutput {
        BearingOutput {
            bearing: f32::NAN,
            raw: f32::INFINITY,
            confidence: f32::NAN,
            snr_db: f32::NEG_INFINITY,
            bearing_uncertainty_deg: Some(f32::NAN),
            signal_strength: f32::NAN,
            signal_present: false,
            resultant_length: f32::NAN,
            tone_peak: f32::NAN,
            lock_quality: Some(f32::NAN),
            phase_error_variance: Some(f32::NAN),
        }
    }

    /// The wire stays valid whatever upstream delivers. The audio sources
    /// zero non-finite samples, but the formatters must not depend on that:
    /// a bare NaN token is not JSON, and the KN5R saturating cast renders a
    /// nonexistent bearing as a clean-looking 0000 due north.
    #[test]
    fn json_stays_valid_on_non_finite_values() {
        let line = JsonFormatter.format(&bad_output());
        assert!(!line.contains("NaN") && !line.contains("inf"), "{line}");
        // Every non-finite numeric must have become null.
        assert!(line.contains(r#""bearing":null"#), "{line}");
        assert!(line.contains(r#""lock_quality":null"#), "{line}");
    }

    #[test]
    fn csv_leaves_non_finite_cells_empty() {
        let line = CsvFormatter.format(&bad_output());
        assert!(!line.contains("NaN") && !line.contains("inf"), "{line}");
        // ts, then bearing/raw/confidence/snr empty: ",,,,"
        assert!(line.contains(",,,,"), "{line}");
    }

    #[test]
    fn kn5r_refuses_a_nonexistent_bearing() {
        assert_eq!(Kn5rFormatter.format(&bad_output()), "");
    }

    #[test]
    fn kn5r_sentence_is_fixed_width_for_finite_input() {
        let mut ok = bad_output();
        ok.bearing = 123.4;
        ok.resultant_length = 0.5;
        ok.tone_peak = 0.25;
        let line = Kn5rFormatter.format(&ok);
        assert_eq!(line.len(), 26, "{line}");
        assert!(line.starts_with("C1234"));
    }
}
