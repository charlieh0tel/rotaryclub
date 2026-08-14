//! Where did the zero-crossing method's noise-dependent bias come from?
//!
//! It came from the noise. Every generator in this repo built its noise as
//! `(x >> 33) as u32 / u32::MAX * 2 - 1`, and a 33-bit shift leaves 31 bits
//! against a 32-bit divisor, so the result spanned [-1, 0) rather than
//! [-1, 1). Half of what was called noise was a DC offset, and what remained
//! carried a seventh of the in-band energy white noise would.
//!
//! A residual DC through the bandpass shifts a sinusoid's zero crossings,
//! which is why the effect scaled with the noise setting, stayed put across
//! seeds, changed sign with the filter width, and left the correlation method
//! alone -- correlating against sin and cos at the tone frequency rejects DC.
//!
//! With the generator fixed the bias is gone: 0.004 samples at noise 0.3
//! where it read -0.16, and the crossing scatter doubles to what it should
//! always have been. The zero-crossing method was never at fault.
//!
//! This is kept because the same shape of mistake will happen again, and
//! because it took an independent implementation to catch: the detector, the
//! AGC, the passband centre and the run length were all cleared first, and a
//! plain sign-change scan reproduced the biased answer exactly.

use std::f32::consts::PI;

use rotaryclub::config::{AgcConfig, DopplerConfig};
use rotaryclub::signal_processing::{AutomaticGainControl, FirBandpass, ZeroCrossingDetector};

