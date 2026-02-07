//! Configuration for the Rotary Club RDF system.
//!
//! ## Channel Assignment
//!
//! To change which audio channel is used for what, modify the `doppler_channel`
//! and `north_tick_channel` fields in `AudioConfig::default()`:
//!
//! ```ignore
//! doppler_channel: ChannelRole::Left,      // or ChannelRole::Right
//! north_tick_channel: ChannelRole::Right,  // or ChannelRole::Left
//! ```

use std::fmt;
use std::str::FromStr;

/// Rotation frequency specification
///
/// Can be specified as either a frequency in Hz or a period in microseconds.
/// Useful when the exact period is known but the frequency is a repeating decimal.
///
/// # Parsing formats
/// - `1602.564` - frequency in Hz (no suffix)
/// - `1602.564hz` or `1602.564Hz` - frequency in Hz (explicit)
/// - `624us` or `624μs` - period in microseconds
///
/// # Example
/// ```
/// use rotaryclub::config::RotationFrequency;
///
/// // 624 μs period = 1602.5641025641... Hz
/// let freq: RotationFrequency = "624us".parse().unwrap();
/// assert!((freq.as_hz() - 1602.564).abs() < 0.001);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct RotationFrequency(f32);

impl RotationFrequency {
    /// Create from frequency in Hz
    pub fn from_hz(hz: f32) -> Self {
        Self(hz)
    }

    /// Create from period in microseconds
    pub fn from_interval_us(us: f32) -> Self {
        Self(1_000_000.0 / us)
    }

    /// Get frequency in Hz
    pub fn as_hz(&self) -> f32 {
        self.0
    }

    /// Get period in microseconds
    #[allow(dead_code)]
    pub fn as_interval_us(&self) -> f32 {
        1_000_000.0 / self.0
    }
}

impl Default for RotationFrequency {
    fn default() -> Self {
        // 624 μs period = 1602.5641025641... Hz
        Self::from_interval_us(624.0)
    }
}

impl fmt::Display for RotationFrequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}hz", self.0)
    }
}

impl FromStr for RotationFrequency {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();

        // Check for microsecond suffix (us or μs)
        if let Some(num) = s.strip_suffix("us").or_else(|| s.strip_suffix("μs")) {
            let us: f32 = num
                .trim()
                .parse()
                .map_err(|_| format!("invalid interval: {}", s))?;
            if us <= 0.0 {
                return Err("interval must be positive".to_string());
            }
            return Ok(Self::from_interval_us(us));
        }

        // Check for Hz suffix (case insensitive)
        let num = s
            .strip_suffix("hz")
            .or_else(|| s.strip_suffix("Hz"))
            .or_else(|| s.strip_suffix("HZ"))
            .unwrap_or(s);

        let hz: f32 = num
            .trim()
            .parse()
            .map_err(|_| format!("invalid frequency: {}", s))?;
        if hz <= 0.0 {
            return Err("frequency must be positive".to_string());
        }
        Ok(Self::from_hz(hz))
    }
}

/// Channel assignment for stereo input
///
/// Specifies which physical audio channel carries which signal type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRole {
    /// Left channel (index 0 in interleaved stereo)
    Left,
    /// Right channel (index 1 in interleaved stereo)
    Right,
}

/// System-wide RDF configuration
///
/// Contains all configuration parameters for the pseudo-Doppler radio direction
/// finding system. Use `RdfConfig::default()` for sensible defaults.
///
/// # Example
/// ```
/// use rotaryclub::config::RdfConfig;
///
/// let mut config = RdfConfig::default();
/// // Customize as needed
/// config.bearing.output_rate_hz = 20.0;
/// ```
#[derive(Debug, Clone, Default)]
pub struct RdfConfig {
    /// Audio input configuration
    pub audio: AudioConfig,
    /// Doppler tone processing configuration
    pub doppler: DopplerConfig,
    /// North reference pulse detection configuration
    pub north_tick: NorthTickConfig,
    /// Bearing output configuration
    pub bearing: BearingConfig,
    /// Automatic gain control configuration
    pub agc: AgcConfig,
}

