use super::{BearingOutput, Formatter, iso8601_timestamp};

/// A float as a JSON value: the number, or `null` where the number does not
/// exist. Bare NaN and Infinity are not JSON, and `format!("{:.1}", f32::NAN)`
/// produces exactly that; the source zeroes non-finite samples, but this
/// formatter guarantees wire validity regardless of what upstream does.
fn json_num(value: f32, precision: usize) -> String {
    if value.is_finite() {
        format!("{value:.precision$}")
    } else {
        "null".to_string()
    }
}

pub struct JsonFormatter;

impl Formatter for JsonFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        let lock = output
            .lock_quality
            .map_or("null".to_string(), |q| json_num(q, 2));
        let pev = output
            .phase_error_variance
            .map_or("null".to_string(), |v| json_num(v, 4));
        format!(
            r#"{{"ts":"{}","bearing":{},"raw":{},"confidence":{},"snr_db":{},"bearing_uncertainty_deg":{},"signal_strength":{},"signal_present":{},"resultant_length":{},"tone_peak":{},"lock_quality":{},"phase_error_variance":{}}}"#,
            iso8601_timestamp(),
            json_num(output.bearing, 1),
            json_num(output.raw, 1),
            json_num(output.confidence, 2),
            json_num(output.snr_db, 1),
            output
                .bearing_uncertainty_deg
                .map(|u| json_num(u, 2))
                .unwrap_or_else(|| "null".into()),
            json_num(output.signal_strength, 2),
            output.signal_present,
            json_num(output.resultant_length, 3),
            json_num(output.tone_peak, 4),
            lock,
            pev
        )
    }
}
