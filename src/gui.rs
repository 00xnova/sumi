//! The Sumi window: a live picture, and the sliders that shape it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui;
use image::RgbaImage;
use sumi::{
    ColorMode, Engine, FontMetrics, GridSpec, Params, Preset, Rgb, Stats, Style, fit_edge,
    load_image, save_image,
};

const ACCENT: egui::Color32 = egui::Color32::from_rgb(196, 72, 48);
const INK: egui::Color32 = egui::Color32::from_rgb(243, 239, 230);
const MUTED: egui::Color32 = egui::Color32::from_rgb(176, 164, 148);
const PREVIEW_EDGE: u32 = 2000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Art,
    Original,
    Split,
}

struct Job {
    generation: u64,
    source_id: u64,
    image: Arc<RgbaImage>,
    params: Params,
}

struct FrameOut {
    generation: u64,
    full: Arc<RgbaImage>,
    preview: Arc<RgbaImage>,
    ramp: Arc<RgbaImage>,
    stats: Stats,
}

enum WorkerEvent {
    Ready(FontMetrics, String),
    Frame(FrameOut),
    Failed { generation: u64, message: String },
    FontFailed(String),
}

pub fn launch(image: Option<PathBuf>) -> eframe::Result<()> {
    let icon = app_icon();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Sumi")
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([960.0, 640.0])
            .with_icon(icon)
            .with_drag_and_drop(true),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "Sumi",
        options,
        Box::new(move |cc| Ok(Box::new(SumiApp::new(cc, image)))),
    )
}

struct SumiApp {
    params: Params,
    source: Option<Arc<RgbaImage>>,
    source_id: u64,
    source_name: String,
    source_path: Option<PathBuf>,
    directory: Option<PathBuf>,
    original: Option<RgbaImage>,
    original_dirty: bool,
    full: Option<Arc<RgbaImage>>,
    preview: Option<Arc<RgbaImage>>,
    ramp: Option<Arc<RgbaImage>>,
    art_tex: Option<egui::TextureHandle>,
    original_tex: Option<egui::TextureHandle>,
    ramp_tex: Option<egui::TextureHandle>,
    stats: Option<Stats>,
    metrics: Option<FontMetrics>,
    font_name: String,
    font_ready: bool,
    error: Option<String>,
    busy: bool,
    generation: u64,
    sent_source: Option<u64>,
    sent_params: Option<Params>,
    view: View,
    fit: bool,
    zoom: f32,
    job_tx: Option<Sender<Job>>,
    event_rx: Receiver<WorkerEvent>,
    worker: Option<thread::JoinHandle<()>>,
}

impl SumiApp {
    fn new(cc: &eframe::CreationContext<'_>, image: Option<PathBuf>) -> Self {
        apply_theme(&cc.egui_ctx);
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>();
        let ctx = cc.egui_ctx.clone();
        let worker = thread::Builder::new()
            .name("sumi-render".to_string())
            .spawn(move || worker_loop(job_rx, event_tx, ctx))
            .ok();

        let mut params = Params::default();
        if let Some(storage) = cc.storage
            && let Some(saved) = eframe::get_value::<Params>(storage, "sumi-params")
        {
            params = saved.sanitize();
        }

        let mut app = Self {
            params,
            source: None,
            source_id: 0,
            source_name: String::new(),
            source_path: None,
            directory: None,
            original: None,
            original_dirty: false,
            full: None,
            preview: None,
            ramp: None,
            art_tex: None,
            original_tex: None,
            ramp_tex: None,
            stats: None,
            metrics: None,
            font_name: "Loading Japanese font…".to_string(),
            font_ready: false,
            error: None,
            busy: false,
            generation: 0,
            sent_source: None,
            sent_params: None,
            view: View::Art,
            fit: true,
            zoom: 1.0,
            job_tx: worker.as_ref().map(|_| job_tx),
            event_rx,
            worker,
        };
        if app.worker.is_none() {
            app.error = Some("Could not start the renderer.".to_string());
        }
        if let Some(path) = image {
            app.load_path(&path);
        }
        app
    }

