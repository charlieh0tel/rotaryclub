use rotaryclub::config::{BearingMethod, NorthTrackingMode, RdfConfig};
use rotaryclub::processing::RdfProcessor;
use rotaryclub::signal_processing::FirBandpass;
use rotaryclub::simulation::noise_at as deterministic_noise_at;
use std::f32::consts::PI;
use std::time::Instant;

const BUFFER_SIZES: &[usize] = &[128, 256, 512];
const ITERATIONS: usize = 180;
const WARMUP_ITERATIONS: usize = 24;
const TICK_MATCH_TOLERANCE_SAMPLES: f32 = 2.0;

#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    amplitude: f32,
    /// White noise added to the north channel, as a peak amplitude. White is
    /// right here: the north highpass at 1 kHz passes most of the spectrum,
    /// so what is generated is close to what the detector meets.
    noise_peak: f32,
    /// Interfering audio power *inside the doppler passband*, relative to the
    /// rotation tone. Measured on the recordings in `data/`: 0.199, 0.793 and
    /// 6.579.
    ///
    /// This used to be a peak amplitude of white noise, which impaired
    /// almost nothing. The doppler bandpass is 500 Hz of 24 kHz, so 98 percent
    /// of white noise is thrown away before it reaches anything, and the
    /// scenario named for low SNR ran at an in-band tone fraction of 0.998.
    ///
    /// It is also not the ratio over the whole channel, which was the next
    /// thing tried and was wrong in the other direction. Real audio is
    /// concentrated well below the doppler band, so matching total power
    /// with flat voice-band noise puts ten times too much where it hurts:
    /// the cleanest recording measures 0.075 of the channel as tone and
    /// achieves 1.6 degrees, while flat noise at that same total ratio gave
    /// 20.7. What decides the bearing is the power in the passband, so that
    /// is what this names and what the generator scales to.
    doppler_passband_noise_to_tone: f32,
    dc_offset: f32,
    second_tone_ratio: f32,
    third_tone_ratio: f32,
    north_jitter_samples: i32,
    north_dropout_stride: Option<usize>,
    north_impulse_stride: Option<usize>,
    north_impulse_amplitude: f32,
}

#[derive(Clone, Copy)]
struct Metrics {
    bearing_success_rate: f32,
    detection_rate: f32,
    false_positive_rate: f32,
    mean_us_per_sample: f64,
    p95_us_per_sample: f64,
    mean_abs_bearing_error_deg: f32,
    p95_abs_bearing_error_deg: f32,
    max_abs_bearing_error_deg: f32,
    mean_abs_tick_error_samples: f32,
    p95_abs_tick_error_samples: f32,
}

/// Per-pulse timing jitter, in samples, as a fraction of `max_abs_jitter`.
///
/// White and fractional. It used to be `sin(0.37 k).round()`, which repeats
/// every 17 rotations: a coherent 94 Hz modulation, forty-seven times the
/// loop bandwidth, which any second-order loop rejects by construction. The
/// scenario therefore measured the stimulus being out of band rather than the
/// tracker doing anything, and the DPLL's advantage in it was not earned.
/// Real jitter has in-band content the loop has to follow.
fn deterministic_jitter_samples(index: usize, max_abs_jitter: i32, draw: u64) -> f64 {
    if max_abs_jitter <= 0 {
        0.0
    } else {
        deterministic_noise_at(index, 0x51D3_7A19_C0DE_2B4Fu64.wrapping_add(draw)) as f64
            * max_abs_jitter as f64
    }
}

fn angle_error_deg(measured: f32, expected: f32) -> f32 {
    let mut err = (measured - expected).abs();
    if err > 180.0 {
        err = 360.0 - err;
    }
    err
}

