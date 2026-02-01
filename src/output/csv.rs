use super::{BearingOutput, Formatter, iso8601_timestamp};

pub struct CsvFormatter;

impl Formatter for CsvFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        format!(
            "{},{:.1},{:.1},{:.2},{:.1},{:.2},{:.2}",
            iso8601_timestamp(),
            output.bearing,
            output.raw,
            output.confidence,
            output.snr_db,
            output.coherence,
            output.signal_strength
        )
    }

    fn header(&self) -> Option<&'static str> {
        Some("ts,bearing,raw,confidence,snr_db,coherence,signal_strength")
    }
}
