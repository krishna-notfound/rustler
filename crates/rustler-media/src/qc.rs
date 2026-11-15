//! QC v1 checks on a finished render. (Task 11)
//!
//! One ffprobe call plus one ffmpeg analysis pass (blackdetect, freezedetect, silencedetect,
//! ebur128 with true peak) plus one frame grab and a 12-tile contact sheet. Every non-passing
//! check carries a `fix_hint` naming a concrete `rustler` command.

use crate::facepath::{FaceBox, FaceSample};
use crate::frames::frame;
use crate::proc::{self, argv};
use crate::tiles::{TileSpec, tiles};
use ab_glyph::{FontArc, PxScale};
use image::{Rgb, RgbImage, imageops};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect as PxRect;
use rustler_core::model::Rect;
use rustler_core::{
    Clip, ErrorCode, QcCheck, QcReport, QcStatus, RenderRecord, RustlerError, TimeMap,
};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

const STAGE: &str = "qc";
const LOUDNESS_TOLERANCE_LU: f64 = 1.5;
const TRUE_PEAK_LIMIT_DBTP: f64 = -1.0;
const DURATION_TOLERANCE_S: f64 = 0.10;
const FIRST_FRAME_MIN_LUMA: f64 = 16.0;
const LONG_SILENCE_S: f64 = 1.5;
const HOOK_WINDOW_S: f64 = 3.0;
const SAFE_ZONE_MIN_Y: f64 = 0.10;
const SAFE_ZONE_MAX_Y: f64 = 0.85;
const CONTACT_SHEET_NOTE: &str = "12-frame contact sheet; look at this image before declaring the clip done";

/// The report as persisted and returned by `qc.run`: the frozen [`QcReport`] fields plus
/// what the agent should do next. (`QcReport` itself is a frozen contract, so the extras
/// travel in this wrapper; it deserializes from, and serializes to, the same JSON object.)
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct QcView {
    #[serde(flatten)]
    pub report: QcReport,
    /// Images the agent must open (Read tool) before calling the clip done.
    pub look_at: Vec<PathBuf>,
    /// One line: pass/fail, counts and the names of the checks that need attention.
    pub verdict: String,
}

/// Wraps a report with `look_at` and `verdict`.
pub fn view(report: QcReport) -> QcView {
    let look_at = vec![report.contact_sheet.clone()];
    let verdict = verdict(&report);
    QcView { report, look_at, verdict }
}

/// Names of checks with the given status.
pub fn names_with(report: &QcReport, status: QcStatus) -> Vec<String> {
    report.checks.iter().filter(|c| c.status == status).map(|c| c.name.clone()).collect()
}

/// One-line human/agent summary of a report.
pub fn verdict(report: &QcReport) -> String {
    let fails = names_with(report, QcStatus::Fail);
    let warns = names_with(report, QcStatus::Warn);
    let total = report.checks.len();
    let sheet = report.contact_sheet.display();
    if !fails.is_empty() {
        format!(
            "FAIL: {} of {total} checks failed ({}); follow each fix_hint, re-render, then re-run QC. Look at {sheet}",
            fails.len(),
            fails.join(", ")
        )
    } else if !warns.is_empty() {
        format!(
            "PASS with {} warning(s) ({}); review the fix_hints, then look at {sheet} before declaring the clip done",
            warns.len(),
            warns.join(", ")
        )
    } else {
        format!("PASS: all {total} checks ok; look at {sheet} before declaring the clip done")
    }
}

