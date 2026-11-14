//! brief.* command tests: set/merge/show/clear, check (duration, banned word, credit, required
//! text, style), description. No ffmpeg needed: clips are not rendered.
mod common;

use common::*;
use rustler_core::ErrorCode;
use serde_json::{Value, json};

async fn setup() -> (tempfile::TempDir, rustler_core::ProjectStore, rustler_registry::Ctx, rustler_registry::Registry, String) {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    // units 0..2: "Hello there friends. Fellow Americans ask not. What your country can do." (~6.3 s)
    let clip = call(
        &reg,
        &ctx,
        "clip.create",
        json!({"name": "demo", "source_id": src.id, "start_unit": 0, "end_unit": 2, "caption_style": "hormozi-pop"}),
    )
    .await
    .unwrap();
    let id = clip["id"].as_str().unwrap().to_string();
    (proj, store, ctx, reg, id)
}

fn rule<'a>(check: &'a Value, name: &str) -> Vec<&'a Value> {
    check["rules"].as_array().unwrap().iter().filter(|r| r["rule"] == name).collect()
}

#[tokio::test]
async fn set_merge_show_replace_clear() {
    let (_p, store, ctx) = new_project();
    let reg = registry();

    let shown = call(&reg, &ctx, "brief.show", json!({})).await.unwrap();
    assert_eq!(shown["exists"], false);
    assert!(shown["next"][0].as_str().unwrap().contains("brief set"));

    let a = call(
        &reg,
        &ctx,
        "brief.set",
        json!({"creator": "Some Creator", "platform": "shorts", "min_duration": 20, "banned_words": ["sponsor", " Sponsor "],
               "required_hashtags": ["ad", "#clips"], "required_mentions": ["creator"],
               "payout": {"model": "per_view", "rate": 1.5, "currency": "USD"}}),
    )
    .await
    .unwrap();
    assert_eq!(a["brief"]["banned_words"], json!(["sponsor"]));
    assert_eq!(a["brief"]["required_hashtags"], json!(["#ad", "#clips"]));
    assert_eq!(a["brief"]["required_mentions"], json!(["@creator"]));
    assert!(store.root.join("brief.json").is_file());

    // Merge: only the given fields change.
    let b = call(&reg, &ctx, "brief.set", json!({"max_duration": 60, "credit_text": "Clip from @creator"})).await.unwrap();
    assert_eq!(b["brief"]["min_duration"], 20.0);
    assert_eq!(b["brief"]["max_duration"], 60.0);
    assert_eq!(b["brief"]["creator"], "Some Creator");
    assert_eq!(b["brief"]["payout"]["model"], "per_view");
    assert_eq!(b["replaced"], false);

    // Validation.
    for bad in [
        json!({"min_duration": 90}),
        json!({"aspect": "tall"}),
        json!({"platform": "myspace"}),
        json!({"required_hashtags": ["two words"]}),
        json!({"max_duration": -1}),
    ] {
        let e = call(&reg, &ctx, "brief.set", bad.clone()).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Validation, "{bad}");
    }
    // The failed calls changed nothing.
    let s = call(&reg, &ctx, "brief.show", json!({})).await.unwrap();
    assert_eq!(s["brief"]["max_duration"], 60.0);

    // Replace overwrites.
    let r = call(&reg, &ctx, "brief.set", json!({"replace": true, "campaign": "Fresh"})).await.unwrap();
    assert_eq!(r["replaced"], true);
    assert!(r["brief"]["creator"].is_null());
    assert_eq!(r["brief"]["campaign"], "Fresh");

    // Dry run writes nothing.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    call(&reg, &dry, "brief.set", json!({"campaign": "Dry"})).await.unwrap();
    let s = call(&reg, &ctx, "brief.show", json!({})).await.unwrap();
    assert_eq!(s["brief"]["campaign"], "Fresh");

    let c = call(&reg, &ctx, "brief.clear", json!({})).await.unwrap();
    assert_eq!(c["cleared"], true);
    assert_eq!(call(&reg, &ctx, "brief.show", json!({})).await.unwrap()["exists"], false);
    let e = call(&reg, &ctx, "brief.check", json!({"clip_id": "clp_none"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn check_catches_duration_banned_word_and_missing_credit() {
    let (_p, store, ctx, reg, id) = setup().await;
    call(
        &reg,
        &ctx,
        "brief.set",
        json!({"min_duration": 20, "max_duration": 60, "aspect": "9:16", "caption_style": "clean-white",
               "banned_words": ["americans", "Sponsor"], "credit_text": "Clip from @creator",
               "required_text": ["what your country can do", "free trial"],
               "required_hashtags": ["#ad"], "required_mentions": ["@creator"]}),
    )
    .await
    .unwrap();

    let c = call(&reg, &ctx, "brief.check", json!({"clip_id": id})).await.unwrap();
    assert_eq!(c["status"], "fail", "{c}");
    // Duration: about 6 s, below the 20 s minimum.
    let d = rule(&c, "duration");
    assert_eq!(d[0]["status"], "fail");
    assert!(d[0]["message"].as_str().unwrap().contains("shorter"), "{c}");
    assert!(!d[0]["fix"].as_array().unwrap().is_empty());
    // Aspect passes (1080x1920).
    assert_eq!(rule(&c, "aspect")[0]["status"], "pass");
    // Caption style: hormozi-pop vs clean-white.
    let cs = rule(&c, "caption_style");
    assert_eq!(cs[0]["status"], "fail");
    assert_eq!(cs[0]["fix"][0], format!("rustler caption set {id} --style clean-white"));
    // Banned word: "americans" is spoken (and captioned), "Sponsor" is not.
    let bw = rule(&c, "banned_words");
    let hit = bw.iter().find(|r| r["detail"]["word"] == "americans").unwrap();
    assert_eq!(hit["status"], "fail");
    assert!(hit["message"].as_str().unwrap().contains("captions"), "{hit}");
    assert!(hit["fix"].as_array().unwrap().iter().any(|f| f.as_str().unwrap().contains("edit remove-words")));
    assert_eq!(bw.iter().find(|r| r["message"].as_str().unwrap().contains("Sponsor")).unwrap()["status"], "pass");
    // Credit overlay missing, with the exact fix command.
    let cr = rule(&c, "credit_text");
    assert_eq!(cr[0]["status"], "fail");
    assert!(
        cr[0]["fix"][0].as_str().unwrap().starts_with(&format!("rustler overlay add-text {id} \"Clip from @creator\" --style lower-third")),
        "{cr:?}"
    );
    // Required text: spoken phrase passes (caption words are the transcript), missing one fails.
    let rt = rule(&c, "required_text");
    assert_eq!(rt[0]["status"], "pass", "{rt:?}");
    assert_eq!(rt[1]["status"], "fail");
    assert!(rt[1]["fix"][0].as_str().unwrap().contains("overlay add-text"));
    // Hashtags/mentions are post items, not failures.
    let pd = rule(&c, "post_description");
    assert_eq!(pd[0]["status"], "info");
    assert!(c["post_items"].as_array().unwrap().iter().any(|p| p == "add hashtag #ad"));
    // No render yet -> warn.
    assert_eq!(rule(&c, "qc")[0]["status"], "warn");
    // Saved for export reports.
    assert!(store.root.join("clips").join(&id).join("brief_check.json").is_file());
    let st = rustler_commands::brief_cmds::brief_status(&store, &id.parse().unwrap()).unwrap();
    assert_eq!(st["status"], "fail");

    // Fix the fixable things; the check follows.
    call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "Clip from @creator", "style": "lower-third", "start": 0.0, "end": 5.0}))
        .await
        .unwrap();
    call(&reg, &ctx, "caption.set", json!({"clip_id": id, "style": "clean-white"})).await.unwrap();
    call(
        &reg,
        &ctx,
        "brief.set",
        json!({"min_duration": 5, "banned_words": ["sponsor"], "required_text": ["clip from @creator", "country"]}),
    )
    .await
    .unwrap();
    let c2 = call(&reg, &ctx, "brief.check", json!({"clip_id": id})).await.unwrap();
    assert_eq!(rule(&c2, "credit_text")[0]["status"], "pass", "{c2}");
    assert_eq!(rule(&c2, "caption_style")[0]["status"], "pass");
    assert_eq!(rule(&c2, "duration")[0]["status"], "pass");
    assert!(rule(&c2, "required_text").iter().all(|r| r["status"] == "pass"), "{c2}");
    assert!(rule(&c2, "banned_words").iter().all(|r| r["status"] == "pass"));
    // Only the "not rendered yet" warning is left.
    assert_eq!(c2["status"], "warn", "{c2}");

    // A banned word in an overlay is found too.
    call(&reg, &ctx, "overlay.add_text", json!({"clip_id": id, "text": "Use my SPONSOR code", "start": 1.0, "end": 3.0})).await.unwrap();
    let c3 = call(&reg, &ctx, "brief.check", json!({"clip_id": id})).await.unwrap();
    let sp = rule(&c3, "banned_words");
    assert_eq!(sp[0]["status"], "fail");
    assert!(sp[0]["message"].as_str().unwrap().contains("overlay ovl_2"), "{c3}");
    assert!(sp[0]["fix"][0].as_str().unwrap().contains("overlay remove"));
}