/// Audio input configuration
///
/// Configures sample rate, buffer size, and channel assignment.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Audio sample rate in Hz (typically 48000)
    pub sample_rate: u32,
    /// Processing buffer size in samples
    pub buffer_size: usize,
    /// Number of audio channels (must be 2 for stereo)
    pub channels: u16,
    /// Which channel contains the FM radio audio (Doppler tone)
    pub doppler_channel: ChannelRole,
    /// Which channel contains the north tick reference
    pub north_tick_channel: ChannelRole,
}

/// Bearing calculation method
///
/// Both methods achieve sub-degree accuracy (<1°) on clean signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BearingMethod {
    /// Zero-crossing detection with sub-sample interpolation (lower CPU)
    ZeroCrossing,
    /// I/Q correlation demodulation (more noise-robust)
    Correlation,
}

/// Doppler tone processing configuration
///
/// Controls how the Doppler-shifted carrier tone is extracted and processed
/// to determine bearing angles.
#[derive(Debug, Clone)]
pub struct DopplerConfig {
    /// Initial/nominal rotation frequency in Hz (actual frequency tracked by DPLL)
    pub expected_freq: f32,
    /// Bandpass filter lower cutoff in Hz
    pub bandpass_low: f32,
    /// Bandpass filter upper cutoff in Hz
    pub bandpass_high: f32,
    /// Filter order (unused with FIR bandpass, kept for compatibility)
    #[allow(dead_code)]
    pub filter_order: usize,
    /// Zero-crossing detection hysteresis to reject noise
    pub zero_cross_hysteresis: f32,
    /// Bearing calculation method to use
    pub method: BearingMethod,
    /// North tick timing adjustment in samples.
    /// The north tick detector triggers at the first sample above threshold,
    /// but the actual threshold crossing occurs in the previous inter-sample
    /// interval. This adjustment (typically 0.5) compensates for that offset.
    pub north_tick_timing_adjustment: f32,
}

/// North reference tracking mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum NorthTrackingMode {
    /// Simple exponential smoothing of rotation period
    Simple,
    /// Digital phase-locked loop (DPLL) for robust tracking
    Dpll,
}

/// Digital Phase-Locked Loop (DPLL) configuration
#[derive(Debug, Clone)]
pub struct DpllConfig {
    /// Initial rotation frequency estimate in Hz
    pub initial_frequency_hz: f32,
    /// DPLL natural frequency in Hz (bandwidth)
    pub natural_frequency_hz: f32,
    /// DPLL damping ratio (0.707 for critical damping)
    pub damping_ratio: f32,
    /// Minimum allowed rotation frequency in Hz
    pub frequency_min_hz: f32,
    /// Maximum allowed rotation frequency in Hz
    pub frequency_max_hz: f32,
}

impl Default for DpllConfig {
    fn default() -> Self {
        Self {
            initial_frequency_hz: 1_000_000.0 / 624.0, // 624 μs period
            natural_frequency_hz: 10.0,
            damping_ratio: 0.707,
            frequency_min_hz: 1400.0,
            frequency_max_hz: 1800.0,
        }
    }
}

/// North reference pulse detection configuration
///
/// Controls detection of the north timing reference pulses used to
/// establish bearing zero reference.
#[derive(Debug, Clone)]
pub struct NorthTickConfig {
    /// Tracking mode (DPLL recommended)
    pub mode: NorthTrackingMode,
    /// Highpass filter cutoff in Hz to isolate pulse transients
    pub highpass_cutoff: f32,
    /// Number of taps for FIR highpass filter (must be odd, default 63)
    pub fir_highpass_taps: usize,
    /// Peak detection threshold (0-1 range)
    pub threshold: f32,
    /// Expected pulse amplitude for timing compensation (0-1 range)
    /// Used to compute threshold crossing offset on FIR impulse response
    pub expected_pulse_amplitude: f32,
    /// Minimum interval between pulses in milliseconds
    pub min_interval_ms: f32,
    /// DPLL configuration (only used when mode is Dpll)
    pub dpll: DpllConfig,
}

/// Bearing output configuration
#[derive(Debug, Clone)]
pub struct BearingConfig {
    /// Moving average window size for smoothing
    pub smoothing_window: usize,
    /// Bearing output rate in Hz
    pub output_rate_hz: f32,
    /// North reference offset for calibration (degrees added to all bearings)
    pub north_offset_degrees: f32,
    /// Timeout in seconds before warning about missing north tick (live capture only)
    pub north_tick_warning_timeout_secs: f32,
}

