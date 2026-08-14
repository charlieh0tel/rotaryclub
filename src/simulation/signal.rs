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
/// `data/` with `examples/signal_census`, the tone is between 0.2 and 7.5
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
    /// rotation tone. Measured on the recordings in `data/`: 0.199, 0.793 and
    /// 6.579.
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
            ..Self::none()
        }
    }

    /// A named level, for sweeping. The three are the three recordings.
    pub fn at_passband_ratio(ratio: f32) -> Self {
        Self {
            passband_noise_to_tone: ratio,
            second_harmonic: 0.10,
            third_harmonic: 0.06,
            ..Self::none()
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
fn noise_at(index: usize, seed: u64) -> f32 {
    let x = mix((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ mix(seed));
    (((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
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
        // Scale by what reaches the Doppler passband, not by total power.
        // How much of the voice band gets there depends on both filters, so it
        // is measured rather than assumed.
        let mut probe = raw.clone();
        if let Ok(mut band) = FirBandpass::new(
            DOPPLER_BAND_LOW_HZ,
            DOPPLER_BAND_HIGH_HZ,
            sample_rate as f32,
            AUDIO_BAND_TAPS,
            100.0,
        ) {
            band.process_buffer(&mut probe);
        }
        let passband_power =
            probe.iter().map(|s| (s * s) as f64).sum::<f64>() / num_samples.max(1) as f64;
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
        let tone = doppler_phase.sin()
            + impairment.second_harmonic * (2.0 * doppler_phase).sin()
            + impairment.third_harmonic * (3.0 * doppler_phase).sin();
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