#[tokio::test]
async fn check_subtracts_transitions_from_duration() {
    let (_p, store, ctx, reg, id) = setup().await;
    let clip = store.load_clip(&id.parse().unwrap()).unwrap();
    let dur = rustler_core::TimeMap::from_segments(&clip.segments).duration();
    call(&reg, &ctx, "brief.set", json!({"min_duration": dur.floor() - 0.5})).await.unwrap();
    let ok = call(&reg, &ctx, "brief.check", json!({"clip_id": id})).await.unwrap();
    assert_eq!(rule(&ok, "duration")[0]["status"], "pass", "{ok}");
    // 2 s of crossfades make the real clip shorter than the minimum.
    std::fs::write(store.root.join("clips").join(&id).join("extras.json"), r#"{"transitions_total": 2.0}"#).unwrap();
    let short = call(&reg, &ctx, "brief.check", json!({"clip_id": id})).await.unwrap();
    assert_eq!(rule(&short, "duration")[0]["status"], "fail", "{short}");
    assert!(rule(&short, "duration")[0]["message"].as_str().unwrap().contains("transitions"));
}

#[tokio::test]
async fn description_draft() {
    let (_p, _s, ctx, reg, id) = setup().await;
    call(
        &reg,
        &ctx,
        "brief.set",
        json!({"platform": "tiktok", "credit_text": "Clip from @creator", "required_mentions": ["creator"], "required_hashtags": ["ad", "clips"]}),
    )
    .await
    .unwrap();
    let d = call(&reg, &ctx, "brief.description", json!({"clip_id": id})).await.unwrap();
    assert_eq!(d["hook"], "Hello there friends.");
    assert_eq!(d["platform"], "TikTok");
    assert_eq!(d["text"], "Hello there friends.\n\nClip from @creator\n\n@creator\n\n#ad #clips");
    assert_eq!(d["hashtags"], json!(["#ad", "#clips"]));
    // The description is also part of the saved check.
    let c = call(&reg, &ctx, "brief.check", json!({"clip_id": id})).await.unwrap();
    assert_eq!(c["description"], d["text"]);
}