    fn load_path(&mut self, path: &Path) {
        match load_image(path) {
            Ok(image) => {
                self.original = Some(fit_edge(&image, 1600));
                self.original_dirty = true;
                self.source = Some(Arc::new(image));
                self.source_id = self.source_id.wrapping_add(1);
                self.source_name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image".to_string());
                self.directory = path.parent().map(Path::to_path_buf);
                self.source_path = Some(path.to_path_buf());
                self.error = None;
                self.fit = true;
                self.full = None;
                self.preview = None;
                self.stats = None;
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
    }

    fn submit(&mut self) {
        let Some(image) = self.source.clone() else {
            return;
        };
        let Some(tx) = &self.job_tx else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        let job = Job {
            generation: self.generation,
            source_id: self.source_id,
            image,
            params: self.params.sanitize(),
        };
        if tx.send(job).is_err() {
            self.busy = false;
            if self.error.is_none() {
                self.error = Some("The renderer stopped.".to_string());
            }
            return;
        }
        self.busy = true;
        self.sent_source = Some(self.source_id);
        self.sent_params = Some(self.params);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                WorkerEvent::Ready(metrics, family) => {
                    self.metrics = Some(metrics);
                    self.font_name = family;
                    self.font_ready = true;
                }
                WorkerEvent::Frame(frame) => {
                    if frame.generation == self.generation {
                        self.full = Some(frame.full);
                        self.preview = Some(frame.preview);
                        self.ramp = Some(frame.ramp);
                        self.stats = Some(frame.stats);
                        self.error = None;
                        self.busy = false;
                        upload(
                            ctx,
                            &mut self.art_tex,
                            "art",
                            self.preview.as_ref().unwrap(),
                            false,
                        );
                        upload(
                            ctx,
                            &mut self.ramp_tex,
                            "ramp",
                            self.ramp.as_ref().unwrap(),
                            true,
                        );
                    }
                }
                WorkerEvent::Failed {
                    generation,
                    message,
                } => {
                    if generation == self.generation {
                        self.error = Some(message);
                        self.busy = false;
                    }
                }
                WorkerEvent::FontFailed(message) => {
                    self.error = Some(message);
                    self.font_ready = false;
                    self.busy = false;
                }
            }
        }
        if self.busy
            && let Some(worker) = &self.worker
            && worker.is_finished()
        {
            self.busy = false;
        }
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Open image").add_filter(
            "Images",
            &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff"],
        );
        if let Some(dir) = &self.directory {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.load_path(&path);
        }
    }

    fn save_dialog(&mut self, extension: &str) {
        let Some(image) = self.full.clone() else {
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title(if extension == "webp" {
                "Save WebP"
            } else {
                "Save PNG"
            })
            .add_filter(extension.to_ascii_uppercase(), &[extension])
            .set_file_name(suggested_name(self.source_path.as_deref(), extension));
        if let Some(dir) = &self.directory {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut path) = dialog.save_file() else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension(extension);
        }
        if let Err(err) = save_image(&path, &image) {
            self.error = Some(err.to_string());
        } else {
            self.directory = path.parent().map(Path::to_path_buf);
            self.error = None;
        }
    }

    fn take_drop(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.load_path(&path);
        }
    }

    fn predicted(&self) -> Option<GridSpec> {
        let metrics = self.metrics?;
        let image = self.source.as_ref()?;
        Some(sumi::output_size(
            &metrics,
            image.width(),
            image.height(),
            &self.params,
        ))
    }
}

impl eframe::App for SumiApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "sumi-params", &self.params);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.take_drop(&ctx);

        let open_shortcut =
            ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O));
        let save_webp = ui.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            )
        });
        let save_png =
            ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
        let fit_shortcut =
            ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0));

        if open_shortcut {
            self.open_dialog();
        }
        if save_png && self.full.is_some() {
            self.save_dialog("png");
        }
        if save_webp && self.full.is_some() {
            self.save_dialog("webp");
        }
        if fit_shortcut {
            self.fit = true;
        }

        let changed = self.source.is_some()
            && (self.sent_source != Some(self.source_id) || self.sent_params != Some(self.params));
        if changed {
            self.submit();
        }
        if self.original_dirty {
            if let Some(image) = &self.original {
                upload(&ctx, &mut self.original_tex, "original", image, false);
            }
            self.original_dirty = false;
        }
        if self.busy || !self.font_ready {
            ctx.request_repaint_after(Duration::from_millis(80));
        }

        let hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
        self.toolbar(ui);
        self.status_bar(ui);
        self.controls(ui);
        self.stage(ui, hovering);
    }
}

