use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Instant;

use clap::Parser;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};

use rotaryclub::audio::{AudioSource, DeviceSource, WavFileSource};
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
    north_offset: f32,
    sample_rate: u32,
}

fn spawn_file_processing(
    fc: &FilePlaybackConfig,
    tx: Sender<GuiUpdate>,
    playback_speed: Arc<AtomicU32>,
    is_playing: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
) -> anyhow::Result<thread::JoinHandle<()>> {
    let chunk_size = fc.config.audio.buffer_size * 2;
    let source: Box<dyn AudioSource> = Box::new(WavFileSource::new(&fc.input_path, chunk_size)?);
    let config = fc.config.clone();
    let remove_dc = fc.remove_dc;
    let north_offset = fc.north_offset;
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

    if let Some(path) = &args.input {
        let file_config = FilePlaybackConfig {
            input_path: path.clone(),
            config: config.clone(),
            remove_dc: args.remove_dc,
            north_offset: config.bearing.north_offset_degrees,
            sample_rate: config.audio.sample_rate,
        };

        let handle = spawn_file_processing(
            &file_config,
            tx,
            Arc::clone(&playback_speed),
            Arc::clone(&is_playing),
            Arc::clone(&stop_requested),
        )?;

        Ok(StartResult {
            handle,
            playback_speed,
            is_playing,
            stop_requested,
            is_file_input: true,
            file_config: Some(file_config),
        })
    } else {
        let source: Box<dyn AudioSource> = Box::new(DeviceSource::new(&config.audio)?);
        let remove_dc = args.remove_dc;
        let dump_audio = args.dump_audio.clone();
        let north_offset = config.bearing.north_offset_degrees;
        let sample_rate = config.audio.sample_rate;
        let speed_clone = Arc::clone(&playback_speed);
        let playing_clone = Arc::clone(&is_playing);
        let stop_clone = Arc::clone(&stop_requested);

        let handle = thread::spawn(move || {
            if let Err(e) = run_processing(
                source,
                config,
                tx.clone(),
                remove_dc,
                dump_audio,
                north_offset,
                sample_rate,
                speed_clone,
                playing_clone,
                stop_clone,
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
    north_offset: f32,
    sample_rate: u32,
    playback_speed: Arc<AtomicU32>,
    is_playing: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
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
                let bearing = (b.bearing_degrees + north_offset).rem_euclid(360.0);
                let raw = (b.raw_bearing + north_offset).rem_euclid(360.0);
                BearingData {
                    bearing,
                    raw,
                    confidence: b.confidence,
                    snr_db: b.metrics.snr_db,
                    coherence: b.metrics.coherence,
                    signal_strength: b.metrics.signal_strength,
                }
            });

            let time_secs = sample_count as f64 / sample_rate as f64;

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

const MAX_HISTORY_SECS: f64 = 120.0;
const DEFAULT_WINDOW_SECS: f64 = 30.0;
const COMPASS_TRAIL_LEN: usize = 50;
const MAX_LOG_LINES: usize = 1000;

struct History {
    bearing: VecDeque<[f64; 2]>,
    raw_bearing: VecDeque<[f64; 2]>,
    snr: VecDeque<[f64; 2]>,
    confidence: VecDeque<[f64; 2]>,
    coherence: VecDeque<[f64; 2]>,
    signal_strength: VecDeque<[f64; 2]>,
    lock_quality: VecDeque<[f64; 2]>,
    compass_trail: VecDeque<(f32, f32, f32, f32)>,
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

        while self.compass_trail.len() > COMPASS_TRAIL_LEN {
            self.compass_trail.pop_front();
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
            is_playing: result.is_playing,
            stop_requested: result.stop_requested,
            loop_enabled: false,
            is_file_input: result.is_file_input,
            file_config: result.file_config,
            processing_handle: Some(result.handle),
        }
    }

    fn restart_processing(&mut self) {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Some(handle) = self.processing_handle.take() {
            let _ = handle.join();
        }

        self.history = History::new();
        self.latest_bearing = None;
        self.latest_confidence = None;
        self.latest_snr = None;
        self.latest_coherence = None;
        self.latest_signal_strength = None;
        self.latest_rotation_freq = None;
        self.latest_lock_quality = None;
        self.latest_phase_error_var = None;
        self.processing_stopped = false;
        self.latest_time = 0.0;

        while self.rx.try_recv().is_ok() {}

        self.stop_requested = Arc::new(AtomicBool::new(false));

        if let Some(fc) = &self.file_config {
            match spawn_file_processing(
                fc,
                self.tx.clone(),
                Arc::clone(&self.playback_speed),
                Arc::clone(&self.is_playing),
                Arc::clone(&self.stop_requested),
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
                        self.history.compass_trail.push_back((
                            b.bearing,
                            b.confidence,
                            b.coherence,
                            b.signal_strength,
                        ));

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
                        self.restart_processing();
                        self.is_playing.store(true, Ordering::Relaxed);
                        return;
                    }
                    self.processing_stopped = true;
                }
            }
        }
    }

    fn draw_compass(&self, ui: &mut egui::Ui) {
        let desired = egui::vec2(300.0, 300.0);
        let (response, painter) = ui.allocate_painter(desired, egui::Sense::hover());
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

        for deg in (0..360).step_by(30) {
            let angle_rad = (deg as f32).to_radians();
            let sin_a = angle_rad.sin();
            let cos_a = angle_rad.cos();
            let inner = if deg % 90 == 0 { 0.85 } else { 0.9 };
            let p1 = center + egui::vec2(sin_a * radius * inner, -cos_a * radius * inner);
            let p2 = center + egui::vec2(sin_a * radius, -cos_a * radius);
            let stroke_width = if deg % 90 == 0 { 2.0 } else { 1.0 };
            painter.line_segment(
                [p1, p2],
                egui::Stroke::new(stroke_width, egui::Color32::from_rgb(120, 120, 140)),
            );
        }

        let cardinals: [(&str, f32); 4] = [("N", 0.0), ("E", 90.0), ("S", 180.0), ("W", 270.0)];
        for (label, deg) in cardinals {
            let angle_rad = deg.to_radians();
            let label_r = radius + 10.0;
            let pos = center + egui::vec2(angle_rad.sin() * label_r, -angle_rad.cos() * label_r);
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(12.0),
                if label == "N" {
                    egui::Color32::from_rgb(255, 100, 100)
                } else {
                    egui::Color32::from_rgb(180, 180, 200)
                },
            );
        }

        let intercardinals = [("NE", 45.0), ("SE", 135.0), ("SW", 225.0), ("NW", 315.0)];
        for (label, deg) in intercardinals {
            let angle_rad = (deg as f32).to_radians();
            let label_r = radius + 10.0;
            let pos = center + egui::vec2(angle_rad.sin() * label_r, -angle_rad.cos() * label_r);
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(9.0),
                egui::Color32::from_rgb(120, 120, 140),
            );
        }

        let trail_len = self.history.compass_trail.len();
        for (i, &(bearing, confidence, coherence, strength)) in
            self.history.compass_trail.iter().enumerate()
        {
            let age_frac = if trail_len > 1 {
                i as f32 / (trail_len - 1) as f32
            } else {
                1.0
            };
            let alpha = (30.0 + age_frac * 200.0) as u8;
            let sat = confidence;
            let gray = 120.0_f32;
            let r = (gray + ((1.0 - confidence) * 255.0 - gray) * sat).clamp(0.0, 255.0) as u8;
            let g = (gray + (confidence * 255.0 - gray) * sat).clamp(0.0, 255.0) as u8;
            let b = (gray + (80.0 - gray) * sat).clamp(0.0, 255.0) as u8;
            let color = egui::Color32::from_rgba_unmultiplied(r, g, b, alpha);

            let angle_rad = bearing.to_radians();
            let dot_r = radius * (0.35 + 0.55 * strength);
            let pos = center + egui::vec2(angle_rad.sin() * dot_r, -angle_rad.cos() * dot_r);
            let dot_size = 1.5 + age_frac * (1.0 + coherence * 3.0);
            painter.circle_filled(pos, dot_size, color);
        }

        if let Some(bearing) = self.latest_bearing {
            let confidence = self.latest_confidence.unwrap_or(0.0);
            let coherence = self.latest_coherence.unwrap_or(0.0);
            let strength = self.latest_signal_strength.unwrap_or(0.0);
            let angle_rad = bearing.to_radians();
            let needle_len = radius * (0.35 + 0.55 * strength);
            let tip =
                center + egui::vec2(angle_rad.sin() * needle_len, -angle_rad.cos() * needle_len);

            let sat = confidence;
            let gray = 120.0_f32;
            let nr = (gray + ((1.0 - confidence) * 255.0 - gray) * sat).clamp(0.0, 255.0) as u8;
            let ng = (gray + (confidence * 255.0 - gray) * sat).clamp(0.0, 255.0) as u8;
            let nb = (gray + (80.0 - gray) * sat).clamp(0.0, 255.0) as u8;
            let needle_color = egui::Color32::from_rgb(nr, ng, nb);
            let stroke_width = 1.0 + coherence * 2.5;

            painter.line_segment([center, tip], egui::Stroke::new(stroke_width, needle_color));
            painter.circle_filled(tip, 2.0 + coherence * 3.0, needle_color);
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
        let smoothed: PlotPoints = self.history.bearing.iter().copied().collect();
        let raw: PlotPoints = self.history.raw_bearing.iter().copied().collect();
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
        let snr_pts: PlotPoints = self.history.snr.iter().copied().collect();
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
        let conf_pts: PlotPoints = self.history.confidence.iter().copied().collect();
        let coh_pts: PlotPoints = self.history.coherence.iter().copied().collect();
        let str_pts: PlotPoints = self.history.signal_strength.iter().copied().collect();
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
        let lq_pts: PlotPoints = self.history.lock_quality.iter().copied().collect();
        Plot::new("lock_quality_plot")
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

        egui::TopBottomPanel::top("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.is_file_input {
                    let playing = self.is_playing.load(Ordering::Relaxed);
                    if ui
                        .button(if playing { "\u{23f8}" } else { "\u{25b6}" })
                        .clicked()
                    {
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
                        egui::RichText::new(format!("{:.1} dB", snr))
                            .color(color)
                            .strong(),
                    );
                } else {
                    ui.label(egui::RichText::new("---").color(egui::Color32::DARK_GRAY));
                }

                if let Some(pev) = self.latest_phase_error_var {
                    ui.separator();
                    ui.label(egui::RichText::new("Phase err:").color(egui::Color32::LIGHT_GRAY));
                    ui.label(
                        egui::RichText::new(format!("{:.4}", pev)).color(egui::Color32::WHITE),
                    );
                }

                if self.is_file_input {
                    ui.separator();
                    ui.label(egui::RichText::new("Speed:").color(egui::Color32::LIGHT_GRAY));
                    let current = f32::from_bits(self.playback_speed.load(Ordering::Relaxed));
                    for (label, value) in [
                        ("0.25x", 0.25_f32),
                        ("0.5x", 0.5),
                        ("1x", 1.0),
                        ("2x", 2.0),
                        ("4x", 4.0),
                        ("Max", 0.0),
                    ] {
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
                    egui::Slider::new(&mut self.history_window, 5.0..=120.0)
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
