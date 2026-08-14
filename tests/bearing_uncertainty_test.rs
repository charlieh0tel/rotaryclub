//! Does the stated bearing uncertainty mean anything?
//!
//! `ConfidenceMetrics::bearing_uncertainty_deg` is the one number in the
//! confidence set that makes a checkable claim, so it is worth checking. Two
//! properties matter and they pull against each other: it has to grow as the
//! signal degrades, and it must not read lower than the scatter it describes.
//!
//! The second is the one with teeth. Reducing the reference term by the
//! averaging the loop performs on top of the detections is correct as filter
//! theory and wrong here, because as the signal degrades the tick's error
//! stops being scatter and becomes a displacement the loop follows. That
//! change passes every other test in the suite and makes this figure
//! understate a worthless bearing sixteenfold.
//!
//! An earlier version of this file justified its design by a systematic bias
//! in the zero-crossing bearing method. There is no such bias: it was an
//! artifact of a noise generator that produced half a DC offset, and the two
//! bearing methods measure within hundredths of a degree of each other once
//! the noise they are judged with is noise.

use std::f32::consts::PI;

use rotaryclub::audio::AudioSource;
use rotaryclub::config::{BearingMethod, NorthTrackingMode, RdfConfig};
use rotaryclub::processing::RdfProcessor;
use rotaryclub::rdf::{BearingCalculator, CorrelationBearingCalculator, NorthTick};
use rotaryclub::simulation::{SignalImpairment, generate_impaired_signal};

const PULSE_HALF_WIDTH: i64 = 12;

fn noise_at(index: usize) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA5A5_1234_9ABC_DEF0;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    (((x >> 32) as u32) as f32 / (u32::MAX as f32)) * 2.0 - 1.0
}

/// Interleaved stereo: Doppler tone left, band-limited north pulses right.
///
/// The pulses sit at the true rotation epochs and the tone's north is at
/// sample zero, so the only bearing error present is the one the pipeline
/// makes.
fn build_signal(
    num_samples: usize,
    sample_rate: f32,
    rotation_hz: f32,
    bearing_deg: f32,
    amplitude: f32,
    noise: f32,
) -> Vec<f32> {
    let period = sample_rate as f64 / rotation_hz as f64;
    let omega = 2.0 * PI * rotation_hz / sample_rate;
    let bearing = bearing_deg.to_radians();

    let mut north = vec![0.0f32; num_samples];
    let mut k = 0i64;
    loop {
        let epoch = k as f64 * period;
        if epoch >= num_samples as f64 - PULSE_HALF_WIDTH as f64 {
            break;
        }
        k += 1;
        let center = epoch.round() as i64;
        for n in (center - PULSE_HALF_WIDTH)..=(center + PULSE_HALF_WIDTH) {
            if n < 0 || n as usize >= num_samples {
                continue;
            }
            let x = n as f64 - epoch;
            let value = if x.abs() < f64::EPSILON {
                1.0
            } else {
                let px = std::f64::consts::PI * x;
                let w = px / PULSE_HALF_WIDTH as f64;
                (px.sin() / px) * (w.sin() / w)
            };
            north[n as usize] += amplitude * value as f32;
        }
    }

    let mut out = Vec::with_capacity(num_samples * 2);
    for (i, &tick) in north.iter().enumerate() {
        out.push((omega * i as f32 - bearing).sin() + noise * noise_at(i));
        out.push(tick + noise * 0.35 * noise_at(i ^ 0x5555));
    }
    out
}

struct Run {
    mean_abs_error_deg: f64,
    /// Mean absolute deviation of the error about its own mean: the part of
    /// the error that scatters, with any constant offset removed.
    scatter_deg: f64,
    mean_stated_sigma_deg: f64,
}

