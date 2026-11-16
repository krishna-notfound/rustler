use rustler_media::camera::{
    CamEvent, CamFace, CamSample, CameraParams, CameraStyle, EventKind, SwitchStyle, ease, motion_stats, plan, plan_detailed,
};
use rustler_media::facepath::CropKey;

const CROP_W: f64 = 0.3164; // 9:16 crop of a 16:9 frame
const FPS: f64 = 4.0;

fn face(cx: f64) -> CamFace {
    CamFace { x: cx - 0.05, y: 0.2, w: 0.1, h: 0.15, score: 0.9, lm: None }
}

/// Face whose nose sits `turn` of the eye distance right of the eye midpoint (> 0 = facing right).
fn turned(cx: f64, turn: f64) -> CamFace {
    let eyes = 0.04;
    let lm = [[cx - eyes / 2.0, 0.25], [cx + eyes / 2.0, 0.25], [cx + turn * eyes, 0.29], [cx - 0.015, 0.32], [cx + 0.015, 0.32]];
    CamFace { lm: Some(lm), ..face(cx) }
}

fn series(seconds: f64, f: impl Fn(f64) -> Option<CamFace>) -> Vec<CamSample> {
    let n = (seconds * FPS) as usize;
    (0..n)
        .map(|i| {
            let t = 10.0 + i as f64 / FPS; // source time, not zero
            CamSample { t, faces: f(t - 10.0).into_iter().collect() }
        })
        .collect()
}

fn centre(k: &CropKey) -> f64 {
    k.x + CROP_W / 2.0
}

/// Crop left edge at time t, linear between keys like the renderer.
fn at(keys: &[CropKey], t: f64) -> f64 {
    if t <= keys[0].t {
        return keys[0].x;
    }
    let k = keys.partition_point(|k| k.t <= t);
    if k >= keys.len() {
        return keys[keys.len() - 1].x;
    }
    let (a, b) = (keys[k - 1], keys[k]);
    if b.t - a.t <= 0.0 { b.x } else { a.x + (b.x - a.x) * (t - a.t) / (b.t - a.t) }
}

fn smooth_params_no_look() -> CameraParams {
    CameraParams { look_room: false, ..CameraParams::default() }
}

#[test]
fn holds_when_the_face_jitters_inside_the_safe_box() {
    // +-0.03 of the frame (under the 0.057 safe half-width) in a pseudo-random pattern
    let wobble = [0.0, 0.03, -0.02, 0.025, -0.03, 0.01, 0.02, -0.025, 0.03, -0.01];
    let s = series(20.0, |t| Some(face(0.5 + wobble[(t * FPS) as usize % wobble.len()])));
    let keys = plan(&s, CROP_W, &[], &smooth_params_no_look());
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert_eq!(keys[0].t, 0.0);
    assert!((centre(&keys[0]) - 0.5).abs() < 0.03);
}

#[test]
fn a_lean_is_one_eased_move_under_the_limits() {
    let p = smooth_params_no_look();
    // a lean: 0.40 -> 0.55 over one second from 5 s
    let s = series(12.0, |t| Some(face(0.40 + 0.15 * (t - 5.0).clamp(0.0, 1.0))));
    let plan = plan_detailed(&s, CROP_W, &[], &p);
    let keys = &plan.keys;
    assert_eq!(plan.moves, 1, "{keys:?}");
    assert_eq!(plan.cuts, 0);
    assert!((centre(&keys[0]) - 0.40).abs() < 1e-3);
    assert!((centre(keys.last().unwrap()) - 0.55).abs() < 1e-3);
    // velocity from the dense keys: ~0 at both ends, never above the limits
    let moving: Vec<&[CropKey]> = keys.windows(2).filter(|w| w[1].t > w[0].t).collect();
    let v: Vec<f64> = moving.iter().map(|w| (w[1].x - w[0].x) / (w[1].t - w[0].t)).collect();
    let move_v: Vec<f64> = v.iter().copied().filter(|v| v.abs() > 1e-9).collect();
    assert!(move_v.len() > 15, "dense keys during the move: {}", move_v.len());
    assert!(move_v[0].abs() < 0.03 && move_v[move_v.len() - 1].abs() < 0.03, "{move_v:?}");
    let vmax = move_v.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(vmax <= p.max_velocity * 1.02, "vmax {vmax}");
    let st = motion_stats(keys);
    assert!(st.max_accel <= p.max_accel * 1.15, "{st:?}");
    assert_eq!(st.moves, 1);
    assert_eq!(st.abrupt_starts, 0);
    assert_eq!(st.reversals, 0);
    assert!(st.jitter < 1e-4, "{st:?}");
    // the move is monotonic and eased (slow-fast-slow)
    let first = move_v[..3].iter().map(|v| v.abs()).sum::<f64>();
    let mid = move_v[move_v.len() / 2 - 1..move_v.len() / 2 + 2].iter().map(|v| v.abs()).sum::<f64>();
    assert!(mid > 4.0 * first, "mid {mid} first {first}");
}

