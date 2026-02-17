use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Instant;

use clap::Parser;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints};

use rotaryclub::audio::{AudioSource, DeviceSource, WavFileSource, list_input_devices};
use rotaryclub::config::{
    BearingMethod, ChannelRole, NorthTrackingMode, RdfConfig, RotationFrequency,
};
use rotaryclub::processing::RdfProcessor;

#[derive(Parser, Debug)]
#[command(name = "rotaryclub_gui")]
#[command(about = "Pseudo Doppler RDF - GUI", long_about = None)]
struct Args {
    #[arg(short = 'm', long, value_enum, default_value = "correlation")]
    method: BearingMethod,

    #[arg(short = 'n', long, value_enum, default_value = "dpll")]
    north_mode: NorthTrackingMode,

    #[arg(long)]
    rotation: Option<RotationFrequency>,

    #[arg(short = 's', long)]
    swap_channels: bool,

    #[arg(short = 'o', long, default_value = "0.0")]
    north_offset: f32,

    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[arg(short = 'i', long)]
    input: Option<PathBuf>,

    #[arg(long)]
    remove_dc: bool,

    #[arg(long)]
    dump_audio: Option<PathBuf>,

    #[arg(long, default_value = "0")]
    north_tick_gain: f32,

    #[arg(long)]
    device: Option<String>,

    #[arg(long)]
    list_devices: bool,
}

struct BearingData {
    bearing: f32,
    raw: f32,
    confidence: f32,
    snr_db: f32,
    coherence: f32,
    signal_strength: f32,
}

enum GuiUpdate {
    Data {
        time_secs: f64,
        bearing: Option<BearingData>,
        rotation_freq: Option<f32>,
        lock_quality: Option<f32>,
        phase_error_variance: Option<f32>,
    },
    Log(String),
    Stopped,
}

struct GuiLogger {
    tx: Sender<GuiUpdate>,
    max_level: log::LevelFilter,
}

impl log::Log for GuiLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.max_level
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let msg = format!("[{}] {}", record.level(), record.args());
            let _ = self.tx.send(GuiUpdate::Log(msg));
        }
    }

    fn flush(&self) {}
}

struct FilePlaybackConfig {
    input_path: PathBuf,
    config: RdfConfig,
    remove_dc: bool,
    sample_rate: u32,
}

fn spawn_file_processing(
    fc: &FilePlaybackConfig,
    tx: Sender<GuiUpdate>,
    playback_speed: Arc<AtomicU32>,
    is_playing: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    north_offset: Arc<AtomicU32>,
    time_offset: f64,
) -> anyhow::Result<thread::JoinHandle<()>> {
    let chunk_size = fc.config.audio.buffer_size * 2;
    let source: Box<dyn AudioSource> = Box::new(WavFileSource::new(&fc.input_path, chunk_size)?);
    let config = fc.config.clone();
    let remove_dc = fc.remove_dc;
    let sample_rate = fc.sample_rate;

    let handle = thread::spawn(move || {
        if let Err(e) = run_processing(
            source,
            config,
            tx.clone(),
            remove_dc,
            None,
            north_offset,
            sample_rate,
            playback_speed,
            is_playing,
            stop_requested,
            time_offset,
        ) {
            let _ = tx.send(GuiUpdate::Log(format!("Processing error: {}", e)));
        }
        let _ = tx.send(GuiUpdate::Stopped);
    });

    Ok(handle)
}

struct StartResult {
    handle: thread::JoinHandle<()>,
    playback_speed: Arc<AtomicU32>,
    is_playing: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    north_offset: Arc<AtomicU32>,
    is_file_input: bool,
    file_config: Option<FilePlaybackConfig>,
}

