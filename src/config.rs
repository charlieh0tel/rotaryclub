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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRole {
    /// Left channel (index 0 in interleaved stereo)
    Left,
    /// Right channel (index 1 in interleaved stereo)
    Right,
}

/// System-wide configuration
#[derive(Debug, Clone, Default)]
pub struct RdfConfig {
    pub audio: AudioConfig,
    pub doppler: DopplerConfig,
    pub north_tick: NorthTickConfig,
    pub bearing: BearingConfig,
}

#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub buffer_size: usize,
    pub channels: u16,
    /// Which channel contains the FM radio audio (Doppler tone)
    pub doppler_channel: ChannelRole,
    /// Which channel contains the north tick reference
    pub north_tick_channel: ChannelRole,
}

#[derive(Debug, Clone)]
pub struct DopplerConfig {
    pub expected_freq: f32,
    pub bandpass_low: f32,
    pub bandpass_high: f32,
    pub filter_order: usize,
    pub zero_cross_hysteresis: f32,
}

#[derive(Debug, Clone)]
pub struct NorthTickConfig {
    pub highpass_cutoff: f32,
    pub filter_order: usize,
    pub threshold: f32,
    pub min_interval_ms: f32,
}

#[derive(Debug, Clone)]
pub struct BearingConfig {
    pub smoothing_window: usize,
    pub max_phase_jump: f32,
    pub output_rate_hz: f32,
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
        }
    }
}

impl Default for NorthTickConfig {
    fn default() -> Self {
        Self {
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
            max_phase_jump: 45.0,
            output_rate_hz: 10.0,
        }
    }
}
