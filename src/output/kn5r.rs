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

/// Mean resultant length of the Doppler phase, from the signal-to-noise ratio.
///
/// KR6DD's engine builds the magnitude field as the length of the vector sum
/// of the per-zero-crossing phase vectors, divided by how many there were: one
/// when they all agree, zero when they are scattered. That is a circular
/// coherence, not a signal level, and it is what a consumer of this field
/// expects to be looking at.
///
/// This pipeline does not keep per-crossing vectors -- the sub-window phase
/// machinery was removed when the uncertainty was re-derived -- but the same
/// quantity follows from the ratio it does keep. For phase scattered with
/// standard deviation sigma the resultant length is exp(-sigma^2 / 2), and a
/// single look at a signal-to-noise power ratio r scatters by 1 / sqrt(r).
fn resultant_length(snr_db: f32) -> f32 {
    if !snr_db.is_finite() {
        return 0.0;
    }
    let snr = 10.0f32.powf(snr_db / 10.0);
    if snr <= f32::EPSILON {
        return 0.0;
    }
    (-1.0 / (2.0 * snr)).exp()
}

impl Formatter for Kn5rFormatter {
    fn format(&self, output: &BearingOutput) -> String {
        let angle = (output.bearing * 10.0).round() as u16 % 3600;
        // Coherence, not signal strength. Sending normalised signal strength
        // here was a guess made without the reference, and it is a different
        // quantity: a strong tone pointing inconsistently would have read
        // near full scale.
        let magnitude = (resultant_length(output.snr_db).clamp(0.0, 1.0) * 999.0).round() as u16;
        // Absolute level, in thousandths of full scale, matching the engine's
        // running maximum of its FIR output. This was fed from the SNR
        // against a notional 40 dB full scale, which is a ratio where the
        // field wants a level.
        let tone_peak = (output.tone_peak.clamp(0.0, 1.0) * 1000.0)
            .round()
            .min(999.0) as u16;
        let ts = timestamp_millis();

        format!("C{angle:04}{magnitude:03}{tone_peak:03}{ts:015}")
    }
}