#[test]
fn the_move_starts_before_the_subject_leaves_the_safe_box() {
    let p = smooth_params_no_look();
    // a slow lean: 0.5 at 5 s -> 0.62 at 6.2 s (0.1 frame widths per second), then still
    let s = series(14.0, |t| Some(face(0.5 + 0.1 * (t - 5.0).clamp(0.0, 1.2))));
    let keys = plan(&s, CROP_W, &[], &p);
    let safe = p.safe_frac * CROP_W;
    let crossing = 5.0 + safe / 0.1; // the face leaves the box here
    let start = keys.iter().find(|k| (k.x - keys[0].x).abs() > 1e-6).map(|k| k.t).unwrap();
    let hold_end = keys[keys.iter().position(|k| (k.x - keys[0].x).abs() > 1e-6).unwrap() - 1].t;
    // the camera follows the lean from its onset (5.0 s) after a 0.1 s reaction, not at the crossing
    assert!(hold_end < crossing - p.anticipation, "move starts at {hold_end}, crossing {crossing}");
    assert!((hold_end - (5.0 + 0.1f64).max(crossing - p.max_lead)).abs() < 0.05, "move starts at {hold_end}, crossing {crossing}");
    // a long slow drift: at most max_lead before the crossing
    let drift = series(20.0, |t| Some(face(0.5 + 0.03 * (t - 2.0).clamp(0.0, 6.0))));
    let dk = plan(&drift, CROP_W, &[], &p);
    let d_hold_end = dk[dk.iter().position(|k| (k.x - dk[0].x).abs() > 1e-6).unwrap() - 1].t;
    let d_cross = 2.0 + safe / 0.03;
    assert!((d_hold_end - (d_cross - p.max_lead)).abs() < 0.05, "drift move starts at {d_hold_end}, crossing {d_cross}");
    assert!(start > hold_end && start - hold_end < 0.05);
    // the face never leaves the frame
    for i in 0..(14.0 * 30.0) as usize {
        let t = i as f64 / 30.0;
        let cx = 0.5 + 0.1 * (t - 5.0).clamp(0.0, 1.2);
        let left = at(&keys, t);
        assert!(cx > left + 0.05 * CROP_W && cx < left + 0.95 * CROP_W, "face {cx} out of [{left}, +crop] at {t}");
    }
}

#[test]
fn look_room_frames_toward_the_side_the_face_points() {
    let p = CameraParams::default();
    for (turn, sign) in [(0.3, 1.0), (-0.3, -1.0)] {
        let s = series(8.0, |_| Some(turned(0.5, turn)));
        let pl = plan_detailed(&s, CROP_W, &[], &p);
        let off = centre(&pl.keys[0]) - 0.5;
        assert!(off * sign > 0.0, "turn {turn}: crop centre offset {off}");
        assert!((off.abs() / CROP_W - 0.08).abs() < 0.005, "turn {turn}: {} of the crop width", off.abs() / CROP_W);
        assert!(pl.look_room_shift * sign > 0.0);
    }
    // a clear profile gets the full 10%
    let s = series(8.0, |_| Some(turned(0.5, 0.6)));
    let off = centre(&plan(&s, CROP_W, &[], &p)[0]) - 0.5;
    assert!((off / CROP_W - 0.10).abs() < 0.005, "{off}");
    // a big close-up (face half the crop wide) gets less room, so the back of the head stays in
    let big = series(8.0, |_| Some(CamFace { x: 0.5 - 0.07, w: 0.14, h: 0.2, ..turned(0.5, 0.6) }));
    let off = centre(&plan(&big, CROP_W, &[], &p)[0]) - 0.5;
    assert!(off > 0.0 && off < 0.04 * CROP_W, "{off}");
    // frontal: centred; look_room off: centred
    let s = series(8.0, |_| Some(turned(0.5, 0.03)));
    assert!((centre(&plan(&s, CROP_W, &[], &p)[0]) - 0.5).abs() < 1e-6);
    let s = series(8.0, |_| Some(turned(0.5, 0.6)));
    assert!((centre(&plan(&s, CROP_W, &[], &CameraParams { look_room: false, ..p })[0]) - 0.5).abs() < 1e-6);
}

#[test]
fn yaw_reads_the_nose_against_the_eye_midpoint() {
    assert!(turned(0.5, 0.3).yaw().unwrap() > 0.2);
    assert!(turned(0.5, -0.3).yaw().unwrap() < -0.2);
    assert!(face(0.5).yaw().is_none());
}

