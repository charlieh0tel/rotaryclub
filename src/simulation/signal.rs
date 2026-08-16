use crate::signal_processing::FirBandpass;
use std::f32::consts::PI;

/// Taps and transition width for the band limiting on the interfering audio.
/// Long enough that the band edges are sharp against a 300 Hz to 3.4 kHz
/// band; this runs once per generated signal, not per sample.
const AUDIO_BAND_TAPS: usize = 255;
const AUDIO_BAND_TRANSITION_HZ: f32 = 150.0;
/// The Doppler passband the impairment is scaled against, matching the
/// shipped `DopplerConfig` defaults.
const DOPPLER_BAND_LOW_HZ: f32 = 1350.0;
const DOPPLER_BAND_HIGH_HZ: f32 = 1850.0;

pub const NORTH_TICK_PULSE_WIDTH_RADIANS: f32 = 0.2;
pub const NORTH_TICK_AMPLITUDE: f32 = 0.8;

/// Half-width, in samples, of the synthesized north pulse.
const NORTH_TICK_HALF_WIDTH_SAMPLES: i64 = 12;

/// A band-limited impulse at a fractional sample position.
///
/// The hardware pulse is ~20 us, well under one sample at 48 kHz, so what an
/// anti-aliased converter records is a band-limited impulse centred on the
/// arrival time rather than a sample that happens to be non-zero. Gating on
/// rotation phase instead would light the first sample at or after the
/// rotation boundary -- ceil(epoch) -- which lags the truth by half a sample
/// on average and shows up downstream as a fixed 6 degrees of bearing.
fn add_north_pulse(channel: &mut [f32], epoch: f64, amplitude: f32) {
    let center = epoch.round() as i64;
    for n in (center - NORTH_TICK_HALF_WIDTH_SAMPLES)..=(center + NORTH_TICK_HALF_WIDTH_SAMPLES) {
        if n < 0 || n as usize >= channel.len() {
            continue;
        }
        let x = n as f64 - epoch;
        let value = if x.abs() < f64::EPSILON {
            1.0
        } else {
            let px = std::f64::consts::PI * x;
            let window = px / NORTH_TICK_HALF_WIDTH_SAMPLES as f64;
            (px.sin() / px) * (window.sin() / window)
        };
        channel[n as usize] += amplitude * value as f32;
    }
}

