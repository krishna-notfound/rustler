//! Commands: setup.run, doctor.run (Task 4).
//!
//! `rustler-tools` cannot call the engine (the bridge depends on it), so the engine-backed
//! parts live here: `setup.run` fetches whisper models (setup step 5) after the tools are
//! installed, and `doctor.run` fills `DoctorReport.engine` from `engine.capabilities`.

use rustler_core::{ErrorCode, RustlerError};
use rustler_engine_bridge::EngineClient;
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, Registry, typed};
use rustler_tools::{
    DoctorReport, InstallPlan, SetupOptions, SetupReport, ToolPaths, doctor, installed_state, optional_features,
    profile_name, setup,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Output of `setup.run`: the tools report plus the models fetched through the engine.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Default)]
pub struct SetupOutput {
    #[serde(flatten)]
    pub report: SetupReport,
    /// Models fetched via the engine (`models.fetch`): whisper plus any heavy-analyzer models you opted into.
    pub models_fetched: Vec<String>,
    pub warnings: Vec<String>,
    /// `lite` (base engine, heavy analyzers opt-in) or `full`.
    pub profile: String,
    /// Optional Python extras installed in the engine venv.
    pub extras: Vec<String>,
    pub sfx_pack: bool,
    /// How to add what is not installed yet.
    pub next: Vec<String>,
}

#[derive(Deserialize, JsonSchema, Default, Debug)]
#[serde(default)]
pub struct DoctorInput {}

#[derive(Deserialize, Debug, Default)]
struct FetchResult {
    #[serde(default)]
    fetched: Vec<String>,
}

fn emit(ctx: &Ctx, message: &str) {
    ctx.progress.emit(ProgressEvent {
        stage: "setup".into(),
        pct: None,
        message: Some(message.to_string()),
        eta_s: None,
    });
}

/// The `models.fetch` params for a plan: whisper always (when any), heavy-analyzer models only for
/// what the plan opted into (`--profile full`, or `--with ocr|speakers|audio-events`). Everything
/// else downloads on first use, so a lite install stays small.
pub fn fetch_params(plan: &InstallPlan) -> serde_json::Value {
    let mut p = serde_json::json!({ "whisper": plan.whisper_models });
    for (on, key) in [
        (plan.prefetch_yunet, "yunet"),
        (plan.prefetch_yamnet, "yamnet"),
        (plan.prefetch_diarization, "diarization"),
        (plan.prefetch_ocr_devanagari, "ocr_devanagari"),
    ] {
        if on {
            p[key] = serde_json::Value::Bool(true);
        }
    }
    p
}

/// Step 5: `models.fetch` through the freshly installed engine.
async fn fetch_models(ctx: &Ctx, paths: &ToolPaths, plan: &InstallPlan) -> rustler_core::Result<Vec<String>> {
    emit(ctx, &format!("downloading models: whisper {}", plan.whisper_models.join(", ")));
    let engine = EngineClient::spawn(paths).await?;
    let res: rustler_core::Result<FetchResult> = engine
        .call("models.fetch", fetch_params(plan), &ctx.progress, &ctx.cancel)
        .await;
    engine.shutdown().await;
    Ok(res?.fetched)
}

async fn setup_run(ctx: Ctx, opts: SetupOptions) -> rustler_core::Result<SetupOutput> {
    let paths = ToolPaths::from_home(&ctx.home);
    let mut report = setup(&paths, &opts, &ctx.progress).await?;
    let mut out = SetupOutput::default();
    out.warnings.extend(report.notes.iter().cloned());

    let plan = report.plan.clone();
    if plan.whisper_models.is_empty() {
        out.warnings.push("model downloads skipped (--no-models): models are fetched on first use".into());
    } else if paths.venv_python.is_file() {
        match fetch_models(&ctx, &paths, &plan).await {
            Ok(fetched) => out.models_fetched = fetched,
            Err(e) => report.failed.push(("models".into(), e.to_string())),
        }
    } else {
        report
            .failed
            .push(("models".into(), "engine is not installed, so models cannot be fetched".into()));
    }

    let failed = report.failed.clone();
    out.report = report;
    let state = installed_state(&paths);
    out.profile = profile_name(&state).to_string();
    out.extras = state.extras.clone();
    out.sfx_pack = state.sfx_pack;
    for f in optional_features(&paths, None) {
        let have = if f.name == "sfx-pack" { state.sfx_pack } else { state.extras.contains(&f.name) };
        if !have {
            out.next.push(format!("rustler setup run --with {}   (opt-in: {})", f.name, f.description));
        }
    }
    if failed.is_empty() {
        return Ok(out);
    }
    let list = failed
        .iter()
        .map(|(n, m)| format!("{n}: {m}"))
        .collect::<Vec<_>>()
        .join("; ");
    let done: Vec<&str> = out
        .report
        .installed
        .iter()
        .chain(out.report.skipped.iter())
        .map(|c| c.name.as_str())
        .collect();
    Err(RustlerError::new(
        ErrorCode::DependencyMissing,
        "setup",
        format!(
            "{} setup step(s) failed: {list} (ok: {})",
            failed.len(),
            done.join(", ")
        ),
    )
    .hint("re-run `rustler setup`; completed steps are skipped")
    .retryable())
}

