//! Synthesis standalone app.
//!
//! M0 spike: an eframe/egui window with a test tone played through cpal,
//! proving the UI and audio stack on macOS while staying portable to Windows.

mod audio;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use audio::{AudioOut, Shared};
use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Synthesis")
            .with_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Synthesis",
        options,
        Box::new(|_cc| Ok(Box::new(SynthesisApp::new()))),
    )
}

struct SynthesisApp {
    shared: Arc<Shared>,
    audio: Result<AudioOut, String>,
    freq_hz: f32,
    gain_percent: f32,
}

impl SynthesisApp {
    fn new() -> Self {
        let shared = Arc::new(Shared::default());
        let audio = AudioOut::start(shared.clone());
        Self {
            freq_hz: shared.freq_hz.load(),
            gain_percent: shared.gain.load() * 100.0,
            shared,
            audio,
        }
    }
}

impl eframe::App for SynthesisApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Synthesis");
                ui.separator();
                ui.label("M0 spike: test tone");
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            match &self.audio {
                Ok(out) => {
                    ui.label(format!(
                        "Output: {}  |  {} Hz  |  {} ch  |  {}",
                        out.device_name, out.sample_rate, out.channels, out.sample_format
                    ));
                }
                Err(e) => {
                    ui.colored_label(egui::Color32::RED, format!("Audio unavailable: {e}"));
                }
            }
            ui.add_space(12.0);

            let playing = self.shared.playing.load(Ordering::Relaxed);
            let label = if playing { "Stop" } else { "Play" };
            if ui
                .add_enabled(self.audio.is_ok(), egui::Button::new(label))
                .clicked()
            {
                self.shared.playing.store(!playing, Ordering::Relaxed);
            }

            if ui
                .add(
                    egui::Slider::new(&mut self.freq_hz, 20.0..=20_000.0)
                        .logarithmic(true)
                        .suffix(" Hz")
                        .text("Frequency"),
                )
                .changed()
            {
                self.shared.freq_hz.store(self.freq_hz);
            }

            if ui
                .add(
                    egui::Slider::new(&mut self.gain_percent, 0.0..=100.0)
                        .suffix(" %")
                        .text("Master gain"),
                )
                .changed()
            {
                self.shared.gain.store(self.gain_percent / 100.0);
            }
        });
    }
}
