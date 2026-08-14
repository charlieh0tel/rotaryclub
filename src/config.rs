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
            if !us.is_finite() || us <= 0.0 {
                return Err("interval must be a positive, finite number".to_string());
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
        if !hz.is_finite() || hz <= 0.0 {
            return Err("frequency must be a positive, finite number".to_string());
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

impl RdfConfig {
    /// Retune every rotation-coupled parameter to the given rotation.
    ///
    /// The defaults (DPLL tracking band, Doppler bandpass, detector dead
    /// time) are all sized for the nominal rotor; setting only the expected
    /// frequency used to leave them fixed, so an out-of-band `--rotation`
    /// silently clamped to the old band or never locked at all. Scaling
    /// everything proportionally preserves the validated ratios at any
    /// rotation rate.
    ///
    /// The coupled parameters are scaled from the *default* config rather
    /// than their current values, so the result depends only on `rotation`
    /// and repeated calls do not compound.
    pub fn apply_rotation(&mut self, rotation: RotationFrequency) {
        let hz = rotation.as_hz();
        let scale = hz / RotationFrequency::default().as_hz();
        let defaults = Self::default();

        self.doppler.expected_freq = hz;
        self.north_tick.dpll.initial_frequency_hz = hz;
        self.north_tick.dpll.frequency_min_hz = defaults.north_tick.dpll.frequency_min_hz * scale;
        self.north_tick.dpll.frequency_max_hz = defaults.north_tick.dpll.frequency_max_hz * scale;
        self.doppler.bandpass_low = defaults.doppler.bandpass_low * scale;
        self.doppler.bandpass_high = defaults.doppler.bandpass_high * scale;
        // Dead time scales with the rotation period so the
        // min_interval-vs-frequency_max validation holds at any rate.
        self.north_tick.min_interval_ms = defaults.north_tick.min_interval_ms / scale;
    }
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
    /// Number of FIR bandpass filter taps (must be odd, default: 127)
    pub bandpass_taps: usize,
    /// Bandpass filter transition bandwidth in Hz (default: 100.0)
    pub bandpass_transition_hz: f32,
    /// Zero-crossing detection hysteresis to reject noise
    pub zero_cross_hysteresis: f32,
    /// Bearing calculation method to use
    pub method: BearingMethod,
    /// North tick timing adjustment in microseconds.
    /// Fine adjustment applied to north tick timing in bearing calculation
    /// after tracker delay compensation.
    ///
    /// Expressed in time rather than samples so a calibration made against
    /// live audio at one sample rate still means the same thing for a
    /// recording at another. Half a sample is 6 degrees of bearing at 48 kHz
    /// and 3 at 96 kHz, so the units are not a formality.
    ///
    /// Positive values shift the effective tick time later; negative values
    /// shift it earlier.
    ///
    /// The default was 0.5 samples for a long time, which is exactly half a
    /// sample.
    /// It was not compensating anything in the bearing calculation: it was
    /// cancelling an artifact of the test signal generator, which lit the
    /// first sample at or after each rotation boundary and so ran half a
    /// sample late. Every bearing-accuracy test used that generator, so the
    /// trim looked necessary and its removal looked harmful. Against a pulse
    /// placed at the true epoch it costs 6 degrees of bearing at 48 kHz.
    pub north_tick_timing_adjustment_us: f32,
}

/// North reference tracking mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum NorthTrackingMode {
    /// Simple exponential smoothing of rotation period
    Simple,
    /// Digital phase-locked loop (DPLL) for robust tracking
    Dpll,
}

/// Sub-sample estimator for the reference pulse arrival time
///
/// The two centroids are the same first moment differing only in how the
/// weight is spread across the pulse. Amplitude weighting gives the skirts
/// more say, which suits a narrow pulse whose neighbours carry the
/// interpolation; energy weighting concentrates on the peak, which suits a
/// wider pulse with skirts worth down-weighting and is measurably better at
/// the shipped highpass cutoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum NorthPulseEstimator {
    /// Index of the largest filtered sample, quantized to whole samples
    HardLimiter,
    /// First moment weighted by sample value
    AmplitudeCentroid,
    /// First moment weighted by sample value squared
    EnergyCentroid,
}

impl NorthPulseEstimator {
    /// Exponent the sample value is raised to when weighting the first
    /// moment. Zero marks an estimator that takes no moment at all.
    pub(crate) fn weight_exponent(self) -> i32 {
        match self {
            NorthPulseEstimator::HardLimiter => 0,
            NorthPulseEstimator::AmplitudeCentroid => 1,
            NorthPulseEstimator::EnergyCentroid => 2,
        }
    }