/// What the channels carry besides the signal.
///
/// The synthetic Doppler channel was the rotation tone and nothing else,
/// which is not what a receiver delivers. Measured against the captures in
/// `data/` with `census_signal`, the tone is between 0.2 and 7.5
/// percent of the channel and the rest is FM audio, and the second and third
/// harmonics run at 7 to 15 percent of the fundamental. The synthetic signal
/// read 100 percent and zero.
///
/// That gap is why the bearing uncertainty was calibrated wrongly twice on
/// synthetic signal: the figure is the Doppler phase spread over the
/// independent count plus the reference term, and with no interference there
/// is no phase spread, so only the reference term was ever exercised.
#[derive(Debug, Clone, Copy)]
pub struct SignalImpairment {
    /// Interfering audio power *inside the Doppler passband*, relative to the
    /// rotation tone. The three conditions used throughout are 0.199, 0.793
    /// and 6.579, which is +7, +1 and -8 dB.
    ///
    /// Defined conditions rather than measured properties of the recordings in
    /// `data/`, though they were introduced as the latter. `metric_in_band_snr`
    /// measures those files at -8.4, +2.8 and +2.6 dB over whole files, or
    /// -4.0, +15.8 and +12.2 dB counting only the louder half of each: a
    /// recording is transmissions separated by squelch noise, so its ratio
    /// spans orders of magnitude and no single value describes it without a
    /// stated rule for choosing segments. A signal built here sets its ratio by
    /// construction, so what it is scaled to is exact whatever the captures do.
    ///
    /// Not the ratio over the whole channel, which is the natural thing to
    /// reach for and is wrong. Real audio sits well below the Doppler band, so
    /// matching total power with flat voice-band noise puts about ten times
    /// too much where it hurts: at the cleanest recording's whole-channel
    /// ratio, flat noise produced 20.7 degrees of bearing error where that
    /// recording achieves 1.6. What decides a bearing is the power in the
    /// passband, so that is what this names.
    pub passband_noise_to_tone: f32,
    /// Amplitude of the second harmonic, relative to the fundamental.
    pub second_harmonic: f32,
    /// Amplitude of the third harmonic, relative to the fundamental.
    pub third_harmonic: f32,
    /// Band the interfering audio occupies, in Hz. Voice-band by default, so
    /// it overlaps the Doppler passband rather than being filtered away
    /// without ever having been a nuisance.
    pub audio_low_hz: f32,
    pub audio_high_hz: f32,
    /// Amplitude of the north pulses, before any gain.
    ///
    /// The recordings in `data/` measure 0.21, 0.44 and 0.78 against a
    /// configured expectation of 0.8, so a factor of nearly four across two
    /// radios. This is the axis the north AGC exists for.
    pub north_pulse_amplitude: f32,
    /// RMS of the white noise added to the north channel.
    ///
    /// White rather than band-limited, unlike the doppler side: the north
    /// highpass at 1 kHz passes most of the spectrum, so what is generated is
    /// close to what the detector meets. The recordings in `data/` measure a
    /// north floor around 0.0006, so anything above about 0.01 here is beyond
    /// what has ever been observed.
    pub north_noise_rms: f32,
    /// How strongly the interference clumps in time, from 0 to just under 1.
    ///
    /// Zero is stationary noise, which is what this generated for a long time
    /// and is not what a radio delivers. Measured on the recordings, the power
    /// in 20 ms windows correlates with the next window at 0.90, 0.91 and
    /// 0.94; the synthetic channel read 0.002. Their power arrives in bursts
    /// with quiet between -- the p95 window carries 1.4 to 5.9 times the
    /// median, against 1.2 here, and 8 to 27 percent of windows are near
    /// silent, against none.
    ///
    /// That difference, not the spectrum, is what made the synthetic channel
    /// two to four times harsher at matched power. Matching the *mean* while
    /// the real thing is quiet most of the time and occasionally much worse
    /// means the typical bearing sees far less than the mean suggests. The
    /// spectral tilt across the passband was the standing explanation and does
    /// not survive measurement: 3.3 dB here against -0.4 to -3.8 there, small
    /// and not even the same sign.
    ///
    /// The mean passband power is held to `passband_noise_to_tone` whatever
    /// this is set to, so the two axes stay independent.
    pub envelope_correlation: f32,
    /// Spread of the interference envelope, in nepers of log power.
    ///
    /// Sets how deep the quiet stretches are and how far the bursts rise.
    /// Zero is a flat envelope regardless of `envelope_correlation`.
    pub envelope_depth: f32,
    /// Amplitude of a second propagation path, relative to the direct one.
    ///
    /// A synthetic stress case, not something measured. A reflection arrives
    /// from a different direction and so carries a different apparent
    /// bearing; the two sum with a drifting relative phase, so the tone fades
    /// where they near cancellation and the bearing swings between them.
    ///
    /// It was added because the recordings' tone varies by 17 to 133 dB
    /// between its 5th and 95th percentile, which is not fading: in those
    /// windows the channel gets louder and the north pulses continue, which
    /// is a transmitter that stopped transmitting. Seventy percent of the
    /// ft-70d capture has no carrier on it.
    ///
    /// Kept because it is the only impairment here that makes a bearing
    /// ambiguous rather than imprecise.
    pub multipath_ratio: f32,
    /// Bearing the reflected path appears to arrive from, in degrees, as an
    /// offset from the true one. Zero would make it indistinguishable from
    /// the direct path and produce fading with no bearing error.
    pub multipath_bearing_offset_deg: f32,
    /// How fast the relative phase of the two paths drifts, in Hz.
    ///
    /// Sets the fading rate. The measured window-to-window correlation of the
    /// tone envelope puts this below a few Hz.
    pub multipath_drift_hz: f32,
    /// Seed, so a run is repeatable.
    pub seed: u64,
}