/// A TTF usable for contact-sheet labels: `home/fonts/**.ttf` (Montserrat first), else a
/// common system font. `None` when the machine has none.
pub fn default_font(paths: &ToolPaths) -> Option<PathBuf> {
    fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                collect(&p, depth + 1, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttf")) {
                out.push(p);
            }
        }
    }
    let mut found = Vec::new();
    collect(&paths.fonts, 0, &mut found);
    found.sort();
    let pick = found
        .iter()
        .find(|p| p.to_string_lossy().to_lowercase().contains("montserrat"))
        .or_else(|| found.first())
        .cloned();
    pick.or_else(|| {
        [
            r"C:\Windows\Fonts\arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/Library/Fonts/Arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    })
}

// ---------------------------------------------------------------------------------------
// ffprobe
// ---------------------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct StreamInfo {
    codec: String,
    width: u32,
    height: u32,
    duration: Option<f64>,
}

#[derive(Debug, Default)]
struct Streams {
    video: Option<StreamInfo>,
    audio: Option<StreamInfo>,
    format_duration: f64,
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

async fn probe_streams(paths: &ToolPaths, file: &Path, cancel: &CancellationToken) -> rustler_core::Result<Streams> {
    let mut args = argv!["-v", "error", "-print_format", "json", "-show_streams", "-show_format"];
    args.push(file.as_os_str().to_owned());
    let out = proc::run(&paths.ffprobe, &args, None, cancel).await.map_err(|mut e| {
        e.stage = STAGE.into();
        e
    })?;
    let j: Value = serde_json::from_slice(&out.stdout).map_err(|e| {
        RustlerError::new(ErrorCode::Media, STAGE, format!("ffprobe returned invalid JSON: {e}"))
    })?;
    let mut s = Streams { format_duration: num(&j["format"]["duration"]).unwrap_or(0.0), ..Streams::default() };
    for st in j["streams"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let info = StreamInfo {
            codec: st["codec_name"].as_str().unwrap_or_default().to_string(),
            width: st["width"].as_u64().unwrap_or(0) as u32,
            height: st["height"].as_u64().unwrap_or(0) as u32,
            duration: num(&st["duration"]),
        };
        match st["codec_type"].as_str() {
            Some("video") if s.video.is_none() => s.video = Some(info),
            Some("audio") if s.audio.is_none() => s.audio = Some(info),
            _ => {}
        }
    }
    Ok(s)
}

// ---------------------------------------------------------------------------------------
// ffmpeg analysis pass
// ---------------------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct Analysis {
    /// (start, end)
    black: Vec<(f64, f64)>,
    /// (start, duration)
    freezes: Vec<(f64, Option<f64>)>,
    /// silencedetect n=-45dB d=0.5 intervals (start, end)
    silences: Vec<(f64, f64)>,
    open_silence: Option<f64>,
    integrated_lufs: Option<f64>,
    true_peak_dbtp: Option<f64>,
    expect: Expect,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
enum Expect {
    #[default]
    Nothing,
    Integrated,
    TruePeak,
}

/// The number following `key` (skipping `:`/spaces), e.g. `kv("a: 1.5 | b", "a:") == Some(1.5)`.
fn kv(line: &str, key: &str) -> Option<f64> {
    let i = line.find(key)? + key.len();
    let rest = line[i..].trim_start_matches([':', ' ', '\t']);
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

impl Analysis {
    fn feed(&mut self, line: &str) {
        let t = line.trim();
        if line.contains("blackdetect") && t.contains("black_start") {
            if let (Some(s), Some(e)) = (kv(t, "black_start"), kv(t, "black_end")) {
                self.black.push((s, e));
            }
        } else if t.contains("freeze_start") {
            if let Some(s) = kv(t, "freeze_start") {
                self.freezes.push((s, None));
            }
        } else if t.contains("freeze_duration") {
            if let (Some(d), Some(last)) = (kv(t, "freeze_duration"), self.freezes.last_mut()) {
                last.1 = Some(d);
            }
        } else if t.contains("silence_start") {
            if let Some(s) = kv(t, "silence_start") {
                self.open_silence = Some(s.max(0.0));
            }
        } else if t.contains("silence_end") {
            if let (Some(e), Some(s)) = (kv(t, "silence_end"), self.open_silence.take()) {
                self.silences.push((s, e.max(s)));
            }
        } else if t.contains("Integrated loudness:") {
            self.expect = Expect::Integrated;
        } else if t.contains("True peak:") {
            self.expect = Expect::TruePeak;
        } else if self.expect == Expect::Integrated && t.starts_with("I:") {
            self.integrated_lufs = kv(t, "I:");
            self.expect = Expect::Nothing;
        } else if self.expect == Expect::TruePeak && t.starts_with("Peak:") {
            self.true_peak_dbtp = kv(t, "Peak:");
            self.expect = Expect::Nothing;
        }
    }

    fn finish(&mut self, total: f64) {
        if let Some(s) = self.open_silence.take() {
            self.silences.push((s, total.max(s)));
        }
    }
}

async fn analyze(
    paths: &ToolPaths,
    file: &Path,
    has_video: bool,
    has_audio: bool,
    total: f64,
    cancel: &CancellationToken,
) -> rustler_core::Result<Analysis> {
    let mut args = argv!["-hide_banner", "-loglevel", "info", "-nostdin", "-nostats", "-i"];
    args.push(file.as_os_str().to_owned());
    if has_video {
        args.extend(argv![
            "-map", "0:v:0",
            "-vf", "blackdetect=d=0.5:pix_th=0.10,freezedetect=n=0.003:d=2",
        ]);
    } else {
        args.extend(argv!["-vn"]);
    }
    if has_audio {
        args.extend(argv![
            "-map", "0:a:0",
            "-af", "silencedetect=n=-45dB:d=0.5,ebur128=peak=true:framelog=quiet",
        ]);
    } else {
        args.extend(argv!["-an"]);
    }
    args.extend(argv!["-f", "null", "-"]);
    let out = proc::run(&paths.ffmpeg, &args, None, cancel).await.map_err(|mut e| {
        e.stage = STAGE.into();
        e
    })?;
    let mut a = Analysis::default();
    for line in String::from_utf8_lossy(&out.stderr).lines() {
        a.feed(line);
    }
    a.finish(total);
    Ok(a)
}

// ---------------------------------------------------------------------------------------
// check construction
// ---------------------------------------------------------------------------------------

fn check(name: &str, status: QcStatus, value: Value, expected: &str, fix_hint: Option<String>) -> QcCheck {
    QcCheck { name: name.into(), status, value, expected: expected.into(), fix_hint }
}

fn overlap(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0 // + 0.0 turns -0.0 into 0.0
}

fn mean_luma(jpg: &Path) -> rustler_core::Result<f64> {
    let img = image::open(jpg)
        .map_err(|e| RustlerError::new(ErrorCode::Media, STAGE, format!("cannot read frame {}: {e}", jpg.display())))?
        .to_luma8();
    let n = img.as_raw().len().max(1) as f64;
    Ok(img.as_raw().iter().map(|&p| f64::from(p)).sum::<f64>() / n)
}

/// Runs every check on `render_path` and writes the contact sheet to `out_dir/<render id>.jpg`.
/// `passed` is true when no check is `Fail`; warnings do not fail QC. Without engine data the
/// face checks report `warn` / `{"skipped": "engine unavailable"}`; see [`run_qc_with`].
pub async fn run_qc(
    paths: &ToolPaths,
    clip: &Clip,
    render: &RenderRecord,
    render_path: &Path,
    font: &Path,
    out_dir: &Path,
) -> rustler_core::Result<QcReport> {
    run_qc_with(paths, clip, render, render_path, font, out_dir, &QcExtras::default()).await
}

/// [`run_qc`] plus the data the media crate cannot gather itself (face samples from the
/// Python engine, overlays, caption timing, the brief).
pub async fn run_qc_with(
    paths: &ToolPaths,
    clip: &Clip,
    render: &RenderRecord,
    render_path: &Path,
    font: &Path,
    out_dir: &Path,
    extras: &QcExtras,
) -> rustler_core::Result<QcReport> {
    if !render_path.is_file() {
        return Err(RustlerError::new(
            ErrorCode::NotFound,
            STAGE,
            format!("render file not found: {}", render_path.display()),
        )
        .hint(format!("re-render with `rustler render run {}`", clip.id)));
    }
    std::fs::create_dir_all(out_dir).map_err(|e| {
        RustlerError::new(ErrorCode::Internal, STAGE, format!("cannot create {}: {e}", out_dir.display()))
    })?;
    let cancel = CancellationToken::new();
    let cid = clip.id.to_string();
    let st = probe_streams(paths, render_path, &cancel).await?;
    let expected_dur = extras.expected_duration.unwrap_or_else(|| TimeMap::from_segments(&clip.segments).duration());
    let actual_dur = st.format_duration;
    let mut checks: Vec<QcCheck> = Vec::new();

    // streams
    let v_ok = st.video.as_ref().is_some_and(|v| v.codec == "h264");
    let a_ok = st.audio.as_ref().is_some_and(|a| a.codec == "aac");
    checks.push(check(
        "streams",
        if v_ok && a_ok { QcStatus::Pass } else { QcStatus::Fail },
        json!({
            "video": st.video.as_ref().map(|v| v.codec.clone()),
            "audio": st.audio.as_ref().map(|a| a.codec.clone()),
        }),
        "h264 video + aac audio",
        (!(v_ok && a_ok)).then(|| {
            format!("re-render with the default encoder: rustler render run {cid} --quality final (output must be h264 + aac; a source without audio gets a silent aac track)")
        }),
    ));

    // dimensions
    if let Some(v) = &st.video {
        let ok = v.width == clip.canvas.width && v.height == clip.canvas.height;
        checks.push(check(
            "dimensions",
            if ok { QcStatus::Pass } else { QcStatus::Fail },
            json!({ "width": v.width, "height": v.height }),
            &format!("{}x{}", clip.canvas.width, clip.canvas.height),
            (!ok).then(|| {
                format!("canvas and file disagree; re-render: rustler render run {cid} (canvas comes from the clip, see `rustler clip show {cid}`)")
            }),
        ));
    }

    // duration + av_sync
    let dur_ok = (actual_dur - expected_dur).abs() <= DURATION_TOLERANCE_S;
    checks.push(check(
        "duration",
        if dur_ok { QcStatus::Pass } else { QcStatus::Fail },
        json!({ "file_s": r2(actual_dur), "timeline_s": r2(expected_dur) }),
        &format!("file duration within {DURATION_TOLERANCE_S} s of the clip timeline"),
        (!dur_ok).then(|| {
            format!("the file is stale or truncated relative to the clip; re-render: rustler render run {cid} (inspect segments with `rustler clip show {cid}`)")
        }),
    ));
    if let (Some(v), Some(a)) = (&st.video, &st.audio) {
        let vd = v.duration.unwrap_or(actual_dur);
        let ad = a.duration.unwrap_or(actual_dur);
        let ok = (vd - ad).abs() <= DURATION_TOLERANCE_S;
        checks.push(check(
            "av_sync",
            if ok { QcStatus::Pass } else { QcStatus::Fail },
            json!({ "video_s": r2(vd), "audio_s": r2(ad), "delta_s": r2(vd - ad) }),
            &format!("video and audio stream durations within {DURATION_TOLERANCE_S} s"),
            (!ok).then(|| {
                format!("audio/video drifted; re-render in one pass: rustler render run {cid} (check that a segment's source has the same length of audio and video)")
            }),
        ));
    }

    // analysis pass (black, freeze, silence, loudness)
    let an = analyze(paths, render_path, st.video.is_some(), st.audio.is_some(), actual_dur, &cancel).await?;

    if st.audio.is_some() {
        let tgt = clip.audio.target_lufs;
        let (i, tp) = (an.integrated_lufs, an.true_peak_dbtp);
        let (status, hint) = if !clip.audio.loudnorm {
            (QcStatus::Pass, None)
        } else {
            let i_ok = i.is_some_and(|i| (i - tgt).abs() <= LOUDNESS_TOLERANCE_LU);
            let tp_ok = tp.is_none_or(|tp| tp <= TRUE_PEAK_LIMIT_DBTP);
            if i_ok && tp_ok {
                (QcStatus::Pass, None)
            } else {
                (
                    QcStatus::Warn,
                    Some(format!(
                        "re-render; loudnorm is on by default: rustler render run {cid} (very short or near-silent clips may not reach the target)"
                    )),
                )
            }
        };
        checks.push(check(
            "loudness",
            status,
            json!({ "integrated_lufs": i, "true_peak_dbtp": tp, "loudnorm": clip.audio.loudnorm }),
            &if clip.audio.loudnorm {
                format!("integrated {tgt} LUFS +/-{LOUDNESS_TOLERANCE_LU} LU, true peak <= {TRUE_PEAK_LIMIT_DBTP} dBTP")
            } else {
                "loudnorm is off for this clip; measured only, not enforced".to_string()
            },
            hint,
        ));
    }

    if st.video.is_some() {
        let black: Vec<Value> = an.black.iter().map(|&(s, e)| json!({"start": r2(s), "end": r2(e)})).collect();
        checks.push(check(
            "black_frames",
            if black.is_empty() { QcStatus::Pass } else { QcStatus::Fail },
            json!({ "intervals": black }),
            "no black interval of 0.5 s or more",
            (!black.is_empty()).then(|| {
                let at = an.black.first().map_or(0.0, |b| b.0);
                format!("black video at {at:.1}s (output time); check layout: rustler clip set-layout {cid} --preset blur-fit (or center-crop with --focus-x), and that the segment range is inside the source: rustler clip show {cid}")
            }),
        ));

        let freezes: Vec<Value> = an
            .freezes
            .iter()
            .map(|&(s, d)| json!({"start": r2(s), "duration": d.map(r2)}))
            .collect();
        checks.push(check(
            "frozen_video",
            if freezes.is_empty() { QcStatus::Pass } else { QcStatus::Warn },
            json!({ "intervals": freezes }),
            "no frozen interval of 2 s or more",
            (!freezes.is_empty()).then(|| {
                let at = an.freezes.first().map_or(0.0, |b| b.0);
                format!("video is frozen from {at:.1}s (output time); look at the contact sheet, then trim the segment: rustler edit trim {cid} --end-delta -2 (or split and drop it: rustler edit split {cid} --out-t {at:.1})")
            }),
        ));
    }

    if st.audio.is_some() {
        let long: Vec<(f64, f64)> =
            an.silences.iter().copied().filter(|&(s, e)| e - s >= LONG_SILENCE_S).collect();
        checks.push(check(
            "long_silence",
            if long.is_empty() { QcStatus::Pass } else { QcStatus::Warn },
            json!({ "intervals": long.iter().map(|&(s, e)| json!({"start": r2(s), "end": r2(e)})).collect::<Vec<_>>() }),
            "no silence of 1.5 s or more (-45 dB)",
            (!long.is_empty()).then(|| {
                format!(
                    "consider cutting the pause at {:.1}s (output time): rustler edit remove-pauses {cid}",
                    long[0].0
                )
            }),
        ));
    }

    // first_frame_not_black
    if st.video.is_some() {
        let probe_jpg = out_dir.join(format!("{}.first.jpg", render.id));
        let res = frame(paths, render_path, 0.1, 270, &probe_jpg).await.and_then(|()| mean_luma(&probe_jpg));
        let _ = std::fs::remove_file(&probe_jpg);
        match res {
            Ok(luma) => {
                let ok = luma > FIRST_FRAME_MIN_LUMA;
                checks.push(check(
                    "first_frame_not_black",
                    if ok { QcStatus::Pass } else { QcStatus::Warn },
                    json!({ "t": 0.1, "mean_luma": r2(luma) }),
                    &format!("mean luma at 0.1 s > {FIRST_FRAME_MIN_LUMA}"),
                    (!ok).then(|| {
                        format!("the opening frame is dark, which weakens the hook; start on picture: rustler edit trim {cid} --start-delta 0.3 (or check the layout: rustler clip set-layout {cid} --preset blur-fit)")
                    }),
                ));
            }
            Err(e) => checks.push(check(
                "first_frame_not_black",
                QcStatus::Warn,
                json!({ "error": e.message }),
                &format!("mean luma at 0.1 s > {FIRST_FRAME_MIN_LUMA}"),
                Some(format!("could not grab the first frame; re-render: rustler render run {cid}")),
            )),
        }
    }

    // hook_window: is there sound in the first 3 s, and is the first 1.5 s silent?
    if st.audio.is_some() {
        let hook = HOOK_WINDOW_S.min(actual_dur);
        let lead = 1.5_f64.min(actual_dur);
        let silent_hook: f64 = an.silences.iter().map(|&s| overlap(s, (0.0, hook))).sum();
        let silent_lead: f64 = an.silences.iter().map(|&s| overlap(s, (0.0, lead))).sum();
        let lead_silent = lead > 0.0 && silent_lead >= lead - 0.05;
        let has_sound = hook <= 0.0 || silent_hook < hook - 0.25;
        checks.push(check(
            "hook_window",
            if lead_silent { QcStatus::Warn } else { QcStatus::Pass },
            json!({
                "sound_in_first_3s": has_sound,
                "silent_in_first_1_5s_s": r2(silent_lead),
                "silent_in_first_3s_s": r2(silent_hook),
            }),
            "sound within the first 1.5 s (audio-active hook)",
            lead_silent.then(|| {
                format!("the first 1.5 s is silent, so the hook lands late; trim start: rustler edit trim {cid} --start-delta 0.5")
            }),
        ));
    }

    // caption_safe_zone (platform UI covers the very top and bottom)
    {
        let y = clip.captions.position_y;
        let bad = clip.captions.enabled && !(SAFE_ZONE_MIN_Y..=SAFE_ZONE_MAX_Y).contains(&y);
        checks.push(check(
            "caption_safe_zone",
            if bad { QcStatus::Warn } else { QcStatus::Pass },
            json!({ "captions_enabled": clip.captions.enabled, "position_y": y }),
            &format!("caption position_y between {SAFE_ZONE_MIN_Y} and {SAFE_ZONE_MAX_Y} (outside the platform UI zones)"),
            bad.then(|| {
                format!("captions sit under platform UI; move them: rustler clip set-captions {cid} --position-y 0.70")
            }),
        ));
    }

    // v2 checks: faces (engine), hook text, platform duration
    if st.video.is_some() {
        checks.extend(face_checks(clip, extras));
        checks.push(hook_text_check(clip, extras));
        checks.push(duration_check(&cid, actual_dur, extras));
    }

    // contact sheet (always Pass)
    let sheet = out_dir.join(format!("{}.jpg", render.id));
    let spec = TileSpec {
        start: 0.0,
        end: actual_dur.max(0.2),
        count: 12,
        cols: 4,
        tile_width: 270,
    };
    tiles(paths, render_path, &spec, &|_| String::new(), font, &sheet).await?;
    checks.push(check(
        "contact_sheet",
        QcStatus::Pass,
        json!(sheet.display().to_string()),
        CONTACT_SHEET_NOTE,
        None,
    ));

    let passed = checks.iter().all(|c| c.status != QcStatus::Fail);
    Ok(QcReport {
        schema: "rustler.qc@1".into(),
        clip_id: clip.id.clone(),
        render_id: render.id.clone(),
        passed,
        checks,
        contact_sheet: sheet,
    })
}

// ---------------------------------------------------------------------------------------
// QC v2: face-aware checks, hook text, platform duration, comparison sheet
// ---------------------------------------------------------------------------------------

/// Sampling rate (frames per second) of the engine's `faces.track` run over the render.
pub const FACE_SAMPLE_FPS: f64 = 2.0;
/// Caption band around `position_y`, as fractions of the frame height.
const CAPTION_BAND_ABOVE: f64 = 0.08;
const CAPTION_BAND_BELOW: f64 = 0.04;
/// Margin kept between the caption band and the face when suggesting a new position.
const CAPTION_FACE_MARGIN: f64 = 0.02;
const CAPTION_OVER_FACE_WARN_RATIO: f64 = 0.20;
const FACE_INSIDE_MIN: f64 = 0.60;
const FACE_VISIBLE_WARN_BELOW: f64 = 0.70;
const FACE_EDGE_EPS: f64 = 0.01;
const FACE_CUT_OFF_WARN_RATIO: f64 = 0.25;
const SHORTS_MAX_S: f64 = 60.0;
const REELS_TIKTOK_MAX_S: f64 = 90.0;
const SKIPPED_ENGINE: &str = "engine unavailable";
/// Value of the face checks when they were not requested (QC stays fast and engine-free by default).
pub const SKIPPED_FACES_OFF: &str = "off by default — look at the contact sheet, or run qc run --faces";

/// Brief limits relevant to QC (all optional; read leniently from `brief.json`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QcBrief {
    pub platform: Option<String>,
    pub min_s: Option<f64>,
    pub max_s: Option<f64>,
}

/// Data QC needs that the media crate cannot gather itself. `QcExtras::default()` means
/// "nothing known": the engine-based checks report `skipped`.
#[derive(Clone, Debug, Default)]
pub struct QcExtras {
    /// `faces.track` samples over the render at [`FACE_SAMPLE_FPS`]; `None` = engine unavailable
    /// (or, with `faces_off`, not requested).
    pub faces: Option<Vec<FaceSample>>,
    /// The face checks were not requested (`qc run --faces` / project `qc_faces`): they report
    /// `pass` with `{"skipped": "off by default ..."}` instead of an engine-unavailable warning.
    pub faces_off: bool,
    /// Start times (output seconds) of the clip's text overlays.
    pub overlay_starts: Vec<f64>,
    /// Output time of the first caption word, when a transcript covers the clip.
    pub first_caption_word_s: Option<f64>,
    pub brief: Option<QcBrief>,
    /// True when the registry has `overlay.add_text` (changes the hook-text hint).
    pub overlay_add_text: bool,
    /// True when `extras.json` holds a face-tracked crop path for the clip.
    pub has_crop_path: bool,
    /// Rendered duration when transitions shorten the clip (overlaps removed); `None` = the
    /// plain TimeMap duration.
    pub expected_duration: Option<f64>,
}

/// Where in the frame a face matters for a clip's layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FaceRegion {
    /// The whole frame (center-crop layouts).
    Whole,
    /// Only the top pane (split-vertical layouts); faces are judged by their centre.
    Top(Rect),
}

/// The face-relevant region of a clip's layouts: the whole frame for any center-crop segment,
/// else the top pane of a split-vertical segment, else `None` (blur-fit and custom layouts).
pub fn face_region(clip: &Clip) -> Option<FaceRegion> {
    let mut top = None;
    for seg in &clip.segments {
        match seg.layout.preset.as_deref() {
            Some("center-crop") => return Some(FaceRegion::Whole),
            Some("split-vertical") => {
                let r = seg
                    .layout
                    .regions
                    .iter()
                    .min_by(|a, b| a.dst.y.partial_cmp(&b.dst.y).unwrap_or(std::cmp::Ordering::Equal))
                    .map_or(Rect { x: 0.0, y: 0.0, w: 1.0, h: 0.5 }, |r| r.dst);
                top = Some(FaceRegion::Top(r));
            }
            _ => {}
        }
    }
    top
}

/// True when QC has any face-based check to run (needs `faces.track` on the render).
pub fn wants_faces(clip: &Clip) -> bool {
    clip.captions.enabled || face_region(clip).is_some()
}

/// The vertical band `[lo, hi]` captions occupy at `position_y`.
pub fn caption_band(position_y: f64) -> (f64, f64) {
    ((position_y - CAPTION_BAND_ABOVE).max(0.0), (position_y + CAPTION_BAND_BELOW).min(1.0))
}

/// A `position_y` whose caption band clears a face spanning `top..bottom` (fractions of the
/// frame height): below it when that fits inside the safe zone, else above it, else `None`.
pub fn suggest_caption_y(top: f64, bottom: f64) -> Option<f64> {
    let below = ((bottom + CAPTION_BAND_ABOVE + CAPTION_FACE_MARGIN) * 100.0 - 1e-9).ceil() / 100.0;
    if below <= SAFE_ZONE_MAX_Y {
        return Some(below);
    }
    let above = ((top - CAPTION_BAND_BELOW - CAPTION_FACE_MARGIN) * 100.0 + 1e-9).floor() / 100.0;
    (above >= SAFE_ZONE_MIN_Y).then_some(above)
}

/// Share of `b`'s area that lies inside the frame (0..1).
fn inside_fraction(b: &FaceBox) -> f64 {
    let area = b.w * b.h;
    if area <= 0.0 {
        return 0.0;
    }
    let iw = ((b.x + b.w).min(1.0) - b.x.max(0.0)).max(0.0);
    let ih = ((b.y + b.h).min(1.0) - b.y.max(0.0)).max(0.0);
    iw * ih / area
}

fn in_region(b: &FaceBox, region: FaceRegion) -> bool {
    match region {
        FaceRegion::Whole => true,
        FaceRegion::Top(r) => {
            let cy = b.y + b.h / 2.0;
            cy >= r.y && cy <= r.y + r.h
        }
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if v.is_empty() { 0.0 } else { v[v.len() / 2] }
}

fn ratio(count: usize, total: usize) -> f64 {
    if total == 0 { 0.0 } else { count as f64 / total as f64 }
}

fn skipped(name: &str, expected: &str, cid: &str, off: bool) -> QcCheck {
    if off {
        // Pass-with-info: nothing is wrong, the check just was not asked for.
        return check(
            name,
            QcStatus::Pass,
            json!({ "skipped": SKIPPED_FACES_OFF }),
            expected,
            Some(format!("to check faces, run: rustler qc run {cid} --faces (or set `project set-settings --qc-faces true`)")),
        );
    }
    check(
        name,
        QcStatus::Warn,
        json!({ "skipped": SKIPPED_ENGINE }),
        expected,
        Some(format!("the Python engine could not run; check `rustler doctor`, then re-run: rustler qc run {cid}")),
    )
}

/// `caption_over_face`, `face_visible_ratio` and `face_cut_off` from engine face samples.
/// Returns nothing when none applies (captions off and a layout where faces do not matter).
pub fn face_checks(clip: &Clip, extras: &QcExtras) -> Vec<QcCheck> {
    let cid = clip.id.to_string();
    let region = face_region(clip);
    let caps = clip.captions.enabled;
    let exp_caption = format!(
        "faces overlap the caption band (position_y -{CAPTION_BAND_ABOVE} .. +{CAPTION_BAND_BELOW}) in at most {:.0}% of samples",
        CAPTION_OVER_FACE_WARN_RATIO * 100.0
    );
    let exp_visible = format!(
        "a face at least {:.0}% inside the frame in at least {:.0}% of samples",
        FACE_INSIDE_MIN * 100.0,
        FACE_VISIBLE_WARN_BELOW * 100.0
    );
    let exp_cut = format!(
        "no face touching the left/right frame edge in more than {:.0}% of samples",
        FACE_CUT_OFF_WARN_RATIO * 100.0
    );
    let Some(samples) = &extras.faces else {
        let mut out = Vec::new();
        if caps {
            out.push(skipped("caption_over_face", &exp_caption, &cid, extras.faces_off));
        }
        if region.is_some() {
            out.push(skipped("face_visible_ratio", &exp_visible, &cid, extras.faces_off));
            out.push(skipped("face_cut_off", &exp_cut, &cid, extras.faces_off));
        }
        return out;
    };
    let n = samples.len();
    let mut out = Vec::new();

    if caps {
        let y = clip.captions.position_y;
        let (lo, hi) = caption_band(y);
        // the largest face overlapping the band, per sample
        let hits: Vec<&FaceBox> = samples
            .iter()
            .filter_map(|s| {
                s.faces
                    .iter()
                    .filter(|f| f.y < hi && f.y + f.h > lo)
                    .max_by(|a, b| (a.w * a.h).partial_cmp(&(b.w * b.h)).unwrap_or(std::cmp::Ordering::Equal))
            })
            .collect();
        let r = ratio(hits.len(), n);
        let warn = r > CAPTION_OVER_FACE_WARN_RATIO;
        let suggestion = if hits.is_empty() {
            None
        } else {
            let top = median(hits.iter().map(|f| f.y).collect());
            let bottom = median(hits.iter().map(|f| f.y + f.h).collect());
            suggest_caption_y(top, bottom)
        };
        out.push(check(
            "caption_over_face",
            if warn { QcStatus::Warn } else { QcStatus::Pass },
            json!({
                "samples": n,
                "overlap_ratio": r2(r),
                "position_y": y,
                "band": [r2(lo), r2(hi)],
                "suggested_position_y": suggestion,
            }),
            &exp_caption,
            warn.then(|| match suggestion {
                Some(s) => format!(
                    "captions cover the speaker's face in {:.0}% of samples; move them: rustler clip set-captions {cid} --position-y {s:.2} (then re-render: rustler render run {cid})",
                    r * 100.0
                ),
                None => format!(
                    "captions cover the speaker's face in {:.0}% of samples and no position clears it; reframe so the face sits higher or smaller: rustler crop track-face {cid} (or pick another layout: rustler clip set-layout {cid} --preset blur-fit)",
                    r * 100.0
                ),
            }),
        ));
    }

    if let Some(region) = region {
        let visible = samples
            .iter()
            .filter(|s| s.faces.iter().any(|f| in_region(f, region) && inside_fraction(f) >= FACE_INSIDE_MIN))
            .count();
        let r = ratio(visible, n);
        let warn = n > 0 && r < FACE_VISIBLE_WARN_BELOW;
        let where_ = match region {
            FaceRegion::Whole => "frame",
            FaceRegion::Top(_) => "top region",
        };
        let fix = face_fix(clip, region, extras.has_crop_path);
        out.push(check(
            "face_visible_ratio",
            if warn { QcStatus::Warn } else { QcStatus::Pass },
            json!({ "samples": n, "visible_ratio": r2(r), "region": where_ }),
            &exp_visible,
            warn.then(|| {
                format!("a face is visible (>= 60% inside the {where_}) in only {:.0}% of samples; {fix}", r * 100.0)
            }),
        ));

        let (mut left, mut right, mut any) = (0usize, 0usize, 0usize);
        for s in samples {
            let l = s.faces.iter().any(|f| in_region(f, region) && f.x <= FACE_EDGE_EPS);
            let rt = s.faces.iter().any(|f| in_region(f, region) && f.x + f.w >= 1.0 - FACE_EDGE_EPS);
            left += usize::from(l);
            right += usize::from(rt);
            any += usize::from(l || rt);
        }
        let r = ratio(any, n);
        let warn = r > FACE_CUT_OFF_WARN_RATIO;
        out.push(check(
            "face_cut_off",
            if warn { QcStatus::Warn } else { QcStatus::Pass },
            json!({ "samples": n, "cut_off_ratio": r2(r), "left_edge_samples": left, "right_edge_samples": right, "region": where_ }),
            &exp_cut,
            warn.then(|| format!("a face touches the left/right frame edge in {:.0}% of samples (cut off); {fix}", r * 100.0)),
        ));
    }
    out
}

/// The "how to reframe" half of a face hint, by layout.
fn face_fix(clip: &Clip, region: FaceRegion, has_crop_path: bool) -> String {
    let cid = &clip.id;
    match region {
        FaceRegion::Whole if has_crop_path => format!(
            "the tracked crop still loses the face; re-run the tracking: rustler crop track-face {cid} (or show the whole frame: rustler clip set-layout {cid} --preset blur-fit)"
        ),
        FaceRegion::Whole => format!(
            "follow the face with a tracked crop: rustler crop track-face {cid} (or show the whole frame: rustler clip set-layout {cid} --preset blur-fit)"
        ),
        FaceRegion::Top(_) => {
            let src = clip.segments.first().map(|s| s.source_id.to_string()).unwrap_or_default();
            format!("re-detect the webcam box: rustler layout detect-webcam {src} --clip-id {cid} (it prints the exact set-layout command)")
        }
    }
}

/// `hook_text_present`: an overlay or caption words inside the first 3 s.
pub fn hook_text_check(clip: &Clip, extras: &QcExtras) -> QcCheck {
    let cid = clip.id.to_string();
    let overlay = extras.overlay_starts.iter().any(|&s| s < HOOK_WINDOW_S);
    let caption = clip.captions.enabled && extras.first_caption_word_s.is_some_and(|t| t < HOOK_WINDOW_S);
    let ok = overlay || caption;
    check(
        "hook_text_present",
        if ok { QcStatus::Pass } else { QcStatus::Warn },
        json!({
            "overlay_in_first_3s": overlay,
            "captions_in_first_3s": caption,
            "first_caption_word_s": extras.first_caption_word_s.map(r2),
        }),
        "a text overlay or caption words within the first 3 s",
        (!ok).then(|| {
            if extras.overlay_add_text {
                format!("no on-screen text in the first 3 s; add a hook title: rustler overlay add-text {cid} --text \"<hook>\" --start 0 --end 3")
            } else if clip.captions.enabled {
                format!("no caption words in the first 3 s (silent or late opening); start on speech: rustler edit trim {cid} --start-delta 0.5 (or rustler edit refine {cid})")
            } else {
                format!("captions are off and there is no overlay; turn captions on so the hook has text: rustler clip set-captions {cid} --enabled true")
            }
        }),
    )
}

/// `duration_for_platform`: the render's length against the brief (or platform defaults).
pub fn duration_check(cid: &str, duration: f64, extras: &QcExtras) -> QcCheck {
    let brief = extras.brief.clone().unwrap_or_default();
    let platform = brief.platform.as_deref().map(str::to_lowercase);
    let platform_max = platform.as_deref().and_then(|p| {
        if p.contains("short") {
            Some((SHORTS_MAX_S, "YouTube Shorts"))
        } else if p.contains("tiktok") || p.contains("reel") || p.contains("instagram") {
            Some((REELS_TIKTOK_MAX_S, "TikTok/Reels"))
        } else {
            None
        }
    });
    let (default_max, default_src) = platform_max.unwrap_or((SHORTS_MAX_S, "default: YouTube Shorts"));
    let max = brief.max_s.unwrap_or(default_max);
    let source = if brief.max_s.is_some() || brief.min_s.is_some() {
        "brief".to_string()
    } else if platform_max.is_some() {
        format!("platform default ({default_src})")
    } else {
        format!("platform default ({default_src}; no brief)")
    };
    let min = brief.min_s;
    let over = duration > max + 0.05;
    let under = min.is_some_and(|m| duration < m - 0.05);
    let expected = match min {
        Some(m) => format!("{m:.0} s <= duration <= {max:.0} s ({source})"),
        None => format!("duration <= {max:.0} s ({source})"),
    };
    check(
        "duration_for_platform",
        if over || under { QcStatus::Warn } else { QcStatus::Pass },
        json!({ "duration_s": r2(duration), "min_s": min, "max_s": max, "source": source, "platform": brief.platform }),
        &expected,
        if over {
            let cut = (duration - max).ceil();
            Some(format!(
                "{duration:.1} s is over the {max:.0} s limit; shorten it: rustler edit trim {cid} --end-delta -{cut:.0} (or tighten the cut: rustler edit refine {cid}, rustler edit remove-pauses {cid})"
            ))
        } else if under {
            let add = (min.unwrap_or(0.0) - duration).ceil();
            Some(format!(
                "{duration:.1} s is under the {:.0} s minimum; extend it: rustler edit trim {cid} --end-delta {add:.0} (or pick a longer range: rustler edit refine {cid})",
                min.unwrap_or(0.0)
            ))
        } else {
            None
        },
    )
}

/// Truncates `text` with an ellipsis to fit `max_w` pixels at `scale`.
fn clip_text(font: &FontArc, scale: PxScale, text: &str, max_w: f32) -> String {
    use ab_glyph::{Font, ScaleFont};
    let sf = font.as_scaled(scale);
    let width = |s: &str| -> f32 { s.chars().map(|c| sf.h_advance(font.glyph_id(c))).sum() };
    if width(text) <= max_w {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cand = chars.iter().collect::<String>().trim_end().to_string() + "...";
        if width(&cand) <= max_w {
            return cand;
        }
    }
    String::new()
}

/// One column of [`compare_sheet`]: an image (a render's contact sheet) with two label lines.
pub struct CompareColumn<'a> {
    pub image: &'a Path,
    /// Headline, e.g. `A  rnd_...`.
    pub title: String,
    /// Second line, e.g. `PASS with 2 warning(s): loudness, hook_text_present`.
    pub subtitle: String,
}

/// Two labelled images side by side (two columns, each scaled to 880 px wide) written to `out`.
pub fn compare_sheet(a: &CompareColumn, b: &CompareColumn, font: &Path, out: &Path) -> rustler_core::Result<()> {
    const COL_W: u32 = 880;
    const HEADER: u32 = 78;
    const PAD: u32 = 16;
    let io = |what: &str, e: &dyn std::fmt::Display| {
        RustlerError::new(ErrorCode::Media, "qc.compare", format!("{what}: {e}"))
    };
    let font_bytes = std::fs::read(font).map_err(|e| io(&format!("cannot read font {}", font.display()), &e))?;
    let font = FontArc::try_from_vec(font_bytes).map_err(|e| io("invalid font file", &e))?;
    let load = |c: &CompareColumn| -> rustler_core::Result<RgbImage> {
        let img = image::open(c.image).map_err(|e| io(&format!("cannot read {}", c.image.display()), &e))?.to_rgb8();
        let h = (f64::from(img.height()) * f64::from(COL_W) / f64::from(img.width().max(1))).round().max(1.0) as u32;
        Ok(imageops::resize(&img, COL_W, h, imageops::FilterType::Triangle))
    };
    let (ia, ib) = (load(a)?, load(b)?);
    let body = ia.height().max(ib.height());
    let (w, h) = (PAD * 3 + COL_W * 2, HEADER + body + PAD * 2);
    let mut sheet = RgbImage::from_pixel(w, h, Rgb([16, 16, 16]));
    for (i, (col, img)) in [(a, &ia), (b, &ib)].into_iter().enumerate() {
        let x0 = PAD + i as u32 * (COL_W + PAD);
        draw_filled_rect_mut(&mut sheet, PxRect::at(x0 as i32, PAD as i32).of_size(COL_W, HEADER - 8), Rgb([0, 0, 0]));
        let title = clip_text(&font, PxScale::from(30.0), &col.title, COL_W as f32 - 16.0);
        let sub = clip_text(&font, PxScale::from(20.0), &col.subtitle, COL_W as f32 - 16.0);
        draw_text_mut(&mut sheet, Rgb([255, 255, 255]), x0 as i32 + 8, PAD as i32 + 6, PxScale::from(30.0), &font, &title);
        draw_text_mut(&mut sheet, Rgb([200, 200, 200]), x0 as i32 + 8, PAD as i32 + 44, PxScale::from(20.0), &font, &sub);
        imageops::replace(&mut sheet, img, i64::from(x0), i64::from(PAD + HEADER));
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    sheet.save(out).map_err(|e| {
        RustlerError::new(ErrorCode::Internal, "qc.compare", format!("cannot write {}: {e}", out.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_analysis_lines() {
        let mut a = Analysis::default();
        for l in [
            "[blackdetect @ 0000022] black_start:0 black_end:3.2 black_duration:3.2",
            "[freezedetect @ 000] lavfi.freezedetect.freeze_start: 1.5",
            "[freezedetect @ 000] lavfi.freezedetect.freeze_duration: 2.4",
            "[silencedetect @ 000] silence_start: 2",
            "[silencedetect @ 000] silence_end: 4.5 | silence_duration: 2.5",
            "[silencedetect @ 000] silence_start: 8.25",
            "[Parsed_ebur128_1 @ 000] Summary:",
            "",
            "  Integrated loudness:",
            "    I:         -14.2 LUFS",
            "    Threshold: -24.2 LUFS",
            "",
            "  True peak:",
            "    Peak:       -1.5 dBFS",
        ] {
            a.feed(l);
        }
        a.finish(10.0);
        assert_eq!(a.black, vec![(0.0, 3.2)]);
        assert_eq!(a.freezes, vec![(1.5, Some(2.4))]);
        assert_eq!(a.silences, vec![(2.0, 4.5), (8.25, 10.0)]);
        assert_eq!(a.integrated_lufs, Some(-14.2));
        assert_eq!(a.true_peak_dbtp, Some(-1.5));
    }

    #[test]
    fn verdict_names_failures_and_warnings() {
        let mk = |s| QcReport {
            schema: "rustler.qc@1".into(),
            clip_id: "clp_x".parse().unwrap(),
            render_id: "rnd_x".parse().unwrap(),
            passed: s != QcStatus::Fail,
            checks: vec![check("black_frames", s, json!(null), "", None)],
            contact_sheet: PathBuf::from("sheet.jpg"),
        };
        assert!(verdict(&mk(QcStatus::Fail)).starts_with("FAIL: 1 of 1 checks failed (black_frames)"));
        assert!(verdict(&mk(QcStatus::Warn)).starts_with("PASS with 1 warning(s) (black_frames)"));
        assert!(verdict(&mk(QcStatus::Pass)).starts_with("PASS: all 1 checks ok"));
    }

    fn clip_with(preset: &str, captions: bool, y: f64) -> Clip {
        serde_json::from_value(json!({
            "schema": "rustler.clip@1",
            "id": "clp_00000000000000000000000001",
            "name": "t",
            "revision": 1,
            "canvas": {"width": 1080, "height": 1920, "fps": 30.0},
            "segments": [{
                "id": "seg_00000000000000000000000001",
                "source_id": "src_00000000000000000000000001",
                "start": 0.0, "end": 10.0,
                "layout": {"preset": preset, "regions": [], "background": "black"}
            }],
            "captions": {"enabled": captions, "preset": "bold-karaoke", "script": "latin", "position_y": y, "max_words_per_line": 3},
            "audio": {"loudnorm": true, "target_lufs": -14.0, "true_peak": -2.0, "lra": 11.0},
            "renders": [],
            "created_at": "2026-11-15T00:00:00Z"
        }))
        .unwrap()
    }

    fn face(x: f64, y: f64, w: f64, h: f64) -> FaceBox {
        FaceBox { x, y, w, h, score: 0.9 }
    }

    fn samples(n: usize, f: impl Fn(usize) -> Vec<FaceBox>) -> Vec<FaceSample> {
        (0..n).map(|i| FaceSample { t: i as f64 * 0.5, faces: f(i) }).collect()
    }

    fn by_name<'a>(checks: &'a [QcCheck], name: &str) -> &'a QcCheck {
        checks.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no check {name}"))
    }

    #[test]
    fn caption_position_avoids_the_face() {
        // face 0.50..0.60: room below the band
        assert_eq!(suggest_caption_y(0.50, 0.60), Some(0.70));
        // face 0.60..0.82: below would leave the safe zone, so above it
        assert_eq!(suggest_caption_y(0.60, 0.82), Some(0.54));
        // face fills the frame: nothing clears it
        assert_eq!(suggest_caption_y(0.05, 0.95), None);
    }

    #[test]
    fn caption_over_face_warns_above_20_percent_and_suggests_a_position() {
        let clip = clip_with("blur-fit", true, 0.70);
        let hit = samples(10, |i| if i < 4 { vec![face(0.3, 0.62, 0.3, 0.2)] } else { vec![] });
        let ex = QcExtras { faces: Some(hit), ..QcExtras::default() };
        let c = face_checks(&clip, &ex);
        assert_eq!(c.len(), 1, "blur-fit has no face-framing checks");
        let cap = by_name(&c, "caption_over_face");
        assert_eq!(cap.status, QcStatus::Warn);
        assert_eq!(cap.value["suggested_position_y"], json!(0.56));
        let hint = cap.fix_hint.as_deref().unwrap();
        assert!(hint.contains("rustler clip set-captions clp_00000000000000000000000001 --position-y 0.56"), "{hint}");

        let few = samples(10, |i| if i < 2 { vec![face(0.3, 0.62, 0.3, 0.2)] } else { vec![] });
        let c = face_checks(&clip, &QcExtras { faces: Some(few), ..QcExtras::default() });
        assert_eq!(by_name(&c, "caption_over_face").status, QcStatus::Pass);
    }

    #[test]
    fn face_visible_and_cut_off() {
        let clip = clip_with("center-crop", false, 0.70);
        // no captions: only the two framing checks
        let none = samples(10, |_| vec![]);
        let c = face_checks(&clip, &QcExtras { faces: Some(none), ..QcExtras::default() });
        assert_eq!(c.len(), 2);
        assert_eq!(by_name(&c, "face_visible_ratio").status, QcStatus::Warn);
        assert!(by_name(&c, "face_visible_ratio").fix_hint.as_deref().unwrap().contains("rustler crop track-face clp_"));
        assert_eq!(by_name(&c, "face_cut_off").status, QcStatus::Pass);

        let edge = samples(10, |i| if i < 5 { vec![face(0.0, 0.3, 0.2, 0.2)] } else { vec![face(0.4, 0.3, 0.2, 0.2)] });
        let c = face_checks(&clip, &QcExtras { faces: Some(edge), ..QcExtras::default() });
        assert_eq!(by_name(&c, "face_visible_ratio").status, QcStatus::Pass);
        assert_eq!(by_name(&c, "face_cut_off").status, QcStatus::Warn);
    }

    #[test]
    fn engine_checks_degrade_to_skipped_warnings() {
        let clip = clip_with("center-crop", true, 0.70);
        let c = face_checks(&clip, &QcExtras::default());
        assert_eq!(c.len(), 3);
        for ck in &c {
            assert_eq!(ck.status, QcStatus::Warn, "{}", ck.name);
            assert_eq!(ck.value, json!({"skipped": "engine unavailable"}));
        }
        // blur-fit without captions has nothing to check
        assert!(face_checks(&clip_with("blur-fit", false, 0.7), &QcExtras::default()).is_empty());
    }

    #[test]
    fn faces_off_is_pass_with_info_not_a_warning() {
        let clip = clip_with("center-crop", true, 0.70);
        let c = face_checks(&clip, &QcExtras { faces_off: true, ..QcExtras::default() });
        assert_eq!(c.len(), 3);
        for ck in &c {
            assert_eq!(ck.status, QcStatus::Pass, "{}", ck.name);
            assert_eq!(ck.value, json!({"skipped": SKIPPED_FACES_OFF}));
            assert!(ck.fix_hint.as_deref().unwrap().contains("qc run clp_"), "{:?}", ck.fix_hint);
        }
        assert_eq!(SKIPPED_FACES_OFF, "off by default — look at the contact sheet, or run qc run --faces");
    }

    #[test]
    fn hook_text_from_overlay_or_captions() {
        let clip = clip_with("center-crop", true, 0.70);
        let none = hook_text_check(&clip, &QcExtras::default());
        assert_eq!(none.status, QcStatus::Warn);
        assert!(none.fix_hint.unwrap().contains("rustler edit trim"));
        let ov = QcExtras { overlay_starts: vec![0.0], overlay_add_text: true, ..QcExtras::default() };
        assert_eq!(hook_text_check(&clip, &ov).status, QcStatus::Pass);
        let late = QcExtras { overlay_starts: vec![5.0], overlay_add_text: true, ..QcExtras::default() };
        let c = hook_text_check(&clip, &late);
        assert_eq!(c.status, QcStatus::Warn);
        assert!(c.fix_hint.unwrap().contains("rustler overlay add-text clp_00000000000000000000000001 --text \"<hook>\" --start 0 --end 3"));
        let cap = QcExtras { first_caption_word_s: Some(0.4), ..QcExtras::default() };
        assert_eq!(hook_text_check(&clip, &cap).status, QcStatus::Pass);
    }

    #[test]
    fn duration_uses_brief_then_platform_defaults() {
        let none = QcExtras::default();
        assert_eq!(duration_check("clp_x", 55.0, &none).status, QcStatus::Pass);
        let over = duration_check("clp_x", 75.0, &none);
        assert_eq!(over.status, QcStatus::Warn);
        assert!(over.fix_hint.unwrap().contains("rustler edit trim clp_x --end-delta -15"));
        let tiktok = QcExtras {
            brief: Some(QcBrief { platform: Some("TikTok".into()), ..QcBrief::default() }),
            ..QcExtras::default()
        };
        assert_eq!(duration_check("clp_x", 75.0, &tiktok).status, QcStatus::Pass);
        assert_eq!(duration_check("clp_x", 95.0, &tiktok).status, QcStatus::Warn);
        let brief = QcExtras {
            brief: Some(QcBrief { platform: None, min_s: Some(20.0), max_s: Some(45.0) }),
            ..QcExtras::default()
        };
        assert_eq!(duration_check("clp_x", 30.0, &brief).status, QcStatus::Pass);
        assert_eq!(duration_check("clp_x", 50.0, &brief).status, QcStatus::Warn);
        let short = duration_check("clp_x", 10.0, &brief);
        assert_eq!(short.status, QcStatus::Warn);
        assert!(short.fix_hint.unwrap().contains("under the 20 s minimum"));
    }
}