fn start_processing(
    args: &Args,
    config: RdfConfig,
    tx: Sender<GuiUpdate>,
) -> anyhow::Result<StartResult> {
    let is_file_input = args.input.is_some();
    let default_speed = if is_file_input { 1.0_f32 } else { 0.0_f32 };
    let playback_speed = Arc::new(AtomicU32::new(default_speed.to_bits()));
    let is_playing = Arc::new(AtomicBool::new(!is_file_input));
    let stop_requested = Arc::new(AtomicBool::new(false));
    let north_offset = Arc::new(AtomicU32::new(
        config.bearing.north_offset_degrees.to_bits(),
    ));

    if let Some(path) = &args.input {
        let file_config = FilePlaybackConfig {
            input_path: path.clone(),
            config: config.clone(),
            remove_dc: args.remove_dc,
            sample_rate: config.audio.sample_rate,
        };

        let handle = spawn_file_processing(
            &file_config,
            tx,
            Arc::clone(&playback_speed),
            Arc::clone(&is_playing),
            Arc::clone(&stop_requested),
            Arc::clone(&north_offset),
            0.0,
        )?;

        Ok(StartResult {
            handle,
            playback_speed,
            is_playing,
            stop_requested,
            north_offset,
            is_file_input: true,
            file_config: Some(file_config),
        })
    } else {
        let source: Box<dyn AudioSource> =
            Box::new(DeviceSource::new(&config.audio, args.device.as_deref())?);
        let remove_dc = args.remove_dc;
        let dump_audio = args.dump_audio.clone();
        let sample_rate = config.audio.sample_rate;
        let speed_clone = Arc::clone(&playback_speed);
        let playing_clone = Arc::clone(&is_playing);
        let stop_clone = Arc::clone(&stop_requested);
        let offset_clone = Arc::clone(&north_offset);

        let handle = thread::spawn(move || {
            if let Err(e) = run_processing(
                source,
                config,
                tx.clone(),
                remove_dc,
                dump_audio,
                offset_clone,
                sample_rate,
                speed_clone,
                playing_clone,
                stop_clone,
                0.0,
            ) {
                let _ = tx.send(GuiUpdate::Log(format!("Processing error: {}", e)));
            }
            let _ = tx.send(GuiUpdate::Stopped);
        });

        Ok(StartResult {
            handle,
            playback_speed,
            is_playing,
            stop_requested,
            north_offset,
            is_file_input: false,
            file_config: None,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn run_processing(
    mut source: Box<dyn AudioSource>,
    config: RdfConfig,
    tx: Sender<GuiUpdate>,
    remove_dc: bool,
    dump_audio: Option<PathBuf>,
    north_offset: Arc<AtomicU32>,
    sample_rate: u32,
    playback_speed: Arc<AtomicU32>,
    is_playing: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    time_offset: f64,
) -> anyhow::Result<()> {
    let mut processor = RdfProcessor::new(&config, remove_dc, true)?;
    let mut sample_count: u64 = 0;
    let mut dump_samples: Vec<f32> = Vec::new();
    let mut wall_start = Instant::now();
    let mut expected_time = 0.0_f64;

    loop {
        if stop_requested.load(Ordering::Relaxed) {
            break;
        }

        if !is_playing.load(Ordering::Relaxed) {
            thread::sleep(std::time::Duration::from_millis(50));
            wall_start = Instant::now();
            expected_time = 0.0;
            continue;
        }

        let Some(audio_data) = source.next_buffer()? else {
            break;
        };

        if dump_audio.is_some() {
            dump_samples.extend_from_slice(&audio_data);
        }

        let frame_samples = audio_data.len() as u64 / 2;
        let tick_results = processor.process_audio(&audio_data);

        let rotation_freq = processor.rotation_frequency();
        let phase_error_variance = processor.phase_error_variance();

        for result in &tick_results {
            let bearing_data = result.bearing.map(|b| {
                let offset = f32::from_bits(north_offset.load(Ordering::Relaxed));
                let bearing = (b.bearing_degrees + offset).rem_euclid(360.0);
                let raw = (b.raw_bearing + offset).rem_euclid(360.0);
                BearingData {
                    bearing,
                    raw,
                    confidence: b.confidence,
                    snr_db: b.metrics.snr_db,
                    coherence: b.metrics.coherence,
                    signal_strength: b.metrics.signal_strength,
                }
            });

            let time_secs = time_offset + sample_count as f64 / sample_rate as f64;

            let update = GuiUpdate::Data {
                time_secs,
                bearing: bearing_data,
                rotation_freq,
                lock_quality: result.north_tick.lock_quality,
                phase_error_variance,
            };

            if tx.send(update).is_err() {
                break;
            }
        }

        sample_count += frame_samples;

        let speed = f32::from_bits(playback_speed.load(Ordering::Relaxed));
        if speed > 0.0 {
            let chunk_duration = frame_samples as f64 / sample_rate as f64 / speed as f64;
            expected_time += chunk_duration;
            let elapsed = wall_start.elapsed().as_secs_f64();
            if expected_time > elapsed {
                thread::sleep(std::time::Duration::from_secs_f64(expected_time - elapsed));
            }
        }
    }

    if let Some(path) = dump_audio {
        rotaryclub::save_wav(
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid path"))?,
            &dump_samples,
            sample_rate,
        )?;
    }

    Ok(())
}

const SPEED_STEPS: [(f32, &str); 6] = [
    (0.25, "0.25x"),
    (0.5, "0.5x"),
    (1.0, "1x"),
    (2.0, "2x"),
    (4.0, "4x"),
    (0.0, "Max"),
];

const MAX_HISTORY_SECS: f64 = 120.0;
const DEFAULT_WINDOW_SECS: f64 = 2.5;
const MIN_WINDOW_SECS: f64 = 1.0;
const MAX_WINDOW_SECS: f64 = 120.0;
const MAX_TRAIL_AGE_SECS: f64 = 10.0;
const MAX_LOG_LINES: usize = 1000;

const PHOSPHOR_COLOR: (u8, u8, u8) = (30, 255, 60);
const TRAIL_CONFIDENCE_THRESHOLD: f32 = 0.5;
const TRAIL_TAU_BASE: f32 = 0.15;
const TRAIL_TAU_SCALE: f32 = 0.3;
const TRAIL_BRIGHTNESS_CUTOFF: f32 = 0.02;
const TRAIL_DOT_BASE: f32 = 1.0;
const TRAIL_DOT_SCALE: f32 = 1.0;
const TRAIL_GLOW_RADIUS_SCALE: f32 = 1.5;
const TRAIL_GLOW_ALPHA_SCALE: f32 = 0.25;
const NEEDLE_MIN_RADIUS_FRAC: f32 = 0.35;
const NEEDLE_RADIUS_RANGE: f32 = 0.55;
const NEEDLE_MIN_BRIGHTNESS: f32 = 0.2;
const NEEDLE_BRIGHTNESS_RANGE: f32 = 0.8;
const NEEDLE_STROKE_BASE: f32 = 1.0;
const NEEDLE_STROKE_SCALE: f32 = 2.5;
const NEEDLE_TIP_BASE: f32 = 2.0;
const NEEDLE_TIP_SCALE: f32 = 3.0;
const COMPASS_HELP_TEXT: &str = "Compass rose encoding:
- Needle brightness = confidence
- Needle width/tip size = coherence
- Needle length/radius = signal strength
- Trail = recent bearings with phosphor decay
- Trail points require confidence >= 0.5";

struct TrailEntry {
    bearing: f32,
    confidence: f32,
    coherence: f32,
    strength: f32,
    time: f64,
}

struct History {
    bearing: VecDeque<[f64; 2]>,
    raw_bearing: VecDeque<[f64; 2]>,
    snr: VecDeque<[f64; 2]>,
    confidence: VecDeque<[f64; 2]>,
    coherence: VecDeque<[f64; 2]>,
    signal_strength: VecDeque<[f64; 2]>,
    lock_quality: VecDeque<[f64; 2]>,
    compass_trail: VecDeque<TrailEntry>,
}

impl History {
    fn new() -> Self {
        Self {
            bearing: VecDeque::new(),
            raw_bearing: VecDeque::new(),
            snr: VecDeque::new(),
            confidence: VecDeque::new(),
            coherence: VecDeque::new(),
            signal_strength: VecDeque::new(),
            lock_quality: VecDeque::new(),
            compass_trail: VecDeque::new(),
        }
    }

    fn prune(&mut self, now: f64) {
        let cutoff = now - MAX_HISTORY_SECS;
        for buf in [
            &mut self.bearing,
            &mut self.raw_bearing,
            &mut self.snr,
            &mut self.confidence,
            &mut self.coherence,
            &mut self.signal_strength,
            &mut self.lock_quality,
        ] {
            while let Some(front) = buf.front() {
                if front[0] < cutoff {
                    buf.pop_front();
                } else {
                    break;
                }
            }
        }

        let trail_cutoff = now - MAX_TRAIL_AGE_SECS;
        while let Some(front) = self.compass_trail.front() {
            if front.time < trail_cutoff {
                self.compass_trail.pop_front();
            } else {
                break;
            }
        }
    }
}

struct RdfGuiApp {
    rx: Receiver<GuiUpdate>,
    tx: Sender<GuiUpdate>,
    history: History,
    log_lines: VecDeque<String>,
    latest_bearing: Option<f32>,
    latest_confidence: Option<f32>,
    latest_snr: Option<f32>,
    latest_coherence: Option<f32>,
    latest_signal_strength: Option<f32>,
    latest_rotation_freq: Option<f32>,
    latest_lock_quality: Option<f32>,
    latest_phase_error_var: Option<f32>,
    processing_stopped: bool,
    latest_time: f64,
    history_window: f64,
    playback_speed: Arc<AtomicU32>,
    north_offset: Arc<AtomicU32>,
    north_offset_degrees: f32,
    is_playing: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    loop_enabled: bool,
    is_file_input: bool,
    file_config: Option<FilePlaybackConfig>,
    processing_handle: Option<thread::JoinHandle<()>>,
}

impl RdfGuiApp {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        rx: Receiver<GuiUpdate>,
        tx: Sender<GuiUpdate>,
        result: StartResult,
    ) -> Self {
        Self {
            rx,
            tx,
            history: History::new(),
            log_lines: VecDeque::new(),
            latest_bearing: None,
            latest_confidence: None,
            latest_snr: None,
            latest_coherence: None,
            latest_signal_strength: None,
            latest_rotation_freq: None,
            latest_lock_quality: None,
            latest_phase_error_var: None,
            processing_stopped: false,
            latest_time: 0.0,
            history_window: DEFAULT_WINDOW_SECS,
            playback_speed: result.playback_speed,
            north_offset: Arc::clone(&result.north_offset),
            north_offset_degrees: f32::from_bits(result.north_offset.load(Ordering::Relaxed)),
            is_playing: result.is_playing,
            stop_requested: result.stop_requested,
            loop_enabled: false,
            is_file_input: result.is_file_input,
            file_config: result.file_config,
            processing_handle: Some(result.handle),
        }
    }

    fn restart_processing(&mut self) {
        self.restart_processing_at(0.0, true);
    }

    fn restart_processing_at(&mut self, time_offset: f64, clear_state: bool) {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Some(handle) = self.processing_handle.take() {
            let _ = handle.join();
        }

        if clear_state {
            self.history = History::new();
            self.latest_bearing = None;
            self.latest_confidence = None;
            self.latest_snr = None;
            self.latest_coherence = None;
            self.latest_signal_strength = None;
            self.latest_rotation_freq = None;
            self.latest_lock_quality = None;
            self.latest_phase_error_var = None;
            self.latest_time = 0.0;
        }

        self.processing_stopped = false;

        while self.rx.try_recv().is_ok() {}

        self.stop_requested = Arc::new(AtomicBool::new(false));

        if let Some(fc) = &self.file_config {
            match spawn_file_processing(
                fc,
                self.tx.clone(),
                Arc::clone(&self.playback_speed),
                Arc::clone(&self.is_playing),
                Arc::clone(&self.stop_requested),
                Arc::clone(&self.north_offset),
                time_offset,
            ) {
                Ok(handle) => self.processing_handle = Some(handle),
                Err(e) => {
                    self.log_lines.push_back(format!("Restart error: {}", e));
                }
            }
        }
    }

    fn drain_updates(&mut self) {
        while let Ok(update) = self.rx.try_recv() {
            match update {
                GuiUpdate::Data {
                    time_secs,
                    bearing,
                    rotation_freq,
                    lock_quality,
                    phase_error_variance,
                } => {
                    self.latest_time = time_secs;
                    self.latest_rotation_freq = rotation_freq;
                    self.latest_lock_quality = lock_quality;
                    self.latest_phase_error_var = phase_error_variance;

                    if let Some(b) = bearing {
                        self.history
                            .bearing
                            .push_back([time_secs, b.bearing as f64]);
                        self.history
                            .raw_bearing
                            .push_back([time_secs, b.raw as f64]);
                        self.history.snr.push_back([time_secs, b.snr_db as f64]);
                        self.history
                            .confidence
                            .push_back([time_secs, b.confidence as f64]);
                        self.history
                            .coherence
                            .push_back([time_secs, b.coherence as f64]);
                        self.history
                            .signal_strength
                            .push_back([time_secs, b.signal_strength as f64]);
                        self.history.compass_trail.push_back(TrailEntry {
                            bearing: b.bearing,
                            confidence: b.confidence,
                            coherence: b.coherence,
                            strength: b.signal_strength,
                            time: time_secs,
                        });

                        self.latest_bearing = Some(b.bearing);
                        self.latest_confidence = Some(b.confidence);
                        self.latest_snr = Some(b.snr_db);
                        self.latest_coherence = Some(b.coherence);
                        self.latest_signal_strength = Some(b.signal_strength);
                    }

                    if let Some(lq) = lock_quality {
                        self.history.lock_quality.push_back([time_secs, lq as f64]);
                    }

                    self.history.prune(time_secs);
                }
                GuiUpdate::Log(msg) => {
                    self.log_lines.push_back(msg);
                    while self.log_lines.len() > MAX_LOG_LINES {
                        self.log_lines.pop_front();
                    }
                }
                GuiUpdate::Stopped => {
                    if self.loop_enabled && self.is_file_input {
                        self.restart_processing_at(self.latest_time, false);
                        self.is_playing.store(true, Ordering::Relaxed);
                        return;
                    }
                    self.processing_stopped = true;
                    self.is_playing.store(false, Ordering::Relaxed);
                }
            }
        }
    }

    fn draw_compass(&mut self, ui: &mut egui::Ui) {
        let desired = egui::vec2(300.0, 300.0);
        let (response, painter) = ui.allocate_painter(desired, egui::Sense::hover());
        let response = response.on_hover_text(COMPASS_HELP_TEXT);
        let rect = response.rect;
        let center = rect.center();
        let radius = rect.width().min(rect.height()) / 2.0 - 15.0;

        let bg = egui::Color32::from_rgb(20, 20, 30);
        painter.rect_filled(rect, 4.0, bg);

        painter.circle_stroke(
            center,
            radius,
            egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 80, 100)),
        );

        for deg in (0..360).step_by(10) {
            let angle_rad = (deg as f32).to_radians();
            let sin_a = angle_rad.sin();
            let cos_a = angle_rad.cos();
            let (inner, stroke_width) = if deg % 90 == 0 {
                (0.82, 2.5)
            } else if deg % 30 == 0 {
                (0.87, 1.5)
            } else {
                (0.92, 0.7)
            };
            let p1 = center + egui::vec2(sin_a * radius * inner, -cos_a * radius * inner);
            let p2 = center + egui::vec2(sin_a * radius, -cos_a * radius);
            painter.line_segment(
                [p1, p2],
                egui::Stroke::new(stroke_width, egui::Color32::from_rgb(120, 120, 140)),
            );
        }

        for deg in (0..360).step_by(30) {
            let angle_rad = (deg as f32).to_radians();
            let label_r = radius + 10.0;
            let pos = center + egui::vec2(angle_rad.sin() * label_r, -angle_rad.cos() * label_r);
            let (label, color) = if deg == 0 {
                ("N".to_string(), egui::Color32::from_rgb(255, 100, 100))
            } else {
                (
                    format!("{:03}", deg),
                    egui::Color32::from_rgb(160, 160, 180),
                )
            };
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                color,
            );
        }

        for &strength in &[0.25_f32, 0.5, 0.75, 1.0] {
            let ring_r = radius * (NEEDLE_MIN_RADIUS_FRAC + NEEDLE_RADIUS_RANGE * strength);
            painter.circle_stroke(
                center,
                ring_r,
                egui::Stroke::new(0.5, egui::Color32::from_rgba_unmultiplied(80, 80, 100, 40)),
            );
            let label_angle: f32 = 45.0_f32.to_radians();
            let label_pos =
                center + egui::vec2(label_angle.sin() * ring_r, -label_angle.cos() * ring_r);
            painter.text(
                label_pos,
                egui::Align2::LEFT_BOTTOM,
                format!("{}%", (strength * 100.0) as u32),
                egui::FontId::proportional(8.0),
                egui::Color32::from_rgba_unmultiplied(140, 140, 160, 80),
            );
        }

        for entry in &self.history.compass_trail {
            if entry.confidence < TRAIL_CONFIDENCE_THRESHOLD {
                continue;
            }

            let age = (self.latest_time - entry.time) as f32;
            let tau = TRAIL_TAU_BASE + TRAIL_TAU_SCALE * entry.confidence;
            let brightness = (-age / tau).exp();
            if brightness < TRAIL_BRIGHTNESS_CUTOFF {
                continue;
            }

            let angle_rad = entry.bearing.to_radians();
            let dot_r = radius * (NEEDLE_MIN_RADIUS_FRAC + NEEDLE_RADIUS_RANGE * entry.strength);
            let pos = center + egui::vec2(angle_rad.sin() * dot_r, -angle_rad.cos() * dot_r);
            let dot_size = TRAIL_DOT_BASE + TRAIL_DOT_SCALE * entry.coherence;

            let glow_alpha = (brightness * TRAIL_GLOW_ALPHA_SCALE * 255.0).clamp(0.0, 255.0) as u8;
            let glow_color = egui::Color32::from_rgba_unmultiplied(
                PHOSPHOR_COLOR.0,
                PHOSPHOR_COLOR.1,
                PHOSPHOR_COLOR.2,
                glow_alpha,
            );
            painter.circle_filled(pos, dot_size * TRAIL_GLOW_RADIUS_SCALE, glow_color);

            let core_alpha = (brightness * 255.0).clamp(0.0, 255.0) as u8;
            let core_color = egui::Color32::from_rgba_unmultiplied(
                PHOSPHOR_COLOR.0,
                PHOSPHOR_COLOR.1,
                PHOSPHOR_COLOR.2,
                core_alpha,
            );
            painter.circle_filled(pos, dot_size, core_color);
        }

        if let Some(bearing) = self.latest_bearing {
            let confidence = self.latest_confidence.unwrap_or(0.0);
            let coherence = self.latest_coherence.unwrap_or(0.0);
            let strength = self.latest_signal_strength.unwrap_or(0.0);
            let angle_rad = bearing.to_radians();
            let needle_len = radius * (NEEDLE_MIN_RADIUS_FRAC + NEEDLE_RADIUS_RANGE * strength);
            let tip =
                center + egui::vec2(angle_rad.sin() * needle_len, -angle_rad.cos() * needle_len);

            let bright = NEEDLE_MIN_BRIGHTNESS + NEEDLE_BRIGHTNESS_RANGE * confidence;
            let needle_color = egui::Color32::from_rgb(
                (PHOSPHOR_COLOR.0 as f32 * bright) as u8,
                (PHOSPHOR_COLOR.1 as f32 * bright) as u8,
                (PHOSPHOR_COLOR.2 as f32 * bright) as u8,
            );
            let stroke_width = NEEDLE_STROKE_BASE + NEEDLE_STROKE_SCALE * coherence;

            painter.line_segment([center, tip], egui::Stroke::new(stroke_width, needle_color));
            painter.circle_filled(
                tip,
                NEEDLE_TIP_BASE + NEEDLE_TIP_SCALE * coherence,
                needle_color,
            );

            let tri_size = 6.0;
            let rim_pos = center + egui::vec2(angle_rad.sin() * radius, -angle_rad.cos() * radius);
            let perp = egui::vec2(angle_rad.cos(), angle_rad.sin());
            let inward = egui::vec2(-angle_rad.sin(), angle_rad.cos());
            let tri_pts = vec![
                rim_pos + inward * tri_size,
                rim_pos - perp * tri_size * 0.5,
                rim_pos + perp * tri_size * 0.5,
            ];
            let marker_bright = NEEDLE_MIN_BRIGHTNESS + NEEDLE_BRIGHTNESS_RANGE * confidence;
            let marker_color = egui::Color32::from_rgb(
                (PHOSPHOR_COLOR.0 as f32 * marker_bright) as u8,
                (PHOSPHOR_COLOR.1 as f32 * marker_bright) as u8,
                (PHOSPHOR_COLOR.2 as f32 * marker_bright) as u8,
            );
            painter.add(egui::Shape::convex_polygon(
                tri_pts,
                marker_color,
                egui::Stroke::NONE,
            ));

            painter.circle_filled(center, 3.0, egui::Color32::from_rgb(200, 200, 220));
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Bearing:").color(egui::Color32::LIGHT_GRAY));
            if let Some(b) = self.latest_bearing {
                ui.label(
                    egui::RichText::new(format!("{:.1}°", b))
                        .color(egui::Color32::WHITE)
                        .strong(),
                );
            } else {
                ui.label(egui::RichText::new("---").color(egui::Color32::DARK_GRAY));
            }
        });

        let quality_color = |v: f32| {
            if v > 0.7 {
                egui::Color32::from_rgb(100, 255, 100)
            } else if v > 0.4 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::from_rgb(255, 100, 100)
            }
        };
        let dash = egui::RichText::new("---").color(egui::Color32::DARK_GRAY);

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Conf:").color(egui::Color32::LIGHT_GRAY));
            if let Some(c) = self.latest_confidence {
                ui.label(egui::RichText::new(format!("{:.2}", c)).color(quality_color(c)));
            } else {
                ui.label(dash.clone());
            }
            ui.label(egui::RichText::new("Coh:").color(egui::Color32::LIGHT_GRAY));
            if let Some(c) = self.latest_coherence {
                ui.label(egui::RichText::new(format!("{:.2}", c)).color(quality_color(c)));
            } else {
                ui.label(dash.clone());
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Strength:").color(egui::Color32::LIGHT_GRAY));
            if let Some(s) = self.latest_signal_strength {
                ui.label(egui::RichText::new(format!("{:.2}", s)).color(quality_color(s)));
            } else {
                ui.label(dash);
            }
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("North offset:").color(egui::Color32::LIGHT_GRAY));
            ui.label(
                egui::RichText::new(format!("{:.0}°", self.north_offset_degrees))
                    .color(egui::Color32::WHITE),
            );
        });
        if ui
            .add(egui::Slider::new(&mut self.north_offset_degrees, 0.0..=360.0).suffix("°"))
            .changed()
        {
            self.north_offset
                .store(self.north_offset_degrees.to_bits(), Ordering::Relaxed);
        }
    }

    fn draw_plots(&self, ui: &mut egui::Ui) {
        let plot_height = 120.0;
        let x_max = self.latest_time.max(self.history_window);
        let x_min = x_max - self.history_window;
        let link_group = ui.id().with("plot_x_link");

        ui.label(
            egui::RichText::new("Bearing")
                .color(egui::Color32::LIGHT_GRAY)
                .small(),
        );
        let in_window = |pts: &VecDeque<[f64; 2]>| -> PlotPoints {
            pts.iter().copied().filter(|p| p[0] >= x_min).collect()
        };

        let smoothed = in_window(&self.history.bearing);
        let raw = in_window(&self.history.raw_bearing);
        Plot::new("bearing_plot")
            .height(plot_height)
            .include_x(x_min)
            .include_x(x_max)
            .include_y(0.0)
            .include_y(360.0)
            .y_axis_label("deg")
            .y_axis_min_width(60.0)
            .link_axis(link_group, [true, false])
            .show_axes([false, true])
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .legend(Legend::default())
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("Smoothed", smoothed).color(egui::Color32::from_rgb(100, 200, 255)),
                );
                plot_ui.line(
                    Line::new("Raw", raw)
                        .color(egui::Color32::from_rgb(100, 200, 255).gamma_multiply(0.4))
                        .style(egui_plot::LineStyle::Dashed { length: 4.0 }),
                );
            });

        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("SNR")
                .color(egui::Color32::LIGHT_GRAY)
                .small(),
        );
        let snr_pts = in_window(&self.history.snr);
        Plot::new("snr_plot")
            .height(plot_height)
            .include_x(x_min)
            .include_x(x_max)
            .y_axis_label("dB")
            .y_axis_min_width(60.0)
            .link_axis(link_group, [true, false])
            .show_axes([false, true])
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show(ui, |plot_ui| {
                plot_ui
                    .line(Line::new("SNR", snr_pts).color(egui::Color32::from_rgb(255, 200, 50)));
            });

        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Confidence / Coherence / Strength")
                .color(egui::Color32::LIGHT_GRAY)
                .small(),
        );
        let conf_pts = in_window(&self.history.confidence);
        let coh_pts = in_window(&self.history.coherence);
        let str_pts = in_window(&self.history.signal_strength);
        Plot::new("quality_plot")
            .height(plot_height)
            .include_x(x_min)
            .include_x(x_max)
            .include_y(0.0)
            .include_y(1.0)
            .y_axis_min_width(60.0)
            .link_axis(link_group, [true, false])
            .show_axes([false, true])
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .legend(Legend::default())
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("Confidence", conf_pts).color(egui::Color32::from_rgb(100, 255, 100)),
                );
                plot_ui.line(
                    Line::new("Coherence", coh_pts).color(egui::Color32::from_rgb(100, 100, 255)),
                );
                plot_ui.line(
                    Line::new("Strength", str_pts).color(egui::Color32::from_rgb(255, 100, 255)),
                );
            });

        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Lock Quality")
                .color(egui::Color32::LIGHT_GRAY)
                .small(),
        );
        let lq_pts = in_window(&self.history.lock_quality);
        Plot::new("lock_quality_plot")
            .height(plot_height)
            .include_x(x_min)
            .include_x(x_max)
            .include_y(0.0)
            .include_y(1.0)
            .x_axis_label("s")
            .y_axis_min_width(60.0)
            .link_axis(link_group, [true, false])
            .show_axes([true, true])
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("Lock Quality", lq_pts).color(egui::Color32::from_rgb(255, 150, 50)),
                );
            });
    }
}

