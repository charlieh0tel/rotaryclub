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
                "Bearing: {:>6.1}° (raw: {:>6.1}°) conf: {:.2} [SNR: {:>5.1} dB, +/-{}, str: {:.2}{}, R: {:.2}, peak: {:.3}, lock: {}, pev: {}]",
                output.bearing,
                output.raw,
                output.confidence,
                output.snr_db,
                output
                    .bearing_uncertainty_deg
                    .map_or("?".to_string(), |u| format!("{u:.1}deg")),
                output.signal_strength,
                if output.signal_present { "" } else { " NO SIG" },
                output.resultant_length,
                output.tone_peak,
                lock,
                pev
            )
        } else {
            // The marker goes on the bearings that are not backed by a
            // signal, not on the ones that are: in service almost everything
            // has a signal behind it, and a word repeated on every good line
            // stops being read long before the one line that matters arrives.
            format!(
                "Bearing: {:>6.1}° (raw: {:>6.1}°) confidence: {:.2}{}",
                output.bearing,
                output.raw,
                output.confidence,
                if output.signal_present {
                    ""
                } else {
                    "  [no signal]"
                }
            )
        }
    }
}
