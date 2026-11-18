//! The Synthesis window (M3): tool palette, transport, ruler, canvas, export.
//!
//! `SynthesisApp::ui` is the whole frame. Audio is optional, so the same app
//! runs headless under the UI test harness.

use eframe::egui;
use synthesis_engine::{BitDepth, Division, LoopRegion, PitchScale, Scene, Tempo};

use crate::audio::AudioOut;
use crate::canvas::{self, CanvasState, Tool};
use crate::document::Document;
use crate::playback::{Command, RenderHandle};
use crate::ruler::{self, RulerDrag};

/// Ring capacity in frames: about 2 s at 48 kHz.
const RING_FRAMES: usize = 96_000;

pub struct SynthesisApp {
    pub doc: Document,
    pub canvas: CanvasState,
    render: Option<RenderHandle>,
    audio: Result<AudioOut, String>,
    pub playing: bool,
    pub playhead_s: f64,
    pub loop_on: bool,
    /// Where the current play command started, and its generation.
    play_start_s: f64,
    play_loop: Option<LoopRegion>,
    generation: u32,
    ruler_drag: Option<RulerDrag>,
    /// Space was held while dragging (a pan), so its release is not Play.
    space_panned: bool,
    export_status: String,
    /// Scene last sent to the render thread, to detect edits.
    sent_scene: Option<Scene>,
}

impl SynthesisApp {
    /// The app with audio output.
    pub fn new() -> Self {
        let (render, consumer) = crate::playback::spawn(RING_FRAMES);
        let audio = AudioOut::start(consumer, render.shared.clone());
        Self::build(Some(render), audio)
    }

    /// The app without audio, for UI tests.
    #[cfg(test)]
    pub fn headless() -> Self {
        Self::build(None, Err("headless".into()))
    }

    fn build(render: Option<RenderHandle>, audio: Result<AudioOut, String>) -> Self {
        let canvas = CanvasState::new();
        Self {
            doc: Document::default(),
            canvas,
            render,
            audio,
            playing: false,
            playhead_s: 0.0,
            loop_on: false,
            play_start_s: 0.0,
            play_loop: None,
            generation: 0,
            ruler_drag: None,
            space_panned: false,
            export_status: String::new(),
            sent_scene: None,
        }
    }

    fn loop_region(&self) -> Option<LoopRegion> {
        if self.loop_on {
            self.doc
                .loop_region
                .or_else(|| LoopRegion::new(0.0, self.doc.scene.duration_s))
        } else {
            None
        }
    }

    fn start_playback(&mut self) {
        let (Some(render), Ok(out)) = (&self.render, &self.audio) else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        self.play_start_s = self.playhead_s;
        self.play_loop = self.loop_region();
        render.send(Command::Play {
            scene: Box::new(self.doc.scene.clone()),
            start_s: self.playhead_s,
            looping: self.play_loop,
            sample_rate: out.sample_rate,
            generation: self.generation,
        });
        self.sent_scene = Some(self.doc.scene.clone());
        self.playing = true;
    }

