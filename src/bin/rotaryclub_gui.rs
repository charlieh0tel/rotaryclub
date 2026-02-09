use std::collections::VecDeque;
use std::path::PathBuf;
use std::thread;

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
}

impl log::Log for GuiLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let msg = format!("[{}] {}", record.level(), record.args());
            let _ = self.tx.send(GuiUpdate::Log(msg));
        }
    }

    fn flush(&self) {}
}

fn start_processing(
    args: &Args,
    config: RdfConfig,
    tx: Sender<GuiUpdate>,
) -> anyhow::Result<thread::JoinHandle<()>> {
    let source: Box<dyn AudioSource> = match &args.input {
        Some(path) => {
            let chunk_size = config.audio.buffer_size * 2;
            Box::new(WavFileSource::new(path, chunk_size)?)
        }
        None => Box::new(DeviceSource::new(&config.audio)?),
    };

    let remove_dc = args.remove_dc;
    let dump_audio = args.dump_audio.clone();
    let north_offset = config.bearing.north_offset_degrees;
    let sample_rate = config.audio.sample_rate;

    let handle = thread::spawn(move || {
        if let Err(e) = run_processing(
            source,
            config,
            tx.clone(),
            remove_dc,
            dump_audio,
            north_offset,
            sample_rate,
        ) {
            let _ = tx.send(GuiUpdate::Log(format!("Processing error: {}", e)));
        }
        let _ = tx.send(GuiUpdate::Stopped);
    });

    Ok(handle)
}

fn run_processing(
    mut source: Box<dyn AudioSource>,
    config: RdfConfig,
    tx: Sender<GuiUpdate>,
    remove_dc: bool,
    dump_audio: Option<PathBuf>,
    north_offset: f32,
    sample_rate: u32,
) -> anyhow::Result<()> {
    let mut processor = RdfProcessor::new(&config, remove_dc, true)?;
    let mut sample_count: u64 = 0;
    let mut dump_samples: Vec<f32> = Vec::new();

    loop {
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

const HISTORY_SECS: f64 = 60.0;
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
    compass_trail: VecDeque<(f32, f32)>,
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
        let cutoff = now - HISTORY_SECS;
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
    history: History,
    log_lines: VecDeque<String>,
    latest_bearing: Option<f32>,
    latest_confidence: Option<f32>,
    latest_snr: Option<f32>,
    latest_rotation_freq: Option<f32>,
    latest_lock_quality: Option<f32>,
    latest_phase_error_var: Option<f32>,
    processing_stopped: bool,
    log_visible: bool,
    _processing_handle: Option<thread::JoinHandle<()>>,
}

impl RdfGuiApp {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        rx: Receiver<GuiUpdate>,
        handle: thread::JoinHandle<()>,
    ) -> Self {
        Self {
            rx,
            history: History::new(),
            log_lines: VecDeque::new(),
            latest_bearing: None,
            latest_confidence: None,
            latest_snr: None,
            latest_rotation_freq: None,
            latest_lock_quality: None,
            latest_phase_error_var: None,
            processing_stopped: false,
            log_visible: true,
            _processing_handle: Some(handle),
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
                        self.history
                            .compass_trail
                            .push_back((b.bearing, b.confidence));

                        self.latest_bearing = Some(b.bearing);
                        self.latest_confidence = Some(b.confidence);
                        self.latest_snr = Some(b.snr_db);
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
        for (i, &(bearing, confidence)) in self.history.compass_trail.iter().enumerate() {
            let age_frac = if trail_len > 1 {
                i as f32 / (trail_len - 1) as f32
            } else {
                1.0
            };
            let alpha = (30.0 + age_frac * 200.0) as u8;
            let green = (confidence * 255.0).clamp(0.0, 255.0) as u8;
            let red = ((1.0 - confidence) * 255.0).clamp(0.0, 255.0) as u8;
            let color = egui::Color32::from_rgba_unmultiplied(red, green, 80, alpha);

            let angle_rad = bearing.to_radians();
            let dot_r = radius * 0.85;
            let pos = center + egui::vec2(angle_rad.sin() * dot_r, -angle_rad.cos() * dot_r);
            let dot_size = 2.0 + age_frac * 2.0;
            painter.circle_filled(pos, dot_size, color);
        }

        if let Some(bearing) = self.latest_bearing {
            let confidence = self.latest_confidence.unwrap_or(0.0);
            let angle_rad = bearing.to_radians();
            let tip = center
                + egui::vec2(
                    angle_rad.sin() * radius * 0.9,
                    -angle_rad.cos() * radius * 0.9,
                );

            let green = (confidence * 255.0).clamp(0.0, 255.0) as u8;
            let red = ((1.0 - confidence) * 255.0).clamp(0.0, 255.0) as u8;
            let needle_color = egui::Color32::from_rgb(red, green, 80);

            painter.line_segment([center, tip], egui::Stroke::new(2.5, needle_color));
            painter.circle_filled(tip, 4.0, needle_color);
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
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Confidence:").color(egui::Color32::LIGHT_GRAY));
            if let Some(c) = self.latest_confidence {
                let color = if c > 0.7 {
                    egui::Color32::from_rgb(100, 255, 100)
                } else if c > 0.4 {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::from_rgb(255, 100, 100)
                };
                ui.label(
                    egui::RichText::new(format!("{:.2}", c))
                        .color(color)
                        .strong(),
                );
            } else {
                ui.label(egui::RichText::new("---").color(egui::Color32::DARK_GRAY));
            }
        });
    }

    fn draw_plots(&self, ui: &mut egui::Ui) {
        let plot_height = 120.0;

        ui.label(
            egui::RichText::new("Bearing")
                .color(egui::Color32::LIGHT_GRAY)
                .small(),
        );
        let smoothed: PlotPoints = self.history.bearing.iter().copied().collect();
        let raw: PlotPoints = self.history.raw_bearing.iter().copied().collect();
        Plot::new("bearing_plot")
            .height(plot_height)
            .include_y(0.0)
            .include_y(360.0)
            .y_axis_label("deg")
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
            .y_axis_label("dB")
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
            .include_y(0.0)
            .include_y(1.0)
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
            .include_y(0.0)
            .include_y(1.0)
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
                    if ui
                        .small_button(if self.log_visible { "Hide" } else { "Show" })
                        .clicked()
                    {
                        self.log_visible = !self.log_visible;
                    }
                    if self.log_visible && ui.small_button("Clear").clicked() {
                        self.log_lines.clear();
                    }
                });
                if self.log_visible {
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
                }
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

    let logger = GuiLogger { tx: tx.clone() };
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

    let handle = start_processing(&args, config, tx)?;

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
        Box::new(move |cc| Ok(Box::new(RdfGuiApp::new(cc, rx, handle)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {}", e))?;

    Ok(())
}
