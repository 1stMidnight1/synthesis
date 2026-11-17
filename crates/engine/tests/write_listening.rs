//! Writes the four curve variants as WAVs for listening, to the folder named
//! in SYNTHESIS_LISTEN. Run with: SYNTHESIS_LISTEN=<dir> cargo test -p synthesis-engine --test write_listening
use synthesis_engine::{BitDepth, CurveSound, Geometry, Point, Scene, Settings, Shape, Voice, render_scene, write_wav_file};

#[test]
fn write_listening_files() {
    let Ok(dir) = std::env::var("SYNTHESIS_LISTEN") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A circle and an arc, each over 2 s, one after another.
    let circle = Shape::outline(Geometry::Circle { center: Point::new(0.5, 69.0), radius_s: 0.5, radius_st: 6.0 }).unwrap();
    let arc = Shape::outline(Geometry::Arc {
        center: Point::new(2.5, 69.0),
        rx_s: 0.5,
        ry_st: 6.0,
        start_rad: 0.0,
        sweep_rad: std::f64::consts::TAU * 0.75,
    })
    .unwrap();
    for (name, mode) in [
        ("curve_literal.wav", CurveSound::Literal),
        ("curve_long_fade.wav", CurveSound::LongFade),
        ("curve_upper_only.wav", CurveSound::UpperOnly),
        ("curve_lower_only.wav", CurveSound::LowerOnly),
    ] {
        let mut s = Scene::new(3.5, Settings::default());
        s.settings.master_gain = 0.7;
        let mut v = Voice::new("curves");
        v.shapes = vec![circle.clone(), arc.clone()];
        v.curve_sound = mode;
        s.voices.push(v);
        let rendered = render_scene(&s).unwrap();
        write_wav_file(&dir.join(name), &rendered, BitDepth::Int24).unwrap();
    }
}