    /// Whether the negative part of the filtered pulse is discarded before
    /// weighting.
    ///
    /// This follows from the exponent rather than being a choice. The
    /// highpassed pulse is bipolar, so a linear moment over it is ill-posed:
    /// negative weights, and a denominator that can pass through zero. A
    /// squared one is not -- squaring already makes sign irrelevant -- and
    /// discarding the negative lobes there throws away weight that sits
    /// symmetrically about the peak. Measured, that costs the energy centroid
    /// 0.69 degrees against 0.44 at the shipped cutoff.
    pub(crate) fn clips_negative(self) -> bool {
        self.weight_exponent() % 2 == 1
    }

    /// Half-width of the window the moment is taken over, in microseconds.
    ///
    /// Wider suits a weighting that spreads across the pulse and narrower one
    /// that concentrates on the peak, so it belongs to the estimator rather
    /// than being one number for all of them. Measured optima on the captures
    /// in `data/` at 48 kHz: two samples for the amplitude centroid, four for
    /// the energy centroid.
    pub(crate) fn window_half_width_us(self) -> f32 {
        match self {
            NorthPulseEstimator::HardLimiter => 0.0,
            NorthPulseEstimator::AmplitudeCentroid => 42.0,
            NorthPulseEstimator::EnergyCentroid => 85.0,
        }
    }
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
            natural_frequency_hz: 2.0,
            damping_ratio: 0.707,
            frequency_min_hz: 1400.0,
            frequency_max_hz: 1650.0,
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
    /// Sub-sample estimator for the pulse arrival time
    pub estimator: NorthPulseEstimator,
    /// Input gain in dB (0.0 = unity, applied before filtering)
    pub gain_db: f32,
    /// Highpass filter cutoff in Hz to isolate pulse transients.
    ///
    /// The filter rejects audio bleeding into the north channel, but it also
    /// discards pulse energy that carries timing information, so the cutoff
    /// trades one against the other.
    ///
    /// Every measurement available favours the low end. With the shipped
    /// estimator, `north_hpf_sweep` puts per-tick timing at 0.44 degrees here
    /// against 0.52 at 5 kHz, and detection is unaffected at every cutoff
    /// tried including none at all. Raising it also costs elsewhere: at
    /// 5 kHz the simple tracker's timing jitter doubles, the coasting budget
    /// shortens because it is earned from how well the rate is known, and the
    /// `low_snr_dc` false-positive rate rises from 0.048 to 0.051, past the
    /// limit the performance gate holds it to.
    ///
    /// That last one is worth stating plainly, because it is the opposite of
    /// the intuition: filtering higher does not reject more junk. In the one
    /// scenario built to test junk, it admits more of it.
    ///
    /// What no capture in `data/` can price is the reason to filter high at
    /// all -- a receiver bleeding audio into the north channel. None of them
    /// does. Raise the cutoff if one turns up.
    pub highpass_cutoff: f32,
    /// Length of the FIR highpass in microseconds.
    ///
    /// Expressed in time rather than taps so the same configuration produces
    /// the same filter at any sample rate. A fixed tap count would not: 63
    /// taps is 1.31 ms at 48 kHz and 0.66 ms at 96, which is a different
    /// filter with a different transition width, not the one asked for. The
    /// tap count is derived from this and forced odd to keep linear phase.
    pub fir_highpass_length_us: f32,
    /// Highpass filter transition bandwidth in Hz (default: 500.0)
    pub highpass_transition_hz: f32,
    /// Peak detection threshold (0-1 range)
    ///
    /// Raising this was measured and rejected. The amplitude at which
    /// detection collapses tracks the threshold, at about 1.6 times it, so
    /// 0.15 detects down to a pulse amplitude of 0.25 and 0.25 only to 0.42.
    /// Against the 0.8 expected that is a factor of 3.2 on receiver level
    /// against 1.9. What the higher threshold buys is detection under channel
    /// noise, and it buys nothing until 0.2 RMS and little until 0.3, by
    /// which point the false positive rate is 0.18 either way. See DESIGN.md.
    pub threshold: f32,
    /// Expected pulse amplitude in the north channel, before `gain_db`.
    ///
    /// Used to compute where the filtered pulse crosses the threshold, which
    /// sets the peak search window. The gain is applied to the buffer first,
    /// so what the threshold actually meets is this times the gain, and the
    /// two are validated together.
    pub expected_pulse_amplitude: f32,
    /// Minimum interval between pulses in milliseconds. Must be shorter
    /// than the period at dpll.frequency_max_hz (0.6 ms supports up to
    /// ~1666 Hz).
    ///
    /// At the default rotation rate this covers 96% of a rotation, which is
    /// why the timing gate can only act on detections arriving late. Trading
    /// some of it for gate reach was measured and rejected: at a noise RMS of
    /// 0.20 against a 0.8 pulse, shortening to 0.45 ms drops detection from
    /// 0.84 to 0.25 and raises false positives from 0.06 to 0.65. The gate
    /// does not rescue it -- it rejects what disagrees with the tracked
    /// rotation, and noise triggers arriving where a pulse is due are
    /// indistinguishable from the pulse. `test_dead_time_rejects_noise_
    /// triggers` pins the shipped behaviour.
    pub min_interval_ms: f32,
    /// How long to keep emitting ticks from the tracked rotation after
    /// pulses stop arriving, in milliseconds. Past this the tracker declares
    /// loss of lock and reacquires.
    pub max_coast_ms: f32,
    /// Width of the detection-timing gate, in standard deviations of the
    /// tracked phase error. Detections further than this from where the
    /// rotation says the pulse should be are rejected. Only applied once the
    /// tracker is locked.
    pub gate_sigma: f32,
    /// DPLL configuration (only used when mode is Dpll)
    pub dpll: DpllConfig,
    /// Weights for lock quality calculation
    pub lock_quality_weights: LockQualityWeights,
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
    /// How the estimated bearing uncertainty becomes a confidence score
    pub confidence: ConfidenceConfig,
}

