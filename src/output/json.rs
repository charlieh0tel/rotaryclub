use super::{BearingOutput, Formatter, iso8601_timestamp};

pub struct JsonFormatter;

impl Formatter for JsonFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        let lock = output
            .lock_quality
            .map_or("null".to_string(), |q| format!("{:.2}", q));
        let pev = output
            .phase_error_variance
            .map_or("null".to_string(), |v| format!("{:.4}", v));
        format!(
            r#"{{"ts":"{}","bearing":{:.1},"raw":{:.1},"confidence":{:.2},"snr_db":{:.1},"coherence":{:.2},"signal_strength":{:.2},"lock_quality":{},"phase_error_variance":{}}}"#,
            iso8601_timestamp(),
            output.bearing,
            output.raw,
            output.confidence,
            output.snr_db,
            output.coherence,
            output.signal_strength,
            lock,
            pev
        )
    }
}
