use super::{BearingOutput, Formatter, iso8601_timestamp};

pub struct JsonFormatter;

impl Formatter for JsonFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        format!(
            r#"{{"ts":"{}","bearing":{:.1},"raw":{:.1},"confidence":{:.2},"snr_db":{:.1},"coherence":{:.2},"signal_strength":{:.2}}}"#,
            iso8601_timestamp(),
            output.bearing,
            output.raw,
            output.confidence,
            output.snr_db,
            output.coherence,
            output.signal_strength
        )
    }
}
