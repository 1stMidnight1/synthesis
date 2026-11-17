//! Synthesis standalone app.
//!
//! The window lives in `app.rs`. Rendering runs on a background thread
//! (`playback.rs`), and the audio callback reads prepared frames (`audio.rs`).

mod app;
mod audio;
mod canvas;
mod document;
mod playback;
mod ruler;
mod view;

#[cfg(test)]
mod ui_tests;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Synthesis")
            .with_inner_size([1280.0, 780.0])
            .with_min_inner_size([720.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Synthesis",
        options,
        Box::new(|_cc| Ok(Box::new(app::SynthesisApp::new()))),
    )
}
