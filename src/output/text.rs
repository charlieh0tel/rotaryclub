use super::{BearingOutput, Formatter};

pub struct TextFormatter {
    verbose: bool,
}

impl TextFormatter {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }
}

impl Formatter for TextFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        if self.verbose {
            let lock = output
                .lock_quality
                .map_or("-".to_string(), |q| format!("{:.2}", q));
            let pev = output
                .phase_error_variance
                .map_or("-".to_string(), |v| format!("{:.4}", v));
            format!(
                "Bearing: {:>6.1}° (raw: {:>6.1}°) conf: {:.2} [SNR: {:>5.1} dB, coh: {:.2}, str: {:.2}, lock: {}, pev: {}]",
                output.bearing,
                output.raw,
                output.confidence,
                output.snr_db,
                output.coherence,
                output.signal_strength,
                lock,
                pev
            )
        } else {
            format!(
                "Bearing: {:>6.1}° (raw: {:>6.1}°) confidence: {:.2}",
                output.bearing, output.raw, output.confidence
            )
        }
    }
}