fn percentile_f32(values: &[f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let idx = ((sorted.len() as f32 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

fn percentile_f64(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let idx = ((sorted.len() as f64 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

fn mean_f32(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

fn mean_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// Half-width, in samples, of the synthesized north pulse.
const NORTH_PULSE_HALF_WIDTH: i64 = 12;

/// Rotation epochs, which are generally fractional.
///
/// Rounding these to the nearest sample would put the reference up to half a
/// sample from where the rotation actually crosses north, which is six degrees
/// of bearing. It would also make the two metrics here disagree about truth: a
/// tracker that recovers the true epoch would score as half a sample of tick
/// error while producing the better bearing.
/// Returns the epochs a pulse is rendered at, and every rotation epoch.
///
/// These differ when a scenario drops pulses. A tracker cannot detect a pulse
/// that was never emitted, so detection is scored against the first. But a
/// DPLL predicting a tick where a dropped pulse belonged is doing exactly what
/// holdover is for, and scoring that as a false positive -- which this did,
/// because it kept only one list -- charges the loop for working. At a dropout
/// stride of 17 that is one spurious false positive per 16 pulses, 5.9%,
/// which is most of the low SNR false positive rate the gate was tuned around.
fn expected_tick_positions(
    total_samples: usize,
    samples_per_rotation: f64,
    scenario: Scenario,
    draw: u64,
) -> (Vec<f64>, Vec<f64>) {
    let mut jittered = Vec::new();
    let mut k = 0usize;
    loop {
        let epoch = k as f64 * samples_per_rotation;
        if epoch >= total_samples as f64 {
            break;
        }
        let jitter = deterministic_jitter_samples(k, scenario.north_jitter_samples, draw);
        jittered.push((epoch + jitter).clamp(0.0, total_samples as f64 - 1.0));
        k += 1;
    }
    jittered.sort_by(f64::total_cmp);
    // Jitter can push two epochs together; the detector's dead time means a
    // pair closer than a sample is one pulse, not two.
    jittered.dedup_by(|a, b| (*a - *b).abs() < 1.0);

    if let Some(stride) = scenario.north_dropout_stride
        && stride > 1
    {
        let rendered = jittered
            .iter()
            .enumerate()
            .filter_map(|(i, p)| if i % stride == 0 { None } else { Some(*p) })
            .collect();
        return (rendered, jittered);
    }

    (jittered.clone(), jittered)
}

/// A band-limited impulse at a fractional sample position, as an anti-aliased
/// converter records a pulse far shorter than a sample.
fn north_pulse_at(global: usize, epoch: f64) -> f32 {
    let x = global as f64 - epoch;
    if x.abs() > NORTH_PULSE_HALF_WIDTH as f64 {
        return 0.0;
    }
    if x.abs() < f64::EPSILON {
        return 1.0;
    }
    let px = std::f64::consts::PI * x;
    let window = px / NORTH_PULSE_HALF_WIDTH as f64;
    ((px.sin() / px) * (window.sin() / window)) as f32
}

/// Interfering audio for the doppler channel, band-limited to the voice band
/// and scaled to the requested power against the tone.
///
/// Generated once for the whole run rather than per chunk, so the band
/// limiting is continuous across chunk boundaries.
fn doppler_audio(
    total_samples: usize,
    sample_rate: f32,
    scenario: Scenario,
    draw: u64,
) -> Vec<f32> {
    if scenario.doppler_passband_noise_to_tone <= 0.0 {
        return vec![0.0f32; total_samples];
    }
    let mut audio: Vec<f32> = (0..total_samples)
        .map(|i| deterministic_noise_at(i, 0x0DDB_A11A_5EED_1234u64.wrapping_add(draw)))
        .collect();
    if let Ok(mut band) = FirBandpass::new(300.0, 3400.0, sample_rate, 255, 150.0) {
        band.process_buffer(&mut audio);
    }

    // Scale by what lands in the doppler passband, not by total power --
    // measured exactly by the shared estimator rather than through a probe
    // FIR whose skirts credited out-of-band audio as in-band (0.57 dB low,
    // at every level). One implementation, shared with the generator in
    // src/simulation, so the two cannot drift apart again.
    let passband_power = rotaryclub::simulation::in_band_power(&audio, sample_rate, 1350.0, 1850.0);
    // A tone of amplitude a has power a^2 / 2.
    let tone_power = (scenario.amplitude * scenario.amplitude / 2.0) as f64;
    let wanted = tone_power * scenario.doppler_passband_noise_to_tone as f64;
    let scale = if passband_power > 0.0 {
        (wanted / passband_power).sqrt() as f32
    } else {
        0.0
    };
    for sample in audio.iter_mut() {
        *sample *= scale;
    }
    audio
}

#[allow(clippy::too_many_arguments)]
fn build_chunk(
    scenario: Scenario,
    chunk_start: usize,
    chunk_size: usize,
    expected_bearing_deg: f32,
    sample_rate: f32,
    rotation_hz: f32,
    tick_positions: &[f64],
    doppler_audio: &[f32],
    draw: u64,
) -> Vec<f32> {
    let omega = 2.0 * PI * rotation_hz / sample_rate;
    let bearing_rad = expected_bearing_deg.to_radians();
    let mut out = Vec::with_capacity(chunk_size * 2);

    // A pulse centred outside this chunk still reaches into it, so the window
    // of epochs to render is wider than the chunk.
    let reach = NORTH_PULSE_HALF_WIDTH as f64;
    let low = tick_positions.partition_point(|&e| e < chunk_start as f64 - reach);
    let high = tick_positions.partition_point(|&e| e <= (chunk_start + chunk_size) as f64 + reach);
    let nearby = &tick_positions[low..high];

    for i in 0..chunk_size {
        let global = chunk_start + i;
        let t = global as f32;
        let p = omega * t - bearing_rad;

        let fundamental = p.sin();
        let second = (2.0 * p).sin();
        let third = (3.0 * p).sin();
        let doppler = scenario.amplitude * fundamental
            + scenario.second_tone_ratio * second
            + scenario.third_tone_ratio * third
            + doppler_audio.get(global).copied().unwrap_or(0.0)
            + scenario.dc_offset;

        let mut north = 0.8
            * nearby
                .iter()
                .map(|&epoch| north_pulse_at(global, epoch))
                .sum::<f32>();
        north += deterministic_noise_at(global, 0xFEED_9876_5432_1001u64.wrapping_add(draw))
            * (scenario.noise_peak * 0.35);
        north += scenario.dc_offset * 0.25;
        if let Some(stride) = scenario.north_impulse_stride
            && stride > 0
            && global % stride == stride / 2
        {
            north += scenario.north_impulse_amplitude;
        }

        out.push(doppler);
        out.push(north);
    }

    out
}

/// Detection and false positive rates.
///
/// `expected` are the pulses actually rendered; `predictable` additionally
/// includes rotations whose pulse was dropped, where a predicted tick is
/// correct behaviour rather than a false alarm.
fn compute_detection_metrics(
    expected: &[f32],
    predictable: &[f32],
    detected: &[f32],
) -> (f32, f32, Vec<f32>) {
    let mut i = 0usize;
    let mut j = 0usize;
    let mut matched = 0usize;
    let mut errors = Vec::new();

    while i < expected.len() && j < detected.len() {
        let err = (detected[j] - expected[i]).abs();
        if err <= TICK_MATCH_TOLERANCE_SAMPLES {
            matched += 1;
            errors.push(err);
            i += 1;
            j += 1;
        } else if detected[j] < expected[i] {
            j += 1;
        } else {
            i += 1;
        }
    }

    // An unmatched detection sitting on a rotation whose pulse was dropped is
    // holdover doing its job, not a false alarm.
    let unmatched = detected.len().saturating_sub(matched);
    let over_dropouts = detected
        .iter()
        .filter(|d| {
            let near = |set: &[f32]| {
                set.iter()
                    .any(|e| (e - **d).abs() <= TICK_MATCH_TOLERANCE_SAMPLES)
            };
            !near(expected) && near(predictable)
        })
        .count();

    let denom = expected.len().max(1) as f32;
    let false_pos = unmatched.saturating_sub(over_dropouts) as f32 / denom;
    let det_rate = matched as f32 / denom;
    (det_rate, false_pos, errors)
}

fn run_case(
    north_mode: NorthTrackingMode,
    bearing_method: BearingMethod,
    scenario: Scenario,
    buffer_size: usize,
    expected_bearing_deg: f32,
    draw: u64,
) -> Metrics {
    let mut config = RdfConfig::default();
    config.north_tick.mode = north_mode;
    config.doppler.method = bearing_method;
    config.audio.buffer_size = buffer_size;
    config.bearing.smoothing_window = 1;

    let sample_rate = config.audio.sample_rate as f32;
    let rotation_hz = config.doppler.expected_freq;
    let samples_per_rotation = sample_rate as f64 / rotation_hz as f64;

    let total_chunks = WARMUP_ITERATIONS + ITERATIONS;
    let total_samples = total_chunks * buffer_size;
    let (tick_positions, predictable_positions) =
        expected_tick_positions(total_samples, samples_per_rotation, scenario, draw);
    let audio = doppler_audio(total_samples, sample_rate, scenario, draw);

    let mut processor = RdfProcessor::new(&config, true, true).expect("rdf processor creation");

    let mut times_us_per_sample = Vec::with_capacity(ITERATIONS);
    let mut bearing_errors = Vec::new();
    let mut detected_ticks = Vec::new();

    for step in 0..total_chunks {
        let start = step * buffer_size;
        let chunk = build_chunk(
            scenario,
            start,
            buffer_size,
            expected_bearing_deg,
            sample_rate,
            rotation_hz,
            &tick_positions,
            &audio,
            draw,
        );

        let t0 = Instant::now();
        let results = processor.process_audio(&chunk);
        let elapsed_us = t0.elapsed().as_secs_f64() * 1_000_000.0;

        if step >= WARMUP_ITERATIONS {
            times_us_per_sample.push(elapsed_us / buffer_size as f64);
            for r in results {
                detected_ticks
                    .push(r.north_tick.sample_index as f32 + r.north_tick.fractional_sample_offset);
                if let Some(b) = r.bearing {
                    bearing_errors.push(angle_error_deg(b.raw_bearing, expected_bearing_deg));
                }
            }
        }
    }

    let measurement_start = WARMUP_ITERATIONS * buffer_size;
    let expected_ticks: Vec<f32> = tick_positions
        .iter()
        .filter(|&&x| x >= measurement_start as f64)
        .map(|&x| x as f32)
        .collect();
    let predictable_ticks: Vec<f32> = predictable_positions
        .iter()
        .filter(|&&x| x >= measurement_start as f64)
        .map(|&x| x as f32)
        .collect();
    let (detection_rate, false_positive_rate, tick_errors) =
        compute_detection_metrics(&expected_ticks, &predictable_ticks, &detected_ticks);

    let bearing_success_rate = if expected_ticks.is_empty() {
        0.0
    } else {
        (bearing_errors.len() as f32 / expected_ticks.len() as f32).min(1.0)
    };

    Metrics {
        bearing_success_rate,
        detection_rate,
        false_positive_rate,
        mean_us_per_sample: mean_f64(&times_us_per_sample),
        p95_us_per_sample: percentile_f64(&times_us_per_sample, 0.95),
        mean_abs_bearing_error_deg: if bearing_errors.is_empty() {
            360.0
        } else {
            mean_f32(&bearing_errors)
        },
        p95_abs_bearing_error_deg: if bearing_errors.is_empty() {
            360.0
        } else {
            percentile_f32(&bearing_errors, 0.95)
        },
        max_abs_bearing_error_deg: if bearing_errors.is_empty() {
            360.0
        } else {
            bearing_errors.iter().copied().fold(0.0f32, f32::max)
        },
        mean_abs_tick_error_samples: mean_f32(&tick_errors),
        p95_abs_tick_error_samples: percentile_f32(&tick_errors, 0.95),
    }
}

fn north_mode_name(mode: NorthTrackingMode) -> &'static str {
    match mode {
        NorthTrackingMode::Dpll => "dpll",
        NorthTrackingMode::Simple => "simple",
    }
}

fn bearing_method_name(method: BearingMethod) -> &'static str {
    match method {
        BearingMethod::Correlation => "correlation",
        BearingMethod::ZeroCrossing => "zero_crossing",
    }
}

/// Independent noise realisations averaged into each reported row.
///
/// One draw is not a measurement: this scenario set contains a row that was
/// bimodal, reading either 0.99 or 0.49 and never the 0.894 its mean claimed.
///
/// Four rather than sixteen, and the number is computed rather than picked.
/// The requirement is that noise must not trip a limit, so for each row and
/// each checked metric the count needed to keep a three-sigma excursion inside
/// the margin is (3 sd / margin)^2. Measured over 32 draws, the largest such
/// requirement across the whole matrix is 1.3.
///
/// Eight rather than four, because the sizing above only asks that noise not
/// trip a limit. `unsupported_metrics` asks the stronger question -- whether a
/// row passes by more than its own spread -- and three rows failed it at four
/// draws, all of them extreme-value columns, which are the most volatile
/// thing here.
///
/// Two things that calculation deliberately ignores. Timing spread is machine
/// load, which no number of draws averages away. And the bearing-error limits
/// on the near-uniform scenarios sit at or above 180 degrees, which a bearing
/// error cannot exceed, so those checks cannot fail and should not size
/// anything; excluded, the requirement falls from 14 to 1.3.
///
/// This is tied to the limits: tightening one shrinks its margin and raises
/// the count needed. Re-run the sizing from the `spread` lines on stderr if
/// a limit moves.
const DRAWS: u64 = 8;

/// Standard error of the mean of one column over the draws.
fn se_of(runs: &[Metrics], f: fn(&Metrics) -> f32) -> f32 {
    let n = runs.len() as f32;
    if n < 2.0 {
        return 0.0;
    }
    let mean = runs.iter().map(f).sum::<f32>() / n;
    let var = runs
        .iter()
        .map(|r| (f(r) - mean) * (f(r) - mean))
        .sum::<f32>()
        / (n - 1.0);
    (var / n).sqrt()
}

fn average_metrics(runs: &[Metrics]) -> Metrics {
    let n = runs.len() as f64;
    let mean64 = |f: fn(&Metrics) -> f64| runs.iter().map(f).sum::<f64>() / n;
    let mean32 = |f: fn(&Metrics) -> f32| runs.iter().map(f).sum::<f32>() / n as f32;
    Metrics {
        bearing_success_rate: mean32(|m| m.bearing_success_rate),
        detection_rate: mean32(|m| m.detection_rate),
        false_positive_rate: mean32(|m| m.false_positive_rate),
        mean_us_per_sample: mean64(|m| m.mean_us_per_sample),
        p95_us_per_sample: mean64(|m| m.p95_us_per_sample),
        mean_abs_bearing_error_deg: mean32(|m| m.mean_abs_bearing_error_deg),
        p95_abs_bearing_error_deg: mean32(|m| m.p95_abs_bearing_error_deg),
        max_abs_bearing_error_deg: mean32(|m| m.max_abs_bearing_error_deg),
        mean_abs_tick_error_samples: mean32(|m| m.mean_abs_tick_error_samples),
        p95_abs_tick_error_samples: mean32(|m| m.p95_abs_tick_error_samples),
    }
}

/// Standard error of the mean for the accuracy columns, to stderr.
///
/// Not in the CSV, which the gate script parses; this is for choosing how many
/// draws a row needs and for seeing which rows are unstable.
fn report_spread(mode: &str, method: &str, scenario: &str, buffer: usize, runs: &[Metrics]) {
    let n = runs.len() as f32;
    let se = |f: fn(&Metrics) -> f32| {
        let m = runs.iter().map(f).sum::<f32>() / n;
        let var = runs.iter().map(|r| (f(r) - m) * (f(r) - m)).sum::<f32>() / (n - 1.0).max(1.0);
        (var / n).sqrt()
    };
    // Every metric a limit is checked against, so the draw count can be
    // chosen from the spread rather than guessed. The timing columns are
    // deliberately absent: their spread is machine load, which more draws do
    // not average away.
    let mean = |f: fn(&Metrics) -> f32| runs.iter().map(f).sum::<f32>() / n;
    eprintln!(
        "spread {mode},{method},{scenario},{buffer}\
         ,{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}\
         ,{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
        mean(|m| m.bearing_success_rate),
        se(|m| m.bearing_success_rate),
        mean(|m| m.detection_rate),
        se(|m| m.detection_rate),
        mean(|m| m.false_positive_rate),
        se(|m| m.false_positive_rate),
        mean(|m| m.mean_abs_bearing_error_deg),
        se(|m| m.mean_abs_bearing_error_deg),
        mean(|m| m.p95_abs_bearing_error_deg),
        se(|m| m.p95_abs_bearing_error_deg),
        mean(|m| m.max_abs_bearing_error_deg),
        se(|m| m.max_abs_bearing_error_deg),
        mean(|m| m.mean_abs_tick_error_samples),
        se(|m| m.mean_abs_tick_error_samples),
        mean(|m| m.p95_abs_tick_error_samples),
        se(|m| m.p95_abs_tick_error_samples),
    );
}

fn main() {
    let scenarios = [
        Scenario {
            name: "clean",
            doppler_passband_noise_to_tone: 0.0,
            amplitude: 1.0,
            noise_peak: 0.0,
            dc_offset: 0.0,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
            north_jitter_samples: 0,
            north_dropout_stride: None,
            north_impulse_stride: None,
            north_impulse_amplitude: 0.0,
        },
        Scenario {
            name: "noisy_jittered",
            doppler_passband_noise_to_tone: 0.8,
            amplitude: 0.9,
            noise_peak: 0.08,
            dc_offset: 0.0,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
            north_jitter_samples: 1,
            north_dropout_stride: None,
            north_impulse_stride: None,
            north_impulse_amplitude: 0.0,
        },
        Scenario {
            name: "harmonic_contaminated",
            doppler_passband_noise_to_tone: 0.2,
            amplitude: 0.9,
            noise_peak: 0.04,
            dc_offset: 0.0,
            second_tone_ratio: 0.20,
            third_tone_ratio: 0.12,
            north_jitter_samples: 0,
            north_dropout_stride: None,
            north_impulse_stride: Some(211),
            north_impulse_amplitude: 0.22,
        },
        Scenario {
            name: "low_snr_dc",
            doppler_passband_noise_to_tone: 6.5,
            amplitude: 0.45,
            noise_peak: 0.40,
            dc_offset: 0.20,
            second_tone_ratio: 0.0,
            third_tone_ratio: 0.0,
            north_jitter_samples: 1,
            north_dropout_stride: Some(17),
            north_impulse_stride: Some(97),
            north_impulse_amplitude: 0.30,
        },
    ];

    let north_modes = [NorthTrackingMode::Dpll, NorthTrackingMode::Simple];
    let bearing_methods = [BearingMethod::Correlation, BearingMethod::ZeroCrossing];
    let expected_bearing_deg = 62.0;

    for north_mode in north_modes {
        for bearing_method in bearing_methods {
            for scenario in scenarios {
                for &buffer_size in BUFFER_SIZES {
                    let draws: Vec<Metrics> = (0..DRAWS)
                        .map(|d| {
                            run_case(
                                north_mode,
                                bearing_method,
                                scenario,
                                buffer_size,
                                expected_bearing_deg,
                                d,
                            )
                        })
                        .collect();
                    let m = average_metrics(&draws);
                    report_spread(
                        north_mode_name(north_mode),
                        bearing_method_name(bearing_method),
                        scenario.name,
                        buffer_size,
                        &draws,
                    );
                    println!(
                        "{}",
                        serde_json::json!({
                            "north_mode": north_mode_name(north_mode),
                            "bearing_method": bearing_method_name(bearing_method),
                            "scenario": scenario.name,
                            "buffer_size": buffer_size,
                            "bearing_success_rate": m.bearing_success_rate,
                            "detection_rate": m.detection_rate,
                            "false_positive_rate": m.false_positive_rate,
                            "mean_us_per_sample": m.mean_us_per_sample,
                            "p95_us_per_sample": m.p95_us_per_sample,
                            "mean_abs_bearing_error_deg": m.mean_abs_bearing_error_deg,
                            "p95_abs_bearing_error_deg": m.p95_abs_bearing_error_deg,
                            "max_abs_bearing_error_deg": m.max_abs_bearing_error_deg,
                            "mean_abs_tick_error_samples": m.mean_abs_tick_error_samples,
                            "p95_abs_tick_error_samples": m.p95_abs_tick_error_samples,
                            // Emitted so `check` can ask whether this row is
                            // precise enough to support the verdict, not just
                            // whether the value is inside the limit.
                            "bearing_success_rate_se": se_of(&draws, |m| m.bearing_success_rate),
                            "detection_rate_se": se_of(&draws, |m| m.detection_rate),
                            "false_positive_rate_se": se_of(&draws, |m| m.false_positive_rate),
                            "mean_abs_bearing_error_deg_se":
                                se_of(&draws, |m| m.mean_abs_bearing_error_deg),
                            "p95_abs_bearing_error_deg_se":
                                se_of(&draws, |m| m.p95_abs_bearing_error_deg),
                            "max_abs_bearing_error_deg_se":
                                se_of(&draws, |m| m.max_abs_bearing_error_deg),
                            "mean_abs_tick_error_samples_se":
                                se_of(&draws, |m| m.mean_abs_tick_error_samples),
                            "p95_abs_tick_error_samples_se":
                                se_of(&draws, |m| m.p95_abs_tick_error_samples),
                            "draws": DRAWS,
                        })
                    );
                }
            }
        }
    }
}
