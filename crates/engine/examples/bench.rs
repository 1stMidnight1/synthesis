//! Render-speed benchmark (M2.5). `cargo run --release -p synthesis-engine --example bench`
use std::time::Instant;
use synthesis_engine::pitch::hz_to_st;
use synthesis_engine::{Geometry, Point, Scene, Settings, Shape, Voice, render_scene};

fn time(name: &str, shapes: Vec<Shape>, voices: usize, dur: f64) -> f64 {
    let mut s = Scene::new(dur, Settings::default());
    for i in 0..voices {
        let mut v = Voice::new(format!("v{i}"));
        v.shapes = shapes.clone();
        s.voices.push(v);
    }
    let reps = 3;
    let mut best = f64::INFINITY;
    for _ in 0..reps {
        let t = Instant::now();
        std::hint::black_box(render_scene(&s).unwrap());
        best = best.min(t.elapsed().as_secs_f64());
    }
    let x = dur / best;
    println!("{name:32} {dur:4.0} s audio in {best:6.3} s = {x:7.1}x realtime");
    x
}

fn main() {
    let st = |hz: f64| hz_to_st(hz, 440.0);
    let line = Shape::outline(Geometry::Line {
        a: Point::new(0.0, st(450.0)),
        b: Point::new(10.0, st(450.0)),
    })
    .unwrap();
    let fill1 = Shape::filled(Geometry::Rectangle {
        a: Point::new(0.0, st(200.0)),
        b: Point::new(10.0, st(400.0)),
    })
    .unwrap();
    let fill5 = Shape::filled(Geometry::Rectangle {
        a: Point::new(0.0, st(100.0)),
        b: Point::new(10.0, st(3200.0)),
    })
    .unwrap();
    let circle = Shape::filled(Geometry::Circle {
        center: Point::new(5.0, st(500.0)),
        radius_s: 5.0,
        radius_st: 18.0,
    })
    .unwrap();
    time("one line", vec![line.clone()], 1, 10.0);
    time("filled 1 octave", vec![fill1], 1, 10.0);
    time("filled 5 octaves", vec![fill5.clone()], 1, 10.0);
    time("filled circle 3 octaves", vec![circle], 1, 10.0);
    time("4 voices x 5-octave fill", vec![fill5], 4, 10.0);
    time("8 voices x one line", vec![line], 8, 10.0);
}