async fn doctor_run(ctx: Ctx, _input: DoctorInput) -> rustler_core::Result<DoctorReport> {
    let paths = ToolPaths::from_home(&ctx.home);
    let mut report = doctor(&paths).await;
    if paths.venv_python.is_file() {
        let caps = async {
            let engine = EngineClient::spawn(&paths).await?;
            let r: rustler_core::Result<serde_json::Value> = engine
                .call("engine.capabilities", serde_json::json!({}), &ctx.progress, &ctx.cancel)
                .await;
            engine.shutdown().await;
            r
        }
        .await;
        match caps {
            Ok(v) => {
                report.optional_features = optional_features(&paths, Some(&v));
                report.engine = Some(v);
            }
            Err(e) => {
                report.missing.push(format!("engine (not responding: {})", e.message));
                report.ready = false;
            }
        }
    }
    Ok(report)
}

/// Registers `setup.run` and `doctor.run`.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "setup.run",
        "Install ffmpeg, yt-dlp, uv, the lightweight base Python engine, whisper `base` and fonts into the Rustler home (idempotent). Heavy analyzers are opt-in: --with ocr,audio-events,speakers,cuda,sfx-pack, or --profile full for everything. Never removes what is installed",
        &[],
        Annotations {
            network: true,
            long_running: true,
            ..Default::default()
        },
        vec![
            Example {
                description: "Default lite install (base engine, whisper base, no heavy analyzers)",
                cli: "rustler setup run",
            },
            Example {
                description: "Add on-screen text recognition and speaker diarization later",
                cli: "rustler setup run --with ocr,speakers",
            },
            Example {
                description: "Everything (all analyzers, models, SFX pack; cuda if an NVIDIA GPU is found)",
                cli: "rustler setup run --profile full",
            },
            Example {
                description: "Also fetch the medium whisper model",
                cli: "rustler setup run --whisper-models base --whisper-models medium",
            },
        ],
        setup_run,
    ));
    reg.register(typed(
        "doctor.run",
        "Report installed tools, encoders (real 1-frame probes), GPU, fonts, engine capabilities, the install profile and which optional heavy features (ocr, audio-events, speakers, cuda, sfx-pack) are available, with the command to enable each",
        &[],
        Annotations {
            read_only: true,
            ..Default::default()
        },
        vec![Example {
            description: "Check the installation",
            cli: "rustler doctor",
        }],
        doctor_run,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustler_tools::{GpuMode, InstallState, Profile};

    #[test]
    fn lite_fetches_only_whisper_base() {
        let plan = SetupOptions::default().plan(&InstallState::default(), false).unwrap();
        assert_eq!(fetch_params(&plan), serde_json::json!({"whisper": ["base"]}));
    }

    #[test]
    fn full_prefetches_every_heavy_model_and_with_only_its_own() {
        let full = SetupOptions { profile: Profile::Full, gpu: GpuMode::None, ..Default::default() };
        let p = fetch_params(&full.plan(&InstallState::default(), false).unwrap());
        assert_eq!(p["yunet"], true);
        assert_eq!(p["yamnet"], true);
        assert_eq!(p["diarization"], true);
        assert_eq!(p["ocr_devanagari"], true);
        let one = SetupOptions { with: vec!["speakers".into()], ..Default::default() };
        let p = fetch_params(&one.plan(&InstallState::default(), false).unwrap());
        assert_eq!(p["diarization"], true);
        assert!(p.get("yamnet").is_none() && p.get("ocr_devanagari").is_none() && p.get("yunet").is_none());
    }
}