impl SumiApp {
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Sumi").strong().size(22.0));
                ui.label(
                    egui::RichText::new("Japanese character art")
                        .color(MUTED)
                        .size(13.0),
                );
                ui.separator();
                if ui.button("Open").on_hover_text("Ctrl+O").clicked() {
                    self.open_dialog();
                }
                let can_save = self.full.is_some();
                if ui
                    .add_enabled(can_save, egui::Button::new("Save PNG"))
                    .on_hover_text("Ctrl+S")
                    .clicked()
                {
                    self.save_dialog("png");
                }
                if ui
                    .add_enabled(can_save, egui::Button::new("Save WebP"))
                    .on_hover_text("Ctrl+Shift+S  ·  lossless, so the strokes stay sharp")
                    .clicked()
                {
                    self.save_dialog("webp");
                }
                if self.source.is_some() {
                    ui.separator();
                    view_button(ui, &mut self.view, View::Art, "Art");
                    view_button(ui, &mut self.view, View::Original, "Original");
                    view_button(ui, &mut self.view, View::Split, "Split");
                    ui.separator();
                    if ui.button("Fit").on_hover_text("Ctrl+0").clicked() {
                        self.fit = true;
                    }
                    if ui.button("−").clicked() {
                        self.fit = false;
                        self.zoom = (self.zoom / 1.15).max(0.05);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("+").clicked() {
                        self.fit = false;
                        self.zoom = (self.zoom * 1.15).min(12.0);
                    }
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(spec) = self.predicted() {
                    ui.label(format!("{}×{} characters", spec.columns, spec.rows));
                    ui.separator();
                    ui.label(format!("{}×{} px", spec.width, spec.height));
                    if spec.capped {
                        ui.separator();
                        ui.label(egui::RichText::new("capped at 8192 px").color(ACCENT));
                    }
                }
                if let Some(stats) = &self.stats {
                    ui.separator();
                    ui.label(format!("{} ms", stats.elapsed_ms));
                }
                if self.font_ready {
                    ui.separator();
                    ui.label(egui::RichText::new(&self.font_name).color(MUTED));
                }
                if self.busy {
                    ui.separator();
                    ui.label("Drawing…");
                }
                if let Some(name) = self.source_name() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(name).color(MUTED));
                    });
                }
            });
        });
    }

    fn source_name(&self) -> Option<&str> {
        if self.source_name.is_empty() {
            None
        } else {
            Some(&self.source_name)
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("controls")
            .resizable(true)
            .default_size(320.0)
            .min_size(260.0)
            .max_size(460.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("controls").show(ui, |ui| {
                    ui.spacing_mut().slider_width = (ui.available_width() - 62.0).max(48.0);
                    section(ui, "Picture");
                    slider_u32(ui, &mut self.params.columns, 24..=200, "Detail", "How many characters fit across the picture.");
                    slider_u32(ui, &mut self.params.cell_px, 10..=48, "Character size", "Pixel height of each character in the saved file.");
                    if let Some(spec) = self.predicted() {
                        ui.label(
                            egui::RichText::new(format!("Export {} × {} px", spec.width, spec.height))
                                .color(MUTED)
                                .size(12.0),
                        );
                    }
                    slider_u32(ui, &mut self.params.levels, 8..=64, "Characters", "How many different characters share the shading.");

                    ui.horizontal_wrapped(|ui| {
                        for style in [Style::Kanji, Style::Kana, Style::Halfwidth] {
                            if ui
                                .selectable_label(self.params.style == style, style.label())
                                .clicked()
                            {
                                self.params.style = style;
                            }
                        }
                    });
                    ui.label(egui::RichText::new(style_hint(self.params.style)).color(MUTED).size(12.0));

                    section(ui, "Tone");
                    slider_f32(ui, &mut self.params.brightness, -0.35..=0.35, "Brightness", "Shift the whole picture lighter or darker.");
                    slider_f32(ui, &mut self.params.contrast, 0.5..=2.0, "Contrast", "Push midtones apart.");
                    slider_f32(ui, &mut self.params.gamma, 0.4..=2.2, "Gamma", "Bend the shadows without moving the extremes as much.");
                    slider_f32(ui, &mut self.params.stretch, 0.0..=1.0, "Stretch tones", "Pull a flat photo out to a fuller range of light and dark.");
                    slider_f32(ui, &mut self.params.outlines, 0.0..=1.0, "Outlines", "Darken edges so faces and shapes read clearly.");
                    slider_f32(ui, &mut self.params.weight, 0.6..=2.0, "Stroke weight", "Make each character heavier or lighter.");
                    ui.checkbox(&mut self.params.invert, "Invert tones");
                    ui.checkbox(&mut self.params.dither, "Soften gradients")
                        .on_hover_text("Mix neighboring characters across smooth areas. Turn this off for flat poster shapes.");

                    section(ui, "Ink");
                    ui.horizontal(|ui| {
                        preset_button(ui, &mut self.params, Preset::Color);
                        preset_button(ui, &mut self.params, Preset::Paper);
                    });
                    ui.horizontal(|ui| {
                        preset_button(ui, &mut self.params, Preset::Screen);
                        preset_button(ui, &mut self.params, Preset::Stamp);
                    });
                    ui.radio_value(&mut self.params.color_mode, ColorMode::Image, "Colors from the photo");
                    ui.radio_value(&mut self.params.color_mode, ColorMode::Ink, "Single ink");
                    if self.params.color_mode == ColorMode::Image {
                        slider_f32(ui, &mut self.params.saturation, 0.0..=2.0, "Color", "How vivid the sampled photo colors are.");
                    }
                    color_row(ui, "Background", &mut self.params.background);
                    if self.params.color_mode == ColorMode::Ink {
                        color_row(ui, "Ink", &mut self.params.ink);
                    }

                    if let Some(tex) = &self.ramp_tex {
                        section(ui, "Characters in use");
                        ui.label(egui::RichText::new("Light to dark").color(MUTED).size(12.0));
                        let size = tex.size();
                        let height = 48.0;
                        let width = height * size[0] as f32 / (size[1].max(1) as f32);
                        egui::ScrollArea::horizontal().id_salt("ramp").show(ui, |ui| {
                            ui.image((tex.id(), egui::vec2(width, height)));
                        });
                    }

                    ui.add_space(12.0);
                    if ui.button("Reset sliders").clicked() {
                        self.params = Params::default();
                    }
                    ui.add_space(8.0);
                });
            });
    }

    fn stage(&mut self, ui: &mut egui::Ui, hovering: bool) {
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(error) = &self.error {
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(92, 36, 30))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.label(error);
                    });
                ui.add_space(8.0);
            }

            if self.source.is_none() {
                empty_state(ui, hovering, &mut || self.open_dialog());
                return;
            }

            let scroll_y = ui.input(|input| input.smooth_scroll_delta.y);
            let available = ui.available_size();
            match self.view {
                View::Original => {
                    show_fitted(ui, self.original_tex.as_ref(), available);
                }
                View::Split => {
                    ui.horizontal(|ui| {
                        let half = egui::vec2((available.x - 12.0) * 0.5, available.y);
                        show_fitted(ui, self.original_tex.as_ref(), half);
                        ui.separator();
                        show_fitted(ui, self.art_tex.as_ref(), half);
                    });
                }
                View::Art => {
                    let Some(tex) = self.art_tex.clone() else {
                        ui.centered_and_justified(|ui| {
                            ui.spinner();
                            ui.label("Drawing the first picture…");
                        });
                        return;
                    };
                    let tex_size = tex.size();
                    let img_w = tex_size[0] as f32;
                    let img_h = tex_size[1] as f32;
                    let fit_zoom = fit_factor(available, img_w, img_h);
                    if self.fit {
                        self.zoom = fit_zoom;
                    }
                    let zoom = self.zoom;
                    let display = egui::vec2(img_w * zoom, img_h * zoom);
                    let image = egui::Image::new((tex.id(), display))
                        .fit_to_exact_size(display)
                        .sense(egui::Sense::click());
                    let response = if self.fit {
                        let gap = ((available.y - display.y) * 0.5).max(0.0);
                        ui.vertical_centered(|ui| {
                            if gap > 0.0 {
                                ui.add_space(gap);
                            }
                            ui.add(image)
                        })
                        .inner
                    } else {
                        let mut source = egui::containers::scroll_area::ScrollSource::ALL;
                        source.mouse_wheel = false;
                        source.drag = egui::containers::scroll_area::DragScroll::Always;
                        egui::ScrollArea::both()
                            .id_salt("art")
                            .auto_shrink([false, false])
                            .scroll_source(source)
                            .show(ui, |ui| ui.add(image))
                            .inner
                    };
                    if response.double_clicked() {
                        self.fit = true;
                    }
                    let panel = ui.max_rect();
                    if ui.rect_contains_pointer(panel) && scroll_y.abs() > 0.0 {
                        let base = if self.fit { fit_zoom } else { self.zoom };
                        self.fit = false;
                        self.zoom = (base * (1.0 + scroll_y * 0.0012)).clamp(0.05, 12.0);
                        ui.ctx().request_repaint();
                    }
                }
            }
        });
    }
}

