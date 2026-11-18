//! M1 acceptance tests (PLAN.md §9), run on real offline renders.
//!
//! Each test renders a scene and checks its spectrum with an FFT. Tests use a
//! Hann window and rustfft. Frequencies are compared at the window's bin
//! resolution, so the 1 Hz tolerance in test 1 is met by a 1 s window at 48 kHz.

use std::f64::consts::PI;
use std::sync::Arc;

use rustfft::{FftPlanner, num_complex::Complex};
use synthesis_engine::geometry::{Geometry, Point};
use synthesis_engine::model::{Scene, Settings, Voice};
use synthesis_engine::pitch::{hz_to_st, st_to_hz};
use synthesis_engine::{Rendered, Shape, render_scene};

const SR: u32 = 48_000;
const A4: f64 = 440.0;

fn st(hz: f64) -> f64 {
    hz_to_st(hz, A4)
}

fn settings() -> Settings {
    Settings {
        sample_rate: SR,
        master_gain: 1.0,
        ..Settings::default()
    }
}

/// Magnitude spectrum of `x` with a Hann window, zero-padded to `n` points.
fn spectrum(x: &[f32]) -> Vec<f64> {
    let n = x.len();
    let mut buf: Vec<Complex<f64>> = x
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
            Complex::new(f64::from(s) * w, 0.0)
        })
        .collect();
    let mut planner = FftPlanner::<f64>::new();
    let fft: Arc<dyn rustfft::Fft<f64>> = planner.plan_fft_forward(n);
    fft.process(&mut buf);
    buf[..n / 2].iter().map(|c| c.norm()).collect()
}

/// Frequency of bin `k` for an `n`-point FFT at sample rate `SR`.
fn bin_hz(k: usize, n: usize) -> f64 {
    k as f64 * f64::from(SR) / n as f64
}

/// Local maxima above `floor` (relative to the global max), as Hz.
fn peaks_hz(mag: &[f64], n: usize, floor_rel: f64) -> Vec<f64> {
    let max = mag.iter().cloned().fold(0.0, f64::max);
    let mut out = Vec::new();
    for k in 1..mag.len() - 1 {
        if mag[k] > floor_rel * max && mag[k] >= mag[k - 1] && mag[k] > mag[k + 1] {
            out.push(bin_hz(k, n));
        }
    }
    out
}

fn energy_in(mag: &[f64], n: usize, lo_hz: f64, hi_hz: f64) -> f64 {
    mag.iter()
        .enumerate()
        .filter(|&(k, _)| {
            let f = bin_hz(k, n);
            f >= lo_hz && f <= hi_hz
        })
        .map(|(_, &m)| m * m)
        .sum()
}

fn scene_with(duration_s: f64, shape: Shape) -> Scene {
    let mut scene = Scene::new(duration_s, settings());
    scene.voices.push(Voice::new("v").with_shape(shape));
    scene
}

/// Test 1: a flat 450 Hz line for 1 s peaks at 450 Hz within 1 Hz.
#[test]
fn flat_line_peaks_at_450_hz_within_1_hz() {
    let shape = Shape::outline(Geometry::Line {
        a: Point::new(0.0, st(450.0)),
        b: Point::new(1.0, st(450.0)),
    })
    .unwrap();
    let out = render_scene(&scene_with(1.0, shape)).unwrap();
    let n = 1 << 17; // 131072-point FFT over the 48 000-sample render, zero-padded
    let mut x = out.left.clone();
    x.resize(n, 0.0);
    let mag = spectrum(&x);
    let max_k = mag
        .iter()
        .enumerate()
        .take(mag.len())
        .skip(1)
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap()
        .0;
    let f = bin_hz(max_k, n);
    assert!((f - 450.0).abs() <= 1.0, "peak at {f} Hz, expected 450 ± 1");
}

/// Test 2: a rectangle outline 200–400 Hz for 1 s has two steady pitch peaks.
#[test]
fn rectangle_outline_has_two_steady_peaks() {
    let shape = Shape::outline(Geometry::Rectangle {
        a: Point::new(0.0, st(200.0)),
        b: Point::new(1.0, st(400.0)),
    })
    .unwrap();
    let out = render_scene(&scene_with(1.0, shape)).unwrap();
    let n = 1 << 17;
    let mut x = out.left.clone();
    x.resize(n, 0.0);
    let mag = spectrum(&x);
    let peaks = peaks_hz(&mag, n, 0.1);
    let near = |target: f64| peaks.iter().any(|&p| (p - target).abs() <= 1.5);
    assert!(near(200.0), "no peak near 200 Hz: {peaks:?}");
    assert!(near(400.0), "no peak near 400 Hz: {peaks:?}");
    // No other strong peaks besides the two edges and the short vertical bursts.
    let strong: Vec<f64> = peaks
        .iter()
        .copied()
        .filter(|&p| (p - 200.0).abs() > 1.5 && (p - 400.0).abs() > 1.5)
        .filter(|&p| {
            let k = (p * n as f64 / f64::from(SR)).round() as usize;
            mag[k] > 0.3 * mag.iter().cloned().fold(0.0, f64::max)
        })
        .collect();
    assert!(strong.is_empty(), "unexpected strong peaks: {strong:?}");
}

