//! Tests – Port der Capso `EditorKitTests` (SpringSimulator, ZoomInterpolator, CursorSmoother, AutoZoomDetector,
//! TrimRegion, FrameTransform, CursorOverlayProvider, CursorFollowPath, Layout).
use super::model::*;
use serde_json::json;

const EPS: f64 = 1e-9;

// ───────────── SpringSimulator ─────────────

#[test]
fn spring_at_rest_stays_at_target() {
    let mut s = SpringSimulator::new(100.0, 10.0, 1.0);
    s.reset(5.0, 0.0);
    let r = s.step(5.0, 1.0 / 60.0);
    assert!((r.position - 5.0).abs() < EPS && r.velocity.abs() < EPS);
}

#[test]
fn spring_moves_toward_target() {
    let mut s = SpringSimulator::new(100.0, 10.0, 1.0);
    let r = s.step(1.0, 0.1);
    assert!(r.position > 0.0 && r.position < 1.0);
}

#[test]
fn spring_converges_after_many_steps() {
    let mut s = SpringSimulator::new(100.0, 20.0, 1.0);
    for _ in 0..100 {
        s.step(1.0, 1.0 / 60.0);
    }
    assert!((s.position - 1.0).abs() < 0.01);
}

#[test]
fn stiff_spring_reaches_target_faster() {
    let mut stiff = SpringSimulator::new(400.0, 30.0, 1.0);
    let mut soft = SpringSimulator::new(50.0, 10.0, 2.0);
    for _ in 0..12 {
        stiff.step(1.0, 1.0 / 60.0);
        soft.step(1.0, 1.0 / 60.0);
    }
    assert!(stiff.position > soft.position);
}

#[test]
fn underdamped_spring_overshoots() {
    let mut s = SpringSimulator::new(200.0, 5.0, 1.0);
    let max = (0..200).map(|_| s.step(1.0, 1.0 / 60.0).position).fold(0.0, f64::max);
    assert!(max > 1.0);
}

#[test]
fn critically_and_overdamped_converge_without_overshoot() {
    for (k, c) in [(100.0, 20.0), (100.0, 60.0)] {
        let mut s = SpringSimulator::new(k, c, 1.0);
        let mut max: f64 = 0.0;
        for _ in 0..600 {
            max = max.max(s.step(1.0, 1.0 / 60.0).position);
        }
        assert!(max <= 1.0 + 1e-6, "Überschwingen bei c={c}");
        assert!((s.position - 1.0).abs() < 1e-3);
    }
}

#[test]
fn spring2d_converges_both_dimensions() {
    let mut s = Spring2D::new(100.0, 20.0, 1.0);
    s.reset(0.0, 0.0);
    for _ in 0..100 {
        s.step(1.0, 0.5, 1.0 / 60.0);
    }
    assert!((s.x() - 1.0).abs() < 0.01 && (s.y() - 0.5).abs() < 0.01);
}

#[test]
fn spring_reset_clears_state() {
    let mut s = SpringSimulator::new(100.0, 10.0, 1.0);
    for _ in 0..10 {
        s.step(1.0, 1.0 / 60.0);
    }
    s.reset(3.0, 0.0);
    let r = s.step(3.0, 1.0 / 60.0);
    assert!((r.position - 3.0).abs() < EPS && r.velocity.abs() < EPS);
}

// ───────────── FrameTransform ─────────────

#[test]
fn frame_transform_identity() {
    let t = FrameTransform::default();
    assert_eq!(t, FrameTransform::IDENTITY);
    assert!(t.is_identity());
    assert_eq!(t.focus(), (0.5, 0.5));
    assert_eq!(FrameTransform::new(2.0, 0.3, 0.7).focus(), (0.3, 0.7));
}

// ───────────── ZoomInterpolator ─────────────

fn manual(start: f64, end: f64, zoom: f64, x: f64, y: f64) -> ZoomSegment {
    ZoomSegment::new(start, end, zoom, ZoomFocusMode::Manual { x, y })
}

fn interp(segs: &[ZoomSegment]) -> ZoomInterpolator {
    ZoomInterpolator::new(segs, None)
}

fn scale(i: &ZoomInterpolator, t: f64) -> f64 {
    i.transform(t, None).scale
}

#[test]
fn zoom_no_segments_identity() {
    assert_eq!(interp(&[]).transform(5.0, Some((0.5, 0.5))), FrameTransform::IDENTITY);
}

#[test]
fn zoom_inside_segment_full_strength() {
    let i = interp(&[manual(2.0, 8.0, 2.0, 0.5, 0.5)]);
    assert!((scale(&i, 5.0) - 2.0).abs() < EPS);
}

