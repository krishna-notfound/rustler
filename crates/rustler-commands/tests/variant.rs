//! variant.*, render.batch and export.batch tests with a real ffmpeg-made 1920x1080 10 s source
//! (skipped with a printed reason when ffmpeg is missing).
mod common;

use common::*;
use rustler_tools::ToolPaths;
use serde_json::{Value, json};

struct Fx {
    _tmp: tempfile::TempDir,
    _proj: tempfile::TempDir,
    ctx: rustler_registry::Ctx,
    store: rustler_core::ProjectStore,
    src: rustler_core::Source,
    reg: rustler_registry::Registry,
}

async fn fx() -> Option<Fx> {
    let (tmp, home) = tools_home()?;
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = ToolPaths::from_home(&home);
    let video = proj.path().join("src.mp4");
    make_video(&ffmpeg_in(&home), &video, 10, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    Some(Fx { _tmp: tmp, _proj: proj, ctx, store, src, reg: registry() })
}

async fn make_clip(f: &Fx, name: &str, start_unit: u32, end_unit: u32) -> String {
    let c = call(
        &f.reg,
        &f.ctx,
        "clip.create",
        json!({"name": name, "source_id": f.src.id, "start_unit": start_unit, "end_unit": end_unit, "caption_style": "bold-karaoke"}),
    )
    .await
    .unwrap();
    c["id"].as_str().unwrap().to_string()
}

fn extras_of(f: &Fx, id: &str) -> Value {
    serde_json::from_slice(&std::fs::read(f.store.root.join("clips").join(id).join("extras.json")).unwrap()).unwrap()
}

#[tokio::test]
async fn variants_clone_extras_and_compare_in_a_grid() {
    let Some(f) = fx().await else { return };
    let base = make_clip(&f, "hook-clip", 0, 2).await;
    // Extras the variants must inherit, plus a hook title on the base.
    let music = f._proj.path().join("bed.wav");
    make_music(&ffmpeg_in(&f.ctx.home), &music, 8);
    let music_path = music.to_string_lossy().replace('\\', "/");
    std::fs::write(
        f.store.root.join("clips").join(&base).join("extras.json"),
        json!({"music": {"path": music_path, "volume_db": -18.0, "duck": true, "fade_in": 0.5, "fade_out": 1.0}, "sfx": [{"id": "s1"}]}).to_string(),
    )
    .unwrap();
    call(&f.reg, &f.ctx, "overlay.add_text", json!({"clip_id": base, "text": "Base hook"})).await.unwrap();

    // Nothing to vary / unknown style / unknown layout are validation errors and create nothing.
    for bad in [
        json!({"clip_id": base}),
        json!({"clip_id": base, "caption_styles": ["nope"]}),
        json!({"clip_id": base, "layouts": ["no-such-layout"]}),
    ] {
        assert!(call(&f.reg, &f.ctx, "variant.create", bad).await.is_err());
    }
    assert_eq!(call(&f.reg, &f.ctx, "clip.list", json!({})).await.unwrap()["clips"].as_array().unwrap().len(), 1);

    // 2 styles x 2 layouts = 4 variants.
    let v = call(
        &f.reg,
        &f.ctx,
        "variant.create",
        json!({"clip_id": base, "caption_styles": ["hormozi-pop", "clean-white"], "layouts": ["center-crop", "blur-fit"]}),
    )
    .await
    .unwrap();
    let list = v["variants"].as_array().unwrap();
    assert_eq!(list.len(), 4, "{v}");
    let names: Vec<&str> = list.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["hook-clip-v1", "hook-clip-v2", "hook-clip-v3", "hook-clip-v4"]);
    let combos: Vec<(String, String)> =
        list.iter().map(|x| (x["caption_style"].as_str().unwrap().to_string(), x["layout"].as_str().unwrap().to_string())).collect();
    assert_eq!(
        combos,
        [
            ("hormozi-pop", "center-crop"),
            ("hormozi-pop", "blur-fit"),
            ("clean-white", "center-crop"),
            ("clean-white", "blur-fit")
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    let ids: Vec<String> = list.iter().map(|x| x["clip_id"].as_str().unwrap().to_string()).collect();
    assert!(!ids.contains(&base));
    for (id, (style, layout)) in ids.iter().zip(&combos) {
        let clip = f.store.load_clip(&id.parse().unwrap()).unwrap();
        assert_eq!(&clip.captions.preset, style);
        assert_eq!(clip.segments[0].layout.preset.as_deref(), Some(layout.as_str()));
        assert!(clip.renders.is_empty());
        // Extras copied, tagged with the base.
        let x = extras_of(&f, id);
        assert_eq!(x["music"]["path"], music_path.as_str());
        assert_eq!(x["sfx"][0]["id"], "s1");
        assert_eq!(x["overlays"][0]["text"], "Base hook");
        assert_eq!(x[rustler_commands::variant_cmds::VARIANT_OF], base.as_str());
    }
    // The base is untouched (still bold-karaoke, no variant_of).
    let b = f.store.load_clip(&base.parse().unwrap()).unwrap();
    assert_eq!(b.captions.preset, "bold-karaoke");
    assert!(extras_of(&f, &base).get("variant_of").is_none());

    // variant.list from the base and from a variant gives the same set.
    let l1 = call(&f.reg, &f.ctx, "variant.list", json!({"clip_id": base})).await.unwrap();
    let l2 = call(&f.reg, &f.ctx, "variant.list", json!({"clip_id": ids[2]})).await.unwrap();
    assert_eq!(l1["variants"].as_array().unwrap().len(), 4);
    assert_eq!(l1["base"], base.as_str());
    assert_eq!(l2["base"], base.as_str());
    // Projects list them.
    assert_eq!(f.store.load_project().unwrap().clips.len(), 5);

    // A hook variant replaces the base's hook title and continues the numbering.
    let h = call(&f.reg, &f.ctx, "variant.create", json!({"clip_id": base, "hooks": ["Nobody saw this coming", "Wait for it"]})).await.unwrap();
    let hv = h["variants"].as_array().unwrap();
    assert_eq!(hv[0]["name"], "hook-clip-v5");
    assert_eq!(hv[1]["hook"], "Wait for it");
    let hx = extras_of(&f, hv[0]["clip_id"].as_str().unwrap());
    assert_eq!(hx["overlays"].as_array().unwrap().len(), 1, "{hx}");
    assert_eq!(hx["overlays"][0]["text"], "Nobody saw this coming");

    // Compare: a labelled grid image of preview frames at the same out_t.
    let cmp = call(
        &f.reg,
        &f.ctx,
        "variant.compare",
        json!({"clip_ids": [base, ids[0], ids[1], ids[2], ids[3]], "out_t": 1.5}),
    )
    .await
    .unwrap();
    let img = cmp["image"].as_str().unwrap();
    assert!(std::fs::metadata(img).unwrap().len() > 5000);
    let (w, h) = image::image_dimensions(img).unwrap();
    assert_eq!(w, 3 * 360);
    assert!(h > 2 * 640, "two rows of 360x640 tiles plus label strips, got {w}x{h}");
    assert_eq!(cmp["items"].as_array().unwrap().len(), 5);
    let _ = std::fs::copy(img, std::env::temp_dir().join("rustler_c1_variant_compare.jpg"));
    eprintln!("compare grid: {img}");
    assert!(call(&f.reg, &f.ctx, "variant.compare", json!({"clip_ids": [base]})).await.is_err());
}

#[tokio::test]
async fn render_batch_continues_on_failure_and_export_batch_skips_failing_clips() {
    let Some(f) = fx().await else { return };
    let good = make_clip(&f, "good", 0, 0).await;
    let bad = make_clip(&f, "bad", 1, 1).await;
    // A third clip that cannot render: its source media is gone.
    let ghost = make_clip(&f, "ghost", 2, 2).await;
    {
        let mut clip = f.store.load_clip(&ghost.parse().unwrap()).unwrap();
        let base = clip.revision;
        let mut src = f.store.load_source(&clip.segments[0].source_id).unwrap();
        // Point a second source at a missing file and move the ghost clip onto it.
        src.id = rustler_core::SourceId::new();
        src.media_path = f.store.root.join("missing.mp4");
        f.store.save_source(&src).unwrap();
        clip.segments[0].source_id = src.id.clone();
        f.store.save_clip(&mut clip, base).unwrap();
    }

    let r = call(&f.reg, &f.ctx, "render.batch", json!({"clip_ids": [good, ghost, bad], "quality": "draft", "parallel": 2})).await.unwrap();
    eprintln!("render.batch: {r}");
    assert_eq!(r["requested"], 3);
    assert_eq!(r["rendered"], 2, "{r}");
    assert_eq!(r["failed"], 1);
    let res = r["results"].as_array().unwrap();
    assert_eq!(res[0]["clip_id"], good.as_str(), "input order is kept");
    assert_eq!(res[0]["status"], "rendered");
    assert_eq!(res[1]["status"], "failed");
    assert_eq!(res[1]["error"]["code"], "not_found");
    assert_eq!(res[2]["status"], "rendered");
    assert!(std::path::Path::new(res[0]["path"].as_str().unwrap()).is_file());
    assert!(r["parallel"].as_u64().unwrap() >= 1);
    assert_eq!(f.store.load_clip(&good.parse().unwrap()).unwrap().renders.len(), 1);

    // only_missing skips what is already rendered.
    let again = call(&f.reg, &f.ctx, "render.batch", json!({"clip_ids": [good, bad], "quality": "draft", "only_missing": true})).await.unwrap();
    assert_eq!(again["skipped"], 2, "{again}");
    assert_eq!(again["rendered"], 0);
    // ... but not once the clip was edited.
    std::thread::sleep(std::time::Duration::from_millis(2500));
    call(&f.reg, &f.ctx, "overlay.add_text", json!({"clip_id": bad, "text": "Edited after render"})).await.unwrap();
    let edited = call(&f.reg, &f.ctx, "render.batch", json!({"clip_ids": [good, bad], "quality": "draft", "only_missing": true})).await.unwrap();
    assert_eq!(edited["rendered"], 1, "{edited}");
    assert_eq!(edited["skipped"], 1);
    assert_eq!(edited["results"][0]["status"], "skipped");
    assert_eq!(edited["results"][1]["status"], "rendered");

    // all: true covers the project; no ids is a validation error.
    assert!(call(&f.reg, &f.ctx, "render.batch", json!({})).await.is_err());

    // Brief: every clip needs the credit overlay. `good` has it, `bad` does not.
    call(&f.reg, &f.ctx, "brief.set", json!({"credit_text": "Clip from @creator", "required_hashtags": ["#ad"]})).await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2500));
    call(
        &f.reg,
        &f.ctx,
        "overlay.add_text",
        json!({"clip_id": good, "text": "Clip from @creator", "style": "lower-third", "start": 0.0, "end": 1.0}),
    )
    .await
    .unwrap();
    // The overlay edit made `good` stale: export.batch refuses until it is re-rendered.
    let stale = call(&f.reg, &f.ctx, "export.batch", json!({"clip_ids": [good], "require_qc": false})).await.unwrap();
    assert!(stale["exported"].as_array().unwrap().is_empty());
    assert!(stale["skipped"][0]["reasons"][0].as_str().unwrap().contains("edited after its last render"), "{stale}");
    call(&f.reg, &f.ctx, "render.batch", json!({"clip_ids": [good], "quality": "draft"})).await.unwrap();

    let ex = call(&f.reg, &f.ctx, "export.batch", json!({"clip_ids": [good, bad, ghost], "require_qc": false})).await.unwrap();
    eprintln!("export.batch: {ex}");
    let exported = ex["exported"].as_array().unwrap();
    assert_eq!(exported.len(), 1, "{ex}");
    assert_eq!(exported[0]["clip_id"], good.as_str());
    assert!(std::path::Path::new(exported[0]["video"].as_str().unwrap()).is_file());
    // Brief check passed except the "no QC yet" warning (QC was not required here).
    assert_eq!(exported[0]["brief"], "warn");
    let skipped = ex["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 2);
    let bad_skip = skipped.iter().find(|s| s["clip_id"] == bad.as_str()).unwrap();
    let why = bad_skip["reasons"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect::<Vec<_>>().join(" | ");
    assert!(why.contains("Clip from @creator"), "{why}");
    assert!(bad_skip["fix"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().contains("overlay add-text")));
    let ghost_skip = skipped.iter().find(|s| s["clip_id"] == ghost.as_str()).unwrap();
    assert_eq!(ghost_skip["reasons"][0], "no render yet");

    // The export report of the exported clip embeds the brief check.
    let report: Value = serde_json::from_slice(&std::fs::read(exported[0]["report"].as_str().unwrap()).unwrap()).unwrap();
    assert!(report["brief"]["rules"].is_array(), "{report}");
    assert_eq!(report["brief"]["clip_id"], good.as_str());
}
