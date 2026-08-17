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
/// wider pulse with skirts worth down-weighting.
///
/// Which is better was measured over twelve noise realisations with common
/// random numbers. On tick timing the amplitude centroid wins at the noise
/// levels the recordings actually sit at -- 0.0022 samples against 0.0037 at
/// a 0.0006 RMS north channel, twelve times out of twelve -- and the two are
/// indistinguishable above that. On bearing, which is what any of this is
/// for, they are indistinguishable everywhere: the difference in scatter is
/// under 0.02 degrees against a scatter of 10.9, and its confidence interval
/// spans zero at every noise level tried.
///
/// On the recordings the two are a tie, and the tie is better evidence than
/// either synthetic result: `sweep_hpf` over 121,073 ticks of
/// `wouxun_..._test1.wav` puts amplitude weighting at 0.704 degrees per tick
/// against 0.688 for energy at this cutoff, a two percent difference. At
/// 5 kHz the ordering reverses and the gap is real, 0.664 against 1.624,
/// which is what the cutoff-dependence above describes.
///
/// So nothing recommends a change, and that is why there has not been one.
/// Both are far inside anything a bearing can see: 0.0037 samples is 0.04
/// degrees, against a bearing scatter of 10.9.
///
/// What did not survive replication is narrower than it once read here: the
/// claim that the two reverse at 0.05 RMS of north noise, which was one
/// synthetic draw sitting outside the spread of the twelve that followed.
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
    /// This follows from the exponent rather than being a choice, and the
    /// reason is measured rather than structural. Not clipping does not mean
    /// weighting by a signed value -- `centroid_offset` takes `|x|` when it
    /// does not clip -- so the hazard usually quoted for a linear moment over
    /// a bipolar signal, negative weights and a denominator through zero,
    /// does not arise here. An earlier version of this comment said it did.
    ///
    /// Measured, unclipped wins for the even moment and loses for the odd
    /// one: the energy centroid reads 0.441 degrees per tick unclipped
    /// against 0.688 clipped with the window held at 4, while the amplitude
    /// centroid reads 0.704 clipped against 1.308 unclipped at its own window
    /// and 0.856 at the best unclipped one.
    ///
    /// What decides it is the width of the pulse, not the size of the
    /// highpass's negative lobes. An earlier version of this comment said the
    /// lobes were suppressed by squaring and not by linear weighting; they
    /// are 3.3 percent of the main lobe under the first and 0.1 percent under
    /// the second, small under both. The evidence, and its limits:
    ///
    /// Computing each estimator's response from the filter kernel alone, an
    /// assumed impulse gets the hard limiter right -- 3.47 degrees predicted
    /// from quantisation, 3.44 to 3.45 measured by two harnesses on both
    /// wouxun captures -- and gets every centroid wrong in magnitude and
    /// order. Model the pulse as a rectangle and jointly fit its width and a
    /// white residual floor to four cells per capture: width comes out 1.5
    /// samples and the floor 0.24 to 0.38 degrees, in line with the 0.28 the
    /// loop-closed measurements bottom out at. The fitted width is effective
    /// rather than measured -- census counts one to two raw samples above
    /// half max, and swapping the rectangle for a triangle of equal rms width
    /// moves cells by up to thirty percent.
    ///
    /// Held out of the fit, the model reproduces the sixteen window-sweep
    /// cells it never saw to 0.08 degrees rms on test1 and 0.18 on test3,
    /// including the non-monotonic energy-window shape (best at 4, worse at
    /// 3 and 5) on both captures, and the full four-way ordering at the
    /// fitted width. Two things it gets wrong: the clipped saturation level
    /// (0.67 to 0.77 predicted, 0.89 to 0.92 measured, and the measured
    /// level is nearly capture-independent, so something real is missing
    /// there) and the unclipped odd moment's mid windows on test3, off by up
    /// to 0.3 degrees.
    ///
    /// So the ordering belongs to the pulse the switcher and the anti-alias
    /// filter deliver, not to the exponent -- as mechanism, supported; as a
    /// complete quantitative account, not achieved. The model puts the
    /// four-way ordering's onset near 1.3 samples of width and inverts it
    /// well below one, which different hardware could reach and nothing here
    /// has measured.
    ///
    /// A consequence worth knowing: clipped, nothing outside the positive
    /// lobe contributes, so the odd moment cannot use a wider window. It
    /// saturates at 0.891 from a half-width of 3 upward, which is the ceiling
    /// the even moment beats by 40 percent.
    pub(crate) fn clips_negative(self) -> bool {
        self.weight_exponent() % 2 == 1
    }

    /// Half-width of the window the moment is taken over, in microseconds.
    ///
    /// Wider suits a weighting that spreads across the pulse and narrower one
    /// that concentrates on the peak, so it belongs to the estimator rather
    /// than being one number for all of them. Both values are at their
    /// measured optimum: sweeping the half-width on the wouxun captures, the
    /// amplitude centroid reads 0.706, 0.704 then 0.891 flat from 3 upward,
    /// and the energy centroid 0.475, 0.546, 0.441, 0.475, 0.470 degrees per
    /// tick from a half-width of 2. So two samples for the amplitude centroid
    /// and four for the energy centroid, at 48 kHz.
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

