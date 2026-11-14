//! asset.* / caption.create_style / caption.styles tests. Tool-dependent parts (ffprobe duration,
//! caption preview frames, a real render with a custom style) skip with a printed reason when
//! ffmpeg is not installed.
mod common;

use common::*;
use rustler_core::ErrorCode;
use rustler_registry::Ctx;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// 16-bit mono 8 kHz PCM WAV of silence.
fn write_wav(path: &Path, secs: f64) {
    let n = (8000.0 * secs) as u32;
    let data = n * 2;
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + data).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data.to_le_bytes());
    b.extend(vec![0u8; data as usize]);
    std::fs::write(path, b).unwrap();
}

fn system_font() -> Option<PathBuf> {
    [
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
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
    dir: PathBuf,
}

/// Temp project + temp home (no tools).
fn fx() -> Fx {
    let (tmp, _store, mut ctx) = new_project();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    ctx.home = home;
    let dir = tmp.path().join("files");
    std::fs::create_dir_all(&dir).unwrap();
    Fx { _tmp: tmp, ctx, reg: registry(), dir }
}

async fn run(f: &Fx, name: &str, input: Value) -> rustler_core::Result<Value> {
    call(&f.reg, &f.ctx, name, input).await
}

fn ids(v: &Value, key: &str) -> Vec<String> {
    v[key].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn add_requires_license_and_lists_searches_removes() {
    let f = fx();
    let wav = f.dir.join("Big Whoosh.wav");
    write_wav(&wav, 0.5);

    // no license / placeholder license: refused, nothing copied
    for lic in [json!({}), json!({"license": ""}), json!({"license": "unknown"})] {
        let mut input = json!({"path": wav, "kind": "sfx"});
        input.as_object_mut().unwrap().extend(lic.as_object().unwrap().clone());
        let e = run(&f, "asset.add", input).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation, "{}", e.message);
        assert!(e.message.contains("license"));
    }
    assert!(!f.ctx.project_dir.as_ref().unwrap().join("assets").exists());
    // wrong extension for the kind
    let e = run(&f, "asset.add", json!({"path": wav, "kind": "font", "license": "OFL-1.1"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    // missing file
    let e = run(&f, "asset.add", json!({"path": f.dir.join("nope.wav"), "kind": "sfx", "license": "CC0-1.0"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);

    // dry run writes nothing
    let mut dry = f.ctx.clone();
    dry.dry_run = true;
    let d = call(&f.reg, &dry, "asset.add", json!({"path": wav, "kind": "sfx", "license": "CC0-1.0"})).await.unwrap();
    assert_eq!(d["dry_run"], true);
    assert!(!f.ctx.project_dir.as_ref().unwrap().join("assets").exists());

    // add into the project (default scope when a project is open)
    let a = run(
        &f,
        "asset.add",
        json!({"path": wav, "kind": "sfx", "name": "Big Whoosh", "tags": ["Whoosh", "transition", "whoosh"],
               "license": "CC0-1.0", "license_url": "https://creativecommons.org/publicdomain/zero/1.0/",
               "source_url": "https://example.org/whoosh"}),
    )
    .await
    .unwrap();
    let id = a["asset"]["id"].as_str().unwrap().to_string();
    assert_eq!(id, "sfx-big-whoosh");
    assert_eq!(a["asset"]["scope"], "project");
    assert_eq!(a["asset"]["license"], "CC0-1.0");
    assert_eq!(a["asset"]["tags"], json!(["whoosh", "transition"]));
    assert_eq!(a["asset"]["sha256"].as_str().unwrap().len(), 64);
    assert!(Path::new(a["asset"]["file"].as_str().unwrap()).is_file());
    // the same file again is a duplicate
    let e = run(&f, "asset.add", json!({"path": wav, "kind": "sfx", "license": "CC0-1.0"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.message.contains("already"));
    // manifest on disk has the documented fields
    let m: Value = serde_json::from_slice(&std::fs::read(f.ctx.project_dir.as_ref().unwrap().join("assets/manifest.json")).unwrap()).unwrap();
    let entry = &m["assets"][0];
    for k in ["id", "kind", "name", "path", "tags", "license", "license_url", "source_url", "added_at", "sha256"] {
        assert!(entry.get(k).is_some(), "manifest entry lacks {k}");
    }

    // global scope
    let wav2 = f.dir.join("boom.wav");
    write_wav(&wav2, 0.25);
    let g = run(&f, "asset.add", json!({"path": wav2, "kind": "sfx", "license": "own work", "scope": "global", "tags": ["impact"]})).await.unwrap();
    assert_eq!(g["asset"]["scope"], "global");
    let gid = g["asset"]["id"].as_str().unwrap().to_string();
    assert!(f.ctx.home.join("assets/manifest.json").is_file());

    // list: kind and tag filters, built-ins present
    let all = run(&f, "asset.list", json!({})).await.unwrap();
    assert_eq!(all["count"], 7 + 2);
    let sfx = run(&f, "asset.list", json!({"kind": "sfx"})).await.unwrap();
    let mut got = ids(&sfx, "assets");
    got.sort();
    let mut want = vec![gid.clone(), id.clone()];
    want.sort();
    assert_eq!(got, want);
    let tagged = run(&f, "asset.list", json!({"tag": "impact"})).await.unwrap();
    assert_eq!(ids(&tagged, "assets"), vec![gid.clone()]);
    let styles = run(&f, "asset.list", json!({"kind": "caption_style"})).await.unwrap();
    assert_eq!(styles["count"], 4);
    let alias = run(&f, "asset.list", json!({"kind": "caption-style"})).await.unwrap();
    assert_eq!(alias["count"], 4);

    // search: name, tag, typo, kind filter, no hit
    let s = run(&f, "asset.search", json!({"query": "whoosh"})).await.unwrap();
    assert_eq!(ids(&s, "hits")[0], id);
    let s = run(&f, "asset.search", json!({"query": "wooosh transition", "kind": "sfx"})).await.unwrap();
    assert_eq!(ids(&s, "hits")[0], id);
    let s = run(&f, "asset.search", json!({"query": "boom"})).await.unwrap();
    assert_eq!(ids(&s, "hits")[0], gid);
    let s = run(&f, "asset.search", json!({"query": "hormozi"})).await.unwrap();
    assert_eq!(ids(&s, "hits")[0], "style-hormozi-pop");
    let s = run(&f, "asset.search", json!({"query": "xylophone"})).await.unwrap();
    assert_eq!(s["count"], 0);
    assert_eq!(run(&f, "asset.search", json!({"query": "  "})).await.unwrap_err().code, ErrorCode::Validation);

    // asset:<id> resolves to the file (project and global), plain paths pass through, unknown id fails
    let store = rustler_core::ProjectStore::open(f.ctx.project_dir.as_ref().unwrap()).unwrap();
    let p = rustler_commands::asset_cmds::resolve_asset_path(&store, &f.ctx.home, &format!("asset:{id}")).unwrap();
    assert!(p.is_file() && p.starts_with(f.ctx.project_dir.as_ref().unwrap().join("assets")));
    let p = rustler_commands::asset_cmds::resolve_asset_path(&store, &f.ctx.home, &format!("asset:{gid}")).unwrap();
    assert!(p.is_file() && p.starts_with(f.ctx.home.join("assets")));
    assert_eq!(rustler_commands::asset_cmds::resolve_asset_path(&store, &f.ctx.home, "C:/x/y.mp3").unwrap(), PathBuf::from("C:/x/y.mp3"));
    assert_eq!(rustler_commands::asset_cmds::resolve_asset_path(&store, &f.ctx.home, "asset:nope").unwrap_err().code, ErrorCode::NotFound);
    // a style has no file
    let e = rustler_commands::asset_cmds::resolve_asset_path(&store, &f.ctx.home, "asset:style-bold-karaoke").unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);

    // remove: built-ins are protected, files are deleted, unknown ids fail
    let e = run(&f, "asset.remove", json!({"id": "style-bold-karaoke"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    let file = a["asset"]["file"].as_str().unwrap().to_string();
    let r = run(&f, "asset.remove", json!({"id": id})).await.unwrap();
    assert_eq!(r["file_deleted"], true);
    assert!(!Path::new(&file).exists());
    assert_eq!(run(&f, "asset.show", json!({"id": id})).await.unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(run(&f, "asset.remove", json!({"id": id})).await.unwrap_err().code, ErrorCode::NotFound);
    let r = run(&f, "asset.remove", json!({"id": gid})).await.unwrap();
    assert_eq!(r["scope"], "global");
    assert_eq!(run(&f, "asset.list", json!({"kind": "sfx"})).await.unwrap()["count"], 0);
}

#[tokio::test]
async fn project_assets_override_global_by_id() {
    let f = fx();
    let a = f.dir.join("Boom.wav");
    let b = f.dir.join("boom2.wav");
    write_wav(&a, 0.2);
    write_wav(&b, 0.3);
    run(&f, "asset.add", json!({"path": a, "kind": "sfx", "name": "Boom", "license": "CC0-1.0", "scope": "global"})).await.unwrap();
    run(&f, "asset.add", json!({"path": b, "kind": "sfx", "name": "Boom", "license": "own work", "scope": "project"})).await.unwrap();
    let l = run(&f, "asset.list", json!({"kind": "sfx"})).await.unwrap();
    assert_eq!(l["count"], 1);
    assert_eq!(l["assets"][0]["scope"], "project");
    assert_eq!(l["assets"][0]["license"], "own work");
    // no project: only the global one is visible
    let mut ctx = f.ctx.clone();
    ctx.project_dir = None;
    let l = call(&f.reg, &ctx, "asset.list", json!({"kind": "sfx"})).await.unwrap();
    assert_eq!(l["assets"][0]["scope"], "global");
    // project scope without a project is an error
    let e = call(&f.reg, &ctx, "asset.add", json!({"path": a, "kind": "sfx", "license": "x-license", "scope": "project"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn font_asset_preview_png_and_font_dirs() {
    let f = fx();
    let Some(font) = system_font() else {
        eprintln!("SKIP: no system font");
        return;
    };
    let a = run(&f, "asset.add", json!({"path": font, "kind": "font", "license": "test-fixture (system font, not redistributed)", "scope": "global", "tags": ["test"]})).await.unwrap();
    let id = a["asset"]["id"].as_str().unwrap().to_string();
    let shown = run(&f, "asset.show", json!({"id": id})).await.unwrap();
    assert_eq!(shown["preview"]["kind"], "image");
    let img = shown["preview"]["image"].as_str().unwrap();
    let (w, h) = image::image_dimensions(img).unwrap();
    assert_eq!((w, h), (1080, 420));
    let _ = std::fs::copy(img, std::env::temp_dir().join("rustler_c2_font_preview.png"));
    // font dirs include the library folder
    let dirs = rustler_commands::asset_cmds::asset_font_dirs(&f.ctx.home, f.ctx.project_dir.as_deref());
    assert_eq!(dirs, vec![f.ctx.home.join("assets").join("fonts")]);
}

#[tokio::test]
async fn show_audio_duration_with_ffprobe() {
    let Some((_t, home)) = tools_home() else { return };
    let mut f = fx();
    f.ctx.home = home;
    let wav = f.dir.join("tone.wav");
    write_wav(&wav, 1.5);
    let a = run(&f, "asset.add", json!({"path": wav, "kind": "music", "license": "own work"})).await.unwrap();
    let s = run(&f, "asset.show", json!({"id": a["asset"]["id"]})).await.unwrap();
    assert_eq!(s["preview"]["kind"], "audio");
    let d = s["preview"]["duration_s"].as_f64().unwrap();
    assert!((d - 1.5).abs() < 0.05, "{d}");
    assert_eq!(s["preview"]["sample_rate"], 8000);
}

#[tokio::test]
async fn custom_caption_style_flow_without_tools() {
    let f = fx();
    // before: unknown style is rejected by clip.set_captions-style validation
    let e = rustler_commands::asset_cmds::check_caption_style(&f.ctx, "t", "neon-pop").unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);

    let st = run(
        &f,
        "caption.create_style",
        json!({"name": "Neon Pop", "base": "hormozi-pop", "active_color": "#00FFFF", "primary": "#FFFFFF", "outline": "#101010",
               "outline_width": 8, "uppercase": false, "size_factor": 0.07}),
    )
    .await
    .unwrap();
    assert_eq!(st["style"]["name"], "neon-pop");
    assert_eq!(st["style"]["active_colour"], "#00FFFF");
    assert_eq!(st["style"]["base"], "hormozi-pop");
    assert_eq!(st["asset_id"], "style-neon-pop");
    assert_eq!(st["scope"], "project");
    // the font (Anton) is not installed in the temp home: a warning, not an error
    assert!(st["warnings"].to_string().contains("Anton"));

    // listed as an asset, in caption.styles and caption.list_styles
    let l = run(&f, "asset.list", json!({"kind": "caption_style"})).await.unwrap();
    assert!(ids(&l, "assets").contains(&"style-neon-pop".to_string()));
    let styles = run(&f, "caption.styles", json!({})).await.unwrap();
    let names: Vec<&str> = styles["styles"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["bold-karaoke", "clean-white", "boxed-highlight", "hormozi-pop", "neon-pop"]);
    assert_eq!(styles["styles"][4]["scope"], "project");
    assert_eq!(styles["styles"][4]["built_in"], false);
    let ls = run(&f, "caption.list_styles", json!({})).await.unwrap();
    assert!(ls["styles"].to_string().contains("neon-pop"));
    // a clip accepts it
    rustler_commands::asset_cmds::check_caption_style(&f.ctx, "t", "neon-pop").unwrap();

    // validation: built-in names, unknown base, bad colours, bad ranges
    for bad in [
        json!({"name": "bold-karaoke"}),
        json!({"name": "x1", "base": "nope"}),
        json!({"name": "x2", "primary": "red"}),
        json!({"name": "x3", "size_factor": 0.9}),
        json!({"name": "!!!"}),
    ] {
        assert_eq!(run(&f, "caption.create_style", bad.clone()).await.unwrap_err().code, ErrorCode::Validation, "{bad}");
    }

    // the ASS written for the custom style carries its colours (cyan active word = &H00FFFF00)
    let custom = rustler_commands::asset_cmds::custom_presets(&f.ctx.home, f.ctx.project_dir.as_deref());
    assert_eq!(custom.len(), 1);
    let words = vec![rustler_captions::CaptionWord { text: "hello".into(), out_start: 0.0, out_end: 0.5 }];
    let track = rustler_core::CaptionTrack { preset: "neon-pop".into(), ..Default::default() };
    let ass = rustler_captions::write_ass_with(&words, &track, &rustler_core::Canvas::default(), "", &custom).unwrap();
    assert!(ass.contains("&H00FFFF00&"), "{ass}");
    assert!(ass.contains("hello"), "uppercase was switched off");
    // plain write_ass keeps working and still rejects it
    assert!(rustler_captions::write_ass(&words, &track, &rustler_core::Canvas::default(), "").is_err());

    // a box style from a pop base; removing the style asset removes it from the listings
    let b = run(&f, "caption.create_style", json!({"name": "yellow-box", "base": "hormozi-pop", "active_kind": "box", "box_color": "#FFD400", "scope": "global"})).await.unwrap();
    assert_eq!(b["style"]["active_kind"], "box");
    assert!(b["style"]["box_padding"].as_f64().unwrap() > 0.0);
    run(&f, "asset.remove", json!({"id": "style-neon-pop"})).await.unwrap();
    let styles = run(&f, "caption.styles", json!({})).await.unwrap();
    assert_eq!(styles["styles"].as_array().unwrap().len(), 5);
    assert_eq!(styles["styles"][4]["name"], "yellow-box");
    assert_eq!(styles["styles"][4]["scope"], "global");
}

#[tokio::test]
async fn caption_style_preview_and_render_with_custom_style() {
    let Some((_t, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let reg = registry();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let video = proj.path().join("v.mp4");
    make_video(&ffmpeg_in(&home), &video, 10, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());

    // the tools_home font is a system font; use its family for the style
    let family = if cfg!(windows) { "Arial" } else { "DejaVu Sans" };
    let created = call(
        &reg,
        &ctx,
        "caption.create_style",
        json!({"name": "mag-box", "base": "boxed-highlight", "font_family": family, "primary": "#FFFFFF", "box_color": "#E000A0", "size_factor": 0.075}),
    )
    .await
    .unwrap();
    assert!(created["warnings"].to_string() == "null" || !created["warnings"].to_string().contains("not found"), "{}", created["warnings"]);

    // asset.show renders a caption preview PNG on a gray 1080x1920 frame
    let shown = call(&reg, &ctx, "asset.show", json!({"id": "style-mag-box"})).await.unwrap();
    assert_eq!(shown["preview"]["kind"], "image", "{shown}");
    let png = shown["preview"]["image"].as_str().unwrap();
    assert_eq!(image::image_dimensions(png).unwrap(), (1080, 1920));
    let _ = std::fs::copy(png, std::env::temp_dir().join("rustler_c2_style_preview.png"));
    // the caption box colour must show: some pixel close to #E000A0
    let rgb = image::open(png).unwrap().to_rgb8();
    let hit = rgb.pixels().filter(|p| (p[0] as i32 - 0xE0).abs() < 25 && (p[1] as i32).abs() < 25 && (p[2] as i32 - 0xA0).abs() < 25).count();
    assert!(hit > 500, "custom box colour not visible in the preview ({hit} px)");
    // built-ins preview through the same path
    let bk = call(&reg, &ctx, "asset.show", json!({"id": "style-hormozi-pop"})).await.unwrap();
    assert_eq!(bk["preview"]["kind"], "image", "{bk}");
    let _ = std::fs::copy(bk["preview"]["image"].as_str().unwrap(), std::env::temp_dir().join("rustler_c2_style_preview_builtin.png"));

    // a clip using the custom style renders a preview frame with it
    let clip = call(&reg, &ctx, "clip.create", json!({"name": "demo", "source_id": src.id, "start_unit": 0, "end_unit": 2, "caption_style": "mag-box"}))
        .await
        .unwrap();
    assert_eq!(clip["captions"]["preset"], "mag-box");
    let id = clip["id"].as_str().unwrap().to_string();
    let pf = call(&reg, &ctx, "render.preview_frame", json!({"clip_id": id, "out_t": 4.3})).await.unwrap();
    let img = pf["image"].as_str().unwrap();
    assert_eq!(image::image_dimensions(img).unwrap(), (1080, 1920));
    let _ = std::fs::copy(img, std::env::temp_dir().join("rustler_c2_clip_preview.jpg"));
    let rgb = image::open(img).unwrap().to_rgb8();
    let hit = rgb.pixels().filter(|p| (p[0] as i32 - 0xE0).abs() < 40 && (p[1] as i32) < 40 && (p[2] as i32 - 0xA0).abs() < 40).count();
    assert!(hit > 300, "custom style box colour not visible in the rendered frame ({hit} px)");
    // unknown styles are still rejected
    let e = call(&reg, &ctx, "clip.set_captions", json!({"clip_id": id, "style": "nope"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    assert!(e.hint.unwrap_or_default().contains("mag-box"));
}

#[tokio::test]
async fn add_music_accepts_asset_id() {
    let Some((_t, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let reg = registry();
    let src = add_source(&store, &proj.path().join("v.mp4"), probe_1080p(60.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let clip = call(&reg, &ctx, "clip.create", json!({"name": "c", "source_id": src.id, "start_unit": 0, "end_unit": 0})).await.unwrap();
    let wav = proj.path().join("bed.wav");
    write_wav(&wav, 1.0);
    let a = call(&reg, &ctx, "asset.add", json!({"path": wav, "kind": "music", "license": "own work", "name": "Bed"})).await.unwrap();
    let id = a["asset"]["id"].as_str().unwrap();
    let m = call(&reg, &ctx, "audio.add_music", json!({"clip_id": clip["id"], "path": format!("asset:{id}")})).await.unwrap();
    let stored = m["music"]["path"].as_str().unwrap().replace('\\', "/");
    assert!(stored.ends_with("assets/music/bed.wav"), "{stored}");
    let e = call(&reg, &ctx, "audio.add_music", json!({"clip_id": clip["id"], "path": "asset:missing"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

/// Network: fetches the CC0 Kenney packs and registers fonts/built-ins. Run on demand:
/// `cargo test -p rustler-commands --test asset -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn setup_assets_fetches_cc0_sfx() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = rustler_tools::ToolPaths::from_home(tmp.path());
    // pretend one of the OFL fonts is installed so font registration is exercised
    let dir = paths.fonts.join("Anton");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Anton-Regular.ttf"), b"not a real font").unwrap();
    let report = rustler_tools::assets::install(&paths, false, &rustler_registry::ProgressSink::noop()).await.unwrap();
    println!("{report:#?}");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(report.sfx >= 20, "{}", report.sfx);
    assert_eq!(report.builtins, 7);
    assert_eq!(report.fonts, 1);
    let entries = rustler_tools::assets::load_manifest(&rustler_tools::assets::assets_root(tmp.path()));
    let sfx: Vec<_> = entries.iter().filter(|e| e.kind == rustler_tools::assets::AssetKind::Sfx).collect();
    assert_eq!(sfx.len(), report.sfx);
    for e in &sfx {
        assert_eq!(e.license, "CC0-1.0");
        assert!(e.source_url.as_deref().unwrap().starts_with("https://kenney.nl/assets/"));
        assert_eq!(e.sha256.len(), 64);
        assert!(rustler_tools::assets::entry_file(&rustler_tools::assets::assets_root(tmp.path()), e).unwrap().is_file());
    }
    // idempotent: a second run downloads nothing and changes nothing
    let again = rustler_tools::assets::install(&paths, false, &rustler_registry::ProgressSink::noop()).await.unwrap();
    assert_eq!(again.sfx, report.sfx);
    // the real library: the new assets are visible through asset.list/search
    let mut ctx = rustler_registry::Ctx::new(None, tmp.path().to_path_buf());
    ctx.dry_run = false;
    let reg = registry();
    let r = call(&reg, &ctx, "asset.search", json!({"query": "swipe whoosh", "kind": "sfx"})).await.unwrap();
    assert!(r["count"].as_u64().unwrap() >= 4, "{r}");
    if let Some((_t, home)) = tools_home() {
        // audio duration through ffprobe on a real .ogg
        ctx.home = tmp.path().to_path_buf();
        let first = r["hits"][0]["id"].as_str().unwrap().to_string();
        let ffdir = tmp.path().join("bin");
        std::fs::create_dir_all(&ffdir).unwrap();
        for n in std::fs::read_dir(home.join("bin")).unwrap().flatten() {
            let _ = std::fs::copy(n.path(), ffdir.join(n.file_name()));
        }
        let s = call(&reg, &ctx, "asset.show", json!({"id": first})).await.unwrap();
        println!("{s}");
        assert!(s["preview"]["duration_s"].as_f64().unwrap() > 0.0);
    }
}