fn run(method: BearingMethod, noise: f32) -> Run {
    let mut config = RdfConfig::default();
    config.doppler.method = method;
    config.bearing.smoothing_window = 1;

    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let truth = 200.0f32;
    let num_samples = (sample_rate * 4.0) as usize;

    let signal = build_signal(
        num_samples,
        sample_rate,
        rotation_hz,
        truth,
        config.north_tick.expected_pulse_amplitude,
        noise,
    );

    let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
    let results = processor.process_signal(&signal);

    let mut errors = Vec::new();
    let mut signed = Vec::new();
    let mut stated = Vec::new();
    for result in &results {
        let Some(bearing) = result.bearing else {
            continue;
        };
        let error = ((bearing.raw_bearing - truth + 540.0).rem_euclid(360.0) - 180.0) as f64;
        errors.push(error.abs());
        signed.push(error);
        if let Some(sigma) = bearing.metrics.bearing_uncertainty_deg {
            stated.push(sigma as f64);
        }
    }

    // The loop is still acquiring at the start of the run, and what it does
    // there is not what this is measuring.
    let tail = |v: &[f64]| {
        let start = v.len() / 2;
        let slice = &v[start.min(v.len().saturating_sub(1))..];
        slice.iter().sum::<f64>() / slice.len().max(1) as f64
    };

    let signed_tail = &signed[signed.len() / 2..];
    let offset = signed_tail.iter().sum::<f64>() / signed_tail.len().max(1) as f64;
    let scatter = signed_tail.iter().map(|e| (e - offset).abs()).sum::<f64>()
        / signed_tail.len().max(1) as f64;

    Run {
        mean_abs_error_deg: tail(&errors),
        scatter_deg: scatter,
        mean_stated_sigma_deg: tail(&stated),
    }
}

#[test]
fn test_stated_uncertainty_grows_with_degradation() {
    for method in [BearingMethod::Correlation, BearingMethod::ZeroCrossing] {
        let quiet = run(method, 0.0);
        let noisy = run(method, 1.0);
        let ruined = run(method, 2.0);

        assert!(
            noisy.mean_stated_sigma_deg > quiet.mean_stated_sigma_deg,
            "{method:?}: uncertainty should grow with noise, got {:.3} at rest \
             and {:.3} under noise",
            quiet.mean_stated_sigma_deg,
            noisy.mean_stated_sigma_deg
        );
        assert!(
            ruined.mean_stated_sigma_deg > noisy.mean_stated_sigma_deg,
            "{method:?}: uncertainty should keep growing as the bearing falls \
             apart, got {:.3} then {:.3}",
            noisy.mean_stated_sigma_deg,
            ruined.mean_stated_sigma_deg
        );
        // The bearing really is ruined at this point, so a figure that still
        // reads like a usable measurement is not describing it.
        assert!(
            ruined.mean_stated_sigma_deg > 5.0,
            "{method:?}: a bearing {:.1} degrees out should not claim {:.2} \
             degrees of uncertainty",
            ruined.mean_abs_error_deg,
            ruined.mean_stated_sigma_deg
        );
    }
}

/// The figure describes precision, and precision is all it can describe.
///
/// It is built from the spread of the phase estimates, so it covers the part
/// of the error that scatters and cannot cover a displacement they all share.
/// A north tick that is late by the same amount every rotation moves every
/// bearing equally and leaves the spread untouched; that is why the reference
/// contributes its own term. Asserting against the scatter is asserting what
/// the number claims, and asserting against total error would be asserting
/// something it does not.
#[test]
fn test_stated_uncertainty_does_not_understate_the_scatter() {
    for method in [BearingMethod::Correlation, BearingMethod::ZeroCrossing] {
        for noise in [0.0f32, 0.3, 0.6, 1.0, 1.5, 2.0] {
            let measured = run(method, noise);
            assert!(
                measured.mean_stated_sigma_deg >= measured.scatter_deg,
                "{method:?} at noise {noise}: claimed {:.3} degrees of \
                 uncertainty against {:.3} degrees of scatter (total error \
                 {:.3}). A figure that reads better than the truth is worse \
                 than none.",
                measured.mean_stated_sigma_deg,
                measured.scatter_deg,
                measured.mean_abs_error_deg
            );
        }
    }
}