/// Slow automatic gain control for the north reference channel.
///
/// On by default. Leaving it off is not the absence of a choice: it is the
/// assumption that the receiver delivers roughly `expected_pulse_amplitude`,
/// and that assumption is already false for one of the two radios in `data/`,
/// which arrives at 0.21 against a configured 0.8 and so sits at the edge of
/// the cliff where detection collapses.
///
/// The case for it being a default rather than an option is that the people
/// it helps are the least likely to find it. A quiet receiver does not
/// announce itself; it produces bearings that are quietly worse, with nothing
/// to suggest a gain control would fix them.
///
/// Enabling it leaves the tick count on all three captures exactly unchanged,
/// which is the evidence that it does not disturb a signal that already
/// works. What has the least evidence behind it is the crest-factor bootstrap
/// described in `NorthPulseAgc` -- the part that decides whether an
/// undetected buffer holds pulses or noise. If something odd shows up on a
/// channel carrying heavy interference, suspect that first, and
/// `enabled: false` restores the fixed-gain behaviour.
#[derive(Debug, Clone, Copy)]
pub struct NorthAgcConfig {
    /// Whether to adapt the north channel gain at all. With this off,
    /// `gain_db` alone decides the level, which is how this shipped.
    ///
    /// Applies to DPLL mode only; the simple tracker keeps its fixed gain.
    ///
    /// What makes the adaptation safe is that it learns a level only while
    /// the oscillator is locked, so that past the noise where the loop stops
    /// locking it has no effect at all rather than converging on nonsense.
    /// The simple tracker has no such signal. An equivalent predicate over
    /// the scatter of its detection intervals was built and measured, and it
    /// was not enough: at a north noise of 0.2 RMS the gain still took
    /// detection from 0.913 to 0.785 and false positives from 0.082 to 0.210,
    /// where in DPLL mode the same noise leaves it neutral. Interval scatter
    /// says less about whether a detection was real than a phase-locked
    /// oscillator does, and closing that gap means giving the simple tracker
    /// a loop, which is the thing it exists not to have.
    pub enabled: bool,
    /// How long the gain takes to settle, in seconds.
    ///
    /// Slow on purpose: the reference tick does not change once the hardware
    /// is running, so the only real transient is startup, and a slow loop
    /// cannot pump on the pulse train itself.
    pub time_constant_secs: f32,
    /// Bounds on the adaptive gain, before `gain_db` is applied.
    pub min_gain: f32,
    pub max_gain: f32,
}

impl Default for NorthAgcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            time_constant_secs: 2.0,
            min_gain: 0.1,
            max_gain: 10.0,
        }
    }
}

/// North reference pulse detection configuration
///
/// Controls detection of the north timing reference pulses used to
/// establish bearing zero reference.
/// Detection threshold for a tracker whose gain control holds the pulse at the
/// expected height, as a fraction of that height.
///
/// Higher than the figure below, and it can be, because the cost of a high
/// threshold is level margin and the AGC is what supplies it. Measured over
/// eight noise draws at the shipped pulse amplitude, this reads 0.95
/// detection with no false positives at 0.2 RMS of channel noise against 0.88
/// and 0.06 at the conservative value, and its amplitude cliff barely moves:
/// detection holds near one down to a pulse of 0.15.
///
/// Equal to an absolute 0.25 at the default pulse and filter, which is the
/// value a DPLL-only deployment was already known to want.
pub const THRESHOLD_FRACTION_GAIN_CONTROLLED: f32 = 0.323;