fn worker_loop(rx: Receiver<Job>, tx: Sender<WorkerEvent>, ctx: egui::Context) {
    let mut engine = match Engine::open() {
        Ok(engine) => engine,
        Err(err) => {
            let _ = tx.send(WorkerEvent::FontFailed(err.to_string()));
            ctx.request_repaint();
            return;
        }
    };
    let _ = tx.send(WorkerEvent::Ready(
        engine.metrics(),
        engine.family().to_string(),
    ));
    ctx.request_repaint();

    while let Ok(mut job) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        let generation = job.generation;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.render(job.source_id, &job.image, &job.params)
        }));
        let event = match outcome {
            Ok(Ok(rendered)) => {
                let full = Arc::new(rendered.image);
                let preview = if full.width().max(full.height()) > PREVIEW_EDGE {
                    Arc::new(fit_edge(&full, PREVIEW_EDGE))
                } else {
                    Arc::clone(&full)
                };
                WorkerEvent::Frame(FrameOut {
                    generation,
                    full,
                    preview,
                    ramp: Arc::new(rendered.ramp),
                    stats: rendered.stats,
                })
            }
            Ok(Err(err)) => WorkerEvent::Failed {
                generation,
                message: err.to_string(),
            },
            Err(_) => {
                engine.clear_cache();
                WorkerEvent::Failed {
                    generation,
                    message: "The picture could not be drawn. Try fewer columns, or another character set.".to_string(),
                }
            }
        };
        if tx.send(event).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn upload(
    ctx: &egui::Context,
    slot: &mut Option<egui::TextureHandle>,
    name: &str,
    image: &RgbaImage,
    nearest: bool,
) {
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    let options = if nearest {
        egui::TextureOptions::NEAREST
    } else {
        egui::TextureOptions::LINEAR
    };
    if let Some(texture) = slot {
        texture.set(color, options);
    } else {
        *slot = Some(ctx.load_texture(name, color, options));
    }
}