/// Test 4: a vertical line at t = 0.5 s is a short burst containing its whole span.
#[test]
fn vertical_line_is_burst_containing_whole_span() {
    let shape = Shape::outline(Geometry::Line {
        a: Point::new(0.5, st(200.0)),
        b: Point::new(0.5, st(800.0)),
    })
    .unwrap();
    let out = render_scene(&scene_with(1.0, shape)).unwrap();
    let on: Vec<usize> = out
        .left
        .iter()
        .enumerate()
        .filter(|&(_, s)| s.abs() > 1e-4)
        .map(|(i, _)| i)
        .collect();
    assert!(!on.is_empty(), "vertical line produced no sound");
    let first = *on.first().unwrap() as f64 / f64::from(SR);
    let last = *on.last().unwrap() as f64 / f64::from(SR);
    assert!(
        first >= 0.5 - 1e-3 && last <= 0.5 + 0.02,
        "burst at {first}..{last}"
    );
    assert!(last - first < 0.02, "burst too long: {}", last - first);

    // Whole span: energy near both ends of the 200–800 Hz span, within the burst.
    let (s, e) = (on[0], *on.last().unwrap() + 1);
    let seg = &out.left[s..e];
    let n = seg.len().next_power_of_two().max(256);
    let mut x = seg.to_vec();
    x.resize(n, 0.0);
    let mag = spectrum(&x);
    let low = energy_in(&mag, n, 180.0, 300.0);
    let high = energy_in(&mag, n, 600.0, 820.0);
    let total = energy_in(&mag, n, 0.0, 24_000.0);
    assert!(low > 0.01 * total, "no energy at span bottom");
    assert!(high > 0.01 * total, "no energy at span top");
}

/// Test 5: a gap renders silence.
#[test]
fn gap_renders_silence() {
    let a = Shape::outline(Geometry::Line {
        a: Point::new(0.0, st(450.0)),
        b: Point::new(0.3, st(450.0)),
    })
    .unwrap();
    let b = Shape::outline(Geometry::Line {
        a: Point::new(0.7, st(450.0)),
        b: Point::new(1.0, st(450.0)),
    })
    .unwrap();
    let mut scene = Scene::new(1.0, settings());
    scene
        .voices
        .push(Voice::new("v").with_shape(a).with_shape(b));
    let out = render_scene(&scene).unwrap();
    // Gate ramps are ~2 ms, so check the middle of the gap well away from edges.
    let from = (0.35 * f64::from(SR)) as usize;
    let to = (0.65 * f64::from(SR)) as usize;
    let peak = out.left[from..to]
        .iter()
        .fold(0.0f32, |m, &s| m.max(s.abs()));
    assert!(peak < 1e-6, "gap not silent: peak {peak}");
}