/// Detection threshold for a tracker that takes the level it is given.
///
/// 0.15 of full scale at the default pulse and filter, the absolute threshold
/// that was measured and settled before this became a fraction.
///
/// The simple tracker cannot use the figure above. Its amplitude cliff is
/// steep and unaided, and at 0.323 it fails detection under combined hum,
/// clipping and baseline drift -- 0.37 against a floor of 0.45 -- where the
/// loop passes every disturbance. This is also what a DPLL runs at with its
/// AGC switched off, since without gain control it has the same exposure.
pub const THRESHOLD_FRACTION_UNAIDED: f32 = 0.19361;

#[derive(Debug, Clone)]
pub struct NorthTickConfig {
    /// Tracking mode (DPLL recommended)
    pub mode: NorthTrackingMode,
    /// Slow gain control for this channel, on by default.
    pub agc: NorthAgcConfig,
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
    /// estimator, `sweep_hpf` puts per-tick timing at 0.44 degrees here
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
    /// Peak detection threshold, as a fraction of the pulse height the
    /// detector expects to see.
    ///
    /// Dimensionless, and deliberately so. The detector compares against the
    /// highpassed signal, whose pulse peaks at `expected_pulse_amplitude`
    /// times the filter's peak response -- times `gain_db` as well when the
    /// AGC is off, since the gain is applied to the buffer first. This is a
    /// fraction of that, so the absolute level is derived rather than
    /// configured.
    ///
    /// An absolute threshold could not stay correct across the things it
    /// depends on. It met a signal that scaled with `gain_db` while itself
    /// staying put, so 0.8 expected at -20 dB presented 0.08 to a threshold
    /// of 0.15 and the tracker silently emitted nothing -- a failure that
    /// needed its own validation check to catch. Derived, that check has
    /// nothing left to reject: a fraction below 1 cannot sit above the pulse.
    /// It also tracks the filter, so changing the highpass no longer quietly
    /// changes the detection margin along with it.
    ///
    /// What the number means: detection collapses when the pulse falls to
    /// about 1.6 times the threshold, so the shipped value holds detection
    /// down to 31 percent of the expected pulse.
    ///
    /// It reproduces the 0.15 absolute threshold that was measured and
    /// settled, and it survived being re-measured as a fraction over sixteen
    /// noise draws. The awkward digits are worth keeping. Rounding up to 0.20
    /// looks free in DPLL mode, where the AGC normalises the level and the
    /// amplitude cliff barely moves, but the simple tracker has no gain
    /// control and its cliff is steep: at a pulse of 0.23 its detection falls
    /// from 0.92 to 0.47 across that three percent. Rounding down to 0.1875
    /// buys a little of that margin back and costs a little noise margin,
    /// 0.86 detection at 0.2 RMS against 0.87. Neither is an improvement, and
    /// the round number would imply a precision the knee does not have.
    ///
    /// Raising it further was rejected earlier for a different reason: what a
    /// higher threshold buys is detection under channel noise, and it buys
    /// nothing until 0.2 RMS and little until 0.3. See DESIGN.md.
    ///
    /// `None` takes the default for the tracker in use, which is not the same
    /// number in both -- see `resolved_threshold_fraction`. Set it to pin one
    /// value regardless.
    pub threshold_fraction: Option<f32>,
    /// Expected pulse amplitude in the north channel, before `gain_db`.
    ///
    /// Sets the detection threshold, the peak search window and, with the AGC
    /// running, the level the gain drives the channel to. The gain is applied
    /// to the buffer first, so what the detector meets is this times the gain
    /// times the filter's peak response.
    ///
    /// Load-bearing for detection since the threshold became a fraction of
    /// it: setting it wrong now moves the threshold too, where before it only
    /// moved the search window. With the AGC on that is self-correcting,
    /// because the gain drives the measured pulse to this value; with the AGC
    /// off it is not.
    pub expected_pulse_amplitude: f32,
    /// Minimum interval between pulses in milliseconds. Must be shorter
    /// than the period at dpll.frequency_max_hz (0.6 ms supports up to
    /// ~1666 Hz).
    ///
    /// At the default rotation rate this covers 96% of a rotation, which is
    /// why the timing gate can only act on detections arriving late. Trading
    /// some of it for gate reach was measured and rejected. Re-measured with
    /// `sweep_config`, against a north channel noise of 0.20 RMS: the shipped
    /// 0.6 ms gives 0.32 samples of tick error and 12.1 degrees of bearing
    /// error, where 0.45 ms gives 0.89 and 86.1, and 0.3 ms gives 0.86 and
    /// 75.1. The figures this comment used to quote -- detection falling from
    /// 0.84 to 0.25 -- came from a generator that produced a half DC offset
    /// carrying a seventh of the in-band energy it claimed, so they were
    /// wrong even though the conclusion they supported was right. The gate
    /// does not rescue it -- it rejects what disagrees with the tracked
    /// rotation, and noise triggers arriving where a pulse is due are
    /// indistinguishable from the pulse. `test_dead_time_rejects_noise_
    /// triggers` pins the shipped behaviour.
    ///
    /// Blanking that much of a rotation has a cost the simple tracker used to
    /// pay in full: whichever crossing opened the window masked the pulse
    /// behind it, and at moderate noise that halved its detection rate. It
    /// now takes the largest sample in the dead time rather than the first
    /// crossing, which keeps the blanking and loses the masking.
    ///
    /// That changed which value the simple tracker wants, and the figures
    /// above no longer describe it. Re-measured over eight draws with the
    /// masking gone, nothing separates any of these at 0.05 RMS or below --
    /// 9613 bearings and 9.6 degrees at every value tried -- and at 0.2 RMS
    /// the simple tracker now prefers a shorter dead time monotonically:
    /// 0.30 ms gives it 9429 bearings and 47.4 degrees against 6166 and 88.6
    /// at the shipped 0.6.
    ///
    /// The DPLL is unaffected by that change and still wants 0.6: at the same
    /// 0.2 RMS it reads 0.44 samples of tick error here against 0.85 at
    /// 0.30 ms. So the optimum is mode-dependent at noise levels three
    /// hundred times anything the recordings show, and identical at the
    /// levels they do show. The shipped value is chosen for the tracker that
    /// ships.
    pub min_interval_ms: f32,
    /// How long to keep emitting ticks from the tracked rotation after
    /// pulses stop arriving, in milliseconds. Past this the tracker declares
    /// loss of lock and reacquires.
    pub max_coast_ms: f32,
    /// Width of the detection-timing gate, in standard deviations of the
    /// tracked phase error. Detections further than this from where the
    /// rotation says the pulse should be are rejected. Only applied once the
    /// tracker is locked.
    ///
    /// The shipped 3.0 was a guess for a long time. Measured over twelve noise
    /// realisations across 1.5 to 6.0, it is inert where the hardware lives:
    /// at the 0.0006 RMS the recordings measure, and at 0.01, tick error is
    /// identical to four decimal places across the whole range, bearing
    /// scatter is identical, and every setting from 2.0 up delivers the same
    /// 9615 bearings. The gate simply never fires on a channel that clean.
    ///
    /// It only does anything far above that, and even there it does not reach
    /// the bearing. At 0.05 RMS a tighter gate is worse on both counts at
    /// once, 0.0129 samples against 0.0108 at the shipped value and 640 fewer
    /// bearings, so there is no trade to make. At 0.2 the direction reverses
    /// -- tighter trends better on timing -- but not significantly over twelve
    /// realisations, and a gate of 6.0 is significantly worse. Bearing scatter
    /// shows no significant dependence on this at any noise level tried.
    ///
    /// An earlier note claimed 0.235 samples against 0.321 at 0.2 RMS, a
    /// twenty-seven percent win for a tighter gate. That was one realisation
    /// drawn from a generator whose seeds were correlated; paired over twelve
    /// independent ones the same comparison gives 0.528 against 0.560, which
    /// is noise.
    pub gate_sigma: f32,
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
    /// Signal strength at or above which the bearing is reported as having a
    /// signal behind it.
    ///
    /// `None` resolves to `MIN_SIGNAL_STRENGTH`.
    ///
    /// This decides a reported verdict, not whether anything is emitted. The
    /// bearing, its uncertainty and every metric are reported either way, so
    /// a consumer can apply its own rule or ignore this one. Suppressing
    /// instead was considered and measured: on the worst channel the
    /// recordings show, the bearings a gate would remove still point at
    /// truth -- a resultant of 0.45 about 6 degrees off -- and an operator
    /// watching scatter narrow cannot integrate what was never sent.
    pub min_signal_strength: Option<f32>,
}