    fn stop_playback(&mut self) {
        if let Some(render) = &self.render {
            self.generation = self.generation.wrapping_add(1);
            render.send(Command::Stop {
                generation: self.generation,
            });
        }
        self.playing = false;
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.stop_playback();
        } else {
            if self.playhead_s >= self.doc.scene.duration_s - 1e-6 {
                self.playhead_s = self.loop_region().map_or(0.0, |r| r.start_s);
            }
            self.start_playback();
        }
    }

    /// Move the playhead. While playing, playback continues from there.
    pub fn seek(&mut self, t_s: f64) {
        self.playhead_s = t_s.clamp(0.0, self.doc.scene.duration_s);
        if self.playing {
            self.start_playback();
        }
    }

    /// Follow the audio: the playhead is the start position plus the frames
    /// the device has actually played, wrapped by the same loop rule.
    fn update_playhead(&mut self) {
        let (Some(render), Ok(out)) = (&self.render, &self.audio) else {
            return;
        };
        if !self.playing {
            return;
        }
        let frames = render.shared.played_frames(self.generation);
        let elapsed = frames as f64 / f64::from(out.sample_rate);
        self.playhead_s =
            synthesis_engine::transport::timeline_pos(self.play_start_s, elapsed, self.play_loop);
        if self.play_loop.is_none() && self.playhead_s >= self.doc.scene.duration_s {
            self.playhead_s = self.doc.scene.duration_s;
            self.stop_playback();
        }
    }

    /// Send edits to the render thread so playback reflects them immediately.
    fn sync_playback(&mut self) {
        if !self.playing {
            return;
        }
        if self.loop_region() != self.play_loop {
            // Loop changed: restart from where the audio is now.
            self.start_playback();
            return;
        }
        if self.sent_scene.as_ref() != Some(&self.doc.scene)
            && let Some(render) = &self.render
        {
            render.send(Command::Update {
                scene: Box::new(self.doc.scene.clone()),
            });
            self.sent_scene = Some(self.doc.scene.clone());
        }
    }

    fn export_wav(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .set_file_name("synthesis.wav")
            .save_file()
        else {
            return;
        };
        self.export_status = match export(&self.doc.scene, &path) {
            Ok(()) => format!("Exported {}", path.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    /// Master output gain, 0 to 100% (OUT-01). Shared by live playback and
    /// export, since both render from the scene's settings. A drag is one
    /// undo step: the value is committed when the slider is released.
    fn master_gain_control(&mut self, ui: &mut egui::Ui) {
        let mut percent = self.doc.scene.settings.master_gain * 100.0;
        ui.label("Master");
        let resp = ui.add(
            egui::Slider::new(&mut percent, 0.0..=100.0)
                .suffix(" %")
                .fixed_decimals(0)
                .clamping(egui::SliderClamping::Always),
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, "Master gain"));
        let new = percent / 100.0;
        if (new - self.doc.scene.settings.master_gain).abs() > 1e-9 && !resp.dragged() {
            self.set_master_gain(new);
        } else if resp.dragged() {
            // Live preview during the drag without an undo step per frame.
            self.doc.scene.settings.master_gain = new;
            self.sync_playback_now();
        }
        if resp.drag_stopped() {
            self.set_master_gain(new);
        }
    }

    pub(crate) fn set_master_gain(&mut self, gain: f64) {
        let before = self.doc.scene.settings.master_gain;
        if (gain - before).abs() < 1e-9 {
            return;
        }
        // Undo must restore the value from before the drag, not the live preview.
        self.doc.scene.settings.master_gain = before;
        self.doc.edit(|s| s.settings.master_gain = gain);
        self.sync_playback_now();
    }

    /// Send the current scene to live playback immediately (used by master gain).
    fn sync_playback_now(&mut self) {
        if self.playing
            && let Some(render) = &self.render
        {
            render.send(Command::Update {
                scene: Box::new(self.doc.scene.clone()),
            });
            self.sent_scene = Some(self.doc.scene.clone());
        }
    }

    pub(crate) fn undo(&mut self) {
        if self.doc.undo() {
            self.canvas.selected.clear();
            self.canvas.status = "Undone".into();
        }
    }

    fn redo(&mut self) {
        if self.doc.redo() {
            self.canvas.selected.clear();
            self.canvas.status = "Redo".into();
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Synthesis");
            ui.separator();
            let label = if self.playing { "Stop" } else { "Play" };
            let can_play = self.audio.is_ok();
            if ui
                .add_enabled(can_play, egui::Button::new(label))
                .on_hover_text("Space")
                .clicked()
            {
                self.toggle_play();
            }
            if ui.button("To start").on_hover_text("Home").clicked() {
                self.seek(0.0);
            }
            ui.toggle_value(&mut self.loop_on, "Loop")
                .on_hover_text("Shift-drag on the ruler to set a loop range");
            ui.separator();
            let (bar, beat, sixteenth) = Tempo::new(self.canvas.bpm)
                .map(|t| t.position(self.playhead_s))
                .unwrap_or((1, 1, 1));
            ui.monospace(format!(
                "{:6.2} s   {bar}.{beat}.{sixteenth}",
                self.playhead_s
            ));
            ui.separator();
            ui.add(
                egui::DragValue::new(&mut self.canvas.bpm)
                    .range(20.0..=300.0)
                    .speed(0.5)
                    .suffix(" BPM"),
            );
            ui.separator();
            if ui
                .add_enabled(self.doc.history.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(self.doc.history.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                self.redo();
            }
            ui.separator();
            self.master_gain_control(ui);
            ui.separator();
            if ui.button("Export WAV...").clicked() {
                self.export_wav();
            }
        });
    }

    fn tool_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sketch");
        for tool in Tool::ALL {
            let key = tool
                .shortcut()
                .map(|k| format!("  {}", k.name()))
                .unwrap_or_default();
            if ui
                .selectable_label(self.canvas.tool == tool, format!("{}{key}", tool.label()))
                .clicked()
            {
                self.canvas.set_tool(tool);
            }
        }
        if self.canvas.tool == Tool::Polygon {
            ui.add(
                egui::DragValue::new(&mut self.canvas.polygon_sides)
                    .range(3..=64)
                    .prefix("Sides: "),
            );
        }
        ui.separator();
        ui.checkbox(&mut self.canvas.snap, "Snap to grid");
        ui.label("Grid divisions");
        typed_division(ui, "X (time, per beat)", &mut self.canvas.x_div);
        typed_division(ui, "Y (pitch, per step)", &mut self.canvas.y_div);
        ui.checkbox(&mut self.canvas.show_grid, "Show grid");
        ui.separator();
        ui.label("Pitch axis");
        for (scale, name) in [
            (PitchScale::Semitones, "Semitones"),
            (PitchScale::LogHz, "Log Hz"),
            (PitchScale::LinearHz, "Linear Hz"),
        ] {
            if ui
                .selectable_label(self.doc.pitch.scale == scale, name)
                .clicked()
            {
                self.doc.pitch.scale = scale;
            }
        }
        if ui.button("Zoom to fit (Z)").clicked() {
            canvas::zoom_to_fit(&mut self.doc);
        }
        ui.separator();
        ui.small(format!(
            "Timeline {:.0} s, {} shape(s)",
            self.doc.scene.duration_s,
            self.doc.scene.voices[self.doc.selected_voice].shapes.len()
        ));
        match &self.audio {
            Ok(out) => {
                ui.small(format!(
                    "Output {} Hz, {} ch",
                    out.sample_rate, out.channels
                ))
                .on_hover_text(format!("{} ({})", out.device_name, out.sample_format));
            }
            Err(e) if e != "headless" => {
                ui.colored_label(egui::Color32::RED, format!("Audio unavailable: {e}"));
            }
            Err(_) => {}
        }
        ui.separator();
        ui.small(
            "Navigate: two-finger scroll pans, pinch or Cmd-scroll zooms, mouse wheel zooms. \
             Middle/right-drag or Space-drag pans. Alt disables snapping.",
        );
    }

    fn global_keys(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (undo, redo, space, home, fit) = ui.input(|i| {
            let m = i.modifiers;
            (
                m.command && !m.shift && i.key_pressed(egui::Key::Z),
                (m.command && m.shift && i.key_pressed(egui::Key::Z))
                    || (m.command && i.key_pressed(egui::Key::Y)),
                // Space toggles play unless it is being held to pan.
                i.key_released(egui::Key::Space) && !m.any(),
                i.key_pressed(egui::Key::Home),
                !m.any() && i.key_pressed(egui::Key::Z),
            )
        });
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if space {
            if !self.space_panned {
                self.toggle_play();
            }
            self.space_panned = false;
        }
        if home {
            self.seek(0.0);
        }
        if fit {
            canvas::zoom_to_fit(&mut self.doc);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.update_playhead();
        if self.playing {
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.key_down(egui::Key::Space) && i.pointer.any_down()) {
            self.space_panned = true;
        }
        self.global_keys(ui);

        egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("tools")
            .resizable(false)
            .exact_size(170.0)
            .show(ui, |ui| self.tool_panel(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let now = ui.input(|i| i.time);
                let hint = self.canvas.tool.hint(self.canvas.pending.len());
                ui.label(egui::RichText::new(hint).small().weak());
                if let Some((msg, at)) = &self.canvas.message
                    && now - at < 4.0
                {
                    ui.separator();
                    ui.label(egui::RichText::new(msg).small());
                }
                if !self.export_status.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&self.export_status).small());
                }
            });
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let bpm = self.canvas.bpm;
            if let Some(t) = ruler::show(
                ui,
                &mut self.doc,
                self.playhead_s,
                bpm,
                &mut self.ruler_drag,
            ) {
                self.seek(t);
            }
            let a4 = self.doc.scene.settings.a4_hz;
            let resp = canvas::show(ui, &mut self.doc, &mut self.canvas, a4);
            // Playhead across the canvas.
            let x = resp.rect.left()
                + ((self.playhead_s - self.doc.time.start_s) / self.doc.time.width_s) as f32
                    * resp.rect.width();
            if resp.rect.x_range().contains(x) {
                ui.painter().line_segment(
                    [
                        egui::pos2(x, resp.rect.top()),
                        egui::pos2(x, resp.rect.bottom()),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_white_alpha(140)),
                );
            }
            if let Some(r) = self.doc.loop_region.filter(|_| self.loop_on) {
                for t in [r.start_s, r.end_s] {
                    let x = resp.rect.left()
                        + ((t - self.doc.time.start_s) / self.doc.time.width_s) as f32
                            * resp.rect.width();
                    if resp.rect.x_range().contains(x) {
                        ui.painter().line_segment(
                            [
                                egui::pos2(x, resp.rect.top()),
                                egui::pos2(x, resp.rect.bottom()),
                            ],
                            egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgba_unmultiplied(110, 165, 255, 90),
                            ),
                        );
                    }
                }
            }
        });

        self.sync_playback();
    }
}