/// Automatic gain control configuration
///
/// Normalizes signal amplitude variations for consistent processing.
#[derive(Debug, Clone)]
pub struct AgcConfig {
    /// Target RMS signal level (0-1 range, typically 0.5)
    pub target_rms: f32,
    /// Attack time constant in milliseconds (how fast gain increases)
    pub attack_time_ms: f32,
    /// Release time constant in milliseconds (how fast gain decreases)
    pub release_time_ms: f32,
    /// Measurement window for RMS calculation in milliseconds
    pub measurement_window_ms: f32,
}

impl AudioConfig {
    /// Extract doppler and north tick channels from stereo samples
    /// Returns (doppler_samples, north_tick_samples)
    pub fn split_channels(&self, stereo_samples: &[(f32, f32)]) -> (Vec<f32>, Vec<f32>) {
        let mut doppler = Vec::with_capacity(stereo_samples.len());
        let mut north_tick = Vec::with_capacity(stereo_samples.len());

        for &(left, right) in stereo_samples {
            let doppler_sample = match self.doppler_channel {
                ChannelRole::Left => left,
                ChannelRole::Right => right,
            };
            let north_tick_sample = match self.north_tick_channel {
                ChannelRole::Left => left,
                ChannelRole::Right => right,
            };
            doppler.push(doppler_sample);
            north_tick.push(north_tick_sample);
        }

        (doppler, north_tick)
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            buffer_size: 1024,
            channels: 2,
            // Default: Left channel = FM audio/Doppler, Right channel = North tick
            doppler_channel: ChannelRole::Left,
            north_tick_channel: ChannelRole::Right,
        }
    }
}

impl Default for DopplerConfig {
    fn default() -> Self {
        Self {
            expected_freq: 1_000_000.0 / 624.0, // 624 μs period
            bandpass_low: 1350.0,
            bandpass_high: 1850.0,
            filter_order: 4,
            zero_cross_hysteresis: 0.01,
            method: BearingMethod::Correlation,
            north_tick_timing_adjustment: 0.5,
        }
    }
}

impl Default for NorthTickConfig {
    fn default() -> Self {
        Self {
            mode: NorthTrackingMode::Dpll,
            highpass_cutoff: 5000.0,
            fir_highpass_taps: 63,
            threshold: 0.15,
            expected_pulse_amplitude: 0.8,
            min_interval_ms: 0.6,
            dpll: DpllConfig::default(),
        }
    }
}

impl Default for BearingConfig {
    fn default() -> Self {
        Self {
            smoothing_window: 5,
            output_rate_hz: 10.0,
            north_offset_degrees: 0.0,
            north_tick_warning_timeout_secs: 2.0,
        }
    }
}

impl Default for AgcConfig {
    fn default() -> Self {
        Self {
            target_rms: 0.5,
            attack_time_ms: 10.0,
            release_time_ms: 100.0,
            measurement_window_ms: 10.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rotation_frequency_from_hz() {
        let freq: RotationFrequency = "1602.564".parse().unwrap();
        assert!((freq.as_hz() - 1602.564).abs() < 0.001);
    }

    #[test]
    fn test_rotation_frequency_from_hz_explicit() {
        let freq: RotationFrequency = "1602.564hz".parse().unwrap();
        assert!((freq.as_hz() - 1602.564).abs() < 0.001);

        let freq: RotationFrequency = "1602.564Hz".parse().unwrap();
        assert!((freq.as_hz() - 1602.564).abs() < 0.001);
    }

    #[test]
    fn test_rotation_frequency_from_interval_us() {
        // 624 μs = 1602.5641025641... Hz
        let freq: RotationFrequency = "624us".parse().unwrap();
        assert!((freq.as_hz() - 1602.5641).abs() < 0.001);
    }

    #[test]
    fn test_rotation_frequency_from_interval_unicode() {
        let freq: RotationFrequency = "624μs".parse().unwrap();
        assert!((freq.as_hz() - 1602.5641).abs() < 0.001);
    }

    #[test]
    fn test_rotation_frequency_invalid() {
        assert!("abc".parse::<RotationFrequency>().is_err());
        assert!("-100hz".parse::<RotationFrequency>().is_err());
        assert!("0us".parse::<RotationFrequency>().is_err());
    }
}
