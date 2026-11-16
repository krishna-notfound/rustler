//! clip.retention / clip.score_set / campaign.analyze (director layer).
mod common;

use common::*;
use rustler_core::ErrorCode;
use serde_json::{Value, json};

fn extras_json(store: &rustler_core::ProjectStore, clip: &str) -> Value {
    serde_json::from_slice(&std::fs::read(store.root.join("clips").join(clip).join("extras.json")).unwrap()).unwrap()
}

fn static_zones(r: &Value) -> Vec<&Value> {
    r["dead_zones"].as_array().unwrap().iter().filter(|z| z["kind"] == "static").collect()
}

/// `(start, end)` pairs of every `effect add ... --start a --end b` in a fix command chain.
fn punch_windows(cmd: &str) -> Vec<(f64, f64)> {
    cmd.split("&&")
        .map(|c| {
            let toks: Vec<&str> = c.split_whitespace().collect();
            let val = |flag: &str| toks[toks.iter().position(|t| *t == flag).unwrap() + 1].parse::<f64>().unwrap();
            assert_eq!(&toks[..3], &["rustler", "effect", "add"], "{c}");
            assert_eq!(toks[4], "punch-in");
            (val("--start"), val("--end"))
        })
        .collect()
}

async fn clip_of(reg: &rustler_registry::Registry, ctx: &rustler_registry::Ctx, src: &rustler_core::Source, start: f64, end: f64) -> String {
    let c = call(reg, ctx, "clip.create", json!({"name": "d", "source_id": src.id, "start": start, "end": end})).await.unwrap();
    c["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn retention_dead_zone_fixed_by_suggested_effect() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(30.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    // 16 s with no visual change: one static dead zone (> 6 s).
    let id = clip_of(&reg, &ctx, &src, 1.0, 17.0).await;

    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": id})).await.unwrap();
    assert_eq!(r["measured_on"], "timeline");
    assert!((r["duration"].as_f64().unwrap() - 16.0).abs() < 1e-9);
    let zones = static_zones(&r);
    assert_eq!(zones.len(), 1, "{r}");
    assert_eq!(zones[0]["start"], 0.0);
    assert_eq!(zones[0]["end"], 16.0);
    assert!(zones[0]["said"].as_str().unwrap().contains("Americans"));
    let fixes = zones[0]["fixes"].as_array().unwrap();
    let actions: Vec<&str> = fixes.iter().map(|f| f["action"].as_str().unwrap()).collect();
    assert_eq!(actions, vec!["punch_in", "text_pop", "cutaway"]);
    assert!(fixes[1]["command"].as_str().unwrap().starts_with(&format!("rustler overlay add-text {id} \"")));
    assert_eq!(r["breakdown"].as_array().unwrap().len(), 7);
    assert!(r["formula"].as_str().unwrap().contains("hook (25)"));
    assert_eq!(r["timeline"].as_array().unwrap().len(), 16);
    assert!(r["timeline"][15]["dead"].as_bool().unwrap());
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("scenes.json")), "{r}");
    let before = r["retention_score"].as_u64().unwrap();
    assert_eq!(extras_json(&store, &id)["meta"]["retention"]["score"], before);

    // Apply exactly what was suggested.
    let windows = punch_windows(fixes[0]["command"].as_str().unwrap());
    assert!(!windows.is_empty());
    for (s, e) in windows {
        call(&reg, &ctx, "effect.add", json!({"clip_id": id, "kind": "punch-in", "start": s, "end": e})).await.unwrap();
    }
    let r2 = call(&reg, &ctx, "clip.retention", json!({"clip_id": id, "per_second": false})).await.unwrap();
    let longest = |r: &serde_json::Value| static_zones(r).iter().map(|z| z["end"].as_f64().unwrap() - z["start"].as_f64().unwrap()).fold(0.0, f64::max);
    assert!(longest(&r2) < 16.0, "{r2}");
    assert!(r2["timeline"].is_null());
    assert!(r2["visual_changes"].as_array().unwrap().iter().any(|c| c["kind"] == "effect"));
    let after = r2["retention_score"].as_u64().unwrap();
    assert!(after > before, "{before} -> {after}");

    // Dry run writes nothing.
    let mut dry = ctx.clone();
    dry.dry_run = true;
    let r3 = call(&reg, &dry, "clip.retention", json!({"clip_id": id})).await.unwrap();
    assert_eq!(r3["dry_run"], true);
    assert_eq!(extras_json(&store, &id)["meta"]["retention"]["score"], after);
}

