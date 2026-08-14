//! KN5R-RDF compatible "C" format output.
//!
//! Fixed-width 26-character format:
//! - Position 0: 'C' literal
//! - Positions 1-4: bearing angle × 10 (0-3599), zero-padded
//! - Positions 5-7: magnitude (0-999), zero-padded
//! - Positions 8-10: tone peak (0-999), zero-padded
//! - Positions 11-25: Unix timestamp in milliseconds, zero-padded
//!
//! Example: `C3469960084001663117493011` = 346.9°, magnitude 960, tone 084.
//! Note the timestamp is zero-padded to 15 digits, so the whole sentence is
//! 26 characters; an earlier version of this example showed 13 and so was 24,
//! which would put every field offset wrong for anyone parsing by position.
//!
//! Reference: <https://github.com/kn5r/kn5r-rdf> (see docs/data-format.md)

use super::{BearingOutput, Formatter, timestamp_millis};

/// SNR, in dB, taken as a full-scale tone peak.
const TONE_PEAK_FULL_SCALE_DB: f32 = 40.0;

pub struct Kn5rFormatter;

impl Formatter for Kn5rFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        let angle = (output.bearing * 10.0).round() as u16 % 3600;
        let magnitude = (output.signal_strength.clamp(0.0, 1.0) * 999.0).round() as u16;
        // What the receiving end expects in "tone peak" is not documented
        // anywhere available here: the reference points at a data-format.md
        // in the kn5r-rdf project, which is not vendored, and nothing in this
        // repo describes it. Feeding it from SNR is a guess, though a better
        // one than the coherence it replaced, which sat near full scale
        // whatever the signal did. Verify against the KN5R side before
        // relying on it.
        let tone_peak =
            ((output.snr_db / TONE_PEAK_FULL_SCALE_DB).clamp(0.0, 1.0) * 999.0).round() as u16;
        let ts = timestamp_millis();

        format!("C{angle:04}{magnitude:03}{tone_peak:03}{ts:015}")
    }
}
