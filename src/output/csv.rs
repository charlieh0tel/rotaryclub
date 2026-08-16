use super::{BearingOutput, Formatter, iso8601_timestamp};

/// A float as a CSV cell: the number, or empty where it does not exist --
/// the same convention the optional columns already use. An unquoted NaN
/// token is at best parser-dependent.
fn csv_num(value: f32, precision: usize) -> String {
    if value.is_finite() {
        format!("{value:.precision$}")
    } else {
        String::new()
    }
}

pub struct CsvFormatter;

impl Formatter for CsvFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        let lock = output.lock_quality.map_or(String::new(), |q| csv_num(q, 2));
        let pev = output
            .phase_error_variance
            .map_or(String::new(), |v| csv_num(v, 4));
        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            iso8601_timestamp(),
            csv_num(output.bearing, 1),
            csv_num(output.raw, 1),
            csv_num(output.confidence, 2),
            csv_num(output.snr_db, 1),
            output
                .bearing_uncertainty_deg
                .map(|u| csv_num(u, 2))
                .unwrap_or_default(),
            csv_num(output.signal_strength, 2),
            output.signal_present,
            csv_num(output.resultant_length, 3),
            csv_num(output.tone_peak, 4),
            lock,
            pev
        )
    }

    fn header(&self) -> Option<&'static str> {
        Some(
            "ts,bearing,raw,confidence,snr_db,bearing_uncertainty_deg,signal_strength,signal_present,resultant_length,tone_peak,lock_quality,phase_error_variance",
        )
    }
}