/// An uncertainty that cannot be estimated must not be reported as zero.
///
/// A reference whose scatter is unknown was previously converted to "the
/// reference is perfect" by an `unwrap_or(0.0)`, so the worst-informed
/// moments produced the most confident output: a DPLL that had just cleared
/// its statistics after a run of rejections, and a single zero crossing with
/// no spread to measure. The contract is that the figure is absent and
/// confidence is zero, which is the absence of a claim rather than a claim of
/// excellence.
#[test]
fn test_unknown_reference_suppresses_the_uncertainty() {
    let config = RdfConfig::default();
    let sample_rate = config.audio.sample_rate as f32;
    let period = sample_rate / config.doppler.expected_freq;

    let mut calc = CorrelationBearingCalculator::new(
        &config.doppler,
        &config.agc,
        config.bearing.confidence,
        sample_rate,
        1,
    )
    .expect("calculator");

    let omega = 2.0 * PI / period;
    let buffer: Vec<f32> = (0..4800).map(|i| (omega * i as f32).sin()).collect();

    let unknown = NorthTick {
        sample_index: 0,
        period: Some(period),
        lock_quality: Some(1.0),
        phase_variance: None,
        fractional_sample_offset: 0.0,
        phase: 0.0,
        frequency: omega,
    };
    let measured = calc
        .process_buffer(&buffer, &unknown)
        .expect("a bearing is still produced");
    assert!(
        measured.metrics.bearing_uncertainty_deg.is_none(),
        "an unknown reference must suppress the uncertainty, got {:?}",
        measured.metrics.bearing_uncertainty_deg
    );
    assert_eq!(
        measured.confidence, 0.0,
        "an unestimatable uncertainty must score zero confidence, not one"
    );

    // The same signal against a reference that does know its scatter.
    let known = NorthTick {
        phase_variance: Some(0.0),
        ..unknown
    };
    let measured = calc
        .process_buffer(&buffer, &known)
        .expect("a bearing is still produced");
    assert!(
        measured.metrics.bearing_uncertainty_deg.is_some(),
        "a known reference must produce an uncertainty"
    );
}

/// The simple tracker estimates its own timing scatter from its intervals.
///
/// It has no oscillator to compare a detection against, but if each tick has
/// timing variance v then the interval between two has variance 2v, so the
/// interval scatter it already measures for its period estimate carries what
/// the bearing uncertainty needs. Reporting nothing instead suppressed the
/// figure and every confidence built on it for the whole mode.
#[test]
fn test_simple_tracker_reports_a_reference_scatter() {
    let mut config = RdfConfig::default();
    config.north_tick.mode = NorthTrackingMode::Simple;
    config.bearing.smoothing_window = 1;

    let sample_rate = config.audio.sample_rate as f32;
    let signal = build_signal(
        (sample_rate * 2.0) as usize,
        sample_rate,
        config.doppler.expected_freq,
        200.0,
        config.north_tick.expected_pulse_amplitude,
        0.0,
    );

    let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
    let results = processor.process_signal(&signal);
    let bearings: Vec<_> = results.iter().filter_map(|r| r.bearing).collect();
    assert!(bearings.len() > 100, "expected a run of bearings");

    // Skip the opening, where there are not yet two intervals to compare.
    let settled = &bearings[bearings.len() / 2..];
    for bearing in settled {
        assert!(
            bearing.metrics.bearing_uncertainty_deg.is_some(),
            "the simple tracker should report an uncertainty once it has              intervals to measure"
        );
        assert!(
            bearing.confidence > 0.0,
            "and a confidence built on it, got {}",
            bearing.confidence
        );
    }
}

/// A predicted tick must report a growing uncertainty as it coasts.
///
/// A coasted tick is not as good as a measured one and its error grows with
/// every rotation predicted -- the budget exists precisely to bound that at
/// half a sample, which is six degrees of bearing. Reporting the last
/// measured scatter unchanged, as this did, left a tick predicted a full
/// second ago claiming what one just detected claims.
///
/// This asserts on the tick's own reported variance rather than on the
/// bearing confidence. An earlier version of this test checked confidence
/// end to end and passed with the defect still present, because confidence
/// drifts during a dropout for unrelated reasons; only the tick's variance
/// isolates the behaviour under test.
#[test]
fn test_coasted_ticks_report_growing_uncertainty() {
    let mut config = RdfConfig::default();
    config.bearing.smoothing_window = 1;

    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let settle = (sample_rate * 3.0) as usize;
    let total = settle + (sample_rate * 0.5) as usize;

    // Pulses stop halfway; the Doppler tone continues.
    let mut signal = build_signal(
        total,
        sample_rate,
        rotation_hz,
        200.0,
        config.north_tick.expected_pulse_amplitude,
        0.0,
    );
    for i in settle..total {
        signal[i * 2 + 1] = 0.0;
    }

    let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
    let results = processor.process_signal(&signal);

    let coasted: Vec<f32> = results
        .iter()
        .filter(|r| r.north_tick.sample_index > settle)
        .filter_map(|r| r.north_tick.phase_variance)
        .collect();

    assert!(
        coasted.len() > 20,
        "expected the tracker to coast over the dropout, got {} ticks",
        coasted.len()
    );

    let first = coasted[0];
    let last = *coasted.last().expect("a coasted tick");
    assert!(
        last > first * 1.5,
        "the reported variance should grow as the coast lengthens: {last} at \
         the end against {first} at the start"
    );
}