impl Default for SynthesisApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for SynthesisApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        SynthesisApp::ui(self, ui);
    }
}

/// A `1/N` field: type N (or `1/N`) and press Enter or leave the field.
/// While the field has focus the typed text is kept in egui's temporary
/// storage. On commit, valid text sets the division and invalid text is
/// dropped, so the field shows the last valid value again.
fn typed_division(ui: &mut egui::Ui, label: &str, div: &mut Division) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).small());
        ui.monospace("1/");
        let id = ui.make_persistent_id(label);
        let typed: Option<String> = ui.data(|d| d.get_temp(id));
        let mut text = typed.unwrap_or_else(|| div.denom.to_string());
        let resp = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(56.0)
                .hint_text("N"),
        );
        // Name the field for assistive technology, as the visible label does.
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
        let enter = resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if resp.lost_focus() || enter {
            if let Some(d) = Division::parse(&text) {
                *div = d;
            }
            ui.data_mut(|d| d.remove::<String>(id));
        } else if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, text));
        }
    });
}

fn export(scene: &Scene, path: &std::path::Path) -> Result<(), String> {
    let rendered = synthesis_engine::render_scene(scene).map_err(|e| e.to_string())?;
    synthesis_engine::write_wav_file(path, &rendered, BitDepth::Int24).map_err(|e| e.to_string())
}