/// Signal strength separating hiss from signal.
///
/// Both methods report the same quantity: the fraction of in-band amplitude
/// that projects onto the tick-locked reference, behind an absolute in-band
/// power gate. On squelch-open hiss that reads 0.000 -- the bandpass strips
/// about 96 percent of broadband power, so hiss fails the power gate --
/// against 0.192 at the 5th percentile of the worst channel the recordings
/// show, so a low floor separates them. It is low on purpose for a second
/// reason: the figure also falls when the reference is merely wrong -- a
/// rotation rate mismatch drops it well below a half while the channel is
/// perfectly alive -- and calling that "no signal" would hide the mismatch
/// rather than report it.
///
/// Zero-crossing used to report crossing density instead, with its own much
/// higher floor. Density discriminated only through the leaky 127-tap
/// bandpass; behind the realized 1023-tap filter, hiss narrows into the
/// passband and matches the tone's crossing density (measured 0.94 to 0.97
/// against 0.94 to 1.00, no floor between them), so the method now reports
/// the projection fraction like correlation does.
pub const MIN_SIGNAL_STRENGTH: f32 = 0.05;

impl ConfidenceConfig {
    /// The signal strength separating hiss from signal.
    ///
    /// Both methods report the same quantity, so one default floor serves;
    /// setting the field explicitly overrides it.
    pub fn resolved_min_signal_strength(&self) -> f32 {
        self.min_signal_strength.unwrap_or(MIN_SIGNAL_STRENGTH)
    }
}