#[test]
fn zoom_before_and_after_segment_identity() {
    let i = interp(&[ZoomSegment::new(5.0, 10.0, 2.0, ZoomFocusMode::FollowCursor)]);
    assert_eq!(i.transform(1.0, Some((0.5, 0.5))), FrameTransform::IDENTITY);
    let i = interp(&[ZoomSegment::new(2.0, 5.0, 2.0, ZoomFocusMode::FollowCursor)]);
    assert!((i.transform(8.0, Some((0.5, 0.5))).scale - 1.0).abs() < EPS);
}

#[test]
fn zoom_in_transition_is_gradual() {
    let i = interp(&[manual(2.0, 8.0, 2.0, 0.5, 0.5)]);
    let early = scale(&i, 2.1);
    assert!(early > 1.0 && early < 2.0);
    assert!(scale(&i, 2.5) > early);
}

#[test]
fn zoom_manual_focus_off_center() {
    let i = interp(&[manual(0.0, 10.0, 2.0, 0.2, 0.8)]);
    let t = i.transform(5.0, None);
    assert!(t.translate_x < 0.5 && t.translate_y > 0.5);
}

#[test]
fn zoom_follow_cursor_uses_cursor_position() {
    let i = interp(&[ZoomSegment::new(0.0, 10.0, 2.0, ZoomFocusMode::FollowCursor)]);
    assert_ne!(i.transform(5.0, Some((0.3, 0.5))).translate_x, i.transform(5.0, Some((0.7, 0.5))).translate_x);
}

#[test]
fn zoom_edge_snap_clamps_focus() {
    let t = interp(&[manual(0.0, 10.0, 2.0, 0.0, 0.0)]).transform(5.0, None);
    assert!((t.translate_x - 0.25).abs() < EPS && (t.translate_y - 0.25).abs() < EPS);
}

#[test]
fn zoom_out_inside_segment() {
    let i = interp(&[manual(2.0, 8.0, 2.0, 0.5, 0.5)]);
    assert!((scale(&i, 8.0 - ZoomInterpolator::RAMP_DURATION) - 2.0).abs() < EPS);
    assert!(scale(&i, 7.999) < 1.001);
    assert_eq!(i.transform(8.0, None), FrameTransform::IDENTITY);
    assert_eq!(i.transform(8.3, None), FrameTransform::IDENTITY);
}

#[test]
fn zoom_ease_in_out_symmetric() {
    let i = interp(&[manual(2.0, 8.0, 3.0, 0.5, 0.5)]);
    let ramp = ZoomInterpolator::RAMP_DURATION;
    assert!((scale(&i, 2.0 + ramp / 2.0) - 2.0).abs() < EPS);
    let early = scale(&i, 2.0 + ramp * 0.1) - 1.0;
    let late = 3.0 - scale(&i, 2.0 + ramp * 0.9);
    assert!((early - late).abs() < EPS);
    assert!(early < 0.02);
}

#[test]
fn zoom_short_segment_splits_ramps() {
    let i = interp(&[manual(2.0, 3.0, 2.0, 0.5, 0.5)]);
    assert!((scale(&i, 2.5) - 2.0).abs() < EPS);
    let s = scale(&i, 2.25);
    assert!(s > 1.4 && s < 1.6);
}

#[test]
fn zoom_back_to_back_without_returning() {
    let i = interp(&[manual(2.0, 5.0, 2.0, 0.3, 0.5), manual(5.0, 9.0, 3.0, 0.7, 0.5)]);
    let mut prev = scale(&i, 4.5);
    for step in 1..=100 {
        let s = scale(&i, 4.5 + step as f64 * 0.01);
        assert!(s >= 2.0 - EPS);
        assert!((s - prev).abs() < 0.05);
        prev = s;
    }
    assert!((scale(&i, 5.0 + ZoomInterpolator::RAMP_DURATION) - 3.0).abs() < EPS);
}

#[test]
fn zoom_segment_at_start_fully_zoomed() {
    let i = interp(&[manual(0.0, 4.0, 2.0, 0.5, 0.5)]);
    assert!((scale(&i, 0.0) - 2.0).abs() < EPS);
}

#[test]
fn zoom_viewport_inside_frame_during_ramp() {
    let i = interp(&[manual(1.0, 6.0, 2.5, 0.0, 1.0)]);
    for step in 0..=100 {
        let t = i.transform(1.0 + step as f64 * 0.01, None);
        let half = 0.5 / t.scale;
        assert!(t.translate_x >= half - EPS && t.translate_x <= 1.0 - half + EPS);
        assert!(t.translate_y >= half - EPS && t.translate_y <= 1.0 - half + EPS);
    }
}

