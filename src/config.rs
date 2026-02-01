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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BearingMethod {
    /// Simple zero-crossing detection (~7° accuracy, lower CPU)
    ZeroCrossing,
    /// I/Q correlation demodulation (~1-2° accuracy, higher CPU)
    Correlation,
}

/// Doppler tone processing configuration
///
/// Controls how the Doppler-shifted carrier tone is extracted and processed
/// to determine bearing angles.
#[derive(Debug, Clone)]
pub struct DopplerConfig {
    /// Expected antenna rotation frequency in Hz (typically 1602 Hz)
    pub expected_freq: f32,
    /// Bandpass filter lower cutoff in Hz
    pub bandpass_low: f32,
    /// Bandpass filter upper cutoff in Hz
    pub bandpass_high: f32,
    /// IIR filter order (higher = steeper rolloff)
    pub filter_order: usize,
    /// Zero-crossing detection hysteresis to reject noise
    pub zero_cross_hysteresis: f32,
    /// Bearing calculation method to use
    pub method: BearingMethod,
}

/// North reference tracking mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NorthTrackingMode {
    /// Simple exponential smoothing of rotation period
    #[allow(dead_code)]
    Simple,
    /// Digital phase-locked loop (DPLL) for robust tracking
    Dpll,
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
    /// IIR filter order
    pub filter_order: usize,
    /// Peak detection threshold (0-1 range)
    pub threshold: f32,
    /// Minimum interval between pulses in milliseconds
    pub min_interval_ms: f32,
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
            expected_freq: 1602.0,
            bandpass_low: 1500.0,
            bandpass_high: 1700.0,
            filter_order: 4,
            zero_cross_hysteresis: 0.01,
            method: BearingMethod::Correlation,
        }
    }
}

impl Default for NorthTickConfig {
    fn default() -> Self {
        Self {
            mode: NorthTrackingMode::Dpll,
            highpass_cutoff: 5000.0,
            filter_order: 2,
            threshold: 0.15,
            min_interval_ms: 0.6,
        }
    }
}

impl Default for BearingConfig {
    fn default() -> Self {
        Self {
            smoothing_window: 5,
            output_rate_hz: 10.0,
            north_offset_degrees: 0.0,
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
