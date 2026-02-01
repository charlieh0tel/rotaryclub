//! KN5R-RDF compatible "C" format output.
//!
//! Fixed-width 26-character format:
//! - Position 0: 'C' literal
//! - Positions 1-4: bearing angle × 10 (0-3599), zero-padded
//! - Positions 5-7: magnitude (0-999), zero-padded
//! - Positions 8-10: tone peak (0-999), zero-padded
//! - Positions 11-25: Unix timestamp in milliseconds, zero-padded
//!
//! Example: `C34699600841663117493011` = 346.9°, magnitude 960, tone 084
//!
//! Reference: <https://github.com/kn5r/kn5r-rdf> (see docs/data-format.md)

use super::{BearingOutput, Formatter, timestamp_millis};

pub struct Kn5rFormatter;

impl Formatter for Kn5rFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        let angle = (output.bearing * 10.0).round() as u16 % 3600;
        let magnitude = (output.signal_strength.clamp(0.0, 1.0) * 999.0).round() as u16;
        let tone_peak = (output.coherence.clamp(0.0, 1.0) * 999.0).round() as u16;
        let ts = timestamp_millis();

        format!("C{angle:04}{magnitude:03}{tone_peak:03}{ts:015}")
    }
}
