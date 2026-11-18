//! Performance guard (M2.5): optimised rendering must stay within one 24-bit
//! quantisation step (2^-23, about -138.5 dBFS) of the reference engine. The
//! fast sine polynomial (about 1e-11 error) and the reordered lattice sum
//! change the last bit of some float samples, so exact equality is not the
//! contract; the export quantises to 24 bits, so the audible contract is
//! that step.
//!
//! `SYNTHESIS_GOLDEN_WRITE=<file>` writes a reference render; the test then
//! compares against `SYNTHESIS_GOLDEN=<file>` when set. Speed targets are in
//! `speed_targets` (release builds only; debug is too slow to judge).

use std::f64::consts::PI;
use synthesis_engine::lanes::{Lane, LanePoint, Stroke};
use synthesis_engine::pitch::hz_to_st;
use synthesis_engine::{
    CurveSound, DepthUnit, Geometry, Micro, Point, Preset, Scene, SceneRenderer, Settings, Shape,
    Voice, render_scene,
};

fn st(hz: f64) -> f64 {
    hz_to_st(hz, 440.0)
}

/// A scene that uses every synthesis path.
pub fn golden_scene() -> Scene {
    let mut s = Scene::new(4.0, Settings::default());
    s.settings.master_gain = 0.5;
    let mut a = Voice::new("a");
    a.shapes = vec![
        Shape::outline(Geometry::Line {
            a: Point::new(0.0, st(220.0)),
            b: Point::new(3.0, st(330.0)),
        })
        .unwrap(),
        Shape::filled(Geometry::Rectangle {
            a: Point::new(0.5, st(300.0)),
            b: Point::new(2.0, st(600.0)),
        })
        .unwrap(),
        Shape::outline(Geometry::Line {
            a: Point::new(2.5, st(150.0)),
            b: Point::new(2.5, st(900.0)),
        })
        .unwrap(),
        Shape::filled(Geometry::Polygon {
            points: vec![
                Point::new(1.0, st(700.0)),
                Point::new(3.5, st(700.0)),
                Point::new(2.2, st(1400.0)),
            ],
        })
        .unwrap(),
        Shape::outline(Geometry::Polyline {
            points: vec![
                Point::new(0.2, st(500.0)),
                Point::new(1.2, st(800.0)),
                Point::new(2.4, st(450.0)),
            ],
        })
        .unwrap(),
    ];
    a.micro = Some(Micro::preset(Preset::Sine, 12.0, 0.3, DepthUnit::Semitones));
    a.lane_left = Lane {
        strokes: vec![
            Stroke::new(vec![LanePoint::new(0.0, 100.0), LanePoint::new(4.0, 40.0)]).unwrap(),
        ],
    };
    a.lane_right = Lane {
        strokes: vec![
            Stroke::new(vec![LanePoint::new(0.0, 60.0), LanePoint::new(4.0, 100.0)]).unwrap(),
        ],
    };
    let mut b = Voice::new("b");
    b.shapes = vec![
        Shape::filled(Geometry::Circle {
            center: Point::new(2.0, st(250.0)),
            radius_s: 0.8,
            radius_st: 5.0,
        })
        .unwrap(),
        Shape::outline(Geometry::Arc {
            center: Point::new(1.5, st(1000.0)),
            rx_s: 1.0,
            ry_st: 4.0,
            start_rad: 0.2,
            sweep_rad: 1.4 * PI,
        })
        .unwrap(),
        Shape::outline(Geometry::Ellipse {
            center: Point::new(3.0, st(500.0)),
            rx_s: 0.6,
            ry_st: 3.0,
        })
        .unwrap(),
    ];
    b.curve_sound = CurveSound::LongFade;
    s.voices.push(a);
    s.voices.push(b);
    s
}

/// Offline render plus a block render with a seek, interleaved, as one buffer.
pub fn golden_render() -> Vec<f32> {
    let s = golden_scene();
    let whole = render_scene(&s).unwrap();
    let mut out = whole.left.clone();
    out.extend_from_slice(&whole.right);
    let mut r = SceneRenderer::new(&s).unwrap();
    out.extend_from_slice(&r.next_block(30_000).left);
    r.seek(2.7);
    out.extend_from_slice(&r.next_block(20_000).left);
    let mut p = SceneRenderer::for_playback(&s, 44_100).unwrap();
    out.extend_from_slice(&p.next_block(22_050).left);
    out
}

#[test]
fn matches_reference_render() {
    let got = golden_render();
    if let Ok(path) = std::env::var("SYNTHESIS_GOLDEN_WRITE") {
        let bytes: Vec<u8> = got.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(path, bytes).unwrap();
        return;
    }
    let Ok(path) = std::env::var("SYNTHESIS_GOLDEN") else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let want: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    assert_eq!(got.len(), want.len());
    let max = got
        .iter()
        .zip(&want)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let first = got
        .iter()
        .zip(&want)
        .position(|(a, b)| a.to_bits() != b.to_bits());
    println!(
        "GOLDEN_MAX_DIFF {max:e} FIRST_BIT_DIFF {first:?} of {}",
        got.len()
    );
    assert!(
        max <= 2f32.powi(-23),
        "output changed by more than one 24-bit step: {max:e}"
    );
}
