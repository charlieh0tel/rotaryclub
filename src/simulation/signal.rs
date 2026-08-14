use crate::signal_processing::FirBandpass;
use std::f32::consts::PI;

/// Taps and transition width for the band limiting on the interfering audio.
/// Long enough that the band edges are sharp against a 300 Hz to 3.4 kHz
/// band; this runs once per generated signal, not per sample.
const AUDIO_BAND_TAPS: usize = 255;
const AUDIO_BAND_TRANSITION_HZ: f32 = 150.0;

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

/// What the Doppler channel carries besides the rotation tone.
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
pub struct DopplerImpairment {
    /// Power in the interfering audio, relative to the rotation tone.
    ///
    /// The reciprocal of the in-band fraction the census reports, near enough:
    /// 20.0 here puts the tone at about 5 percent of the channel.
    pub audio_to_tone_power: f32,
    /// Amplitude of the second harmonic, relative to the fundamental.
    pub second_harmonic: f32,
    /// Amplitude of the third harmonic, relative to the fundamental.
    pub third_harmonic: f32,
    /// Band the interfering audio occupies, in Hz. Voice-band by default, so
    /// it overlaps the Doppler passband rather than being filtered away
    /// without ever having been a nuisance.
    pub audio_low_hz: f32,
    pub audio_high_hz: f32,
    /// Seed, so a run is repeatable.
    pub seed: u64,
}

impl DopplerImpairment {
    /// Nothing but the tone, which is what this generator used to produce.
    pub fn none() -> Self {
        Self {
            audio_to_tone_power: 0.0,
            second_harmonic: 0.0,
            third_harmonic: 0.0,
            audio_low_hz: 300.0,
            audio_high_hz: 3400.0,
            seed: 0x51D3_7A19_C0DE_2B4F,
        }
    }

    /// Impairment in the range the captures in `data/` actually show.
    ///
    /// Deliberately the mild end of what was measured: the tone at about 5
    /// percent of the channel, against 0.2 to 7.5 measured, and harmonics at
    /// 10 percent against 7 to 15. A generator that sits at the worst
    /// observed case makes every test a test of the worst case.
    pub fn representative() -> Self {
        Self {
            audio_to_tone_power: 20.0,
            second_harmonic: 0.10,
            third_harmonic: 0.06,
            ..Self::none()
        }
    }
}

/// Deterministic uniform noise on [-1, 1).
fn noise_at(index: usize, seed: u64) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
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
        DopplerImpairment::none(),
    )
}

/// As `generate_test_signal_with_bearing_fn`, with the Doppler channel
/// carrying interference alongside the rotation tone.
pub fn generate_impaired_signal<F>(
    duration_secs: f32,
    sample_rate: u32,
    rotation_hz: f32,
    bearing_fn: F,
    impairment: DopplerImpairment,
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
        add_north_pulse(&mut north, epoch, NORTH_TICK_AMPLITUDE);
        rotation += 1;
    }

    // Interfering audio, band-limited to the voice band so it overlaps the
    // Doppler passband. White noise across the whole spectrum would be almost
    // entirely removed by the bandpass and would understate the interference
    // by the ratio of the two bandwidths, which is what makes this the wrong
    // shortcut: it would look like impairment and behave like none.
    let audio = if impairment.audio_to_tone_power > 0.0 {
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
        // Scale to the requested power against the tone, whose power is 1/2
        // for a unit sine.
        let power = raw.iter().map(|s| (s * s) as f64).sum::<f64>() / num_samples.max(1) as f64;
        let wanted = 0.5 * impairment.audio_to_tone_power as f64;
        let scale = if power > 0.0 {
            (wanted / power).sqrt() as f32
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

    let mut samples = Vec::with_capacity(num_samples * 2);
    for (i, &north_tick) in north.iter().enumerate() {
        let t = i as f32 / sample_rate as f32;
        let bearing_radians = bearing_fn(t).to_radians();
        let doppler_phase = rotation_hz * t * 2.0 * PI - bearing_radians;
        let tone = doppler_phase.sin()
            + impairment.second_harmonic * (2.0 * doppler_phase).sin()
            + impairment.third_harmonic * (3.0 * doppler_phase).sin();
        samples.push(tone + audio[i]);
        samples.push(north_tick);
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
