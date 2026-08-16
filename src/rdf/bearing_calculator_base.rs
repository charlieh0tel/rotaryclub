use crate::config::{AgcConfig, ConfidenceConfig, DopplerConfig};
use crate::error::{RdfError, Result};
use crate::signal_processing::{AutomaticGainControl, FirBandpass, MovingAverage};

use super::NorthTick;
use super::bearing::validate_confidence_config;

/// Shared signal processing components for bearing calculators
///
/// Contains the common AGC, bandpass filter, smoother, and work buffer
/// used by all bearing calculator implementations.
pub struct BearingCalculatorBase {
    agc: AutomaticGainControl,
    bandpass: FirBandpass,
    filter_group_delay: usize,
    /// Samples over which in-band noise decorrelates, which is the sample rate
    /// over twice the filter's measured noise-equivalent bandwidth. See
    /// `independent_looks`.
    noise_correlation_samples: f32,
    /// The configured trim, converted from microseconds to samples once.
    north_tick_timing_adjustment: f32,
    confidence: ConfidenceConfig,
    pub sample_counter: usize,
    buffer_start_sample: usize,
    bearing_smoother_cos: MovingAverage,
    bearing_smoother_sin: MovingAverage,
    pub work_buffer: Vec<f32>,
}

impl BearingCalculatorBase {
    /// Create a new bearing calculator base with shared components
    pub fn new(
        doppler_config: &DopplerConfig,
        agc_config: &AgcConfig,
        confidence: ConfidenceConfig,
        sample_rate: f32,
        smoothing: usize,
    ) -> Result<Self> {
        if smoothing == 0 {
            return Err(RdfError::Config(
                "bearing smoothing_window must be at least 1".to_string(),
            ));
        }
        validate_confidence_config(&confidence)?;

        let bandpass = FirBandpass::new(
            doppler_config.bandpass_low,
            doppler_config.bandpass_high,
            sample_rate,
            doppler_config.bandpass_taps,
            doppler_config.bandpass_transition_hz,
        )?;
        let filter_group_delay = bandpass.group_delay_samples();
        // Decorrelation from the filter as built, not as requested. Two
        // errors lived here. The interval used the one-sided width, but a
        // real bandpass passes +-f -- a two-sided band of width 2B -- so
        // white noise decorrelates over fs/2B, not fs/B; that alone
        // overstated the uncertainty by root two, measured at 1.40 against
        // known truth. And the width was the nominal design width, which the
        // old 127-tap filter did not remotely realize (NEB 1000 Hz against
        // 500 nominal), so the accounting inherited the design's fiction.
        // The measured noise-equivalent bandwidth is what the noise actually
        // sees, whatever the tap budget achieved.
        let neb_hz = bandpass
            .noise_equivalent_bandwidth_hz(sample_rate, doppler_config.expected_freq)
            .max(f32::EPSILON);
        let noise_correlation_samples = (sample_rate / (2.0 * neb_hz)).max(1.0);

        Ok(Self {
            agc: AutomaticGainControl::new(agc_config, sample_rate),
            bandpass,
            filter_group_delay,
            noise_correlation_samples,
            north_tick_timing_adjustment: doppler_config.north_tick_timing_adjustment_us
                * 1e-6
                * sample_rate,
            confidence,
            sample_counter: 0,
            buffer_start_sample: 0,
            bearing_smoother_cos: MovingAverage::new(smoothing),
            bearing_smoother_sin: MovingAverage::new(smoothing),
            work_buffer: Vec::new(),
        })
    }

    /// How many independent looks at the bearing the last buffer holds.
    ///
    /// Not the number of rotations in it. What limits independence is how fast
    /// the interference decorrelates, and the doppler bandpass makes that
    /// slow: a 500 Hz passband at 48 kHz decorrelates over about 96 samples,
    /// so a 512-sample buffer holds five independent looks rather than the
    /// seventeen rotations it contains.
    pub fn independent_looks(&self) -> f32 {
        (self.work_buffer.len() as f32 / self.noise_correlation_samples).max(1.0)
    }

