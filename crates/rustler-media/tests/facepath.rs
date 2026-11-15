use rustler_media::facepath::{CropKey, FaceBox, FaceSample, SmoothParams, crop_keys};

const CROP_W: f64 = 0.316; // 9:16 crop of a 16:9 frame

fn face(cx: f64, w: f64) -> FaceBox {
    FaceBox { x: cx - w / 2.0, y: 0.2, w, h: w * 1.5, score: 0.9 }
}

fn one(t: f64, cx: f64) -> FaceSample {
    FaceSample { t, faces: vec![face(cx, 0.1)] }
}

fn series(n: usize, fps: f64, t0: f64, f: impl Fn(usize) -> Option<f64>) -> Vec<FaceSample> {
    (0..n)
        .map(|i| {
            let t = t0 + i as f64 / fps;
            match f(i) {
                Some(cx) => one(t, cx),
                None => FaceSample { t, faces: vec![] },
            }
        })
        .collect()
}

fn speed_ok(keys: &[CropKey], cap: f64) {
    for w in keys.windows(2) {
        let v = (w[1].x - w[0].x).abs() / (w[1].t - w[0].t);
        assert!(v <= cap + 1e-9, "speed {v} > {cap} between {:?} and {:?}", w[0], w[1]);
    }
}

#[test]
fn empty_samples_no_keys() {
    assert!(crop_keys(&[], CROP_W, &SmoothParams::default()).is_empty());
}

#[test]
fn constant_face_single_key() {
    let s = series(40, 4.0, 12.0, |_| Some(0.5));
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].t, 0.0);
    assert!((keys[0].x - (0.5 - CROP_W / 2.0)).abs() < 1e-9);
}

#[test]
fn key_times_are_relative_to_first_sample() {
    let s = series(40, 4.0, 100.0, |i| Some(0.3 + 0.01 * i as f64));
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert_eq!(keys[0].t, 0.0);
    assert!(keys.iter().all(|k| k.t >= 0.0 && k.t <= 10.0));
}

#[test]
fn linear_motion_monotonic_within_speed_cap() {
    // centre 0.3 -> 0.7 over 10 s (0.04 widths/s).
    let s = series(41, 4.0, 0.0, |i| Some(0.3 + 0.4 * i as f64 / 40.0));
    let p = SmoothParams::default();
    let keys = crop_keys(&s, CROP_W, &p);
    assert!(keys.len() > 3, "expected several keys, got {keys:?}");
    assert!(keys.windows(2).all(|w| w[1].x > w[0].x && w[1].t > w[0].t), "{keys:?}");
    speed_ok(&keys, p.max_speed);
    let last = keys.last().unwrap();
    assert!(last.x > 0.45 && last.x < 0.7 - CROP_W / 2.0 + 0.02, "end {last:?}");
}

#[test]
fn fast_jump_is_speed_limited() {
    let s = series(40, 4.0, 0.0, |i| Some(if i < 8 { 0.2 } else { 0.8 }));
    let p = SmoothParams::default();
    let keys = crop_keys(&s, CROP_W, &p);
    assert!(keys.len() > 2);
    speed_ok(&keys, p.max_speed);
    assert!(keys.windows(2).all(|w| w[1].x >= w[0].x));
}

#[test]
fn jitter_inside_dead_zone_emits_nothing() {
    let s = series(60, 4.0, 0.0, |i| Some(0.5 + if i % 2 == 0 { 0.01 } else { -0.01 }));
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert_eq!(keys.len(), 1, "{keys:?}");
}

#[test]
fn missing_samples_hold_then_return_to_centre() {
    // face at 0.8 for 2 s, then lost for 2 s: hold (no new keys).
    let s = series(16, 4.0, 0.0, |i| if i < 8 { Some(0.8) } else { None });
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!((keys[0].x - (0.8 - CROP_W / 2.0)).abs() < 1e-9);

    // lost for 8 s: after 3 s the crop drifts back toward the centre (0.5), speed-capped.
    let p = SmoothParams::default();
    let s = series(48, 4.0, 0.0, |i| if i < 8 { Some(0.8) } else { None });
    let keys = crop_keys(&s, CROP_W, &p);
    assert!(keys.len() > 1, "{keys:?}");
    assert!(keys[1].t >= 2.0 + HOLD_MIN, "moved too early: {keys:?}");
    let last = keys.last().unwrap();
    assert!(last.x < keys[0].x, "{keys:?}");
    assert!((last.x + CROP_W / 2.0 - 0.5).abs() <= p.dead_zone + 1e-9, "{last:?}");
    speed_ok(&keys, p.max_speed);
}