#[test]
fn zoom_nearly_touching_or_overlapping_never_dips() {
    for (first_end, second_start) in [(5.0, 5.03), (5.0, 4.2)] {
        let i = interp(&[manual(2.0, first_end, 2.0, 0.3, 0.5), manual(second_start, 9.0, 3.0, 0.7, 0.5)]);
        for step in 0..=300 {
            let t = 3.0 + step as f64 * 0.01;
            assert!(scale(&i, t) >= 2.0 - EPS, "Einbruch bei {t}");
        }
    }
}

#[test]
fn zoom_never_jumps() {
    let specs: [&[(f64, f64, f64)]; 3] = [&[(2.0, 8.0, 2.0), (2.3, 9.0, 3.0)], &[(2.0, 9.0, 2.0), (4.0, 5.0, 3.0)], &[(2.0, 6.0, 3.0), (5.0, 9.0, 2.0)]];
    for spec in specs {
        let segs: Vec<ZoomSegment> = spec.iter().map(|&(s, e, z)| manual(s, e, z, 0.5, 0.5)).collect();
        let i = interp(&segs);
        let mut prev = scale(&i, 1.0);
        for step in 1..=10_000 {
            let t = 1.0 + step as f64 * 0.001;
            let s = scale(&i, t);
            assert!((s - prev).abs() < 0.02, "Sprung bei {t}: {prev} → {s}");
            prev = s;
        }
    }
}

// ───────────── CursorSmoother ─────────────

fn ev(t: f64, x: f64, y: f64, kind: CursorEventType) -> CursorEvent {
    CursorEvent::new(t, x, y, kind)
}

fn telemetry() -> Vec<CursorEvent> {
    use CursorEventType::*;
    vec![ev(0.0, 0.1, 0.1, Move), ev(0.5, 0.5, 0.5, Move), ev(1.0, 0.5, 0.5, LeftClick), ev(1.5, 0.9, 0.9, Move), ev(2.0, 0.9, 0.1, Move)]
}

#[test]
fn raw_position_interpolates_linearly() {
    let e = [ev(0.0, 0.1, 0.1, CursorEventType::Move), ev(0.1, 0.5, 0.5, CursorEventType::Move)];
    let (x, y) = CursorSmoother::new(&e, CursorSmoothingConfig::default()).raw_position(0.05);
    assert!((x - 0.3).abs() < 1e-10 && (y - 0.3).abs() < 1e-10);
}

#[test]
fn raw_position_holds_across_long_gap() {
    let e = [ev(0.0, 0.2, 0.2, CursorEventType::Move), ev(2.0, 0.8, 0.8, CursorEventType::Move)];
    let (x, y) = CursorSmoother::new(&e, CursorSmoothingConfig::default()).raw_position(1.0);
    assert!((x - 0.2).abs() < 1e-10 && (y - 0.2).abs() < 1e-10);
}

#[test]
fn raw_position_clamps() {
    let e = telemetry();
    let s = CursorSmoother::new(&e, CursorSmoothingConfig::default());
    assert_eq!(s.raw_position(-1.0), (0.1, 0.1));
    assert_eq!(s.raw_position(99.0), (0.9, 0.1));
}

#[test]
fn smoothing_rounds_off_sharp_turn() {
    let e = telemetry();
    let s = CursorSmoother::new(&e, CursorSmoothingConfig::default());
    let tl = s.build_smoothed_timeline(60.0, 2.0);
    let (rx, ry) = s.raw_position(0.5);
    let (sx, sy) = tl.position(0.5);
    assert!((sx - rx).abs() > 1e-4 || (sy - ry).abs() > 1e-4);
}

#[test]
fn smoothed_timeline_sample_count() {
    let e = telemetry();
    assert_eq!(CursorSmoother::new(&e, CursorSmoothingConfig::default()).build_smoothed_timeline(60.0, 2.0).sample_count(), 121);
}

#[test]
fn click_events_preserved() {
    let e = telemetry();
    let c = CursorSmoother::new(&e, CursorSmoothingConfig::default()).click_events();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].kind, CursorEventType::LeftClick);
    assert!((c[0].t - 1.0).abs() < 1e-10);
}

#[test]
fn disabled_smoothing_returns_raw() {
    let e = telemetry();
    let cfg = CursorSmoothingConfig { enabled: false, ..Default::default() };
    let s = CursorSmoother::new(&e, cfg);
    let tl = s.build_smoothed_timeline(60.0, 2.0);
    for i in 0..tl.sample_count() {
        let t = i as f64 / 60.0;
        let (rx, ry) = s.raw_position(t);
        let (sx, sy) = tl.position(t);
        assert!((sx - rx).abs() < 1e-10 && (sy - ry).abs() < 1e-10, "Abweichung bei {t}");
    }
}