impl Default for ConfidenceConfig {
    fn default() -> Self {
        Self {
            half_confidence_deg: 6.0,
            min_signal_strength: None,
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
            // 1023, because 127 could not realize the design: it delivered
            // a noise-equivalent bandwidth of 1000 Hz against the 500
            // nominal, +2.3 dB of ripple at the rotation tone and a -10.6 dB
            // stopband -- twice the in-band noise a real 500 Hz filter
            // admits, degrading every SNR-derived figure. At 1023 taps the
            // measured NEB is 583 Hz, the stopband -42 dB, the tone flat,
            // and the group delay 10.7 ms, which is nothing at these output
            // rates. The uncertainty accounting measures the NEB from the
            // taps either way, so a smaller budget costs accuracy but can no
            // longer miscalibrate the stated figure.
            bandpass_taps: 1023,
            bandpass_transition_hz: 100.0,
            zero_cross_hysteresis: 0.01,
            method: BearingMethod::Correlation,
            north_tick_timing_adjustment_us: 0.0,
        }
    }
}

impl NorthTickConfig {
    /// The detection threshold this configuration actually uses.
    ///
    /// A threshold is a trade between noise margin and level margin, and only
    /// one of the two trackers can pay for it. Where gain control holds the
    /// pulse at the expected height the level margin is supplied by the AGC,
    /// so the threshold can be set high enough to reject noise triggers
    /// outright. Where it is not, the same setting spends margin the tracker
    /// does not have.
    ///
    /// So the default follows the AGC rather than the tracking mode as such.
    /// A DPLL with its AGC disabled is in the same position as the simple
    /// tracker and gets the same conservative value.
    pub fn resolved_threshold_fraction(&self) -> f32 {
        self.threshold_fraction.unwrap_or({
            let gain_controlled = matches!(self.mode, NorthTrackingMode::Dpll) && self.agc.enabled;
            if gain_controlled {
                THRESHOLD_FRACTION_GAIN_CONTROLLED
            } else {
                THRESHOLD_FRACTION_UNAIDED
            }
        })
    }
}

impl Default for NorthTickConfig {
    fn default() -> Self {
        Self {
            mode: NorthTrackingMode::Dpll,
            agc: NorthAgcConfig::default(),
            estimator: NorthPulseEstimator::EnergyCentroid,
            gain_db: 0.0,
            highpass_cutoff: 1000.0,
            fir_highpass_length_us: 1312.5,
            highpass_transition_hz: 500.0,
            threshold_fraction: None,
            expected_pulse_amplitude: 0.8,
            min_interval_ms: 0.6,
            max_coast_ms: 1000.0,
            gate_sigma: 3.0,
            dpll: DpllConfig::default(),
        }
    }
}

impl Default for BearingConfig {
    fn default() -> Self {
        Self {
            smoothing_window: 5,
            // 20 Hz because that is what the KN5R side emits: KR6DD's engine
            // divides each second into twenty batch sections and sends one
            // sentence per section. Nothing here needs that rate, but a
            // consumer built against it may. This is the default for an
            // embedder constructing the config directly; the binary always
            // sets it from `--output-rate`, whose own default is 10 Hz.
            output_rate_hz: 20.0,
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