/// The stated uncertainty must bracket the scatter of the reported bearings,
/// without wild over-caution, on real captures.
///
/// The synthetic tests above check the shape of the figure. This checks its
/// calibration against signal nobody synthesized, which is the only place the
/// independence assumption can be judged: the bandpass carries about its own
/// length of history, so estimates taken closer together than its impulse
/// response are not separate looks at the bearing.
///
/// Getting that wrong is not subtle. Counting every rotation as independent
/// understates the scatter by half; counting none of them overstates it
/// threefold. Both were shipped at different points, the second because the
/// evidence against the first came from a noise generator that turned out to
/// be half a DC offset.
///
/// Measured, stated against actual: 1.04 and 0.97 on the two wouxun captures,
/// and 0.60 on the ft-70d one. The last is the badly degraded capture, whose
/// bearings scatter by 55 degrees, and it understates. That is the reference
/// term's known limit rather than the independence model's: as a signal
/// degrades the tick's error stops being scatter and becomes a displacement
/// the loop follows, which nothing inside the tracker can see. Both numbers
/// say the same thing about that capture -- 33 degrees and 55 degrees are
/// each far past usable -- but the direction of the error is the unsafe one
/// and it is worth knowing where it sets in.
#[test]
fn test_uncertainty_is_calibrated_against_real_captures() {
    let captures: Vec<std::path::PathBuf> = match std::fs::read_dir("data") {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "wav"))
            .collect(),
        // The captures are not redistributed with every checkout.
        Err(_) => return,
    };
    if captures.is_empty() {
        return;
    }

    for capture in captures {
        let mut config = RdfConfig::default();
        config.bearing.smoothing_window = 1;
        let Ok(mut source) =
            rotaryclub::audio::WavFileSource::new(&capture, config.audio.buffer_size)
        else {
            continue;
        };

        let mut processor = RdfProcessor::new(&config, false, true).expect("processor");
        let mut raw = Vec::new();
        let mut stated = Vec::new();
        while let Ok(Some(buffer)) = AudioSource::next_buffer(&mut source) {
            for result in processor.process_audio(&buffer) {
                if let Some(bearing) = result.bearing
                    && let Some(u) = bearing.metrics.bearing_uncertainty_deg
                {
                    raw.push(bearing.raw_bearing as f64);
                    stated.push(u as f64);
                }
            }
        }
        if raw.len() < 2000 {
            continue;
        }

        // Scatter of the reported bearings about a local mean, so that a
        // capture whose bearing wanders does not read the wander as error.
        // It wanders a long way: about the mean of a whole capture these
        // three scatter by roughly ninety degrees, which is fading and
        // multipath rather than anything the estimator did.
        //
        // The window has to be long. At sixty-four reports it was measuring
        // the doppler term alone: an error in the north epoch displaces a
        // whole run of consecutive bearings together, so a short window
        // subtracts the reference term off with the local mean and never sees
        // it. Checked against synthetic signal, where the truth is known and
        // needs no estimator at all, a sixty-four window reads 2.2 times low
        // and this one reads 0.93 of the true scatter.
        //
        // The ratio is then formed inside each window and summarised, rather
        // than summarising each side across the whole capture and dividing.
        // Both quantities are strongly skewed here -- a few bad stretches
        // carry most of the scatter and most of the stated uncertainty -- so
        // a median taken over all reports lands in the quiet part of the run
        // while a median taken over windows does not. Done the wrong way
        // round these captures read 0.16; paired, they read 0.6 to 0.7.
        let window = 512usize;
        let mut ratios = Vec::new();
        for (bearings, claimed) in raw.chunks(window).zip(stated.chunks(window)) {
            if bearings.len() < window {
                break;
            }
            let (mut c, mut s) = (0.0f64, 0.0f64);
            for b in bearings {
                let r = b.to_radians();
                c += r.cos();
                s += r.sin();
            }
            let mean = s.atan2(c);
            let variance = bearings
                .iter()
                .map(|b| {
                    let d = (b.to_radians() - mean)
                        .sin()
                        .atan2((b.to_radians() - mean).cos());
                    d * d
                })
                .sum::<f64>()
                / bearings.len() as f64;
            let actual = variance.sqrt().to_degrees();
            let mut claimed = claimed.to_vec();
            claimed.sort_by(f64::total_cmp);
            ratios.push(claimed[claimed.len() / 2] / actual.max(f64::EPSILON));
        }
        ratios.sort_by(f64::total_cmp);
        let ratio = ratios[ratios.len() / 2];

        let name = capture.file_name().unwrap_or_default().to_string_lossy();
        assert!(
            (0.4..2.0).contains(&ratio),
            "{name}: the stated uncertainty runs at {ratio:.2} of the bearing \
             scatter actually seen. Far below 1 the figure claims better than \
             it delivers; far above it, it is so cautious it carries no \
             information."
        );
    }
}