#[test]
fn no_lag_on_steady_motion() {
    let events: Vec<CursorEvent> = (0..=240)
        .map(|i| {
            let t = i as f64 / 60.0;
            ev(t, 0.1 + 0.2 * t, 0.8 - 0.15 * t, CursorEventType::Move)
        })
        .collect();
    for preset in [CursorSmoothingPreset::Snappy, CursorSmoothingPreset::Smooth, CursorSmoothingPreset::Floaty] {
        let s = CursorSmoother::new(&events, preset.config());
        let tl = s.build_smoothed_timeline(60.0, 4.0);
        let mut t = 1.5;
        while t <= 2.5 {
            let (rx, ry) = s.raw_position(t);
            let (sx, sy) = tl.position(t);
            assert!((sx - rx).abs() < 0.001 && (sy - ry).abs() < 0.001, "Verzögerung bei {t} ({preset:?})");
            t += 0.25;
        }
    }
}

#[test]
fn smoothing_presets() {
    let smooth = CursorSmoothingPreset::Smooth.config();
    assert_eq!(smooth, CursorSmoothingConfig::default());
    assert!(CursorSmoothingPreset::Snappy.config().stiffness > smooth.stiffness);
    assert!(CursorSmoothingPreset::Floaty.config().stiffness < smooth.stiffness);
    assert!(CursorSmoothingPreset::Floaty.config().mass > smooth.mass);
    assert_eq!(CursorSmoothingPreset::Snappy.config().preset(), CursorSmoothingPreset::Snappy);
    assert_eq!(CursorSmoothingConfig { stiffness: 123.0, ..Default::default() }.preset(), CursorSmoothingPreset::Smooth);
}

// ───────────── CursorFollowPath ─────────────

#[test]
fn follow_path_holds_small_moves_and_pans_to_settled_spot() {
    let mut samples = vec![];
    let mut t: f64 = 0.0;
    while t <= 4.0 {
        let (x, y) = if t < 2.0 { (0.3 + 0.05 * (t * 7.0).sin(), 0.3) } else { (0.8, 0.7) };
        samples.push(Waypoint { time: t, x, y });
        t += 0.05;
    }
    let path = CursorFollowPath { waypoints: CursorFollowPath::waypoints_from(&samples) };
    assert_eq!(path.waypoints.len(), 2);
    let (x0, _) = path.position(1.0);
    assert!((x0 - 0.3).abs() < 0.05);
    let (x1, y1) = path.position(3.9);
    assert!((x1 - 0.8).abs() < 1e-6 && (y1 - 0.7).abs() < 1e-6);
}

// ───────────── CursorOverlayProvider ─────────────

#[test]
fn click_scale_squeezes_and_recovers() {
    let e = [ev(1.0, 0.5, 0.5, CursorEventType::LeftClick)];
    let o = CursorOverlayProvider::new(&e, false);
    assert_eq!(o.click_scale(0.9), 1.0);
    assert!((o.click_scale(1.0) - 0.8).abs() < EPS);
    assert_eq!(o.click_scale(1.5), 1.0);
}

#[test]
fn ripple_grows_holds_and_fades() {
    let e = [ev(1.0, 0.5, 0.5, CursorEventType::LeftClick), ev(1.6, 0.5, 0.5, CursorEventType::MouseUp)];
    let o = CursorOverlayProvider::new(&e, false);
    assert!(o.ripple(0.5).is_none());
    assert_eq!(o.ripple(1.0), Some((0.6, 1.0)));
    assert_eq!(o.ripple(1.4), Some((1.0, 1.0)));
    let (r, a) = o.ripple(1.7).unwrap();
    assert!(r > 1.0 && a < 1.0);
    assert!(o.ripple(1.81).is_none());
}

#[test]
fn idle_cursor_fades_out_and_back_in() {
    use CursorEventType::*;
    let e = [ev(0.0, 0.5, 0.5, Move), ev(0.5, 0.5, 0.5, Move), ev(5.0, 0.6, 0.6, Move), ev(5.1, 0.6, 0.6, Move)];
    let o = CursorOverlayProvider::new(&e, true);
    assert_eq!(o.opacity(0.55), 1.0);
    assert_eq!(o.opacity(2.0), 0.0);
    assert!(o.opacity(4.85) > 0.0 && o.opacity(4.85) < 1.0);
    assert_eq!(o.opacity(5.05), 1.0);
    assert_eq!(CursorOverlayProvider::new(&e, false).opacity(2.0), 1.0);
}

// ───────────── AutoZoomDetector ─────────────

fn dwell(from: f64, to: f64, x: f64, y: f64) -> Vec<CursorEvent> {
    let mut out = vec![];
    let mut n = 0;
    loop {
        let t = from + n as f64 / 60.0;
        if t > to {
            break;
        }
        out.push(ev(t, x, y, CursorEventType::Move));
        n += 1;
    }
    out
}