impl eframe::App for RdfGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_updates();
        ctx.request_repaint();

        if ctx.input(|i| i.key_pressed(egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if self.is_file_input {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                let playing = self.is_playing.load(Ordering::Relaxed);
                if !playing && self.processing_stopped {
                    self.restart_processing();
                }
                self.is_playing.store(!playing, Ordering::Relaxed);
            }

            let current_speed = f32::from_bits(self.playback_speed.load(Ordering::Relaxed));
            let current_idx = SPEED_STEPS
                .iter()
                .position(|&(v, _)| (v - current_speed).abs() < 0.001)
                .unwrap_or(2);

            if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                let next = (current_idx + 1).min(SPEED_STEPS.len() - 1);
                self.playback_speed
                    .store(SPEED_STEPS[next].0.to_bits(), Ordering::Relaxed);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                let prev = current_idx.saturating_sub(1);
                self.playback_speed
                    .store(SPEED_STEPS[prev].0.to_bits(), Ordering::Relaxed);
            }
        }

        egui::TopBottomPanel::top("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.is_file_input {
                    let playing = self.is_playing.load(Ordering::Relaxed);
                    if ui
                        .button(if playing { "\u{23f8}" } else { "\u{25b6}" })
                        .clicked()
                    {
                        if !playing && self.processing_stopped {
                            self.restart_processing();
                        }
                        self.is_playing.store(!playing, Ordering::Relaxed);
                    }
                    if ui.button("\u{23ee}").clicked() {
                        self.is_playing.store(false, Ordering::Relaxed);
                        self.restart_processing();
                    }
                    let loop_btn = egui::Button::new(egui::RichText::new("\u{1f501}").color(
                        if self.loop_enabled {
                            egui::Color32::BLACK
                        } else {
                            egui::Color32::LIGHT_GRAY
                        },
                    ));
                    let loop_btn = if self.loop_enabled {
                        loop_btn.fill(egui::Color32::from_rgb(100, 200, 255))
                    } else {
                        loop_btn
                    };
                    if ui.add(loop_btn).clicked() {
                        self.loop_enabled = !self.loop_enabled;
                    }
                    ui.separator();
                }

                if self.processing_stopped {
                    ui.label(
                        egui::RichText::new("STOPPED")
                            .color(egui::Color32::from_rgb(255, 80, 80))
                            .strong(),
                    );
                    ui.separator();
                }

                ui.label(egui::RichText::new("DPLL Lock:").color(egui::Color32::LIGHT_GRAY));
                if let Some(lq) = self.latest_lock_quality {
                    let color = if lq > 0.8 {
                        egui::Color32::from_rgb(100, 255, 100)
                    } else if lq > 0.5 {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::from_rgb(255, 100, 100)
                    };
                    ui.label(
                        egui::RichText::new(format!("{:.2}", lq))
                            .color(color)
                            .strong(),
                    );
                } else {
                    ui.label(egui::RichText::new("---").color(egui::Color32::DARK_GRAY));
                }

                ui.separator();

                ui.label(egui::RichText::new("Rotation:").color(egui::Color32::LIGHT_GRAY));
                if let Some(freq) = self.latest_rotation_freq {
                    ui.label(
                        egui::RichText::new(format!("{:.1} Hz", freq))
                            .color(egui::Color32::WHITE)
                            .strong(),
                    );
                } else {
                    ui.label(egui::RichText::new("---").color(egui::Color32::DARK_GRAY));
                }

                ui.separator();

                ui.label(egui::RichText::new("SNR:").color(egui::Color32::LIGHT_GRAY));
                if let Some(snr) = self.latest_snr {
                    let color = if snr > 15.0 {
                        egui::Color32::from_rgb(100, 255, 100)
                    } else if snr > 5.0 {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::from_rgb(255, 100, 100)
                    };
                    ui.label(
                        egui::RichText::new(format!("{:>6.1} dB", snr))
                            .monospace()
                            .color(color)
                            .strong(),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("  ---.- dB")
                            .monospace()
                            .color(egui::Color32::DARK_GRAY),
                    );
                }

                if let Some(pev) = self.latest_phase_error_var {
                    ui.separator();
                    ui.label(egui::RichText::new("Phase err:").color(egui::Color32::LIGHT_GRAY));
                    ui.label(
                        egui::RichText::new(format!("{:.4}", pev)).color(egui::Color32::WHITE),
                    );
                }

                ui.separator();
                let total_secs = self.latest_time;
                let minutes = (total_secs / 60.0) as u64;
                let secs = total_secs % 60.0;
                ui.label(
                    egui::RichText::new(format!("{:02}:{:04.1}", minutes, secs))
                        .color(egui::Color32::WHITE)
                        .strong(),
                );

                if self.is_file_input {
                    ui.separator();
                    ui.label(egui::RichText::new("Speed:").color(egui::Color32::LIGHT_GRAY));
                    let current = f32::from_bits(self.playback_speed.load(Ordering::Relaxed));
                    for &(value, label) in &SPEED_STEPS {
                        let active = (current - value).abs() < 0.001;
                        let btn = egui::Button::new(egui::RichText::new(label).color(if active {
                            egui::Color32::BLACK
                        } else {
                            egui::Color32::LIGHT_GRAY
                        }));
                        let btn = if active {
                            btn.fill(egui::Color32::from_rgb(100, 200, 255))
                        } else {
                            btn
                        };
                        if ui.add(btn).clicked() {
                            self.playback_speed
                                .store(value.to_bits(), Ordering::Relaxed);
                        }
                    }
                }
            });
        });

        egui::TopBottomPanel::bottom("debug_log")
            .resizable(true)
            .default_height(150.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Debug Log")
                            .color(egui::Color32::LIGHT_GRAY)
                            .strong(),
                    );
                    if ui.small_button("Clear").clicked() {
                        self.log_lines.clear();
                    }
                });
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.log_lines {
                            ui.label(
                                egui::RichText::new(line)
                                    .font(egui::FontId::monospace(11.0))
                                    .color(egui::Color32::from_rgb(180, 180, 180)),
                            );
                        }
                    });
            });

        egui::SidePanel::left("compass_panel")
            .default_width(350.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Compass")
                            .color(egui::Color32::WHITE)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    self.draw_compass(ui);
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Window:").color(egui::Color32::LIGHT_GRAY));
                ui.add(
                    egui::Slider::new(&mut self.history_window, MIN_WINDOW_SECS..=MAX_WINDOW_SECS)
                        .suffix("s")
                        .logarithmic(true),
                );
            });
            ui.add_space(2.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.draw_plots(ui);
            });
        });
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.list_devices {
        let devices = list_input_devices()?;
        if devices.is_empty() {
            eprintln!("No input devices found.");
        } else {
            for name in &devices {
                println!("{}", name);
            }
        }
        return Ok(());
    }

    let log_level = match args.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        2 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };

    let (tx, rx) = crossbeam_channel::unbounded::<GuiUpdate>();

    let logger = GuiLogger {
        tx: tx.clone(),
        max_level: log_level,
    };
    log::set_boxed_logger(Box::new(logger)).ok();
    log::set_max_level(log_level);

    let mut config = RdfConfig::default();
    config.doppler.method = args.method;
    config.north_tick.mode = args.north_mode;
    config.bearing.north_offset_degrees = args.north_offset;

    if let Some(rotation) = args.rotation {
        let hz = rotation.as_hz();
        config.doppler.expected_freq = hz;
        config.north_tick.dpll.initial_frequency_hz = hz;
    }

    config.north_tick.gain_db = args.north_tick_gain;

    if args.swap_channels {
        config.audio.doppler_channel = ChannelRole::Right;
        config.audio.north_tick_channel = ChannelRole::Left;
    }

    let result = start_processing(&args, config, tx.clone())?;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("Rotary Club - Pseudo Doppler RDF"),
        ..Default::default()
    };

    eframe::run_native(
        "Rotary Club RDF",
        native_options,
        Box::new(move |cc| Ok(Box::new(RdfGuiApp::new(cc, rx, tx, result)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {}", e))?;

    Ok(())
}