#[tokio::test]
async fn first_frame_hook_detection() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();

    // Speech starts at 0.0 of the clip.
    let fast = clip_of(&reg, &ctx, &src, 1.0, 7.0).await;
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": fast})).await.unwrap();
    assert_eq!(r["first_frame_hook"]["present"], true);
    assert_eq!(r["first_frame_hook"]["reasons"], json!(["speech"]));
    assert_eq!(r["speech_start_delay"], 0.0);

    // One second of nothing before the first word: no hook, and a silence dead zone up front.
    let slow = clip_of(&reg, &ctx, &src, 0.0, 7.0).await;
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": slow})).await.unwrap();
    assert_eq!(r["first_frame_hook"]["present"], false, "{r}");
    assert_eq!(r["speech_start_delay"], 1.0);
    let sil: Vec<&Value> = r["dead_zones"].as_array().unwrap().iter().filter(|z| z["kind"] == "silence").collect();
    assert_eq!(sil[0]["start"], 0.0);
    assert_eq!(sil[0]["fixes"][0]["action"], "trim_head");
    assert!(sil[0]["fixes"][0]["command"].as_str().unwrap().contains("--start-delta 0.90"));
    let no_hook = r["breakdown"][0]["points"].as_f64().unwrap();

    // A hook title from frame 0 fixes it.
    call(&reg, &ctx, "overlay.add_text", json!({"clip_id": slow, "text": "He said this to a nation", "start": 0.0, "end": 3.0})).await.unwrap();
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": slow})).await.unwrap();
    assert_eq!(r["first_frame_hook"]["reasons"], json!(["text_overlay"]));
    assert_eq!(r["first_frame_hook"]["text_overlay"], "He said this to a nation");
    assert_eq!(r["breakdown"][0]["points"].as_f64().unwrap() - no_hook, 10.0);
}

#[tokio::test]
async fn ending_types() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();

    let full = clip_of(&reg, &ctx, &src, 1.0, 7.0).await;
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": full})).await.unwrap();
    assert_eq!(r["ending"]["kind"], "sentence_final", "{}", r["ending"]);
    assert_eq!(r["ending"]["last_line"], "What your country can do.");

    let cut = clip_of(&reg, &ctx, &src, 1.0, 4.2).await;
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": cut})).await.unwrap();
    assert_eq!(r["ending"]["kind"], "hard_stop", "{}", r["ending"]);
    let hard = r["breakdown"].as_array().unwrap().iter().find(|f| f["name"] == "ending").unwrap()["points"].as_f64().unwrap();
    assert_eq!(hard, 0.0);

    call(&reg, &ctx, "overlay.add_text", json!({"clip_id": full, "text": "Follow for part 2", "start": 4.0, "end": 6.0, "style": "lower-third"}))
        .await
        .unwrap();
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": full})).await.unwrap();
    assert_eq!(r["ending"]["kind"], "cta", "{}", r["ending"]);

    // Loop: a clip whose last line repeats the opening's words.
    let src2 = add_source(&store, &media, probe_1080p(10.0, true, true));
    let spec = [("Discipline", 0.0, 0.5), ("beats", 0.55, 0.9), ("talent.", 0.95, 1.4), ("Every", 1.6, 1.9), ("day,", 1.95, 2.3), ("discipline.", 2.4, 3.0)];
    write_transcript(&store, &src2.id, spec.iter().enumerate().map(|(i, (t, s, e))| word(i as u32, t, *s, *e)).collect());
    let lp = clip_of(&reg, &ctx, &src2, 0.0, 3.2).await;
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": lp})).await.unwrap();
    assert_eq!(r["ending"]["kind"], "loopable", "{}", r["ending"]);
}