impl SignalImpairment {
    /// Nothing but the tone, which is what this generator used to produce.
    pub fn none() -> Self {
        Self {
            passband_noise_to_tone: 0.0,
            second_harmonic: 0.0,
            third_harmonic: 0.0,
            north_pulse_amplitude: NORTH_TICK_AMPLITUDE,
            north_noise_rms: 0.0,
            audio_low_hz: 300.0,
            audio_high_hz: 3400.0,
            envelope_correlation: 0.0,
            envelope_depth: 0.0,
            multipath_ratio: 0.0,
            multipath_bearing_offset_deg: 0.0,
            multipath_drift_hz: 0.0,
            seed: 0x51D3_7A19_C0DE_2B4F,
        }
    }

    /// Impairment in the range the captures in `data/` actually show.
    ///
    /// The middle of the three, not the worst: a generator pinned to the worst
    /// observed case makes every test silently a worst-case test.
    pub fn representative() -> Self {
        Self {
            passband_noise_to_tone: 0.8,
            second_harmonic: 0.10,
            third_harmonic: 0.06,
            ..Self::bursty()
        }
    }

    /// Interference that clumps in time the way the recordings' does.
    ///
    /// The envelope figures are chosen to land inside the range the three
    /// captures measure rather than on any one of them: correlation 0.898
    /// against their 0.90 to 0.94, and a depth reproducing a p95-over-median
    /// of 3.58 against their 1.4 to 5.9.
    pub fn bursty() -> Self {
        Self {
            envelope_correlation: 0.94,
            envelope_depth: 0.5,
            ..Self::none()
        }
    }

    /// A channel with a reflection in it. Synthetic; see `multipath_ratio`.
    ///
    /// Deliberately not part of `representative()`, and the distinction is not
    /// bookkeeping. Noise and interference degrade the precision of a bearing;
    /// multipath changes what the bearing *is*, because the reflection really
    /// does arrive from somewhere else and the sum really does point between
    /// them. A test asserting the pipeline recovers a known bearing to three
    /// degrees is not made harder by this, it is made meaningless -- three of
    /// them failed the moment it was switched on by default, which is the
    /// correct response and the reason it is opt-in.
    ///
    /// The parameters give a null about 19 dB deep. They were tuned against
    /// a calibration target that turned out to be an artifact, so treat them
    /// as a plausible reflection rather than a measured one.
    pub fn multipath() -> Self {
        Self {
            multipath_ratio: 0.45,
            multipath_bearing_offset_deg: 60.0,
            multipath_drift_hz: 0.7,
            ..Self::bursty()
        }
    }

    /// A named level, for sweeping. The three are the three recordings.
    pub fn at_passband_ratio(ratio: f32) -> Self {
        Self {
            passband_noise_to_tone: ratio,
            second_harmonic: 0.10,
            third_harmonic: 0.06,
            ..Self::bursty()
        }
    }
}

/// Avalanche: every output bit depends on every input bit.
fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    x
}