/// How a bearing's estimated uncertainty becomes a confidence score.
///
/// Confidence used to be a weighted sum of SNR, coherence and signal
/// strength. Two of those three barely moved -- coherence changed by 0.0016
/// across a sweep that took the bearing from a sixth of a degree of error to
/// forty -- so they acted as a constant offset and floored the score near
/// 0.59 however bad the bearing was. It is now a function of the estimated
/// bearing uncertainty, which is the one figure that both moves and makes a
/// checkable claim.
#[derive(Debug, Clone, Copy)]
pub struct ConfidenceConfig {
    /// Bearing uncertainty, in degrees, at which confidence reads one half.
    ///
    /// The default is half a sample of north timing at 48 kHz, which is six
    /// degrees of bearing. That makes a confidence of 0.5 mean something
    /// stateable: the bearing is as good as the reference timing can be
    /// resolved to.
    pub half_confidence_deg: f32,
    /// Signal strength below which no bearing is reported at all.
    ///
    /// This is a validity gate rather than a quality term. Signal strength
    /// answers whether there was anything to measure, not whether the answer
    /// is good: the zero-crossing detector goes on finding crossings as the
    /// signal degrades, so it stays near unity long after the bearing has
    /// stopped being usable.
    ///
    /// The floor is low on purpose, because the two methods do not measure
    /// the same thing by this name. Zero crossing reports the fraction of
    /// expected crossings it found, which is liveness. Correlation reports
    /// the fraction of power that correlated with its reference, which also
    /// falls when the reference is merely wrong -- a rotation rate mismatch
    /// drops it well below a half while the channel is perfectly alive, and
    /// suppressing the bearing there would hide the mismatch rather than
    /// report it. Only genuine absence should reach this.
    pub min_signal_strength: f32,
}

impl Default for ConfidenceConfig {
    fn default() -> Self {
        Self {
            half_confidence_deg: 6.0,
            min_signal_strength: 0.05,
        }
    }
}

/// Weights for DPLL lock quality calculation
///
/// Lock quality combines phase stability and frequency stability scores.
#[derive(Debug, Clone, Copy)]
pub struct LockQualityWeights {
    /// Weight for phase error component (default: 0.7)
    pub phase_weight: f32,
    /// Weight for frequency stability component (default: 0.3)
    pub frequency_weight: f32,
}

impl Default for LockQualityWeights {
    fn default() -> Self {
        Self {
            phase_weight: 0.7,
            frequency_weight: 0.3,
        }
    }
}

/// Automatic gain control configuration
///
/// Normalizes signal amplitude variations for consistent processing.
#[derive(Debug, Clone)]
pub struct AgcConfig {
    /// Target RMS signal level (0-1 range, default 0.3)
    pub target_rms: f32,
    /// Attack time constant in milliseconds (how fast gain decreases for loud signals)
    pub attack_time_ms: f32,
    /// Release time constant in milliseconds (how fast gain recovers for quiet signals)
    pub release_time_ms: f32,
    /// Measurement window for RMS calculation in milliseconds
    pub measurement_window_ms: f32,
    /// Minimum gain (prevents excessive attenuation, default: 0.1 = -20dB)
    pub min_gain: f32,
    /// Maximum gain (prevents excessive amplification, default: 5.0 = +14 dB)
    pub max_gain: f32,
}