#[tokio::test]
async fn score_set_validates_and_is_shown_and_kept() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    let src = add_source(&store, &media, probe_1080p(10.0, true, true));
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    let id = clip_of(&reg, &ctx, &src, 1.0, 7.0).await;

    for bad in [json!({"hook": 26, "flow": 1, "value": 1, "trend": 1}), json!({"hook": 1, "flow": -1, "value": 1, "trend": 1})] {
        let mut input = bad.clone();
        input["clip_id"] = json!(id);
        let e = call(&reg, &ctx, "clip.score_set", input).await.unwrap_err();
        assert_eq!(code(&e), ErrorCode::Validation, "{bad}");
        assert_eq!(e.stage, "clip.score_set");
    }
    assert!(!store.root.join("clips").join(&id).join("extras.json").exists(), "failed sets wrote nothing");

    call(&reg, &ctx, "clip.retention", json!({"clip_id": id})).await.unwrap();
    let s = call(&reg, &ctx, "clip.score_set", json!({"clip_id": id, "hook": 22, "flow": 18, "value": 20.5, "trend": 0, "notes": "opens on the claim"}))
        .await
        .unwrap();
    assert_eq!(s["score"]["total"], 60.5);
    assert_eq!(s["score"]["by"], "agent");
    assert!(s["retention"]["score"].is_u64(), "{s}");
    let x = extras_json(&store, &id);
    assert_eq!(x["meta"]["score"]["hook"], 22.0);
    assert!(x["meta"]["retention"].is_object(), "score_set keeps meta.retention");

    let show = call(&reg, &ctx, "clip.show", json!({"clip_id": id})).await.unwrap();
    assert_eq!(show["extras"]["meta"]["score"]["total"], 60.5);
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": id})).await.unwrap();
    assert_eq!(r["agent_score"]["notes"], "opens on the claim");
    assert_eq!(extras_json(&store, &id)["meta"]["score"]["total"], 60.5, "retention keeps meta.score");
    let rep = rustler_commands::director_cmds::report_score(&store, &id.parse::<rustler_core::ClipId>().unwrap());
    assert_eq!(rep["agent"]["total"], 60.5);
    assert!(rep["retention"]["score"].is_u64());
}

#[tokio::test]
async fn campaign_analyze_with_and_without_brief() {
    let (proj, store, ctx) = new_project();
    let media = proj.path().join("v.mp4");
    std::fs::write(&media, b"x").unwrap();
    add_source(&store, &media, probe_1080p(45.0 * 60.0, true, true));
    let reg = registry();

    // Nothing set: generic per-view plan, counts from the source length.
    let a = call(&reg, &ctx, "campaign.analyze", json!({})).await.unwrap();
    assert_eq!(a["found"], false);
    assert_eq!(a["from"], "none");
    assert_eq!(a["strategy"]["payout_model"], "unknown");
    assert_eq!(a["strategy"]["recommended_clips"]["total_min"], 23);
    assert_eq!(a["strategy"]["recommended_clips"]["total_max"], 32);
    assert!(a["checklist"].as_array().unwrap().iter().any(|c| c["id"] == "retention"));

    // brief.json.
    call(
        &reg,
        &ctx,
        "brief.set",
        json!({"platform": "shorts", "min_duration": 30, "max_duration": 60, "credit_text": "Clip from @maker", "required_hashtags": ["podcast"],
               "banned_words": ["crypto"], "payout": {"model": "per_view", "rate": 1.5, "currency": "USD"}}),
    )
    .await
    .unwrap();
    let a = call(&reg, &ctx, "campaign.analyze", json!({})).await.unwrap();
    assert_eq!(a["from"], "brief");
    assert_eq!(a["duration_window"], json!({"min": 30.0, "max": 60.0, "from": "brief"}));
    assert_eq!(a["strategy"]["payout_model"], "per_view");
    let must: Vec<&str> = a["hard_rules"]["must"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
    assert!(must.contains(&"duration 30-60 s"), "{must:?}");
    assert!(must.iter().any(|m| m.contains("Clip from @maker")));
    assert_eq!(a["hard_rules"]["must_not"][0], "say, caption or show \"crypto\"");
    assert_eq!(a["required"]["hashtags"], json!(["#podcast"]));
    assert!(a["strategy"]["hints"].as_array().unwrap().iter().any(|h| h.as_str().unwrap().contains("1.5 USD per 1000 views")), "{a}");
    let credit = a["checklist"].as_array().unwrap().iter().find(|c| c["id"] == "credit").unwrap();
    assert!(credit["command"].as_str().unwrap().starts_with("rustler overlay add-text <clip_id> \"Clip from @maker\""));

    // A campaign file (task 1.7 format), picked by id; per-clip payout.
    std::fs::create_dir_all(store.root.join("campaigns")).unwrap();
    std::fs::write(
        store.root.join("campaigns").join("drop.json"),
        r#"{"id":"cmp_drop","name":"Drop","platform":"tiktok","payout":{"model":"per_clip","rate":20},"duration":{"min":15,"max":45},
            "mentions_required":["maker"],"rules":[{"id":"r1","text":"Do not show other brands","kind":"agent"},{"id":"r2","text":"Credit in the first 3 seconds","kind":"machine"}]}"#,
    )
    .unwrap();
    let a = call(&reg, &ctx, "campaign.analyze", json!({"campaign_id": "cmp_drop", "source_minutes": 45})).await.unwrap();
    assert_eq!(a["from"], "campaign");
    assert_eq!(a["name"], "Drop");
    assert_eq!(a["strategy"]["payout_model"], "per_clip");
    assert!(a["strategy"]["recommended_clips"]["total_max"].as_u64().unwrap() <= 6);
    assert_eq!(a["required"]["mentions"], json!(["@maker"]));
    assert_eq!(a["hard_rules"]["must_not"], json!(["Do not show other brands"]));
    let r2 = a["checklist"].as_array().unwrap().iter().find(|c| c["id"] == "r2").unwrap();
    assert_eq!(r2["by"], "rustler");
    let r1 = a["checklist"].as_array().unwrap().iter().find(|c| c["id"] == "r1").unwrap();
    assert_eq!(r1["by"], "agent");
    assert!(r1["command"].as_str().unwrap().starts_with("rustler campaign mark <clip_id> r1"), "{r1}");
    assert!(a["checklist"].as_array().unwrap().iter().any(|c| c["id"] == "attach"));

    // Inline brief wins over brief.json; unknown campaign is not_found.
    let a = call(&reg, &ctx, "campaign.analyze", json!({"brief": {"platform": "reels"}})).await.unwrap();
    assert_eq!(a["from"], "inline");
    assert_eq!(a["duration_window"]["from"], "platform_default");
    let e = call(&reg, &ctx, "campaign.analyze", json!({"campaign_id": "nope"})).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::NotFound);
    assert!(e.hint.as_deref().unwrap_or_default().contains("drop"));
}