fn click(t: f64, x: f64, y: f64) -> CursorEvent {
    ev(t, x, y, CursorEventType::LeftClick)
}

fn focus(s: &ZoomSegment) -> (f64, f64) {
    match s.focus_mode {
        ZoomFocusMode::Manual { x, y } => (x, y),
        _ => panic!("manueller Fokus erwartet"),
    }
}

#[test]
fn auto_zoom_empty() {
    assert!(AutoZoomDetector::detect(&[], 10.0).is_empty());
}

#[test]
fn auto_zoom_single_click() {
    let out = AutoZoomDetector::detect(&[click(5.0, 0.3, 0.7)], 10.0);
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].start_time, out[0].end_time, out[0].zoom_level), (4.0, 7.0, 1.5));
    assert_eq!(out[0].source, SegmentSource::Auto);
    assert_eq!(focus(&out[0]), (0.3, 0.7));
}

#[test]
fn auto_zoom_clamped_to_boundaries() {
    let out = AutoZoomDetector::detect(&[click(0.2, 0.5, 0.5)], 10.0);
    assert_eq!((out[0].start_time, out[0].end_time), (0.0, 3.0));
    let out = AutoZoomDetector::detect(&[click(9.8, 0.5, 0.5)], 10.0);
    assert_eq!((out[0].start_time, out[0].end_time), (7.0, 10.0));
}

#[test]
fn auto_zoom_clicks_spacing() {
    let out = AutoZoomDetector::detect(&[click(5.0, 0.5, 0.5), click(5.5, 0.5, 0.5)], 10.0);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].start_time, 4.0);
    let out = AutoZoomDetector::detect(&[click(3.0, 0.5, 0.5), click(8.0, 0.5, 0.5)], 12.0);
    assert_eq!(out.len(), 2);
    assert_eq!((out[0].start_time, out[1].start_time), (2.0, 7.0));
    let out = AutoZoomDetector::detect(&[click(3.0, 0.2, 0.2), click(5.5, 0.8, 0.8)], 12.0);
    assert_eq!(out.len(), 1);
}

#[test]
fn auto_zoom_dwells() {
    let out = AutoZoomDetector::detect(&dwell(4.0, 6.0, 0.25, 0.75), 10.0);
    assert_eq!(out.len(), 1);
    assert!((out[0].start_time - 3.5).abs() < 1e-9 && (out[0].end_time - 6.5).abs() < 1e-9);
    let (x, y) = focus(&out[0]);
    assert!((x - 0.25).abs() < 0.001 && (y - 0.75).abs() < 0.001);
    assert!(AutoZoomDetector::detect(&dwell(4.0, 4.2, 0.5, 0.5), 10.0).is_empty());
    let out = AutoZoomDetector::detect(&dwell(1.0, 9.0, 0.4, 0.6), 12.0);
    assert_eq!(out.len(), 1);
    assert!((out[0].start_time - 3.2).abs() < 0.001 && (out[0].end_time - 6.8).abs() < 0.001);
}

#[test]
fn auto_zoom_sparse_same_position_bursts() {
    use CursorEventType::Move;
    let events = [
        ev(0.5, 0.35, 0.25, Move),
        ev(0.55, 0.35, 0.25, Move),
        ev(5.0, 0.35, 0.25, Move),
        ev(5.05, 0.35, 0.25, Move),
        ev(11.0, 0.75, 0.7, Move),
        ev(11.05, 0.75, 0.7, Move),
        ev(13.0, 0.75, 0.7, Move),
        ev(13.05, 0.75, 0.7, Move),
    ];
    let out = AutoZoomDetector::detect(&events, 18.0);
    assert_eq!(out.len(), 2);
    assert!((out[0].start_time - 0.975).abs() < 0.001);
    assert!((out[1].start_time - 10.525).abs() < 0.001);
}

#[test]
fn auto_zoom_click_and_distant_dwell() {
    let mut events = dwell(8.5, 10.0, 0.7, 0.7);
    events.push(click(3.0, 0.2, 0.2));
    let out = AutoZoomDetector::detect(&events, 12.0);
    assert_eq!(out.len(), 2);
    assert_eq!((out[0].start_time, out[0].end_time), (2.0, 5.0));
    assert!((out[1].start_time - 7.75).abs() < 1e-9 && (out[1].end_time - 10.75).abs() < 1e-9);
}

#[test]
fn auto_zoom_dense_click_burst_zooms_further() {
    let out = AutoZoomDetector::detect(&[click(3.0, 0.5, 0.5), click(3.3, 0.52, 0.5), click(3.6, 0.5, 0.52)], 12.0);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].zoom_level, 2.0);
    assert!((out[0].start_time - 2.0).abs() < 1e-9 && (out[0].end_time - 5.6).abs() < 1e-9);
}

