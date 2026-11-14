//! Demo server with fake commands: `cargo run -p rustler-gui --example demo -- <project_dir>`.
use rustler_gui::{GuiOptions, serve};
use rustler_registry::{Annotations, Ctx, Registry, typed};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

fn cmd(reg: &mut Registry, name: &'static str, f: fn(Value) -> Value) {
    reg.register(typed(
        name,
        "fake demo command",
        &[],
        Annotations::default(),
        vec![],
        move |_ctx: Ctx, i: Value| async move { Ok(f(i)) },
    ));
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir: PathBuf = std::env::args().nth(1).expect("project dir").into();
    let mut reg = Registry::new();
    cmd(&mut reg, "project.show", |_| json!({"project": {"id":"prj_demo","name":"demo","revision":3,"sources":["src_1"],"clips":["clp_1"]}}));
    cmd(&mut reg, "source.list", |_| json!({"sources":[{"id":"src_1","name":"interview.mp4","probe":{"duration":312.4,"width":1920,"height":1080,"has_audio":true,"has_video":true}}]}));
    cmd(&mut reg, "transcript.get", |_| json!({"units":[
        {"id":1,"start":0.4,"end":3.2,"text":"So the first thing nobody tells you about shipping a product"},
        {"id":2,"start":3.4,"end":6.9,"text":"is that the launch is the easy part."},
        {"id":3,"start":7.2,"end":11.0,"text":"The hard part is the boring Tuesday after."}]}));
    cmd(&mut reg, "transcript.search", |_| json!({"matches":[{"unit_id":3,"start":7.2,"end":11.0,"text":"The hard part is the boring Tuesday after."}]}));
    cmd(&mut reg, "see.tiles", |_| json!({"image":"tiles.jpg","tiles":[]}));
    cmd(&mut reg, "caption.list_styles", |_| json!({"styles":["bold-karaoke","clean-white","boxed-highlight","hormozi-pop"]}));
    cmd(&mut reg, "clip.list", |_| json!({"clips":[{"id":"clp_1","name":"Hook"}]}));
    cmd(&mut reg, "clip.show", |_| json!({"clip":{"id":"clp_1","name":"Hook","captions":{"enabled":true,"preset":"clean-white"},
        "segments":[{"id":"seg_1","source_id":"src_1","start":0.3,"end":11.1,"layout":{"preset":"blur-fit"}}],
        "renders":[{"id":"rnd_1","path":"render.mp4","quality":"final","encoder":"libx264","created_at":"2026-11-15T10:00:00Z","qc":"qc.json"}]},
        "duration":10.8}));
    cmd(&mut reg, "render.run", |_| json!({"render_id":"rnd_2","path":"render.mp4"}));
    cmd(&mut reg, "qc.run", |_| json!({"passed":false,"contact_sheet":"tiles.jpg","checks":[
        {"name":"resolution","status":"pass","value":"1080x1920","expected":"1080x1920"},
        {"name":"loudness","status":"warn","value":-16.2,"expected":"-14 +-1 LUFS","fix_hint":"enable loudnorm"},
        {"name":"captions_in_safe_zone","status":"fail","value":0.93,"expected":"< 0.85","fix_hint":"lower position_y"}]}));
    let mut ctx = Ctx::new(Some(dir.clone()), dir.join("home"));
    ctx.project_dir = Some(dir);
    serve(Arc::new(reg), ctx, GuiOptions { port: Some(0), open_browser: false }).await
}