#[tokio::test]
async fn retention_measures_a_current_render() {
    let Some((_tmp, home)) = tools_home() else { return };
    let (proj, store, mut ctx) = new_project();
    ctx.home = home.clone();
    let paths = rustler_tools::ToolPaths::from_home(&home);
    let video = proj.path().join("v.mp4");
    make_video(&ffmpeg_in(&home), &video, 10, 1920, 1080, true);
    let probe = rustler_media::probe(&paths, &video).await.unwrap();
    let src = add_source(&store, &video, probe);
    write_transcript(&store, &src.id, sample_words());
    let reg = registry();
    // A second, plain red source: the cut from the test pattern to red is a hard visual change.
    let red = proj.path().join("red.mp4");
    let st = std::process::Command::new(ffmpeg_in(&home))
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=red:size=1920x1080:rate=30:duration=4"])
        .args(["-f", "lavfi", "-i", "sine=frequency=330:duration=4", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&red)
        .status()
        .unwrap();
    assert!(st.success());
    let red_src = add_source(&store, &red, rustler_media::probe(&paths, &red).await.unwrap());
    let id = clip_of(&reg, &ctx, &src, 1.0, 4.0).await;
    call(&reg, &ctx, "clip.add_segment", json!({"clip_id": id, "source_id": red_src.id, "start": 0.0, "end": 3.0})).await.unwrap();
    call(&reg, &ctx, "effect.add", json!({"clip_id": id, "kind": "punch-in", "start": 1.0, "end": 2.0})).await.unwrap();
    let rr = call(&reg, &ctx, "render.run", json!({"clip_id": id, "quality": "draft", "prefer_gpu": false})).await.unwrap();

    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": id})).await.unwrap();
    assert_eq!(r["measured_on"], "render", "{r}");
    assert_eq!(r["render_id"], rr["render_id"]);
    assert_eq!(r["inputs"]["frame_scores"], true);
    assert_eq!(r["inputs"]["audio"], "render");
    // The cut to red at 3.0 s shows up as a frame-difference spike on the render.
    let spikes: Vec<f64> =
        r["visual_changes"].as_array().unwrap().iter().filter(|c| c["kind"] == "frame_change").map(|c| c["t"].as_f64().unwrap()).collect();
    assert!(spikes.iter().any(|t| (t - 3.0).abs() < 0.2), "{spikes:?} {}", r["visual_changes"]);
    assert!(r["visual_changes"].as_array().unwrap().iter().any(|c| c["kind"] == "cut" && c["t"] == 3.0));
    assert!(r["timeline"][0]["motion"].is_number());
    assert!(r["timeline"][0]["loudness_lufs"].is_number(), "{}", r["timeline"][0]);

    // An edit after the render makes it stale: timeline only, with a warning.
    call(&reg, &ctx, "effect.add", json!({"clip_id": id, "kind": "flash", "start": 4.0, "end": 4.2})).await.unwrap();
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": id})).await.unwrap();
    assert_eq!(r["measured_on"], "timeline");
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("older than the last edit")), "{r}");
    // Asking for that render explicitly still measures it.
    let r = call(&reg, &ctx, "clip.retention", json!({"clip_id": id, "render_id": rr["render_id"]})).await.unwrap();
    assert_eq!(r["measured_on"], "render");
}
