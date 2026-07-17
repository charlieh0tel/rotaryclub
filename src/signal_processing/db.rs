//! Decibel conversions.
//!
//! Amplitude quantities (gains, sample ratios) use 20 dB per decade; power
//! quantities (SNR, energy ratios) use 10 dB per decade.

/// Convert decibels to a linear amplitude factor.
pub fn db_to_amplitude(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Convert decibels to a linear power factor.
pub fn db_to_power(db: f32) -> f32 {
    10.0_f32.powf(db / 10.0)
}

/// Convert a linear amplitude ratio to decibels.
pub fn amplitude_to_db(ratio: f32) -> f32 {
    20.0 * ratio.log10()
}

/// Convert a linear power ratio to decibels.
pub fn power_to_db(ratio: f32) -> f32 {
    10.0 * ratio.log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_conversions_round_trip() {
        assert!((db_to_amplitude(20.0) - 10.0).abs() < 1e-4);
        assert!((db_to_power(20.0) - 100.0).abs() < 1e-2);
        assert!((amplitude_to_db(10.0) - 20.0).abs() < 1e-4);
        assert!((power_to_db(100.0) - 20.0).abs() < 1e-4);
        assert!((amplitude_to_db(db_to_amplitude(-3.5)) + 3.5).abs() < 1e-4);
        assert!((power_to_db(db_to_power(-3.5)) + 3.5).abs() < 1e-4);
        assert_eq!(db_to_amplitude(0.0), 1.0);
        assert_eq!(db_to_power(0.0), 1.0);
    }
}