#[test]
fn auto_zoom_typing_attaches_focusing_click() {
    let mut events = vec![click(2.0, 0.3, 0.4)];
    events.extend((0..6).map(|i| ev(3.0 + i as f64 * 0.2, 0.3, 0.4, CursorEventType::KeyDown)));
    let out = AutoZoomDetector::detect(&events, 12.0);
    assert_eq!(out.len(), 1);
    assert!((out[0].start_time - 1.0).abs() < 1e-9);
    assert!((out[0].end_time - (4.0 + 0.8)).abs() < 1e-9);
    assert_eq!(focus(&out[0]), (0.3, 0.4));
}

// ───────────── TrimRegion / Projekt ─────────────

#[test]
fn trim_kept_ranges() {
    let r = |a, b| TrimRegion::new(a, b);
    let tr = |s, e| TimeRange { start: s, end: e };
    assert_eq!(TrimRegion::kept_ranges(10.0, &[r(2.0, 4.0), r(7.0, 8.0)]), vec![tr(0.0, 2.0), tr(4.0, 7.0), tr(8.0, 10.0)]);
    assert_eq!(TrimRegion::kept_ranges(8.0, &[r(2.0, 5.0), r(4.0, 6.0)]), vec![tr(0.0, 2.0), tr(6.0, 8.0)]);
    assert_eq!(TrimRegion::kept_ranges(8.0, &[r(-2.0, 1.0), r(3.0, 3.0), r(5.0, 12.0)]), vec![tr(1.0, 5.0)]);
    assert_eq!(TrimRegion::kept_ranges(8.0, &[]), vec![tr(0.0, 8.0)]);
    assert!(TrimRegion::kept_ranges(0.0, &[]).is_empty());
}

#[test]
fn trim_region_basics() {
    assert_eq!(TrimRegion::new(5.0, 15.5).duration(), 10.5);
    assert_ne!(TrimRegion::new(0.0, 1.0).id, TrimRegion::new(0.0, 1.0).id);
}

#[test]
fn project_effective_duration() {
    let mut p = RecordingProject { video_duration: 60.0, ..Default::default() };
    assert_eq!(p.effective_duration(), 60.0);
    p.trim_regions = vec![TrimRegion::new(0.0, 5.0), TrimRegion::new(20.0, 25.0)];
    assert_eq!(p.effective_duration(), 50.0);
    p.video_duration = 30.0;
    p.trim_regions = vec![TrimRegion::new(0.0, 100.0)];
    assert_eq!(p.effective_duration(), 0.0);
}

#[test]
fn head_tail_trim_and_time_mapping() {
    let mut p = RecordingProject { video_duration: 10.0, ..Default::default() };
    p.set_head_trim(1.0);
    p.set_tail_trim(9.0);
    p.trim_regions.push(TrimRegion::new(4.0, 5.0));
    assert_eq!((p.effective_start(), p.effective_end()), (1.0, 9.0));
    assert_eq!(p.timeline_cuts().len(), 1);
    let kept = p.kept_ranges();
    assert!((effective_to_source(&kept, 0.0) - 1.0).abs() < EPS);
    assert!((effective_to_source(&kept, 3.5) - 5.5).abs() < EPS);
    assert!((source_to_effective(&kept, 5.5) - 3.5).abs() < EPS);
    assert!((source_to_effective(&kept, 4.5) - 3.0).abs() < EPS);
    assert_eq!(skip_trim_regions(&p.trim_regions, 4.2, 10.0), 5.0);
    assert_eq!(skip_trim_regions(&p.trim_regions, 0.0, 10.0), 1.0);
}

#[test]
fn project_json_roundtrip_and_defaults() {
    let mut p = RecordingProject { video_duration: 12.0, ..Default::default() };
    replace_zoom_segments(&mut p.effect_segments, &[manual(1.0, 3.0, 2.0, 0.2, 0.3)]);
    p.effect_segments.push(RecordingEffectSegment::blur(4.0, 6.0));
    let s = serde_json::to_string(&p).unwrap();
    let back: RecordingProject = serde_json::from_str(&s).unwrap();
    assert_eq!(back, p);
    assert_eq!(back.zoom_segments().len(), 1);
    let minimal: RecordingProject = serde_json::from_value(json!({ "videoDuration": 3.0 })).unwrap();
    assert_eq!(minimal.cursor_smoothing, CursorSmoothingConfig::default());
    assert!(minimal.hides_idle_cursor);
}

