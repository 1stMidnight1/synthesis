//! Synthesis standalone app (M3): pitch canvas, transport, and WAV export.
//!
//! UI lives here and in `canvas.rs`. Rendering runs on a background thread
//! (`playback.rs`), and the audio callback reads prepared frames (`audio.rs`).
//! The UI never waits on the audio thread.

mod audio;
mod canvas;
mod document;
mod playback;

use eframe::egui;
use synthesis_engine::{BitDepth, PitchScale, Scene, Tempo, render_scene, write_wav_file};

use audio::AudioOut;
use canvas::{CanvasState, Tool};
use document::Document;
use playback::{Command, RenderHandle};

/// Ring capacity in frames: about 1 s at 48 kHz, enough headroom for a few blocks.
const RING_FRAMES: usize = 48_000;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Synthesis")
            .with_inner_size([1200.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Synthesis",
        options,
        Box::new(|_cc| Ok(Box::new(SynthesisApp::new()))),
    )
}

struct SynthesisApp {
    doc: Document,
    canvas: CanvasState,
    render: RenderHandle,
    audio: Result<AudioOut, String>,
    playing: bool,
    playhead_s: f64,
    loop_on: bool,
    export_status: String,
}

impl SynthesisApp {
    fn new() -> Self {
        let (render, consumer) = playback::spawn(RING_FRAMES);
        let shared = render.shared.clone();
        let audio = AudioOut::start(consumer, shared);
        let mut canvas = CanvasState::new();
        canvas.status = "Draw with the tools. Middle-drag to pan, wheel to zoom.".into();
        Self {
            doc: Document::default(),
            canvas,
            render,
            audio,
            playing: false,
            playhead_s: 0.0,
            loop_on: false,
            export_status: String::new(),
        }
    }

    fn a4(&self) -> f64 {
        self.doc.scene.settings.a4_hz
    }

    /// Restart playback from the current playhead with the current scene.
    fn start_playback(&mut self) {
        self.render.send(Command::Load {
            scene: Box::new(self.doc.scene.clone()),
            start_s: self.playhead_s,
        });
        self.playing = true;
    }

    fn stop_playback(&mut self) {
        self.render.send(Command::Stop);
        self.playing = false;
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.stop_playback();
        } else {
            if self.playhead_s >= self.doc.scene.duration_s {
                self.playhead_s = 0.0;
            }
            self.start_playback();
        }
    }

    /// Advance the playhead using wall time, wrapping at the loop or the end.
    fn tick_playhead(&mut self, dt: f64) {
        if !self.playing {
            return;
        }
        let loop_region = if self.loop_on {
            synthesis_engine::LoopRegion::new(0.0, self.doc.scene.duration_s)
        } else {
            None
        };
        self.playhead_s = synthesis_engine::transport::advance(
            self.playhead_s,
            dt,
            self.doc.scene.duration_s,
            loop_region,
        );
        if !self.loop_on && self.playhead_s >= self.doc.scene.duration_s {
            self.stop_playback();
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
}

fn export(scene: &Scene, path: &std::path::Path) -> Result<(), String> {
    let rendered = render_scene(scene).map_err(|e| e.to_string())?;
    write_wav_file(path, &rendered, BitDepth::Int24).map_err(|e| e.to_string())
}

impl eframe::App for SynthesisApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let dt = f64::from(ui.input(|i| i.stable_dt)).min(0.1);
        self.tick_playhead(dt);
        if self.playing {
            ui.ctx().request_repaint();
        }

        let undo_pressed =
            ui.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z));
        let redo_pressed =
            ui.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z));
        if undo_pressed {
            self.doc.undo();
        }
        if redo_pressed {
            self.doc.redo();
        }

        egui::Panel::top("top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Synthesis");
                ui.separator();
                let label = if self.playing { "Stop" } else { "Play" };
                if ui
                    .add_enabled(self.audio.is_ok(), egui::Button::new(label))
                    .clicked()
                {
                    self.toggle_play();
                }
                if ui.button("Rewind").clicked() {
                    self.stop_playback();
                    self.playhead_s = 0.0;
                }
                ui.checkbox(&mut self.loop_on, "Loop");
                ui.separator();
                let (bar, beat, sixteenth) = Tempo::new(self.canvas.bpm)
                    .map(|t| t.position(self.playhead_s))
                    .unwrap_or((1, 1, 1));
                ui.monospace(format!(
                    "{:6.2} s  |  {bar}.{beat}.{sixteenth}",
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
                    self.doc.undo();
                }
                if ui
                    .add_enabled(self.doc.history.can_redo(), egui::Button::new("Redo"))
                    .clicked()
                {
                    self.doc.redo();
                }
                ui.separator();
                if ui.button("Export WAV").clicked() {
                    self.export_wav();
                }
            });
        });

        egui::Panel::left("tools").show(ui, |ui| {
            ui.heading("Tools");
            for tool in Tool::ALL {
                if ui
                    .selectable_label(self.canvas.tool == tool, tool.label())
                    .clicked()
                {
                    self.canvas.tool = tool;
                    self.canvas.pending.clear();
                    self.canvas.drag_start = None;
                }
            }
            ui.separator();
            ui.checkbox(&mut self.canvas.fill_new, "Fill new shapes");
            ui.checkbox(&mut self.canvas.snap, "Snap");
            ui.checkbox(&mut self.canvas.show_grid, "Grid");
            ui.separator();
            ui.label("Pitch scale");
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
            ui.separator();
            ui.label(format!("Duration: {:.0} s", self.doc.scene.duration_s));
            ui.label(format!("Voices: {}", self.doc.scene.voices.len()));
            if let Ok(out) = &self.audio {
                ui.label(format!(
                    "Output: {} | {} Hz | {} ch | {}",
                    out.device_name, out.sample_rate, out.channels, out.sample_format
                ));
            } else if let Err(e) = &self.audio {
                ui.colored_label(egui::Color32::RED, format!("Audio unavailable: {e}"));
            }
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.canvas.status);
                if !self.export_status.is_empty() {
                    ui.separator();
                    ui.label(&self.export_status);
                }
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            let a4 = self.a4();
            canvas::show(ui, &mut self.doc, &mut self.canvas, a4);
        });
    }
}