/// Deterministic uniform noise on [-1, 1).
///
/// Public because every harness that wants reproducible noise was writing its
/// own copy of it -- sixteen files carried one at the last count, and they
/// have already diverged twice. The first time cost a shifted output range
/// that put a DC offset through twelve of them; the second was the seed
/// mixing described below, fixed here and nowhere else. Reach for this rather
/// than pasting another.
///
/// The seed is mixed before it is combined, not after, and that is the whole
/// of the difference between independent realisations and correlated ones.
/// Folding a raw seed in and relying on the finalizer to scatter it
/// avalanches the value but not the difference between two streams: two seeds
/// differing in their low bits stay differing in their low bits through the
/// shift-xor, and the multiply that follows turns that into a near-constant
/// offset rather than a fresh draw. Measured on the stream this replaced,
/// seeds 1 and 2 correlated at 0.97, and the default seed against the next
/// one along at 0.76.
///
/// That makes every error bar taken over seeds far too small, and it is
/// invisible unless it is looked for: the runs differ, they simply differ far
/// less than they should. Mixing the seed first puts all of these below 0.02.
pub fn noise_at(index: usize, seed: u64) -> f32 {
    let x = mix((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ mix(seed));
    (((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

/// Length of one step of the interference envelope, in milliseconds.
///
/// Matches the window the recordings' envelope statistics were measured over,
/// and is the timescale a bearing is computed on, which is the one that
/// decides whether a burst is seen as a burst or averaged away.
const ENVELOPE_STEP_MS: f32 = 20.0;

/// Give the interference the clumped time structure real audio has.
///
/// An AR(1) process on log power, so the envelope is smooth on the scale of
/// syllables rather than jumping every sample, and log-normal rather than
/// normal so it cannot go negative and so quiet stretches are deep. The result
/// is renormalised to unit mean power, which keeps this independent of
/// `passband_noise_to_tone`: changing how bursty the interference is must not
/// change how much of it there is, or the two could never be swept separately.
/// Mean power of `samples` inside `[low_hz, high_hz)`, measured exactly.
///
/// Hann-windowed direct correlation on 16384-sample segments, summed over the
/// band's bin grid -- the coherent-gain factors cancel between bins, so the
/// sum is the band power by Parseval. This replaces a probe bandpass FIR
/// whose transition skirts passed out-of-band audio and credited it as
/// in-band: scaled through that probe, every generated "in-band ratio" came
/// out 12.3 percent low (0.57 dB), at every level, against an independent
/// FFT. Scaling is linear in power, so one measurement and one scale factor
/// make the generated ratio exact.
pub fn in_band_power(samples: &[f32], sample_rate: f32, low_hz: f32, high_hz: f32) -> f64 {
    const SEGMENT: usize = 16384;
    if samples.len() < SEGMENT {
        // Short signals get one segment of whatever length there is.
        return in_band_power_segment(samples, sample_rate, low_hz, high_hz);
    }
    let mut total = 0.0f64;
    let mut segments = 0usize;
    for chunk in samples.chunks_exact(SEGMENT) {
        total += in_band_power_segment(chunk, sample_rate, low_hz, high_hz);
        segments += 1;
    }
    total / segments.max(1) as f64
}

fn in_band_power_segment(seg: &[f32], sample_rate: f32, low_hz: f32, high_hz: f32) -> f64 {
    let n = seg.len();
    if n == 0 {
        return 0.0;
    }
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect();
    // Hann's incoherent power gain is the mean of w^2 = 3/8; dividing it out
    // makes the summed bin powers read as the band's mean power.
    let power_gain: f64 = window.iter().map(|w| (w * w) as f64).sum::<f64>() / n as f64;
    let windowed: Vec<f32> = seg.iter().zip(&window).map(|(s, w)| s * w).collect();
    let resolution = sample_rate / n as f32;
    let mut hz = (low_hz / resolution).ceil() * resolution;
    let mut band = 0.0f64;
    while hz < high_hz {
        let omega = 2.0 * PI * hz / sample_rate;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &s) in windowed.iter().enumerate() {
            let phase = omega * i as f32;
            re += (s * phase.cos()) as f64;
            im += (s * phase.sin()) as f64;
        }
        band += 2.0 * ((re / n as f64).powi(2) + (im / n as f64).powi(2));
        hz += resolution;
    }
    band / power_gain
}

fn apply_envelope(audio: &mut [f32], sample_rate: u32, impairment: SignalImpairment) {
    let rho = impairment.envelope_correlation.clamp(0.0, 0.999);
    let depth = impairment.envelope_depth.max(0.0);
    if depth <= 0.0 {
        return;
    }

    let step = ((ENVELOPE_STEP_MS * 1e-3 * sample_rate as f32) as usize).max(1);
    let steps = audio.len().div_ceil(step) + 1;
    let drive = (1.0 - rho * rho).sqrt();

    let mut level = Vec::with_capacity(steps);
    let mut state = 0.0f32;
    for k in 0..steps {
        // Twelve draws for something near normal, as elsewhere in this file.
        let mut acc = 0.0f32;
        for j in 0..12 {
            acc += noise_at(k * 12 + j, impairment.seed ^ 0x454E_5645_4C4F_5045);
        }
        state = rho * state + drive * (acc / 2.0);
        level.push((depth * state).exp());
    }

    // Interpolate between steps so the envelope has no edges of its own to
    // put energy where the interference is not supposed to have any.
    for (i, sample) in audio.iter_mut().enumerate() {
        let position = i as f32 / step as f32;
        let k = position as usize;
        let frac = position - k as f32;
        let a = level[k.min(level.len() - 1)];
        let b = level[(k + 1).min(level.len() - 1)];
        *sample *= a + (b - a) * frac;
    }

    let power = audio.iter().map(|s| (s * s) as f64).sum::<f64>() / audio.len().max(1) as f64;
    if power > 0.0 {
        let scale = (1.0 / power).sqrt() as f32;
        for sample in audio.iter_mut() {
            *sample *= scale;
        }
    }
}

/// Generate synthetic RDF test signal with fixed bearing
/// Returns interleaved stereo samples [L, R, L, R, ...]
/// Left = Doppler tone, Right = North tick
pub fn generate_test_signal(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    bearing_degrees: f32,
) -> Vec<f32> {
    generate_test_signal_with_bearing_fn(duration_secs, sample_rate, rotation_hz, |_| {
        bearing_degrees
    })
}

/// Generate synthetic RDF test signal with time-varying bearing
/// The bearing_fn takes time in seconds and returns bearing in degrees
pub fn generate_test_signal_with_bearing_fn<F>(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    bearing_fn: F,
) -> Vec<f32>
where
    F: Fn(f32) -> f32,
{
    generate_impaired_signal(
        duration_secs,
        sample_rate,
        rotation_hz,
        bearing_fn,
        SignalImpairment::none(),
    )
}

/// As `generate_test_signal_with_bearing_fn`, with the Doppler channel
/// carrying interference alongside the rotation tone.
pub fn generate_impaired_signal<F>(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    bearing_fn: F,
    impairment: SignalImpairment,
) -> Vec<f32>
where
    F: Fn(f32) -> f32,
{
    let num_samples = (duration_secs * sample_rate as f32) as usize;
    let samples_per_rotation = sample_rate as f64 / rotation_hz as f64;

    let mut north = vec![0.0f32; num_samples];
    let mut rotation = 0i64;
    loop {
        let epoch = rotation as f64 * samples_per_rotation;
        if epoch >= num_samples as f64 {
            break;
        }
        add_north_pulse(&mut north, epoch, impairment.north_pulse_amplitude);
        rotation += 1;
    }

    // Interfering audio, band-limited to the voice band so it overlaps the
    // Doppler passband. White noise across the whole spectrum would be almost
    // entirely removed by the bandpass and would understate the interference
    // by the ratio of the two bandwidths, which is what makes this the wrong
    // shortcut: it would look like impairment and behave like none.
    let audio = if impairment.passband_noise_to_tone > 0.0 {
        let mut raw: Vec<f32> = (0..num_samples)
            .map(|i| noise_at(i, impairment.seed))
            .collect();
        if let Ok(mut band) = FirBandpass::new(
            impairment.audio_low_hz,
            impairment.audio_high_hz,
            sample_rate as f32,
            AUDIO_BAND_TAPS,
            AUDIO_BAND_TRANSITION_HZ,
        ) {
            band.process_buffer(&mut raw);
        }
        apply_envelope(&mut raw, sample_rate, impairment);
        // Scale by what reaches the Doppler passband, not by total power.
        // How much of the voice band gets there depends on the audio filter
        // and the envelope, so it is measured rather than assumed -- and
        // measured exactly; see in_band_power for the probe-FIR skirt leak
        // this replaces.
        let passband_power = in_band_power(
            &raw,
            sample_rate as f32,
            DOPPLER_BAND_LOW_HZ,
            DOPPLER_BAND_HIGH_HZ,
        );
        // A unit sine has power 1/2.
        let wanted = 0.5 * impairment.passband_noise_to_tone as f64;
        let scale = if passband_power > 0.0 {
            (wanted / passband_power).sqrt() as f32
        } else {
            0.0
        };
        for sample in raw.iter_mut() {
            *sample *= scale;
        }
        raw
    } else {
        vec![0.0f32; num_samples]
    };

    // North channel noise, twelve uniform draws for a rough normal. Each draw
    // is uniform on [-1, 1) with variance 1/3, so twelve of them have a
    // standard deviation of 2 and the divisor is 2, not 6. Getting that wrong
    // is how two sweeps came to run at a third of the noise they claimed.
    let north_noise = |index: usize| -> f32 {
        if impairment.north_noise_rms <= 0.0 {
            return 0.0;
        }
        let mut acc = 0.0f32;
        for j in 0..12 {
            acc += noise_at(index * 12 + j, impairment.seed ^ 0x4E4F_5254_4831);
        }
        acc / 2.0 * impairment.north_noise_rms
    };

    let mut samples = Vec::with_capacity(num_samples * 2);
    for (i, &north_tick) in north.iter().enumerate() {
        let t = i as f32 / sample_rate as f32;
        let bearing_radians = bearing_fn(t).to_radians();
        let doppler_phase = rotation_hz * t * 2.0 * PI - bearing_radians;
        let mut tone = doppler_phase.sin()
            + impairment.second_harmonic * (2.0 * doppler_phase).sin()
            + impairment.third_harmonic * (3.0 * doppler_phase).sin();
        if impairment.multipath_ratio > 0.0 {
            // The reflection carries its own apparent bearing and its own
            // slowly drifting phase. Summing them in the signal, rather than
            // perturbing the bearing afterwards, is what makes the fading and
            // the bearing error the same event instead of two independent
            // ones: they both come from the paths approaching cancellation.
            let reflected_bearing =
                bearing_radians + impairment.multipath_bearing_offset_deg.to_radians();
            let drift = 2.0 * PI * impairment.multipath_drift_hz * t;
            let reflected_phase = rotation_hz * t * 2.0 * PI - reflected_bearing + drift;
            tone += impairment.multipath_ratio
                * (reflected_phase.sin()
                    + impairment.second_harmonic * (2.0 * reflected_phase).sin()
                    + impairment.third_harmonic * (3.0 * reflected_phase).sin());
        }
        samples.push(tone + audio[i]);
        samples.push(north_tick + north_noise(i));
    }

    samples
}

/// Generate a pure Doppler signal for a given bearing (no north tick)
pub fn generate_doppler_signal_for_bearing(
    num_samples: usize,
    sample_rate: f32,
    rotation_hz: f32,
    bearing_degrees: f32,
) -> Vec<f32> {
    let bearing_radians = bearing_degrees.to_radians();
    let omega = 2.0 * PI * rotation_hz;

    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate;
            (omega * t - bearing_radians).sin()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_signal_length() {
        let signal = generate_test_signal(1.0, 48000, 500.0, 0.0);
        assert_eq!(signal.len(), 48000 * 2);
    }

    #[test]
    fn test_generate_signal_interleaved() {
        let signal = generate_test_signal(0.01, 48000, 500.0, 0.0);

        assert_eq!(signal.len() % 2, 0);

        let left: Vec<f32> = signal.iter().step_by(2).copied().collect();
        let left_rms: f32 = (left.iter().map(|x| x * x).sum::<f32>() / left.len() as f32).sqrt();
        assert!(
            left_rms > 0.1,
            "Left channel should contain signal, got RMS {}",
            left_rms
        );

        let right: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();
        let right_max = right.iter().fold(0.0f32, |a, &b| a.max(b));
        assert!(
            right_max > NORTH_TICK_AMPLITUDE * 0.5,
            "Right channel should have tick pulses"
        );
    }

    #[test]
    fn test_generate_multiple_bearings() {
        for bearing in [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
            let signal = generate_test_signal(0.1, 48000, 500.0, bearing);
            assert_eq!(signal.len(), 4800 * 2);
        }
    }
}

#[cfg(test)]
mod in_band_power_tests {
    use super::*;

    /// The estimator against a known answer: a sine inside the band reads its
    /// full power, one outside reads nothing.
    #[test]
    fn sine_power_lands_in_the_right_band() {
        let sr = 48000.0f32;
        let inside: Vec<f32> = (0..48000)
            .map(|i| 0.7 * (2.0 * PI * 1602.564 * i as f32 / sr).sin())
            .collect();
        let p = in_band_power(&inside, sr, 1350.0, 1850.0);
        let expected = 0.5 * 0.7f64 * 0.7f64;
        assert!(
            (p - expected).abs() / expected < 0.01,
            "in-band sine: {p} vs {expected}"
        );

        let outside: Vec<f32> = (0..48000)
            .map(|i| 0.7 * (2.0 * PI * 500.0 * i as f32 / sr).sin())
            .collect();
        let p = in_band_power(&outside, sr, 1350.0, 1850.0);
        assert!(p < expected * 1e-3, "out-of-band sine leaked: {p}");
    }

    /// White noise is flat, so the band holds its bandwidth's share of the
    /// total power.
    #[test]
    fn white_noise_reads_its_bandwidth_fraction() {
        let sr = 48000.0f32;
        let noise: Vec<f32> = (0..480_000).map(|i| noise_at(i, 0x1B5E_ED01)).collect();
        let total = noise.iter().map(|s| (s * s) as f64).sum::<f64>() / noise.len() as f64;
        let band = in_band_power(&noise, sr, 1350.0, 1850.0);
        let expected = total * 500.0 / 24000.0;
        assert!(
            (band - expected).abs() / expected < 0.05,
            "band fraction: {band} vs {expected}"
        );
    }

    /// The generated in-band ratio equals the stated one. The scale is set
    /// through in_band_power, which the two tests above validate against
    /// analytic answers, so this is a wiring check rather than a tautology.
    /// The probe-FIR scaling this replaced generated 12.3 percent low at
    /// every level.
    ///
    /// Checked at the worst ratio only. The tone-noise cross-term of one
    /// finite draw is roughly +-0.02 in power whatever the ratio -- measured
    /// across seeds it flips sign, so it is zero-mean, not a bias -- which
    /// makes it 10 percent of the interference at the cleanest ratio, 3 at
    /// the middle one, and 0.4 at the worst. Only the last supports a
    /// tolerance tight enough to catch the bug this test exists to catch,
    /// and the scale plumbing is level-independent, so one level pins it.
    #[test]
    fn generated_ratio_matches_stated() {
        let sr = 48000u32;
        for stated in [6.579f32] {
            let sig = generate_impaired_signal(
                8.0,
                sr,
                1602.564,
                |_| 200.0,
                SignalImpairment::at_passband_ratio(stated),
            );
            let dop: Vec<f32> = sig.iter().step_by(2).copied().collect();
            let total = in_band_power(&dop, sr as f32, 1350.0, 1850.0);
            // Unit tone contributes 1/2; the rest is the interference.
            let measured = (total - 0.5) / 0.5;
            assert!(
                (measured - f64::from(stated)).abs() / f64::from(stated) < 0.02,
                "stated {stated}, measured {measured}"
            );
        }
    }
}