#[test]
fn single_sample_detection_jumps_are_ignored() {
    // a false detection far away in one sample, a dropout, and a two-sample spike
    let s = series(16.0, |t| {
        let i = (t * FPS).round() as usize;
        match i {
            20 => Some(face(0.9)),
            30 => None,
            40 | 41 => Some(face(0.15)),
            _ => Some(face(0.42)),
        }
    });
    let keys = plan(&s, CROP_W, &[], &smooth_params_no_look());
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!((centre(&keys[0]) - 0.42).abs() < 1e-3);
}

#[test]
fn a_source_shot_cut_is_a_crop_cut_not_a_pan() {
    let s = series(10.0, |t| Some(face(if t < 4.0 { 0.3 } else { 0.7 })));
    let pl = plan_detailed(&s, CROP_W, &[], &smooth_params_no_look());
    assert_eq!(pl.cuts, 1, "{:?}", pl.keys);
    assert_eq!(pl.moves, 0);
    let k = pl.keys.windows(2).find(|w| w[0].t == w[1].t).expect("two keys at the cut");
    assert!((k[0].t - 3.875).abs() < 1e-6, "cut at {}", k[0].t);
    assert!((centre(&k[0]) - 0.3).abs() < 1e-3 && (centre(&k[1]) - 0.7).abs() < 1e-3);
}

fn two_people(left: f64, right: f64, switch_at: f64) -> Vec<CamSample> {
    // the speaker crop follows the person talking: left until switch_at, then right
    series(10.0, |t| Some(face(if t < switch_at { left } else { right })))
}

#[test]
fn speaker_switch_cuts_when_wide_and_pans_when_close() {
    let p = smooth_params_no_look();
    let ev = |t: f64| vec![CamEvent { t: 10.0 + t, kind: EventKind::SpeakerSwitch }];
    // 0.5 apart -> auto = cut, exactly at the turn start (10 + 5.1 source s)
    let pl = plan_detailed(&two_people(0.25, 0.75, 5.0), CROP_W, &ev(5.1), &p);
    assert_eq!((pl.cuts, pl.moves), (1, 0), "{:?}", pl.keys);
    assert!(pl.keys.windows(2).any(|w| w[0].t == w[1].t && (w[0].t - 5.1).abs() < 1e-6));
    // 0.25 apart (> jump) -> auto = pan of at most 0.6 s centred on the turn start
    let pl = plan_detailed(&two_people(0.4, 0.65, 5.0), CROP_W, &ev(5.1), &p);
    assert_eq!((pl.cuts, pl.moves), (0, 1), "{:?}", pl.keys);
    let moving: Vec<&CropKey> = pl.keys.iter().filter(|k| k.t > 0.0).collect();
    let (s0, s1) = (moving.first().unwrap().t, moving.last().unwrap().t);
    assert!(s1 - s0 <= 0.6 + 1e-6 && s1 - s0 >= 0.3, "pan {s0}..{s1}");
    assert!(s0 < 5.1 && s1 > 5.1);
    let st = motion_stats(&pl.keys);
    assert_eq!(st.abrupt_starts, 0, "{st:?}");
    // forced styles
    let cut = CameraParams { switch: SwitchStyle::Cut, ..p };
    assert_eq!(plan_detailed(&two_people(0.4, 0.65, 5.0), CROP_W, &ev(5.1), &cut).cuts, 1);
    let pan = CameraParams { switch: SwitchStyle::Pan, ..p };
    let pl = plan_detailed(&two_people(0.25, 0.75, 5.0), CROP_W, &ev(5.1), &pan);
    assert_eq!((pl.cuts, pl.moves), (0, 1));
    // a shot cut next to the speaker switch wins: always a cut
    let both = vec![CamEvent { t: 15.0, kind: EventKind::SpeakerSwitch }, CamEvent { t: 14.9, kind: EventKind::ShotCut }];
    assert_eq!(plan_detailed(&two_people(0.4, 0.65, 5.0), CROP_W, &both, &pan).cuts, 1);
}

#[test]
fn locked_holds_one_framing_per_shot() {
    let p = CameraParams { look_room: false, ..CameraParams::for_style(CameraStyle::Locked) };
    let s = series(12.0, |t| Some(face(if t < 6.0 { 0.40 + 0.04 * (t * 2.0).sin() } else { 0.50 })));
    let pl = plan_detailed(&s, CROP_W, &[], &p);
    assert_eq!(pl.keys.len(), 1, "{:?}", pl.keys);
    // follow reacts to smaller moves than smooth
    let lean = series(10.0, |t| Some(face(if t < 5.0 { 0.45 } else { 0.50 })));
    assert_eq!(plan_detailed(&lean, CROP_W, &[], &smooth_params_no_look()).moves, 0);
    let follow = CameraParams { look_room: false, ..CameraParams::for_style(CameraStyle::Follow) };
    assert_eq!(plan_detailed(&lean, CROP_W, &[], &follow).moves, 1);
}