fn empty_state(ui: &mut egui::Ui, hovering: bool, open: &mut dyn FnMut()) {
    ui.vertical_centered(|ui| {
        let height = ui.available_height();
        let gap = if height.is_finite() {
            (height * 0.22).clamp(12.0, 220.0)
        } else {
            48.0
        };
        ui.add_space(gap);
        let title = if hovering {
            "Release to open"
        } else {
            "Drop a photo here"
        };
        ui.label(egui::RichText::new(title).strong().size(28.0));
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(
                "Sumi redraws it with Japanese characters, then saves a PNG or WebP.",
            )
            .color(MUTED),
        );
        ui.add_space(16.0);
        if ui.button("Open image").clicked() {
            open();
        }
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("PNG, JPEG, WebP, GIF, BMP, TIFF")
                .color(MUTED)
                .size(12.0),
        );
    });
}

fn show_fitted(ui: &mut egui::Ui, texture: Option<&egui::TextureHandle>, available: egui::Vec2) {
    let Some(texture) = texture else {
        ui.add_sized(available, egui::Spinner::new());
        return;
    };
    let size = texture.size();
    let zoom = fit_factor(available, size[0] as f32, size[1] as f32);
    let display = egui::vec2(size[0] as f32 * zoom, size[1] as f32 * zoom);
    ui.add(egui::Image::new((texture.id(), display)).fit_to_exact_size(display));
}

fn fit_factor(available: egui::Vec2, width: f32, height: f32) -> f32 {
    if width <= 1.0 || height <= 1.0 {
        return 1.0;
    }
    ((available.x - 8.0) / width)
        .min((available.y - 8.0) / height)
        .clamp(0.05, 8.0)
}

fn view_button(ui: &mut egui::Ui, view: &mut View, value: View, label: &str) {
    if ui.selectable_label(*view == value, label).clicked() {
        *view = value;
    }
}

fn preset_button(ui: &mut egui::Ui, params: &mut Params, preset: Preset) {
    let selected = Preset::matching(params) == Some(preset);
    if ui.selectable_label(selected, preset.label()).clicked() {
        preset.apply(params);
    }
}