#[test]
fn telemetry_parsing_is_robust() {
    let t = parse_telemetry(&json!({
        "samples": [{"t": 0.5, "x": 0.2, "y": 0.3}, {"t": 0.0, "x": 0.1, "y": 0.1}, {"x": 1}],
        "clicks": [{"t": 0.6, "x": 0.25, "y": 0.35}],
        "keys": [{"t": 0.7}]
    }));
    assert_eq!(t.events.len(), 4);
    assert_eq!(t.events[0].t, 0.0);
    assert_eq!(t.events[2].kind, CursorEventType::LeftClick);
    assert_eq!((t.events[3].x, t.events[3].y), (0.25, 0.35));
    assert!(parse_telemetry(&json!({"foo": 1})).events.is_empty());
    let capso = parse_telemetry(&json!({"recordingAreaWidth": 1280, "recordingAreaHeight": 720, "events": [{"timestamp": 1.0, "x": 0.5, "y": 0.5, "type": "rightClick"}]}));
    assert_eq!(capso.events[0].kind, CursorEventType::RightClick);
    assert_eq!(capso.area_width, Some(1280.0));
}

// ───────────── Layout / Komposition ─────────────

#[test]
fn layout_output_size_matches_frame_compositor() {
    let mut p = RecordingProject { video_width: 1919.0, video_height: 1080.0, ..Default::default() };
    let l = Layout::base(&p);
    assert_eq!((l.out_w, l.out_h, l.frame_x), (1919.0, 1080.0, 0.0));
    p.background_style.enabled = true;
    p.background_style.padding = 20.0;
    let l = Layout::base(&p);
    assert_eq!((l.out_w, l.out_h), (1960.0, 1120.0));
    p.aspect_ratio = "1:1".into();
    let l = Layout::base(&p);
    assert_eq!(l.out_w, l.out_h);
    assert!((l.frame_y - (l.out_h - 1080.0) / 2.0).abs() < EPS);
}

#[test]
fn export_scale_respects_quality_boxes() {
    assert_eq!(export_scale(3840.0, 2160.0, "maximum", false), 1.0);
    assert!((export_scale(3840.0, 2160.0, "social", false) - 0.5).abs() < EPS);
    assert!((export_scale(1080.0, 1920.0, "web", false) - 720.0 / 1080.0).abs() < EPS);
    assert_eq!(export_scale(800.0, 600.0, "web", false), 1.0);
    assert!((export_scale(1440.0, 900.0, "web", true) - 0.5).abs() < EPS);
    assert_eq!(gif_fps("maximum", 10.0), 25.0);
    assert!((gif_fps("maximum", 60.0) - 10.0).abs() < EPS);
}

#[test]
fn placement_follows_zoom() {
    let p = RecordingProject { video_width: 1000.0, video_height: 500.0, ..Default::default() };
    let l = Layout::base(&p);
    assert_eq!(l.place(&FrameTransform::IDENTITY, 0.25, 0.5), (250.0, 250.0));
    let z = FrameTransform::new(2.0, 0.25, 0.5);
    assert_eq!(l.place(&z, 0.25, 0.5), (500.0, 250.0));
    assert_eq!(l.place(&z, 0.5, 0.5), (1000.0, 250.0));
}

#[test]
fn corner_radius_clamped() {
    let b = BackgroundStyle { corner_radius: 500.0, ..Default::default() };
    assert_eq!(b.clamped_corner_radius(1920.0, 1080.0), 120.0);
    assert_eq!(b.clamped_corner_radius(100.0, 60.0), 30.0);
    let ((sx, sy), (ex, ey)) = BackgroundStyle { gradient_angle: 0.0, ..Default::default() }.gradient_unit_points();
    assert!((sx - 0.0).abs() < EPS && (sy - 0.5).abs() < EPS && (ex - 1.0).abs() < EPS && (ey - 0.5).abs() < EPS);
}

#[test]
fn engine_frame_state() {
    let mut p = RecordingProject { video_duration: 10.0, ..Default::default() };
    replace_zoom_segments(&mut p.effect_segments, &[manual(2.0, 6.0, 2.0, 0.5, 0.5)]);
    p.effect_segments.push(RecordingEffectSegment::blur(1.0, 3.0));
    let e = Engine::new(p, &[ev(0.0, 0.5, 0.5, CursorEventType::Move), ev(4.0, 0.5, 0.5, CursorEventType::LeftClick)]);
    let f = e.frame(4.05);
    assert!((f.zoom.scale - 2.0).abs() < EPS);
    let c = f.cursor.expect("Cursor");
    assert!(c.ripple.is_some() && c.click_scale < 1.0);
    assert!(f.blurs.is_empty());
    assert_eq!(e.frame(2.0).blurs.len(), 1);
}