const HOLD_MIN: f64 = 2.5; // 3 s hold measured from the last detection, minus smoothing slack

#[test]
fn no_face_at_all_centres() {
    let s = series(20, 4.0, 0.0, |_| None);
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert_eq!(keys.len(), 1);
    assert!((keys[0].x - (0.5 - CROP_W / 2.0)).abs() < 1e-9);
}

#[test]
fn leading_gap_uses_first_detection() {
    let s = series(24, 4.0, 0.0, |i| if i < 4 { None } else { Some(0.7) });
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!((keys[0].x - (0.7 - CROP_W / 2.0)).abs() < 1e-9);
}

#[test]
fn two_faces_swapping_size_do_not_make_the_crop_jump() {
    // A at 0.25, B at 0.75. A is larger, except for 6 samples (longer than the median window) where B is slightly larger.
    let s: Vec<FaceSample> = (0..40)
        .map(|i| {
            let b_bigger = (16..22).contains(&i);
            let (wa, wb) = if b_bigger { (0.10, 0.12) } else { (0.12, 0.10) };
            FaceSample { t: i as f64 / 4.0, faces: vec![face(0.25, wa), face(0.75, wb)] }
        })
        .collect();
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert_eq!(keys.len(), 1, "crop jumped to the other face: {keys:?}");
    assert!((keys[0].x - (0.25 - CROP_W / 2.0).max(0.0)).abs() < 1e-9);
}

#[test]
fn clearly_larger_new_face_takes_over() {
    // A (small) then B much larger: following the dominant face is intended.
    let s: Vec<FaceSample> = (0..40)
        .map(|i| {
            let faces = if i < 8 {
                vec![face(0.25, 0.10)]
            } else {
                vec![face(0.25, 0.05), face(0.75, 0.2)]
            };
            FaceSample { t: i as f64 / 4.0, faces }
        })
        .collect();
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert!(keys.last().unwrap().x > 0.4, "{keys:?}");
}

#[test]
fn left_edge_is_clamped_to_frame() {
    let s = series(20, 4.0, 0.0, |_| Some(0.02));
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert_eq!(keys[0].x, 0.0);
    let s = series(20, 4.0, 0.0, |_| Some(0.99));
    let keys = crop_keys(&s, CROP_W, &SmoothParams::default());
    assert!((keys[0].x - (1.0 - CROP_W)).abs() < 1e-9);
}

#[test]
fn engine_json_deserializes() {
    let j = r#"{"t": 1.25, "faces": [{"x":0.4,"y":0.1,"w":0.1,"h":0.2,"score":0.93}]}"#;
    let s: FaceSample = serde_json::from_str(j).unwrap();
    assert_eq!(s.faces.len(), 1);
    assert_eq!(s.t, 1.25);
}

/// Found on real two-person footage: after a long still stretch the first key of the next move
/// landed far from the previous key, and the render's linear interpolation made the crop drift
/// across the whole still stretch (the speaker left the frame for seconds).
#[test]
fn hold_keys_keep_the_crop_still_until_the_move_starts() {
    // 0.2 for 12 s, then 0.8 for 8 s (5 fps).
    let s = series(100, 5.0, 0.0, |i| Some(if i < 60 { 0.2 } else { 0.8 }));
    let at = |keys: &[CropKey], t: f64| {
        let k = keys.partition_point(|k| k.t <= t);
        if k == 0 {
            return keys[0].x;
        }
        if k == keys.len() {
            return keys[k - 1].x;
        }
        let (a, b) = (keys[k - 1], keys[k]);
        a.x + (b.x - a.x) * (t - a.t) / (b.t - a.t)
    };
    let quick = SmoothParams { median_window: 3, min_cutoff: 2.0, beta: 0.05, dead_zone: 0.02, max_speed: 1.6, hold_keys: false };
    let drifting = crop_keys(&s, CROP_W, &quick);
    assert!((at(&drifting, 6.0) - at(&drifting, 0.0)).abs() > 0.05, "documents the old drift: {drifting:?}");
    let p = SmoothParams { hold_keys: true, ..quick };
    let held = crop_keys(&s, CROP_W, &p);
    let x0 = at(&held, 0.0);
    for t in [1.0, 4.0, 8.0, 11.5] {
        assert!((at(&held, t) - x0).abs() < 0.01, "crop moved at {t}: {held:?}");
    }
    assert!(at(&held, 15.0) > 0.5, "{held:?}");
    assert!(held.windows(2).all(|w| w[1].t > w[0].t), "{held:?}");
}