    /// Independent looks, deflated by the residual's measured correlation.
    ///
    /// The count above assumes the in-band interference is white across the
    /// filter's bandwidth, which synthetic noise is and recorded audio is
    /// not: voice energy inside the passband is spectrally concentrated, so
    /// its residual stays correlated across the nominal decorrelation
    /// interval and the white count overstates the evidence -- measured on
    /// the captures, the stated uncertainty ran at 0.38 of the actual
    /// scatter with the white count, against roughly parity on white
    /// synthetic noise.
    ///
    /// The correction is measured per buffer, not assumed: the work
    /// buffer's autocorrelation at the decorrelation lag, with the tone's
    /// own analytic contribution `P_tone cos(w tau)` removed, is the
    /// residual's lag correlation, and an AR(1) at that lag scales the
    /// count by (1 - rho) / (1 + rho). White residual: rho near zero, count
    /// unchanged, so the synthetic calibration is untouched.
    pub fn effective_looks(&self, tone_power: f32, omega: f32) -> f32 {
        let looks = self.independent_looks();
        let lag = self.noise_correlation_samples.round().max(1.0) as usize;
        let n = self.work_buffer.len();
        if n < lag * 8 {
            return looks;
        }
        let mut r_work = 0.0f64;
        let mut p_work = 0.0f64;
        for i in 0..n - lag {
            r_work += (self.work_buffer[i] * self.work_buffer[i + lag]) as f64;
            p_work += (self.work_buffer[i] * self.work_buffer[i]) as f64;
        }
        let m = (n - lag) as f64;
        let r_work = r_work / m;
        let p_work = p_work / m;
        let r_tone = tone_power as f64 * (omega * lag as f32).cos() as f64;
        let p_resid = p_work - tone_power as f64;
        if p_resid <= f64::EPSILON {
            return looks;
        }
        let rho = ((r_work - r_tone) / p_resid).clamp(0.0, 0.95) as f32;
        (looks * (1.0 - rho) / (1.0 + rho)).max(1.0)
    }

    /// Get the confidence weights for combining metrics
    pub fn confidence(&self) -> &ConfidenceConfig {
        &self.confidence
    }

    /// Preprocess the input buffer: copy to work buffer, apply AGC and bandpass filter.
    /// Also records the buffer start position for multi-tick processing.
    pub fn preprocess(&mut self, input: &[f32]) {
        self.buffer_start_sample = self.sample_counter;
        self.work_buffer.clear();
        self.work_buffer.extend_from_slice(input);
        self.agc.process_buffer(&mut self.work_buffer);
        self.bandpass.process_buffer(&mut self.work_buffer);
    }

    /// Calculate the sample offset from the north tick using buffer_start_sample.
    /// Returns buffer_start_sample - tick.sample_index (can be negative if tick is
    /// within the current buffer).
    pub fn offset_from_north_tick(&self, north_tick: &NorthTick) -> isize {
        self.buffer_start_sample as isize - north_tick.sample_index as isize
    }

    /// Calculate samples elapsed since north tick for a buffer sample index.
    ///
    /// `sample_index_in_buffer` can be fractional (for interpolated indices).
    /// This includes FIR group-delay compensation, configured timing trim, and
    /// tracker-provided fractional tick offset.
    pub fn samples_since_tick(&self, north_tick: &NorthTick, sample_index_in_buffer: f32) -> f32 {
        self.offset_from_north_tick(north_tick) as f32 + sample_index_in_buffer
            - self.filter_group_delay as f32
            + self.north_tick_timing_adjustment
            - north_tick.fractional_sample_offset
    }

    /// Apply circular smoothing to a raw bearing value.
    /// Uses vector averaging (cos/sin components) to handle 0°/360° wraparound.
    pub fn smooth_bearing(&mut self, raw_bearing: f32) -> f32 {
        let rad = raw_bearing.to_radians();
        let avg_cos = self.bearing_smoother_cos.add(rad.cos());
        let avg_sin = self.bearing_smoother_sin.add(rad.sin());
        avg_sin.atan2(avg_cos).to_degrees().rem_euclid(360.0)
    }

    /// Advance the sample counter by the given amount
    pub fn advance_counter(&mut self, samples: usize) {
        self.sample_counter += samples;
    }
}