#[test]
fn ease_is_minimum_jerk() {
    assert_eq!(ease(0.0), 0.0);
    assert_eq!(ease(1.0), 1.0);
    assert!((ease(0.5) - 0.5).abs() < 1e-12);
    let d = 1e-4;
    assert!((ease(d) - ease(0.0)) / d < 1e-6);
}

#[test]
fn motion_stats_flags_jitter_and_robotic_steps() {
    // the old pipeline's shape: a step every 0.25 s back and forth
    let jittery: Vec<CropKey> = (0..80).map(|i| CropKey { t: i as f64 * 0.25, x: 0.3 + if i % 2 == 0 { 0.0 } else { 0.012 } }).collect();
    let st = motion_stats(&jittery);
    assert!(st.reversals > 50 && st.jitter > 0.02, "{st:?}");
    // hold keys + linear ramp: starts at full speed
    let ramp = vec![CropKey { t: 0.0, x: 0.2 }, CropKey { t: 2.0, x: 0.2 }, CropKey { t: 2.25, x: 0.25 }, CropKey { t: 6.0, x: 0.25 }];
    assert_eq!(motion_stats(&ramp).abrupt_starts, 1);
    // a cut is not motion
    let cut = vec![CropKey { t: 0.0, x: 0.2 }, CropKey { t: 3.0, x: 0.2 }, CropKey { t: 3.0, x: 0.6 }, CropKey { t: 6.0, x: 0.6 }];
    let st = motion_stats(&cut);
    assert_eq!((st.cuts, st.moves, st.max_velocity), (1, 0, 0.0));
}

#[test]
fn a_reaction_shot_of_the_other_person_is_a_cut_even_when_the_face_barely_moves() {
    // Real trial pattern: guest close-up facing left at 0.47, a 1.5 s reaction shot of the host
    // facing right at 0.50 (face box a little higher), then the guest again.
    let shot = |t: f64| if (5.0..6.5).contains(&t) { CamFace { y: 0.17, ..turned(0.50, 0.5) } } else { turned(0.47, -0.6) };
    let s = series(12.0, |t| Some(shot(t)));
    let pl = plan_detailed(&s, CROP_W, &[], &CameraParams::default());
    assert_eq!((pl.cuts, pl.moves), (2, 0), "{:?}", pl.keys);
    let cut_times: Vec<f64> = pl.keys.windows(2).filter(|w| w[0].t == w[1].t).map(|w| w[0].t).collect();
    assert_eq!(cut_times, vec![4.875, 6.375]);
    // look-room per shot: the guest framed with room on the left, the host with room on the right
    assert!(centre(&pl.keys[0]) < 0.47 - 0.08 * CROP_W, "{:?}", pl.keys[0]);
    let host = at(&pl.keys, 5.5) + CROP_W / 2.0;
    assert!(host > 0.50 + 0.05 * CROP_W, "host framed at {host}");
    // a real head turn (over a second, face in place) is not a cut
    let turn = series(12.0, |t| Some(turned(0.47, (-0.6 + 1.1 * (t - 5.0)).clamp(-0.6, 0.5))));
    assert_eq!(plan_detailed(&turn, CROP_W, &[], &CameraParams::default()).cuts, 0);
}

#[test]
fn a_close_up_keeps_its_margin_inside_the_crop() {
    // a close-up (face 0.2 of the frame = 63% of the crop) drifts 0.05 right: inside the safe box,
    // but its edge would sit 2.5% of the crop from the crop edge; the 8% margin makes the camera
    // follow with one eased move
    let p = smooth_params_no_look();
    let big = |cx: f64| CamFace { x: cx - 0.1, y: 0.2, w: 0.2, h: 0.3, score: 0.9, lm: None };
    let s = series(12.0, |t| Some(big(0.5 + 0.05 * ((t - 4.0) / 0.5).clamp(0.0, 1.0))));
    let pl = plan_detailed(&s, CROP_W, &[], &p);
    assert_eq!(pl.cuts, 0, "{:?}", pl.keys);
    assert!(pl.moves >= 1, "{:?}", pl.keys);
    let x = at(&pl.keys, 11.0);
    let margin = 0.08 * CROP_W;
    assert!(x + CROP_W - 0.65 >= margin - 0.005 && 0.45 - x >= margin - 0.005, "crop {x} keys {:?}", pl.keys);
    let st = motion_stats(&pl.keys);
    assert_eq!(st.abrupt_starts, 0, "{st:?}");
    // without the margin rule the camera would have held (face edge 2.5% from the crop edge)
    let no_margin = plan_detailed(&s, CROP_W, &[], &CameraParams { margin_frac: 0.0, ..p });
    assert_eq!(no_margin.moves, 0, "{:?}", no_margin.keys);
}
