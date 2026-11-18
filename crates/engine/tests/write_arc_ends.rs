//! Writes isolated arcs, literal and long-fade, for listening to the start and
//! end clicks. Run with: SYNTHESIS_LISTEN=<dir> cargo test -p synthesis-engine --test write_arc_ends
use std::f64::consts::PI;
use synthesis_engine::{
    BitDepth, CurveSound, Geometry, Point, Scene, Settings, Shape, Voice, render_scene,
    write_wav_file,
};

#[test]
fn write_arc_ends() {
    let Ok(dir) = std::env::var("SYNTHESIS_LISTEN") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    // One arc per second, so each start and end is separated by silence.
    let mut shapes = Vec::new();
    for k in 0..4 {
        let t0 = 0.5 + k as f64 * 1.5;
        shapes.push(
            Shape::outline(Geometry::Arc {
                center: Point::new(t0 + 0.5, 69.0),
                rx_s: 0.5,
                ry_st: 6.0,
                start_rad: 0.0,
                sweep_rad: PI,
            })
            .unwrap(),
        );
    }
    for (name, mode) in [
        ("arc_ends_literal.wav", CurveSound::Literal),
        ("arc_ends_long_fade.wav", CurveSound::LongFade),
    ] {
        let mut s = Scene::new(7.0, Settings::default());
        s.settings.master_gain = 0.5;
        let mut v = Voice::new("arcs");
        v.shapes = shapes.clone();
        v.curve_sound = mode;
        s.voices.push(v);
        write_wav_file(&dir.join(name), &render_scene(&s).unwrap(), BitDepth::Int24).unwrap();
    }
}
