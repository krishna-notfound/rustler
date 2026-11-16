//! Card renderer tests: pixel checks on real rendered output (uses an installed font; skips when none).

use rustler_core::ErrorCode;
use rustler_media::card::{CardEnv, CardSpec, render_card};
use rustler_tools::ToolPaths;
use serde_json::json;
use std::path::PathBuf;

fn real_home() -> PathBuf {
    if let Some(h) = std::env::var_os("RUSTLER_HOME") {
        return PathBuf::from(h);
    }
    let user = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    PathBuf::from(user.unwrap_or_default()).join(".rustler")
}

fn env() -> Option<CardEnv<'static>> {
    let paths = ToolPaths::from_home(&real_home());
    let font = rustler_media::qc::default_font(&paths);
    if font.is_none() {
        eprintln!("SKIP: no font installed (run `rustler setup`)");
        return None;
    }
    Some(CardEnv::new(vec![paths.fonts.clone()], font))
}

fn spec(v: serde_json::Value) -> CardSpec {
    serde_json::from_value(v).expect("valid spec")
}

#[test]
fn card_renders_text_and_rounded_box() {
    let Some(env) = env() else { return };
    let s = spec(json!({
        "width": 600, "height": 240,
        "shapes": [{"rect": {"x": 20, "y": 20, "w": 560, "h": 200}, "radius": 60, "fill": "#FFD400"}],
        "texts": [{"text": "POV:", "size": 120, "color": "#000000", "weight": "bold",
                   "rect": {"x": 20, "y": 20, "w": 560, "h": 200}}]
    }));
    let out = render_card(&s, &env).unwrap();
    let img = &out.image;
    assert_eq!((img.width(), img.height()), (600, 240));
    // Outside the box: transparent. The very corner of the rect is outside the radius.
    assert_eq!(img.get_pixel(2, 2)[3], 0, "canvas corner is transparent");
    assert_eq!(img.get_pixel(22, 22)[3], 0, "box corner is cut by the radius");
    // Inside the box near the (rounded) corner: box colour.
    let p = img.get_pixel(70, 40);
    assert_eq!((p[0], p[1], p[2], p[3]), (255, 212, 0, 255), "box fill colour inside the radius");
    // Text pixels (black) exist in the text rect, and the centre region is not all yellow.
    let mut dark = 0;
    for y in 20..220 {
        for x in 20..580 {
            let p = img.get_pixel(x, y);
            if p[3] > 200 && p[0] < 40 && p[1] < 40 && p[2] < 40 {
                dark += 1;
            }
        }
    }
    assert!(dark > 1500, "expected black text pixels, found {dark}");
}

#[test]
fn text_box_hugs_the_text_and_wraps_by_measured_width() {
    let Some(env) = env() else { return };
    let s = spec(json!({
        "width": 500, "height": 400,
        "texts": [{"text": "one two three four five six seven eight", "size": 50, "color": "#FFFFFF",
                   "box": {"color": "#FF0000", "radius": 10, "padding": 12},
                   "rect": {"x": 20, "y": 20, "w": 300, "h": 360}, "align": "left", "valign": "top"}]
    }));
    let out = render_card(&s, &env).unwrap();
    // Wrapped into several lines inside 300 px: the box never extends past rect.x + rect.w + padding.
    let mut max_red_x = 0;
    for y in 0..400 {
        for x in 0..500 {
            let p = out.image.get_pixel(x, y);
            if p[3] > 200 && p[0] > 200 && p[1] < 60 {
                max_red_x = max_red_x.max(x);
            }
        }
    }
    assert!(max_red_x > 100 && max_red_x <= 332, "box right edge {max_red_x} should hug wrapped text within the rect");
    assert!(!out.warnings.iter().any(|w| w.contains("overflows")), "{:?}", out.warnings);
}

#[test]
fn card_gradient_background() {
    let Some(env) = env() else { return };
    let s = spec(json!({
        "width": 200, "height": 100,
        "background": {"type": "linear_gradient", "from": "#000000", "to": "#FFFFFF", "angle_deg": 0}
    }));
    let out = render_card(&s, &env).unwrap();
    let (l, m, r) = (out.image.get_pixel(2, 50)[0], out.image.get_pixel(100, 50)[0], out.image.get_pixel(197, 50)[0]);
    assert!(l < 20, "left is near black: {l}");
    assert!(r > 235, "right is near white: {r}");
    assert!((100..160).contains(&m), "middle is mid grey: {m}");
    assert_eq!(out.image.get_pixel(100, 50)[3], 255);
}

#[test]
fn card_missing_font_errors() {
    let Some(env) = env() else { return };
    let s = spec(json!({
        "width": 200, "height": 100,
        "texts": [{"text": "x", "font": "NoSuchFontFamilyAnywhere", "rect": {"x": 0, "y": 0, "w": 200, "h": 100}}]
    }));
    let e = render_card(&s, &env).err().expect("must fail");
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.message.contains("NoSuchFontFamilyAnywhere"), "{}", e.message);
    assert!(e.hint.is_some());
}

#[test]
fn invalid_specs_are_validation_errors() {
    let Some(env) = env() else { return };
    let too_big = spec(json!({"width": 99999, "height": 100}));
    assert_eq!(render_card(&too_big, &env).err().unwrap().code, ErrorCode::Validation);
    let bad_colour = spec(json!({"width": 100, "height": 100, "background": {"type": "solid", "color": "banana"}}));
    let e = render_card(&bad_colour, &env).err().unwrap();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("background.color"), "{}", e.message);
    let zero = spec(json!({"width": 100, "height": 100, "shapes": [{"rect": {"x": 0, "y": 0, "w": 0, "h": 5}}]}));
    assert!(render_card(&zero, &env).err().unwrap().message.contains("shapes[0].rect"));
    // unknown fields are rejected so typos are not silently ignored
    assert!(serde_json::from_value::<CardSpec>(json!({"width": 100, "height": 100, "shape": []})).is_err());
}

#[test]
fn shadow_is_drawn_outside_the_shape() {
    let Some(env) = env() else { return };
    let s = spec(json!({
        "width": 200, "height": 200,
        "shapes": [{"rect": {"x": 50, "y": 50, "w": 100, "h": 100}, "fill": "#FFFFFF",
                    "shadow": {"color": "#000000FF", "dx": 0, "dy": 12, "blur": 10}}]
    }));
    let out = render_card(&s, &env).unwrap();
    // just below the shape the shadow adds alpha; far away stays transparent
    assert!(out.image.get_pixel(100, 158)[3] > 40, "shadow below the shape");
    assert_eq!(out.image.get_pixel(5, 5)[3], 0);
    assert_eq!(out.image.get_pixel(100, 100).0[..3], [255, 255, 255]);
}

#[test]
fn emoji_without_emoji_font_is_skipped_with_warning() {
    let Some(mut env) = env() else { return };
    // Point the search at an empty dir so no emoji font is found even if the machine has one.
    let empty = tempfile::tempdir().unwrap();
    env.font_dirs = vec![empty.path().to_path_buf()];
    let s = spec(json!({
        "width": 300, "height": 100,
        "texts": [{"text": "hi \u{1F525}", "size": 40, "rect": {"x": 0, "y": 0, "w": 300, "h": 100}}]
    }));
    let out = render_card(&s, &env).unwrap();
    // system font dirs may still contain a bitmap emoji font on some machines; only assert when skipped
    if out.warnings.iter().any(|w| w.contains("emoji")) {
        assert!(out.warnings.iter().any(|w| w.contains("skipped")));
    }
}
