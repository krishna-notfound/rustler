//! Commands: setup.run, doctor.run (Task 4).
//!
//! `rustler-tools` cannot call the engine (the bridge depends on it), so the engine-backed
//! parts live here: `setup.run` fetches whisper models (setup step 5) after the tools are
//! installed, and `doctor.run` fills `DoctorReport.engine` from `engine.capabilities`.

use rustler_core::{ErrorCode, RustlerError};
use rustler_engine_bridge::EngineClient;
use rustler_registry::{Annotations, Ctx, Example, ProgressEvent, Registry, typed};
use rustler_tools::{DoctorReport, SetupOptions, SetupReport, ToolPaths, doctor, setup};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Output of `setup.run`: the tools report plus the models fetched through the engine.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Default)]
pub struct SetupOutput {
    #[serde(flatten)]
    pub report: SetupReport,
    /// Whisper models fetched via the engine (`models.fetch`).
    pub models_fetched: Vec<String>,
    pub warnings: Vec<String>,
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
    });
}

/// Step 5: `models.fetch` through the freshly installed engine.
async fn fetch_models(ctx: &Ctx, paths: &ToolPaths, models: &[String]) -> rustler_core::Result<Vec<String>> {
    emit(ctx, "downloading whisper, face (YuNet), audio-event (YAMNet) and speaker-diarization models");
    let engine = EngineClient::spawn(paths).await?;
    let res: rustler_core::Result<FetchResult> = engine
        .call(
            "models.fetch",
            // Pre-fetch everything the engine would otherwise download on first use, so agents
            // in sandboxes don't hit a surprise download mid-edit.
            serde_json::json!({ "whisper": models, "yunet": true, "yamnet": true, "diarization": true }),
            &ctx.progress,
            &ctx.cancel,
        )
        .await;
    engine.shutdown().await;
    Ok(res?.fetched)
}

async fn setup_run(ctx: Ctx, opts: SetupOptions) -> rustler_core::Result<SetupOutput> {
    let paths = ToolPaths::from_home(&ctx.home);
    let mut report = setup(&paths, &opts, &ctx.progress).await?;
    let mut out = SetupOutput::default();
    out.warnings.extend(report.notes.iter().cloned());

    if opts.whisper_models.is_empty() {
        out.warnings.push("no whisper models requested; skipping model download".into());
    } else if paths.venv_python.is_file() {
        match fetch_models(&ctx, &paths, &opts.whisper_models).await {
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
            Ok(v) => report.engine = Some(v),
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
        "Install ffmpeg, yt-dlp, uv, the Python engine, whisper models and fonts into the Rustler home (idempotent)",
        &[],
        Annotations {
            network: true,
            long_running: true,
            ..Default::default()
        },
        vec![
            Example {
                description: "Default lite install",
                cli: "rustler setup",
            },
            Example {
                description: "Also fetch the medium whisper model",
                cli: "rustler setup --whisper-models small --whisper-models medium",
            },
        ],
        setup_run,
    ));
    reg.register(typed(
        "doctor.run",
        "Report installed tools, encoders (real 1-frame probes), GPU, fonts and engine capabilities",
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
