//! KN5R-RDF compatible "C" format output.
//!
//! Fixed-width 26-character format:
//! - Position 0: 'C' literal
//! - Positions 1-4: bearing angle × 10 (0-3599), zero-padded
//! - Positions 5-7: angle vector average magnitude (0-999), zero-padded
//! - Positions 8-10: FIR filtered Doppler tone peak value (0-999), zero-padded
//! - Positions 11-25: Unix timestamp in milliseconds, zero-padded
//!
//! Example: `C3469960084001663117493011` = 346.9°, magnitude 960, tone 084.
//! Note the timestamp is zero-padded to 15 digits, so the whole sentence is
//! 26 characters; an earlier version of this example showed 13 and so was 24,
//! which would put every field offset wrong for anyone parsing by position.
//!
//! Reference: <https://github.com/kn5r/kn5r-rdf> `docs/data-format.md` for
//! the layout, and KR6DD's `RPiDDFengine` for what the fields contain. The
//! two are not the same source and the second is the one that decides: the
//! format document names the fields, the engine defines them.

use super::{BearingOutput, Formatter, timestamp_millis};

pub struct Kn5rFormatter;

impl Formatter for Kn5rFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        let angle = (output.bearing * 10.0).round() as u16 % 3600;
        // Both fields are carried by the tracker in their own units and only
        // scaled here. The pipeline has no reason to know this format wants
        // thousandths, and every other output wants the unscaled value.
        let magnitude = (output.resultant_length.clamp(0.0, 1.0) * 999.0).round() as u16;
        let tone_peak = (output.tone_peak.clamp(0.0, 1.0) * 1000.0)
            .round()
            .min(999.0) as u16;
        let ts = timestamp_millis();

        format!("C{angle:04}{magnitude:03}{tone_peak:03}{ts:015}")
    }
}