/// The same calibration, on synthetic signal at the three impairment levels
/// the recordings sit at.
///
/// Weaker evidence than the captures, and deliberately kept alongside them
/// rather than instead of them: here the noise is one we chose, so the test
/// can only confirm the model against an assumption. What it adds is a sweep
/// -- the captures give three fixed points and no control -- and a check that
/// runs without `data/` present.
///
/// It is only worth anything because the synthetic doppler channel now carries
/// interference at all. It used to be the rotation tone and nothing else, so
/// the phase spread that dominates this figure on real signal was absent, and
/// every synthetic measurement of it was measuring the reference term alone.
///
/// It is also the stronger measurement of the two, for one reason: the true
/// bearing is known here, so the scatter is taken about it directly and no
/// estimator stands between the measurement and the answer. The capture test
/// has to infer a reference from the reports themselves.
///
/// The figure still reads more conservative here than on the recordings --
/// about 1.3 against about 0.9 -- for the same reason the perf scenarios are
/// harsher than the captures at matched passband power: flat noise scatters a
/// per-rotation phase estimate more than shaped audio does.
#[test]
fn test_uncertainty_is_calibrated_across_impairment() {
    let config = RdfConfig::default();
    let truth = 200.0f32;

    let mut previous_uncertainty = 0.0f64;

    // The three recordings measure 0.199, 0.793 and 6.579.
    for ratio in [0.2f32, 0.8, 6.5] {
        let signal = generate_impaired_signal(
            6.0,
            config.audio.sample_rate,
            config.doppler.expected_freq,
            |_| truth,
            SignalImpairment::at_passband_ratio(ratio),
        );

        let mut run = RdfConfig::default();
        run.bearing.smoothing_window = 1;
        let mut processor = RdfProcessor::new(&run, false, true).expect("processor");
        let results = processor.process_signal(&signal);

        let mut raw = Vec::new();
        let mut stated = Vec::new();
        for result in &results {
            if let Some(bearing) = result.bearing
                && let Some(u) = bearing.metrics.bearing_uncertainty_deg
            {
                raw.push(bearing.raw_bearing as f64);
                stated.push(u as f64);
            }
        }
        assert!(
            raw.len() > 2000,
            "expected a run of bearings at ratio {ratio}, got {}",
            raw.len()
        );

        // Scatter about the bearing the signal was generated at. The
        // capture test cannot do this and has to use a windowed local mean
        // instead; where both can be run, that estimator reads low, so
        // prefer this one wherever the truth is available.
        let reference = (truth as f64).to_radians();
        let variance = raw
            .iter()
            .map(|b| {
                let d = (b.to_radians() - reference)
                    .sin()
                    .atan2((b.to_radians() - reference).cos());
                d * d
            })
            .sum::<f64>()
            / raw.len() as f64;
        let actual = variance.sqrt().to_degrees();

        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let claimed = median(&mut stated);

        assert!(
            claimed > previous_uncertainty,
            "the stated uncertainty should grow with impairment: {claimed:.2} at \
             a passband ratio of {ratio} against {previous_uncertainty:.2} at the \
             level below"
        );
        previous_uncertainty = claimed;

        let calibration = claimed / actual;
        assert!(
            (0.7..2.0).contains(&calibration),
            "at a passband noise ratio of {ratio}: stated {claimed:.2} deg \
             against an actual bearing scatter of {actual:.2}, a ratio of \
             {calibration:.2}"
        );
    }
}
