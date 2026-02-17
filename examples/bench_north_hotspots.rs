use rotaryclub::signal_processing::{FirFilterCore, PeakDetector};
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone)]
struct OldPeakDetectorRescan {
    threshold: f32,
    min_samples_between_peaks: usize,
    peak_search_window_samples: usize,
    samples_since_peak: usize,
    last_sample: f32,
    above_threshold: bool,
}

impl OldPeakDetectorRescan {
    fn new(threshold: f32, min_interval_samples: usize, peak_search_window_samples: usize) -> Self {
        Self {
            threshold,
            min_samples_between_peaks: min_interval_samples,
            peak_search_window_samples: peak_search_window_samples.max(1),
            samples_since_peak: min_interval_samples,
            last_sample: 0.0,
            above_threshold: false,
        }
    }

    fn detect_peak(&mut self, sample: f32) -> bool {
        self.samples_since_peak += 1;
        let crossed_threshold = !self.above_threshold
            && self.last_sample <= self.threshold
            && sample > self.threshold
            && self.samples_since_peak >= self.min_samples_between_peaks;
        self.above_threshold = sample > self.threshold;
        self.last_sample = sample;
        if crossed_threshold {
            self.samples_since_peak = 0;
        }
        crossed_threshold
    }

    fn find_all_peaks(&mut self, buffer: &[f32]) -> Vec<(usize, f32)> {
        let mut peaks = Vec::new();
        for (i, &sample) in buffer.iter().enumerate() {
            if self.detect_peak(sample) {
                let window_end = (i + self.peak_search_window_samples).min(buffer.len());
                let mut peak_idx = i;
                let mut peak_amp = sample;
                for (rel_idx, &candidate) in buffer[i..window_end].iter().enumerate() {
                    if candidate > peak_amp {
                        peak_amp = candidate;
                        peak_idx = i + rel_idx;
                    }
                }
                peaks.push((peak_idx, peak_amp));
            }
        }
        peaks
    }
}

#[derive(Clone)]
struct OldFirFilterCoreModulo {
    taps: Vec<f64>,
    delay_line: Vec<f64>,
    pos: usize,
}

impl OldFirFilterCoreModulo {
    fn new(taps: Vec<f64>) -> Self {
        Self {
            delay_line: vec![0.0; taps.len()],
            taps,
            pos: 0,
        }
    }

    fn process(&mut self, sample: f32) -> f32 {
        self.delay_line[self.pos] = sample as f64;

        let mut output = 0.0f64;
        let n = self.taps.len();
        for i in 0..n {
            let delay_idx = (self.pos + n - i) % n;
            output += self.taps[i] * self.delay_line[delay_idx];
        }

        self.pos = (self.pos + 1) % n;
        output as f32
    }

    fn process_buffer(&mut self, buffer: &mut [f32]) {
        for sample in buffer {
            *sample = self.process(*sample);
        }
    }
}

fn mk_signal(len: usize, pulse_every: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; len];
    let mut x = 0x1234_5678u64;
    for (i, sample) in out.iter_mut().enumerate() {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        let noise = (((x >> 33) as u32) as f32 / u32::MAX as f32) * 0.06 - 0.03;
        let pulse = if i % pulse_every == 2 { 0.25 } else { 0.0 };
        *sample = noise + pulse;
    }
    out
}

fn mk_taps(n: usize) -> Vec<f64> {
    let mut taps = Vec::with_capacity(n);
    let mut sum = 0.0f64;
    for i in 0..n {
        let x = ((i as f64 + 1.0) * 0.113).sin().abs() + 1e-6;
        taps.push(x);
        sum += x;
    }
    for tap in &mut taps {
        *tap /= sum;
    }
    taps
}

fn bench_peak_detector() {
    let threshold = 0.15f32;
    let scenarios = [
        ("sparse/w16", 20usize, 16usize, 30usize, 180_000usize),
        ("dense/w64", 1usize, 64usize, 2usize, 80_000usize),
        ("med/w64", 4usize, 64usize, 6usize, 100_000usize),
    ];

    let mut checksum = 0usize;
    for (label, min_interval, window, pulse_every, iters) in scenarios {
        let buffer = mk_signal(4096, pulse_every);
        let mut old = OldPeakDetectorRescan::new(threshold, min_interval, window);
        let mut new = PeakDetector::with_peak_search_window(threshold, min_interval, window);

        let old_once = old.find_all_peaks(&buffer);
        let new_once = new.find_all_peaks(&buffer);
        assert_eq!(old_once, new_once, "Peak detector output mismatch for {label}");

        let t0 = Instant::now();
        for _ in 0..iters {
            checksum = checksum.wrapping_add(old.find_all_peaks(&buffer).len());
        }
        let dt_old = t0.elapsed();

        let t1 = Instant::now();
        for _ in 0..iters {
            checksum = checksum.wrapping_add(new.find_all_peaks(&buffer).len());
        }
        let dt_new = t1.elapsed();

        println!(
            "PeakDetector {label}: old(rescan) = {:.3?}, new(one-pass) = {:.3?}, speedup = {:.2}x",
            dt_old,
            dt_new,
            dt_old.as_secs_f64() / dt_new.as_secs_f64()
        );
    }
    black_box(checksum);
}

fn bench_fir_core() {
    let taps = mk_taps(127);
    let input = mk_signal(2048, 30);
    let iters = 8_000usize;

    let mut old = OldFirFilterCoreModulo::new(taps.clone());
    let mut new = FirFilterCore::new(taps);

    // Parity check
    let mut old_buf = input.clone();
    let mut new_buf = input.clone();
    old.process_buffer(&mut old_buf);
    new.process_buffer(&mut new_buf);
    let max_abs_err = old_buf
        .iter()
        .zip(new_buf.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(max_abs_err < 1e-6, "FIR output mismatch: max_abs_err={max_abs_err}");

    let t0 = Instant::now();
    let mut checksum = 0.0f32;
    for _ in 0..iters {
        let mut buf = input.clone();
        old.process_buffer(&mut buf);
        checksum += buf[0];
    }
    let dt_old = t0.elapsed();

    let t1 = Instant::now();
    for _ in 0..iters {
        let mut buf = input.clone();
        new.process_buffer(&mut buf);
        checksum += buf[0];
    }
    let dt_new = t1.elapsed();

    black_box(checksum);
    println!(
        "FirFilterCore old(modulo) = {:.3?}, new(split-loop) = {:.3?}, speedup = {:.2}x",
        dt_old,
        dt_new,
        dt_old.as_secs_f64() / dt_new.as_secs_f64()
    );
}

fn main() {
    bench_peak_detector();
    bench_fir_core();
}