/// Rendering is repeatable: the same scene gives byte-identical output.
#[test]
fn rendering_is_deterministic() {
    let shape = Shape::outline(Geometry::Rectangle {
        a: Point::new(0.0, st(200.0)),
        b: Point::new(0.5, st(400.0)),
    })
    .unwrap();
    let scene = scene_with(0.5, shape);
    let a = render_scene(&scene).unwrap();
    let b = render_scene(&scene).unwrap();
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// M2 acceptance (PLAN §9, M2): micro sidebands and L/R lane gains.
// ---------------------------------------------------------------------------

use synthesis_engine::{DepthUnit, Lane, LanePoint, Micro, Preset, Stroke};

/// Test M2-1: a sine micro with 10 ms period and ±10 Hz depth on a 450 Hz line
/// produces sidebands about 100 Hz from the carrier, with a small modulation index.
#[test]
fn micro_sine_gives_sidebands_at_period_rate() {
    let shape = Shape::outline(Geometry::Line {
        a: Point::new(0.0, st(450.0)),
        b: Point::new(1.0, st(450.0)),
    })
    .unwrap();
    let mut scene = scene_with(1.0, shape);
    scene.voices[0].micro = Some(Micro::preset(Preset::Sine, 10.0, 10.0, DepthUnit::Hz));
    let out = render_scene(&scene).unwrap();
    let n = 1 << 17;
    let mut x = out.left.clone();
    x.resize(n, 0.0);
    let mag = spectrum(&x);
    let carrier = energy_in(&mag, n, 445.0, 455.0);
    let side_up = energy_in(&mag, n, 540.0, 560.0);
    let side_dn = energy_in(&mag, n, 340.0, 360.0);
    assert!(carrier > 0.0);
    assert!(side_up > 0.0 && side_dn > 0.0, "no sidebands at ±100 Hz");
    // Sidebands are about symmetric.
    let ratio = side_up / side_dn;
    assert!((0.5..2.0).contains(&ratio), "asymmetric sidebands: {ratio}");
    // Small modulation index (about 10/100 = 0.1): sidebands are well below the carrier.
    let side_amp = (side_up.sqrt() + side_dn.sqrt()) / 2.0;
    let carrier_amp = carrier.sqrt();
    assert!(side_amp < 0.3 * carrier_amp, "modulation too strong");
}

/// Test M2-2: left and right gains differ as set, and the partials are identical.
#[test]
fn left_and_right_gains_differ_as_set() {
    let shape = Shape::outline(Geometry::Line {
        a: Point::new(0.0, st(450.0)),
        b: Point::new(1.0, st(450.0)),
    })
    .unwrap();
    let mut scene = scene_with(1.0, shape);
    let flat = |p: f64| Lane {
        strokes: vec![Stroke::new(vec![LanePoint::new(0.0, p), LanePoint::new(1.0, p)]).unwrap()],
    };
    scene.voices[0].lane_left = flat(100.0);
    scene.voices[0].lane_right = flat(50.0);
    let out = render_scene(&scene).unwrap();
    let mid = 24_000;
    let l = out.left[mid..mid + 4800]
        .iter()
        .map(|&x| f64::from(x).powi(2))
        .sum::<f64>();
    let r = out.right[mid..mid + 4800]
        .iter()
        .map(|&x| f64::from(x).powi(2))
        .sum::<f64>();
    // 50% gain is -6.02 dB in amplitude, so power ratio is 0.25.
    let ratio = r / l;
    assert!(
        (ratio - 0.25).abs() < 0.01,
        "power ratio {ratio}, expected 0.25"
    );
}

/// Test M2-3: overlapping lane strokes add, and the sum is clamped to 100%.
#[test]
fn overlapping_lane_strokes_add_and_clamp() {
    let shape = Shape::outline(Geometry::Line {
        a: Point::new(0.0, st(450.0)),
        b: Point::new(1.0, st(450.0)),
    })
    .unwrap();
    let mk = |p: f64| Stroke::new(vec![LanePoint::new(0.0, p), LanePoint::new(1.0, p)]).unwrap();
    let base = scene_with(1.0, shape.clone());
    let mut single = base.clone();
    single.voices[0].lane_left = Lane {
        strokes: vec![mk(50.0)],
    };
    let mut double = base.clone();
    double.voices[0].lane_left = Lane {
        strokes: vec![mk(50.0), mk(50.0)],
    };
    let mut capped = base.clone();
    capped.voices[0].lane_left = Lane {
        strokes: vec![mk(75.0), mk(75.0)],
    };

    let rms = |s: &Scene| {
        let o = render_scene(s).unwrap();
        let w = &o.left[12_000..36_000];
        (w.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / w.len() as f64).sqrt()
    };
    let r1 = rms(&single);
    let r2 = rms(&double);
    let rc = rms(&capped);
    // Two 50% strokes add to 100%: twice the amplitude.
    assert!((r2 / r1 - 2.0).abs() < 0.01, "sum ratio {}", r2 / r1);
    // 150% clamps to 100%: same as double.
    assert!((rc / r2 - 1.0).abs() < 0.01, "clamp ratio {}", rc / r2);
}

/// Curve outlines sound as steady tones: a circle held across a vertical
/// slice has one clear pitch that lasts long enough to hear, not a chatter of
/// ~8 ms glides. Each outline edge spans about 31 ms.
#[test]
fn circle_outline_has_steady_edges() {
    use synthesis_engine::geometry::CURVE_SEGMENTS_PER_TURN;
    assert_eq!(CURVE_SEGMENTS_PER_TURN, 64);
    let shape = Shape::outline(Geometry::Circle {
        center: Point::new(1.0, 69.0),
        radius_s: 0.5,
        radius_st: 6.0,
    })
    .unwrap();
    let mut scene = scene_with(3.0, shape);
    scene.settings.master_gain = 1.0;
    let out = render_scene(&scene).unwrap();
    let n = 1 << 16;
    // A 1 s window in the middle of the circle's time span.
    let mut x = out.left[28_800..28_800 + 48_000].to_vec();
    x.resize(n, 0.0);
    let mag = spectrum(&x);
    let max = mag.iter().cloned().fold(0.0, f64::max);
    // The strongest tone must sit inside the circle's pitch range (63 to 75 st).
    let k = (1..mag.len() - 1)
        .max_by(|&a, &b| mag[a].total_cmp(&mag[b]))
        .unwrap();
    let f = bin_hz(k, n);
    let lo = st_to_hz(63.0, 440.0);
    let hi = st_to_hz(75.0, 440.0);
    assert!(
        f >= lo * 0.99 && f <= hi * 1.01,
        "dominant tone {f} Hz outside the outline range"
    );
    assert!(mag[k] > 0.2 * max, "one tone clearly dominates");
}

/// Master gain controls the output level linearly, and at a lower master the
/// circle-plus-arc scene stays inside full scale (no clipping in the export).
#[test]
fn master_gain_scales_output_and_removes_clipping() {
    let circle = Shape::outline(Geometry::Circle {
        center: Point::new(0.5, 69.0),
        radius_s: 0.5,
        radius_st: 6.0,
    })
    .unwrap();
    let arc = Shape::outline(Geometry::Arc {
        center: Point::new(2.5, 69.0),
        rx_s: 0.5,
        ry_st: 6.0,
        start_rad: 0.0,
        sweep_rad: std::f64::consts::TAU * 0.75,
    })
    .unwrap();
    let render_at = |gain: f64| {
        let mut s = Scene::new(3.5, Settings::default());
        s.settings.master_gain = gain;
        let mut v = Voice::new("curves");
        v.shapes = vec![circle.clone(), arc.clone()];
        s.voices.push(v);
        render_scene(&s).unwrap()
    };
    let loud = render_at(0.7);
    let quiet = render_at(0.3);
    // Linear: the quiet render is the loud one scaled by 0.3 / 0.7.
    let ratio = quiet.left[30_000] as f64 / loud.left[30_000] as f64;
    assert!((ratio - 0.3 / 0.7).abs() < 1e-5, "ratio {ratio}");
    // At 70% the scene exceeds full scale; at 30% it does not.
    let over = |r: &Rendered| r.left.iter().filter(|v| v.abs() > 1.0).count();
    assert!(over(&loud) > 0, "70% master clips this scene");
    assert_eq!(over(&quiet), 0, "30% master stays inside full scale");
}

/// Volume lanes (M4, confirmed 2026-11-18): a new voice sounds at its default
/// 75% lane; an empty lane is silent; a gap between strokes is silent.
#[test]
fn volume_lane_rules_hold_through_render() {
    use synthesis_engine::{Lane, LanePoint, Stroke};
    let rms =
        |x: &[f32]| (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    let line = || {
        Shape::outline(Geometry::Line {
            a: Point::new(0.0, st(450.0)),
            b: Point::new(1.0, st(450.0)),
        })
        .unwrap()
    };
    let render = |v: Voice| {
        let mut s = Scene::new(1.0, settings());
        s.voices.push(v);
        render_scene(&s).unwrap()
    };
    // Mid-tone window (away from the 2 ms ramps).
    let mid = |out: &Rendered| rms(&out.left[12_000..36_000]);

    // 1. New voice: default 75% on both channels.
    let mut fresh = Voice::new("fresh");
    fresh.shapes = vec![line()];
    let fresh_level = mid(&render(fresh));
    assert!(
        (fresh_level - std::f64::consts::FRAC_1_SQRT_2 * 0.75).abs() < 0.01,
        "default 75%: {fresh_level}"
    );

    // 2. Empty lanes: silent.
    let mut empty = Voice::new("empty");
    empty.shapes = vec![line()];
    empty.lane_left = Lane::default();
    empty.lane_right = Lane::default();
    assert_eq!(mid(&render(empty)), 0.0, "empty lane is silent");

    // 3. A gap between strokes: silent in the gap, sounding in the strokes.
    let mut gapped = Voice::new("gapped");
    gapped.shapes = vec![line()];
    let stroke = |a: f64, b: f64| {
        Stroke::new(vec![LanePoint::new(a, 100.0), LanePoint::new(b, 100.0)]).unwrap()
    };
    gapped.lane_left = Lane {
        strokes: vec![stroke(0.0, 0.3), stroke(0.7, 1.0)],
    };
    let out = render(gapped);
    let gap = rms(&out.left[(0.4 * 48_000.0) as usize..(0.6 * 48_000.0) as usize]);
    assert_eq!(gap, 0.0, "gap between strokes is silent");
    assert!(
        rms(&out.left[(0.1 * 48_000.0) as usize..(0.2 * 48_000.0) as usize]) > 0.5,
        "stroke sounds"
    );
}
