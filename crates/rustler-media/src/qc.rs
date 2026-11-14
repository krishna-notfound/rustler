//! QC v1 checks on a finished render. (Task 11)
//!
//! One ffprobe call plus one ffmpeg analysis pass (blackdetect, freezedetect, silencedetect,
//! ebur128 with true peak) plus one frame grab and a 12-tile contact sheet. Every non-passing
//! check carries a `fix_hint` naming a concrete `rustler` command.

use crate::frames::frame;
use crate::proc::{self, argv};
use crate::tiles::{TileSpec, tiles};
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
/// `passed` is true when no check is `Fail`; warnings do not fail QC.
pub async fn run_qc(
    paths: &ToolPaths,
    clip: &Clip,
    render: &RenderRecord,
    render_path: &Path,
    font: &Path,
    out_dir: &Path,
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
    let expected_dur = TimeMap::from_segments(&clip.segments).duration();
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
}