fn color_row(ui: &mut egui::Ui, label: &str, color: &mut Rgb) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut channels = [color.r, color.g, color.b];
        if ui.color_edit_button_srgb(&mut channels).changed() {
            *color = Rgb::new(channels[0], channels[1], channels[2]);
        }
    });
}

fn slider_u32(
    ui: &mut egui::Ui,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
    label: &str,
    tip: &str,
) {
    ui.label(label).on_hover_text(tip);
    ui.add(egui::Slider::new(value, range)).on_hover_text(tip);
}

fn slider_f32(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    label: &str,
    tip: &str,
) {
    ui.label(label).on_hover_text(tip);
    ui.add(egui::Slider::new(value, range)).on_hover_text(tip);
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).strong().color(ACCENT).size(12.0));
    ui.separator();
}

fn style_hint(style: Style) -> &'static str {
    match style {
        Style::Kanji => "Kana for the lights, kanji where the ink gets heavy.",
        Style::Kana => "Hiragana and katakana only.",
        Style::Halfwidth => "Narrow katakana, like a terminal.",
    }
}

fn suggested_name(path: Option<&Path>, extension: &str) -> String {
    let stem = path
        .and_then(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sumi".to_string());
    format!("{stem}-sumi.{extension}")
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    let panel = egui::Color32::from_rgb(34, 29, 26);
    let bg = egui::Color32::from_rgb(20, 17, 15);
    visuals.window_fill = panel;
    visuals.panel_fill = panel;
    visuals.extreme_bg_color = bg;
    visuals.faint_bg_color = egui::Color32::from_rgb(46, 39, 35);
    visuals.code_bg_color = bg;
    visuals.override_text_color = Some(INK);
    visuals.weak_text_color = Some(MUTED);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = egui::Color32::from_rgb(226, 168, 112);
    visuals.slider_trailing_fill = true;
    visuals.window_corner_radius = egui::CornerRadius::same(8);
    visuals.menu_corner_radius = egui::CornerRadius::same(6);
    style_widget(
        &mut visuals.widgets.noninteractive,
        panel,
        MUTED,
        egui::Color32::from_rgb(72, 60, 54),
    );
    style_widget(
        &mut visuals.widgets.inactive,
        egui::Color32::from_rgb(54, 46, 41),
        INK,
        egui::Color32::from_rgb(72, 60, 54),
    );
    style_widget(
        &mut visuals.widgets.hovered,
        egui::Color32::from_rgb(86, 58, 50),
        INK,
        ACCENT,
    );
    style_widget(
        &mut visuals.widgets.active,
        ACCENT,
        egui::Color32::from_rgb(255, 248, 242),
        ACCENT,
    );
    style_widget(
        &mut visuals.widgets.open,
        egui::Color32::from_rgb(72, 48, 42),
        INK,
        ACCENT,
    );
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals(visuals);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
    });
}

fn style_widget(
    widget: &mut egui::style::WidgetVisuals,
    fill: egui::Color32,
    text: egui::Color32,
    stroke: egui::Color32,
) {
    widget.bg_fill = fill;
    widget.weak_bg_fill = fill;
    widget.fg_stroke = egui::Stroke::new(1.0, text);
    widget.bg_stroke = egui::Stroke::new(1.0, stroke);
    widget.corner_radius = egui::CornerRadius::same(6);
}

fn app_icon() -> egui::IconData {
    let size = 256u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let center = 128.0f32;
    let outer = 112.0f32;
    let inner = 96.0f32;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let index = ((y * size + x) * 4) as usize;
            if distance <= outer && distance >= inner {
                rgba[index] = 194;
                rgba[index + 1] = 58;
                rgba[index + 2] = 46;
                rgba[index + 3] = 255;
            } else if distance < inner {
                rgba[index] = 244;
                rgba[index + 1] = 239;
                rgba[index + 2] = 228;
                rgba[index + 3] = 255;
            }
        }
    }
    let arm = 16i32;
    let reach = 54i32;
    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let dx = x - 128;
            let dy = y - 126;
            let vertical = dx.abs() <= arm && dy > -reach && dy < reach - 8;
            let horizontal = dy.abs() <= arm && dx > -reach + 6 && dx < reach;
            if vertical || horizontal {
                let index = ((y as u32 * size + x as u32) * 4) as usize;
                rgba[index] = 168;
                rgba[index + 1] = 36;
                rgba[index + 2] = 30;
                rgba[index + 3] = 255;
            }
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}