impl AudioConfig {
    fn split_channels_internal<I>(&self, stereo_samples: I, capacity: usize) -> (Vec<f32>, Vec<f32>)
    where
        I: IntoIterator<Item = (f32, f32)>,
    {
        let mut doppler = Vec::with_capacity(capacity);
        let mut north_tick = Vec::with_capacity(capacity);

        for (left, right) in stereo_samples {
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

    /// Extract doppler and north tick channels from stereo samples
    /// Returns (doppler_samples, north_tick_samples)
    pub fn split_channels(&self, stereo_samples: &[(f32, f32)]) -> (Vec<f32>, Vec<f32>) {
        self.split_channels_internal(stereo_samples.iter().copied(), stereo_samples.len())
    }

    /// Extract doppler and north tick channels from any stereo pair iterator
    pub fn split_channels_iter<I>(&self, stereo_samples: I, capacity: usize) -> (Vec<f32>, Vec<f32>)
    where
        I: IntoIterator<Item = (f32, f32)>,
    {
        self.split_channels_internal(stereo_samples, capacity)
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
            bandpass_taps: 127,
            bandpass_transition_hz: 100.0,
            zero_cross_hysteresis: 0.01,
            method: BearingMethod::Correlation,
            north_tick_timing_adjustment_us: 0.0,
        }
    }
}

impl Default for NorthTickConfig {
    fn default() -> Self {
        Self {
            mode: NorthTrackingMode::Dpll,
            estimator: NorthPulseEstimator::EnergyCentroid,
            gain_db: 0.0,
            highpass_cutoff: 1000.0,
            fir_highpass_length_us: 1312.5,
            highpass_transition_hz: 500.0,
            threshold: 0.15,
            expected_pulse_amplitude: 0.8,
            min_interval_ms: 0.6,
            max_coast_ms: 1000.0,
            gate_sigma: 3.0,
            dpll: DpllConfig::default(),
            lock_quality_weights: LockQualityWeights::default(),
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
            confidence: ConfidenceConfig::default(),
        }
    }
}

impl Default for AgcConfig {
    fn default() -> Self {
        Self {
            target_rms: 0.3,
            attack_time_ms: 10.0,
            release_time_ms: 100.0,
            measurement_window_ms: 10.0,
            min_gain: 0.1,
            max_gain: 5.0,
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

    #[test]
    fn test_apply_rotation_scales_coupled_parameters() {
        // --rotation used to change only the expected/initial frequency,
        // leaving the DPLL band and bandpass at their nominal-rotor values,
        // so e.g. 1700 Hz was accepted then silently clamped to the band.
        for hz in [800.0_f32, 1700.0, 2400.0] {
            let mut config = RdfConfig::default();
            config.apply_rotation(RotationFrequency::from_hz(hz));

            assert_eq!(config.doppler.expected_freq, hz);
            assert_eq!(config.north_tick.dpll.initial_frequency_hz, hz);
            assert!(
                config.north_tick.dpll.frequency_min_hz < hz
                    && hz < config.north_tick.dpll.frequency_max_hz,
                "{} Hz outside derived band {}-{}",
                hz,
                config.north_tick.dpll.frequency_min_hz,
                config.north_tick.dpll.frequency_max_hz
            );
            assert!(
                config.doppler.bandpass_low < hz && hz < config.doppler.bandpass_high,
                "{} Hz outside derived bandpass",
                hz
            );
            // The derived config must satisfy the tracker's dead-time
            // validation at any rate.
            crate::rdf::NorthReferenceTracker::new(&config.north_tick, 48_000.0)
                .expect("derived config should construct a tracker");

            // Applying the same rotation again must be a no-op (scaling is
            // from defaults, not compounding from current values).
            let mut twice = RdfConfig::default();
            twice.apply_rotation(RotationFrequency::from_hz(hz));
            twice.apply_rotation(RotationFrequency::from_hz(hz));
            assert_eq!(
                twice.north_tick.dpll.frequency_max_hz, config.north_tick.dpll.frequency_max_hz,
                "repeated apply_rotation compounded"
            );
            assert_eq!(twice.doppler.bandpass_high, config.doppler.bandpass_high);
            assert_eq!(
                twice.north_tick.min_interval_ms,
                config.north_tick.min_interval_ms
            );
        }
    }

    #[test]
    fn test_rotation_frequency_rejects_non_finite() {
        // NaN/inf pass a `<= 0` check and used to propagate through the DPLL
        // into the output (including bare NaN in JSON).
        assert!("NaN".parse::<RotationFrequency>().is_err());
        assert!("nanhz".parse::<RotationFrequency>().is_err());
        assert!("inf".parse::<RotationFrequency>().is_err());
        assert!("infus".parse::<RotationFrequency>().is_err());
    }
}
