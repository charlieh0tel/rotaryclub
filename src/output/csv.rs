use super::{BearingOutput, Formatter, iso8601_timestamp};

pub struct CsvFormatter;

impl Formatter for CsvFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        let lock = output
            .lock_quality
            .map_or(String::new(), |q| format!("{:.2}", q));
        let pev = output
            .phase_error_variance
            .map_or(String::new(), |v| format!("{:.4}", v));
        format!(
            "{},{:.1},{:.1},{:.2},{:.1},{},{:.2},{},{:.3},{:.4},{},{}",
            iso8601_timestamp(),
            output.bearing,
            output.raw,
            output.confidence,
            output.snr_db,
            output
                .bearing_uncertainty_deg
                .map(|u| format!("{u:.2}"))
                .unwrap_or_default(),
            output.signal_strength,
            output.signal_present,
            output.resultant_length,
            output.tone_peak,
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
