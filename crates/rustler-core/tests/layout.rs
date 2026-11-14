//! Task 3 layout preset tests.

use rustler_core::ErrorCode;
use rustler_core::layout::*;
use rustler_core::model::*;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.001
}

#[test]
fn center_crop_landscape() {
    let l = preset("center-crop", 1920, 1080, &Canvas::default(), &LayoutParams::default()).unwrap();
    assert_eq!(l.preset.as_deref(), Some("center-crop"));
    assert_eq!(l.regions.len(), 1);
    let r = &l.regions[0];
    assert!(near(r.src.w, 0.3164), "w={}", r.src.w);
    assert!(near(r.src.h, 1.0));
    assert!(near(r.src.x + r.src.w / 2.0, 0.5));
    assert!(near(r.src.y, 0.0));
    assert_eq!(r.dst, Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
    assert_eq!(r.fit, Fit::Cover);
}

#[test]
fn center_crop_focus_is_clamped() {
    let p = LayoutParams { focus_x: Some(0.9), ..Default::default() };
    let r = &preset("center-crop", 1920, 1080, &Canvas::default(), &p).unwrap().regions[0];
    assert!(near(r.src.x + r.src.w, 1.0)); // clamped at right edge, not past it
    let p = LayoutParams { focus_x: Some(0.25), ..Default::default() };
    let r = &preset("center-crop", 1920, 1080, &Canvas::default(), &p).unwrap().regions[0];
    assert!(near(r.src.x + r.src.w / 2.0, 0.25));
    let p = LayoutParams { focus_x: Some(1.5), ..Default::default() };
    assert_eq!(
        preset("center-crop", 1920, 1080, &Canvas::default(), &p).unwrap_err().code,
        ErrorCode::Validation
    );
}

#[test]
fn center_crop_narrow_source_crops_vertically() {
    // A 600x1920 source is narrower than 9:16, so the crop is vertical.
    let r = &preset("center-crop", 600, 1920, &Canvas::default(), &LayoutParams::default()).unwrap().regions[0];
    assert!(near(r.src.w, 1.0));
    assert!(r.src.h < 1.0 && near(r.src.y + r.src.h / 2.0, 0.5));
    // Output aspect equals canvas aspect: (w*600)/(h*1920) == 1080/1920.
    assert!(near(r.src.w * 600.0 / (r.src.h * 1920.0), 1080.0 / 1920.0));
}

#[test]
fn blur_fit_landscape() {
    let l = preset("blur-fit", 1920, 1080, &Canvas::default(), &LayoutParams::default()).unwrap();
    assert_eq!(l.background, Background::BlurFill);
    assert_eq!(l.regions.len(), 1);
    let r = &l.regions[0];
    assert_eq!(r.fit, Fit::Contain);
    assert_eq!(r.src, Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
    assert!(near(r.dst.w, 1.0));
    assert!(near(r.dst.h, 0.3164), "h={}", r.dst.h);
    assert!(near(r.dst.y + r.dst.h / 2.0, 0.5));
}

#[test]
fn blur_fit_portrait_source_fits_height() {
    let r = &preset("blur-fit", 800, 1920, &Canvas::default(), &LayoutParams::default()).unwrap().regions[0];
    assert!(near(r.dst.h, 1.0));
    assert!(r.dst.w < 1.0 && near(r.dst.x + r.dst.w / 2.0, 0.5));
}

#[test]
fn split_vertical() {
    let p = LayoutParams {
        top: Some(Rect { x: 0.75, y: 0.0, w: 0.25, h: 0.33 }),
        bottom: Some(Rect { x: 0.0, y: 0.0, w: 0.75, h: 1.0 }),
        split: Some(0.5),
        ..Default::default()
    };
    let l = preset("split-vertical", 1920, 1080, &Canvas::default(), &p).unwrap();
    assert_eq!(l.regions.len(), 2);
    assert_eq!(l.regions[0].dst, Rect { x: 0.0, y: 0.0, w: 1.0, h: 0.5 });
    assert_eq!(l.regions[1].dst, Rect { x: 0.0, y: 0.5, w: 1.0, h: 0.5 });
    assert_eq!(l.regions[0].src, p.top.unwrap());
    assert_eq!(l.regions[1].src, p.bottom.unwrap());
    assert!(l.regions.iter().all(|r| r.fit == Fit::Cover));
    // default split is 0.5
    let p2 = LayoutParams { split: None, ..p };
    let l2 = preset("split-vertical", 1920, 1080, &Canvas::default(), &p2).unwrap();
    assert_eq!(l2.regions[0].dst.h, 0.5);
}

#[test]
fn split_vertical_validation() {
    let bottom = Some(Rect { x: 0.0, y: 0.0, w: 0.75, h: 1.0 });
    let p = LayoutParams { top: None, bottom, ..Default::default() };
    let e = preset("split-vertical", 1920, 1080, &Canvas::default(), &p).unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("split-vertical needs --top and --bottom"), "{}", e.message);
    let p = LayoutParams {
        top: Some(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }),
        bottom,
        split: Some(1.2),
        ..Default::default()
    };
    assert_eq!(
        preset("split-vertical", 1920, 1080, &Canvas::default(), &p).unwrap_err().code,
        ErrorCode::Validation
    );
}

#[test]
fn unknown_preset_lists_presets() {
    let e = preset("zoom", 1920, 1080, &Canvas::default(), &LayoutParams::default()).unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    for name in PRESETS {
        assert!(e.message.contains(name), "{} missing from {}", name, e.message);
    }
}

#[test]
fn zero_source_dims_rejected() {
    let e = preset("center-crop", 0, 1080, &Canvas::default(), &LayoutParams::default()).unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}
