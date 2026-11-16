//! asset.render_card / text.create_style / text.styles tests (real PNGs, real fonts).
mod common;

use common::*;
use rustler_core::ErrorCode;
use rustler_registry::Ctx;
use serde_json::{Value, json};
use std::path::PathBuf;

fn system_font() -> Option<PathBuf> {
    [
        r"C:\Windows\Fonts\arialbd.ttf",
        r"C:\Windows\Fonts\arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

struct Fx {
    _tmp: tempfile::TempDir,
    ctx: Ctx,
    reg: rustler_registry::Registry,
    font: PathBuf,
}

/// Temp project + temp home whose `fonts/` holds one system font (so the default font resolves).
fn fx() -> Option<Fx> {
    let font = system_font().or_else(|| {
        eprintln!("SKIP: no system font");
        None
    })?;
    let (tmp, _store, mut ctx) = new_project();
    let home = tmp.path().join("home");
    let fonts = home.join("fonts").join("test");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::copy(&font, fonts.join(font.file_name().unwrap())).unwrap();
    ctx.home = home;
    Some(Fx { _tmp: tmp, ctx, reg: registry(), font })
}

async fn run(f: &Fx, name: &str, input: Value) -> rustler_core::Result<Value> {
    call(&f.reg, &f.ctx, name, input).await
}

fn pov_spec() -> Value {
    json!({
        "width": 700, "height": 260,
        "shapes": [{"rect": {"x": 10, "y": 10, "w": 680, "h": 240}, "radius": 50, "fill": "#FFD400"}],
        "texts": [{"text": "POV:", "size": 130, "color": "#000000", "weight": "bold",
                   "rect": {"x": 10, "y": 10, "w": 680, "h": 240}}]
    })
}

#[tokio::test]
async fn render_card_saves_an_image_asset() {
    let Some(f) = fx() else { return };
    let r = run(&f, "asset.render_card", json!({"name": "POV Hook", "spec": pov_spec(), "tags": ["Hook"]})).await.unwrap();
    assert_eq!(r["asset_id"], "image-pov-hook");
    assert_eq!(r["scope"], "project");
    assert_eq!(r["replaced"], false);
    assert_eq!(r["image"], r["path"], "image is the PNG so vision agents see it");
    let png = PathBuf::from(r["path"].as_str().unwrap());
    assert!(png.is_file());
    let img = image::open(&png).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (700, 260));
    assert_eq!(img.get_pixel(1, 1)[3], 0, "transparent outside the box");
    assert_eq!(img.get_pixel(100, 60).0, [255, 212, 0, 255]);

    // It appears in `asset list --kind image` with agent-made metadata.
    let l = run(&f, "asset.list", json!({"kind": "image"})).await.unwrap();
    let a = l["assets"].as_array().unwrap().iter().find(|a| a["id"] == "image-pov-hook").expect("listed");
    assert_eq!(a["license"], "agent-generated");
    assert_eq!(a["kind"], "image");
    let tags: Vec<&str> = a["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    assert!(tags.contains(&"created_by:agent") && tags.contains(&"hook") && tags.contains(&"card"), "{tags:?}");
    assert_eq!(a["sha256"].as_str().unwrap().len(), 64);

    // Same name again replaces (same id and file), different colour.
    let mut v2 = pov_spec();
    v2["shapes"][0]["fill"] = json!("#00FF00");
    let r2 = run(&f, "asset.render_card", json!({"name": "pov hook", "spec": v2})).await.unwrap();
    assert_eq!(r2["asset_id"], "image-pov-hook");
    assert_eq!(r2["replaced"], true);
    let img2 = image::open(&png).unwrap().to_rgba8();
    assert_eq!(img2.get_pixel(100, 60).0, [0, 255, 0, 255]);
    let l = run(&f, "asset.list", json!({"kind": "image"})).await.unwrap();
    assert_eq!(l["assets"].as_array().unwrap().iter().filter(|a| a["id"] == "image-pov-hook").count(), 1);
}

#[tokio::test]
async fn render_card_global_scope_and_dry_run() {
    let Some(f) = fx() else { return };
    let r = run(&f, "asset.render_card", json!({"name": "badge", "spec": pov_spec(), "scope": "global"})).await.unwrap();
    assert_eq!(r["scope"], "global");
    assert!(PathBuf::from(r["path"].as_str().unwrap()).starts_with(f.ctx.home.join("assets")));

    let mut dry = f.ctx.clone();
    dry.dry_run = true;
    let d = call(&f.reg, &dry, "asset.render_card", json!({"name": "ghost", "spec": pov_spec()})).await.unwrap();
    assert_eq!(d["dry_run"], true);
    assert!(!PathBuf::from(d["path"].as_str().unwrap()).exists());
}

#[tokio::test]
async fn render_card_errors_are_clear() {
    let Some(f) = fx() else { return };
    let mut spec = pov_spec();
    spec["texts"][0]["font"] = json!("NoSuchFontFamily");
    let e = run(&f, "asset.render_card", json!({"name": "x", "spec": spec})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert!(e.message.contains("NoSuchFontFamily"));
    // nothing was registered
    let l = run(&f, "asset.list", json!({"kind": "image"})).await.unwrap();
    assert_eq!(l["count"], 0);

    let mut bad = pov_spec();
    bad["shapes"][0]["fill"] = json!("not-a-colour");
    let e = run(&f, "asset.render_card", json!({"name": "x", "spec": bad})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}

#[tokio::test]
async fn font_asset_id_and_text_style_round_trip() {
    let Some(f) = fx() else { return };
    // Add the font as an asset, use its id in a style.
    let font = f.font.clone();
    let a = run(&f, "asset.add", json!({"path": font, "kind": "font", "name": "Card Font", "license": "own work"})).await.unwrap();
    let font_id = a["asset"]["id"].as_str().unwrap().to_string();

    let bad = run(&f, "text.create_style", json!({"name": "s", "font": "Nope Nope", "size": 80, "color": "#000000"})).await.unwrap_err();
    assert_eq!(bad.code, ErrorCode::NotFound);
    let bad = run(&f, "text.create_style", json!({"name": "s", "font": font_id, "size": 80, "color": "red-ish"})).await.unwrap_err();
    assert_eq!(bad.code, ErrorCode::Validation);
    let bad = run(&f, "text.create_style", json!({"name": "s", "font": font_id, "size": 80, "color": "#000", "anim": {"kind": "explode"}})).await.unwrap_err();
    assert_eq!(bad.code, ErrorCode::Validation);

    let c = run(
        &f,
        "text.create_style",
        json!({"name": "Hook Yellow", "font": font_id, "size": 90, "color": "#000000", "weight": "bold",
               "box": {"color": "#FFD400", "radius": 24, "padding": 22}, "uppercase": true,
               "shadow": {"dx": 0, "dy": 6, "blur": 8}, "outline": {"color": "#FFFFFF", "width": 2},
               "anim": {"kind": "pop", "duration_s": 0.25}}),
    )
    .await
    .unwrap();
    assert_eq!(c["asset_id"], "text-hook-yellow");
    assert_eq!(c["replaced"], false);
    assert!(PathBuf::from(c["file"].as_str().unwrap()).is_file());

    let s = run(&f, "text.styles", json!({})).await.unwrap();
    assert_eq!(s["count"], 1);
    let st = &s["styles"][0];
    assert_eq!(st["name"], "hook-yellow");
    assert_eq!(st["scope"], "project");
    assert_eq!(st["style"]["size"], 90.0);
    assert_eq!(st["style"]["font"], font_id);
    assert_eq!(st["style"]["box"]["radius"], 24.0);
    assert_eq!(st["style"]["uppercase"], true);
    assert_eq!(st["style"]["anim"]["kind"], "pop");

    // It is listed as a text_style asset and replaced (not duplicated) when created again.
    let l = run(&f, "asset.list", json!({"kind": "text_style"})).await.unwrap();
    assert_eq!(l["count"], 1);
    let again = run(&f, "text.create_style", json!({"name": "hook yellow", "font": font_id, "size": 100, "color": "#111111"})).await.unwrap();
    assert_eq!(again["replaced"], true);
    let s = run(&f, "text.styles", json!({})).await.unwrap();
    assert_eq!((s["count"].clone(), s["styles"][0]["style"]["size"].clone()), (json!(1), json!(100.0)));

    // A card text can reference the style by name; its box colour shows up in the PNG.
    let card = json!({
        "width": 700, "height": 260,
        "texts": [{"text": "pov:", "style": "hook-yellow", "box": {"color": "#00AAFF", "radius": 24, "padding": 22},
                   "rect": {"x": 10, "y": 10, "w": 680, "h": 240}}]
    });
    let r = run(&f, "asset.render_card", json!({"name": "styled", "spec": card})).await.unwrap();
    let img = image::open(r["path"].as_str().unwrap()).unwrap().to_rgba8();
    let blue = img.pixels().filter(|p| p[3] > 250 && p[2] > 200 && p[0] < 20).count();
    assert!(blue > 2000, "box colour override from the card text applies: {blue}");

    // Unknown style name is a NotFound.
    let e = run(&f, "asset.render_card", json!({"name": "y", "spec": {"width": 100, "height": 100, "texts": [{"text": "a", "style": "ghost", "rect": {"x": 0, "y": 0, "w": 100, "h": 100}}]}})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}