fn noise_at(index: usize, seed: u64) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    (((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

fn main() {
    let doppler = DopplerConfig::default();
    let sample_rate = 48_000.0f32;
    let rotation_hz = doppler.expected_freq;
    let omega = 2.0 * PI * rotation_hz / sample_rate;
    let period = sample_rate / rotation_hz;
    let degrees_per_sample = 360.0 / period;

    println!(
        "one sample = {degrees_per_sample:.2} deg, hysteresis {}",
        doppler.zero_cross_hysteresis
    );
    println!(
        "\n{:>8} {:>10} {:>12} {:>12} {:>10} {:>10} {:>10}",
        "noise", "passband", "mean (samp)", "mean (deg)", "sd (samp)", "amplitude", "zeroxings"
    );

    // Is the bias an artifact of the passband being centred at 1600 while
    // the tone sits at 1602.564? Sweep the passband centre against a fixed
    // tone: if the offset is what pulls the crossings, the bias goes through
    // zero when the two coincide.
    // Is the generator's noise actually white at the tone frequency? A
    // deterministic hash of the sample index need not be. A coherent
    // component at the tone would add a fixed phase to it, shift the
    // crossings by an amount proportional to the noise scale, and do so
    // consistently across seeds -- which is exactly the bias observed.
    println!("\ncoherent content of the noise at the tone frequency");
    println!(
        "{:>20} {:>14} {:>14} {:>10}",
        "generator", "|correlation|", "rms", "ratio"
    );
    for (name, seed) in [
        ("hash seed A", 0xA5A5_1234_9ABC_DEF0u64),
        ("hash seed B", 0x1111_2222_3333_4444),
        ("hash seed C", 0xDEAD_BEEF_CAFE_0001),
    ] {
        let n = 48_000usize;
        let (mut i_sum, mut q_sum, mut power) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..n {
            let v = noise_at(i, seed) as f64;
            let phase = omega as f64 * i as f64;
            i_sum += v * phase.cos();
            q_sum += v * phase.sin();
            power += v * v;
        }
        let magnitude = ((i_sum / n as f64).powi(2) + (q_sum / n as f64).powi(2)).sqrt();
        let rms = (power / n as f64).sqrt();
        println!(
            "{name:>20} {magnitude:>14.6} {rms:>14.6} {:>10.4}",
            magnitude / rms
        );
    }
    // A white sequence of this length should show a correlation of about
    // rms / sqrt(2n), which is the number to compare the ratio against.
    println!(
        "{:>20} {:>14} {:>14} {:>10.4}",
        "white expectation",
        "",
        "",
        1.0 / (2.0 * 48_000.0f64).sqrt()
    );

    // The default passband is 1350-1850, a half width of 250 around 1600,
    // against a tone at 1602.564. Sweep the width at that centre, and the
    // centre at that width, to see what the bias actually depends on.
    for (noise, seed, centre_hz, half_width) in [
        (0.0f32, 0xA5A5_1234_9ABC_DEF0u64, 1600.0f32, 250.0f32),
        (0.0, 0xA5A5_1234_9ABC_DEF0, 1600.0, 200.0),
        (0.0, 0xA5A5_1234_9ABC_DEF0, 1600.0, 150.0),
        (0.3, 0xA5A5_1234_9ABC_DEF0, 1600.0, 250.0),
        (0.3, 0xA5A5_1234_9ABC_DEF0, 1600.0, 200.0),
        (0.3, 0xA5A5_1234_9ABC_DEF0, 1600.0, 150.0),
        (1.0, 0xA5A5_1234_9ABC_DEF0, 1600.0, 250.0),
        (1.0, 0xA5A5_1234_9ABC_DEF0, 1600.0, 200.0),
        (1.0, 0xA5A5_1234_9ABC_DEF0, 1600.0, 150.0),
    ] {
        let n = 480_000usize;
        let mut signal: Vec<f32> = (0..n)
            .map(|i| (omega * i as f32).sin() + noise * noise_at(i, seed))
            .collect();

        let mut agc = AutomaticGainControl::new(&AgcConfig::default(), sample_rate);
        let mut bandpass = FirBandpass::new(
            centre_hz - half_width,
            centre_hz + half_width,
            sample_rate,
            doppler.bandpass_taps,
            doppler.bandpass_transition_hz,
        )
        .expect("bandpass");
        agc.process_buffer(&mut signal);
        bandpass.process_buffer(&mut signal);
        let group_delay = bandpass.group_delay_samples() as f32;

        let amplitude = signal
            .iter()
            .skip(2000)
            .fold(0.0f32, |acc, s| acc.max(s.abs()));

        // How often does the filtered signal actually cross zero upward? If
        // it is once per cycle, latching the first such crossing cannot be
        // what biases the answer.
        let mut upward = 0usize;
        for w in signal.windows(2).skip(2000) {
            if w[0] <= 0.0 && w[1] > 0.0 {
                upward += 1;
            }
        }

        let mut detector = ZeroCrossingDetector::new(doppler.zero_cross_hysteresis);
        let crossings = detector.find_all_crossings(&signal);

        // The same signal, crossed the plain way: every upward sign change,
        // interpolated, with no hysteresis and no selection. If this is
        // unbiased where the detector is not, the bias is the detector's.
        let mut plain = Vec::new();
        for (i, w) in signal.windows(2).enumerate() {
            if w[0] <= 0.0 && w[1] > 0.0 {
                let t = (i + 1) as f32 - w[1] / (w[1] - w[0]) - group_delay;
                if t >= 2000.0 {
                    let k = (t / period).round();
                    plain.push((t - k * period) as f64);
                }
            }
        }
        let plain_mean = plain.iter().sum::<f64>() / plain.len().max(1) as f64;

        // The tone rises through zero at every whole multiple of the period.
        // The filter delays that by its group delay.
        let mut errors = Vec::new();
        for c in &crossings {
            let t = c - group_delay;
            if t < 2000.0 {
                continue;
            }
            let k = (t / period).round();
            errors.push((t - k * period) as f64);
        }
        let mean = errors.iter().sum::<f64>() / errors.len().max(1) as f64;
        let sd = (errors.iter().map(|e| (e - mean) * (e - mean)).sum::<f64>()
            / errors.len().max(1) as f64)
            .sqrt();

        println!(
            "{:>8.1} {:>10} {:>12.4} {:>12.3} {:>10.4} {:>10.3} {:>10}",
            noise,
            format!("{centre_hz:.0}+-{half_width:.0}"),
            mean,
            mean * degrees_per_sample as f64,
            sd,
            amplitude,
            format!("{:.2}/cyc", upward as f32 / errors.len().max(1) as f32)
        );
        println!(
            "{:>19}  plain scan: mean {plain_mean:>9.4} over {} crossings",
            "",
            plain.len()
        );
    }
}