#[test]
fn renderer_produces_frame() {
    let mut p = RecordingProject { video_duration: 2.0, video_width: 64.0, video_height: 36.0, ..Default::default() };
    p.background_style.enabled = true;
    p.background_style.padding = 8.0;
    p.effect_segments.push(RecordingEffectSegment::blur(0.0, 2.0));
    let layout = Layout::base(&p);
    let engine = Engine::new(p, &[ev(0.0, 0.5, 0.5, CursorEventType::Move)]);
    let mut r = super::render::Renderer::new(engine, layout).unwrap();
    let mut src = tiny_skia::Pixmap::new(64, 36).unwrap();
    src.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    let out = r.render(src.as_ref(), 1.0);
    assert_eq!((out.width(), out.height()), (80, 52));
    let px = |x: u32, y: u32| out.pixel(x, y).unwrap();
    assert!((25..=51).contains(&px(1, 1).red())); // Hintergrund dunkelgrau (Schatten dunkelt leicht ab)
    assert!(px(20, 20).red() > 200); // Video (rot)
}

// ───────────── Export (Integration, braucht ein Video: NRZ_TEST_VIDEO=/pfad/video.mp4) ─────────────

#[test]
#[ignore]
fn export_integration() {
    use super::export::{probe, run, Job};
    use std::sync::{atomic::AtomicBool, Arc};
    let video = std::path::PathBuf::from(std::env::var("NRZ_TEST_VIDEO").expect("NRZ_TEST_VIDEO"));
    let (d, w, h) = probe(&video).expect("probe");
    let mut p = RecordingProject { video_duration: d, video_width: w, video_height: h, ..Default::default() };
    p.trim_regions = vec![TrimRegion::new(0.0, 0.5), TrimRegion::new(2.0, 2.5)];
    let out = video.with_file_name("cut.mp4");
    let job = |p: RecordingProject, gif: bool, dest: std::path::PathBuf, events: Vec<CursorEvent>| Job {
        video: video.clone(),
        project: p,
        events,
        gif,
        quality: "maximum".into(),
        dest,
        cancel: Arc::new(AtomicBool::new(false)),
    };
    run(&job(p.clone(), false, out.clone(), vec![]), &|_, _| {}).expect("Schnitt-Export");
    let (cd, ..) = probe(&out).expect("probe cut");
    assert!((cd - (d - 1.0)).abs() < 0.15, "Schnittdauer {cd}");

    p.background_style.enabled = true;
    p.background_style.color_type = BackgroundColorType::Gradient;
    replace_zoom_segments(&mut p.effect_segments, &[ZoomSegment::new(1.0, 3.0, 2.0, ZoomFocusMode::FollowCursor)]);
    p.effect_segments.push(RecordingEffectSegment::blur(0.0, d));
    let events: Vec<CursorEvent> = (0..40).map(|i| CursorEvent::new(i as f64 * 0.1, 0.2 + i as f64 * 0.015, 0.4, if i == 20 { CursorEventType::LeftClick } else { CursorEventType::Move })).collect();
    let out2 = video.with_file_name("comp.mp4");
    run(&job(p.clone(), false, out2.clone(), events.clone()), &|_, _| {}).expect("Kompositions-Export");
    let (cd2, w2, h2) = probe(&out2).expect("probe comp");
    assert!((cd2 - (d - 1.0)).abs() < 0.15, "Dauer {cd2}");
    assert_eq!((w2, h2), (w + 40.0, h + 40.0));

    let out3 = video.with_file_name("anim.gif");
    run(&job(p, true, out3.clone(), events), &|_, _| {}).expect("GIF-Export");
    let bytes = std::fs::read(&out3).unwrap();
    assert!(bytes.starts_with(b"GIF89a") && bytes.len() > 10_000);
}

#[test]
fn telemetry_user_cursor_and_format() {
    let v = serde_json::json!({
        "samples": [{"t": 0.0, "x": 0.5, "y": 0.5}],
        "ups": [{"t": 0.5, "x": 0.5, "y": 0.5}],
        "userShowsCursor": false,
        "outputFormat": "gif"
    });
    let tel = crate::receditor::model::parse_telemetry(&v);
    assert_eq!(tel.user_shows_cursor, Some(false));
    assert_eq!(tel.output_format.as_deref(), Some("gif"));
    assert!(tel.events.iter().any(|e| e.kind == crate::receditor::model::CursorEventType::MouseUp));
    // „Mauszeiger anzeigen“ aus → kein Cursor-Overlay im Editor
    assert!(!crate::receditor::new_project_shows_cursor(&tel));
    let mut on = tel.clone();
    on.user_shows_cursor = None;
    assert!(crate::receditor::new_project_shows_cursor(&on), "ältere Dateien ohne Feld: Cursor zeigen");
    assert!(!crate::receditor::new_project_shows_cursor(&crate::receditor::model::Telemetry::default()));
}
