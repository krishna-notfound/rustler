//! Filtergraph compiler: Clip + sources -> one ffmpeg invocation. (Task 9)
//!
//! The graph itself is a pure function of the inputs ([`build_graph`]); [`compile_with`] also
//! stages the side files the graph names by bare relative path (`graph.txt`, `captions.ass`,
//! `fonts/`, `crop_*.cmd`) into `workdir`, because ffmpeg runs with cwd = workdir (no Windows
//! drive-colon escaping inside the filter script; autoclip trick, MIT).
//!
//! Render-graph discipline adapted from bridgeclip `layout_renderer.py` (MIT): frames are allocated
//! on the cumulative output timeline so rounding never accumulates at cuts, timestamps are rebuilt
//! after concat/loudnorm, and NaNs from loudnorm on digital silence are scrubbed before AAC.
//! Music ducking from opensource-clipping `audio_bgm.py` (MIT) with `amix normalize=0` and fades.

use crate::encoders::EncoderChoice;
use rustler_core::{
    Background, Canvas, Clip, ErrorCode, Fit, MediaProbe, Rect, Region, RustlerError, SegmentId,
    SourceId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const STAGE: &str = "render.compile";
const SR: u64 = 48_000;
/// Segment end may exceed the probed duration by this much (container rounding).
const END_TOLERANCE: f64 = 0.05;
const AUDIO_FORMAT: &str = "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo";
const FINITE_AUDIO: &str = "aeval='if(isnan(val(ch))+isinf(val(ch)),0,val(ch))':c=same";
pub const GRAPH_FILE: &str = "graph.txt";
pub const PREVIEW_GRAPH_FILE: &str = "graph_preview.txt";
pub const ASS_FILE: &str = "captions.ass";
pub const FONTS_DIR: &str = "fonts";

#[derive(Clone, Debug, PartialEq)]
pub struct SourceMedia {
    pub id: SourceId,
    pub path: PathBuf,
    pub probe: MediaProbe,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledRender {
    /// Full ffmpeg argv (without the program).
    pub args: Vec<OsString>,
    /// Contents of `workdir/graph.txt`.
    pub filter_script: String,
    pub workdir: PathBuf,
}

/// Background music under the whole clip.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct MusicBed {
    pub path: PathBuf,
    /// Gain applied to the music before mixing (default -18 dB).
    pub volume_db: f64,
    /// Sidechain-duck the music under speech (default true).
    pub duck: bool,
    /// Seconds (default 0.5).
    pub fade_in: f64,
    /// Seconds (default 1.0).
    pub fade_out: f64,
}

impl MusicBed {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            volume_db: -18.0,
            duck: true,
            fade_in: 0.5,
            fade_out: 1.0,
        }
    }
}

/// One crop-path keyframe: at segment-relative time `t` (seconds from the segment start) the
/// crop's left edge is `x` (normalized 0..1 of the source display width).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
pub struct CropKey {
    pub t: f64,
    pub x: f64,
}

/// Data carried alongside the frozen Clip model (5-hour scope additions).
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug, Default)]
pub struct CompileExtras {
    pub music: Option<MusicBed>,
    /// Crop path per segment; it drives the crop `x` of that segment's first region
    /// (`layout.regions[0]`). Width/height stay fixed (sendcmd changes only `x`).
    pub crop_paths: HashMap<SegmentId, Vec<CropKey>>,
}

pub fn compile(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    ass_file: Option<&Path>,
    fonts_dir: &Path,
    enc: &EncoderChoice,
    out: &Path,
    workdir: &Path,
) -> rustler_core::Result<CompiledRender> {
    compile_with(
        clip,
        sources,
        ass_file,
        fonts_dir,
        enc,
        out,
        workdir,
        &CompileExtras::default(),
    )
}

/// Output duration of `clip` (sum of segment durations).
pub fn output_duration(clip: &Clip) -> f64 {
    clip.segments.iter().map(|s| s.end - s.start).sum()
}

#[allow(clippy::too_many_arguments)]
pub fn compile_with(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    ass_file: Option<&Path>,
    fonts_dir: &Path,
    enc: &EncoderChoice,
    out: &Path,
    workdir: &Path,
    extras: &CompileExtras,
) -> rustler_core::Result<CompiledRender> {
    let plan = build_graph(clip, sources, ass_file.is_some(), extras)?;

    std::fs::create_dir_all(workdir).map_err(|e| io_err("create workdir", workdir, e))?;
    write_file(&workdir.join(GRAPH_FILE), &plan.graph)?;
    write_file(
        &workdir.join(PREVIEW_GRAPH_FILE),
        &format!("{};\n[aout]anullsink", plan.graph),
    )?;
    for (name, body) in &plan.cmd_files {
        write_file(&workdir.join(name), body)?;
    }
    if let Some(ass) = ass_file {
        stage_ass(ass, workdir)?;
        stage_fonts(fonts_dir, workdir)?;
    }

    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-y", "-loglevel", "error"]
        .iter()
        .map(OsString::from)
        .collect();
    for seg in &clip.segments {
        let src = &sources[&seg.source_id];
        let end = seg.end.min(src.probe.duration.max(seg.start + 0.001));
        args.push("-ss".into());
        args.push(secs(seg.start).into());
        args.push("-to".into());
        args.push(secs(end).into());
        args.push("-i".into());
        args.push(abs(&src.path).into_os_string());
    }
    if let Some(m) = &extras.music {
        args.extend(["-stream_loop", "-1", "-i"].iter().map(OsString::from));
        args.push(abs(&m.path).into_os_string());
    }
    args.extend(
        [
            "-/filter_complex",
            GRAPH_FILE,
            "-map",
            "[vout]",
            "-map",
            "[aout]",
        ]
        .iter()
        .map(OsString::from),
    );
    args.extend(enc.args.iter().map(OsString::from));
    args.extend(
        ["-c:a", "aac", "-b:a", "160k", "-ar", "48000", "-r"]
            .iter()
            .map(OsString::from),
    );
    args.push(fps_str(clip.canvas.fps).into());
    args.extend(["-movflags", "+faststart"].iter().map(OsString::from));
    args.push(abs(out).into_os_string());

    Ok(CompiledRender {
        args,
        filter_script: plan.graph,
        workdir: workdir.to_path_buf(),
    })
}

/// The pure part of the compiler: the filter script plus the sendcmd files it references.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphPlan {
    pub graph: String,
    /// (bare file name in workdir, contents)
    pub cmd_files: Vec<(String, String)>,
}

pub fn build_graph(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    captions: bool,
    extras: &CompileExtras,
) -> rustler_core::Result<GraphPlan> {
    validate(clip, sources, extras)?;
    let canvas = clip.canvas;
    let cw = even(canvas.width as f64);
    let ch = even(canvas.height as f64);
    let fps = fps_str(canvas.fps);
    let mut parts: Vec<String> = Vec::new();
    let mut cmd_files = Vec::new();

    let mut cum = 0.0f64;
    let mut total_samples = 0u64;
    for (i, seg) in clip.segments.iter().enumerate() {
        let src = &sources[&seg.source_id];
        let d = seg.end - seg.start;
        let (f0, f1) = (frame_at(cum, canvas.fps), frame_at(cum + d, canvas.fps));
        let (a0, a1) = (sample_at(cum), sample_at(cum + d));
        cum += d;
        let frames = (f1 - f0).max(1);
        let samples = a1 - a0;
        total_samples += samples;

        let crop_keys = extras
            .crop_paths
            .get(&seg.id)
            .filter(|k| !k.is_empty() && !seg.layout.regions.is_empty());
        segment_video(
            &mut parts,
            &mut cmd_files,
            i,
            seg.layout.regions.as_slice(),
            seg.layout.background,
            &src.probe,
            &canvas,
            (cw, ch),
            &fps,
            frames,
            crop_keys.map(|v| v.as_slice()),
        );

        if src.probe.has_audio {
            parts.push(format!(
                "[{i}:a:0]aresample=48000:async=1:first_pts=0,{AUDIO_FORMAT},apad,\
                 asetpts=N/SR/TB,atrim=end_sample={samples},asetpts=N/SR/TB[a{i}]"
            ));
        } else {
            parts.push(format!(
                "anullsrc=r=48000:cl=stereo,{AUDIO_FORMAT},atrim=end_sample={samples},\
                 asetpts=N/SR/TB[a{i}]"
            ));
        }
    }

    let n = clip.segments.len();
    let cat_in: String = (0..n).map(|i| format!("[v{i}][a{i}]")).collect();
    parts.push(format!("{cat_in}concat=n={n}:v=1:a=1[vcat][acat]"));

    // Concat runs on a microsecond time base; rebuild exact frame timestamps.
    let mut vpost = format!("[vcat]settb={},setpts=N", tb_str(canvas.fps));
    if captions {
        let _ = write!(vpost, ",ass={ASS_FILE}:fontsdir={FONTS_DIR}");
    }
    vpost.push_str(",format=yuv420p[vout]");
    parts.push(vpost);

    let mut apost = String::new();
    if clip.audio.loudnorm {
        let _ = write!(
            apost,
            "loudnorm=I={}:TP={}:LRA={},aresample=48000,{FINITE_AUDIO},",
            num(clip.audio.target_lufs),
            num(clip.audio.true_peak),
            num(clip.audio.lra)
        );
    }
    // loudnorm's buffered flush can leave a PTS jump; rebuild the sample clock after it.
    let _ = write!(apost, "{AUDIO_FORMAT},asetpts=N/SR/TB[aout]");

    match &extras.music {
        None => parts.push(format!("[acat]{apost}")),
        Some(m) => {
            let k = n;
            let total = total_samples as f64 / SR as f64;
            let fi = m.fade_in.clamp(0.0, total / 2.0);
            let fo = m.fade_out.clamp(0.0, total / 2.0);
            let mut mus = format!(
                "[{k}:a:0]aresample=48000,{AUDIO_FORMAT},volume={}dB,\
                 atrim=end_sample={total_samples},asetpts=N/SR/TB",
                num(m.volume_db)
            );
            if fi > 0.0 {
                let _ = write!(mus, ",afade=t=in:st=0:d={}", num(fi));
            }
            if fo > 0.0 {
                let _ = write!(mus, ",afade=t=out:st={}:d={}", num(total - fo), num(fo));
            }
            mus.push_str("[mus]");
            parts.push(mus);
            let mix = "amix=inputs=2:duration=first:dropout_transition=0:normalize=0";
            if m.duck {
                parts.push("[acat]asplit=2[speech][sc]".to_string());
                parts.push(
                    "[mus][sc]sidechaincompress=threshold=0.08:ratio=5:attack=100:release=1000[duck]"
                        .to_string(),
                );
                parts.push(format!("[speech][duck]{mix},{apost}"));
            } else {
                parts.push(format!("[acat][mus]{mix},{apost}"));
            }
        }
    }

    Ok(GraphPlan {
        graph: parts.join(";\n"),
        cmd_files,
    })
}

struct Px {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Source rect (normalized) -> even pixel crop inside `sw`x`sh`.
fn src_px(r: &Rect, sw: u32, sh: u32) -> Px {
    let (fw, fh) = (even(sw as f64).max(2), even(sh as f64).max(2));
    let w = even(r.w * sw as f64).clamp(2, fw);
    let h = even(r.h * sh as f64).clamp(2, fh);
    let x = even(r.x * sw as f64).min(even((fw - w) as f64));
    let y = even(r.y * sh as f64).min(even((fh - h) as f64));
    Px { x, y, w, h }
}

fn dst_px(r: &Rect, cw: u32, ch: u32) -> Px {
    let w = even(r.w * cw as f64).clamp(2, cw);
    let h = even(r.h * ch as f64).clamp(2, ch);
    Px {
        x: even(r.x * cw as f64).min(cw - 2),
        y: even(r.y * ch as f64).min(ch - 2),
        w,
        h,
    }
}

#[allow(clippy::too_many_arguments)]
fn segment_video(
    parts: &mut Vec<String>,
    cmd_files: &mut Vec<(String, String)>,
    i: usize,
    regions: &[Region],
    background: Background,
    probe: &MediaProbe,
    canvas: &Canvas,
    (cw, ch): (u32, u32),
    fps: &str,
    frames: u64,
    crop_keys: Option<&[CropKey]>,
) {
    // Regions drawn back to front; the crop path follows regions[0] wherever it sorts.
    let mut order: Vec<usize> = (0..regions.len()).collect();
    order.sort_by_key(|&j| regions[j].z);
    let full = |r: &Region| {
        let d = dst_px(&r.dst, cw, ch);
        r.fit == Fit::Cover && d.x == 0 && d.y == 0 && d.w == cw && d.h == ch
    };
    // A full-canvas Cover region at the back hides any background: skip compositing it.
    let base_is_region = order.first().is_some_and(|&j| full(&regions[j]));
    let blur = background == Background::BlurFill && !base_is_region;
    let uses = order.len() + usize::from(blur);

    let prep = format!(
        "[{i}:v:0]fps={fps}:start_time=0:round=near,tpad=stop_mode=clone:stop_duration=1,\
         trim=end_frame={frames},setpts=PTS-STARTPTS"
    );
    let inputs: Vec<String> = (0..uses.max(1)).map(|k| format!("[s{i}in{k}]")).collect();
    if uses <= 1 {
        parts.push(format!("{prep}{}", inputs[0]));
    } else {
        parts.push(format!("{prep},split={uses}{}", inputs.concat()));
    }
    let mut next_in = inputs.iter();

    let mut base: Option<String> = None;
    if !base_is_region {
        if blur {
            let (qw, qh) = (even(cw as f64 / 4.0).max(2), even(ch as f64 / 4.0).max(2));
            parts.push(format!(
                "{}scale={qw}:{qh}:force_original_aspect_ratio=increase,crop={qw}:{qh},\
                 gblur=sigma=10,lutyuv=y=val*0.6,scale={cw}:{ch},setsar=1[s{i}bg]",
                next_in.next().unwrap()
            ));
        } else {
            if order.is_empty() {
                // No regions at all: consume the (unused) input so the graph has no dangling pad.
                parts.push(format!("{}nullsink", next_in.next().unwrap()));
            }
            parts.push(format!(
                "color=c=black:s={cw}x{ch}:r={fps},trim=end_frame={frames},setsar=1[s{i}bg]"
            ));
        }
        base = Some(format!("[s{i}bg]"));
    }

    for (pos, &j) in order.iter().enumerate() {
        let r = &regions[j];
        let s = src_px(&r.src, probe.width, probe.height);
        let d = dst_px(&r.dst, cw, ch);
        let label = next_in.next().unwrap();
        let mut chain = String::new();
        match crop_keys.filter(|_| j == 0) {
            Some(keys) => {
                // Named crop driven by sendcmd: only x changes, w/h stay fixed for the shot.
                let name = format!("crop_s{i}r{j}.cmd");
                let target = format!("crop@s{i}r{j}");
                let (x0, body) = sendcmd_body(keys, &target, probe.width, s.w, canvas.fps, frames);
                cmd_files.push((name.clone(), body));
                let _ = write!(
                    chain,
                    "sendcmd=f={name},{target}={}:{}:{x0}:{}",
                    s.w, s.h, s.y
                );
            }
            None => {
                let _ = write!(chain, "crop={}:{}:{}:{}", s.w, s.h, s.x, s.y);
            }
        }
        match r.fit {
            Fit::Cover => {
                let _ = write!(
                    chain,
                    ",scale={}:{}:force_original_aspect_ratio=increase,crop={}:{},setsar=1",
                    d.w, d.h, d.w, d.h
                );
            }
            Fit::Contain => {
                let _ = write!(
                    chain,
                    ",scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1",
                    d.w, d.h
                );
            }
        }
        match &base {
            None => {
                // Full-canvas Cover region becomes the base.
                parts.push(format!("{label}{chain}[s{i}c{pos}]"));
            }
            Some(b) => {
                parts.push(format!("{label}{chain}[s{i}r{pos}]"));
                let (ox, oy) = match r.fit {
                    Fit::Cover => (d.x.to_string(), d.y.to_string()),
                    Fit::Contain => (
                        format!("{}+({}-overlay_w)/2", d.x, d.w),
                        format!("{}+({}-overlay_h)/2", d.y, d.h),
                    ),
                };
                parts.push(format!("{b}[s{i}r{pos}]overlay=x={ox}:y={oy}[s{i}c{pos}]"));
            }
        }
        base = Some(format!("[s{i}c{pos}]"));
    }
    let last = base.expect("segment has a base");
    parts.push(format!("{last}setsar=1,format=yuv420p[v{i}]"));
}

/// Interpolates the keys at every output frame and writes a sendcmd line only when the even
/// pixel x changes. Times sit half a frame before the frame they apply to, so 4-decimal rounding
/// can never push a change onto the following frame. Returns (initial x, file body).
fn sendcmd_body(
    keys: &[CropKey],
    target: &str,
    src_w: u32,
    crop_w: u32,
    fps: f64,
    frames: u64,
) -> (u32, String) {
    let mut keys: Vec<CropKey> = keys
        .iter()
        .copied()
        .filter(|k| k.t.is_finite() && k.x.is_finite())
        .collect();
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    let max_x = even((even(src_w as f64).saturating_sub(crop_w)) as f64);
    let at = |t: f64| -> u32 {
        let x = if keys.is_empty() {
            0.0
        } else if t <= keys[0].t {
            keys[0].x
        } else if t >= keys[keys.len() - 1].t {
            keys[keys.len() - 1].x
        } else {
            let k = keys.partition_point(|k| k.t <= t);
            let (a, b) = (keys[k - 1], keys[k]);
            let span = b.t - a.t;
            if span <= 0.0 {
                b.x
            } else {
                a.x + (b.x - a.x) * (t - a.t) / span
            }
        };
        even((x * src_w as f64).max(0.0)).min(max_x)
    };
    let x0 = at(0.0);
    let mut body = String::new();
    let mut prev = x0;
    for f in 1..frames {
        let x = at(f as f64 / fps);
        if x != prev {
            let t = (f as f64 - 0.5) / fps;
            let _ = writeln!(body, "{t:.4} {target} x {x};");
            prev = x;
        }
    }
    (x0, body)
}

fn validate(
    clip: &Clip,
    sources: &HashMap<SourceId, SourceMedia>,
    extras: &CompileExtras,
) -> rustler_core::Result<()> {
    let v = |m: String| RustlerError::new(ErrorCode::Validation, STAGE, m);
    if clip.segments.is_empty() {
        return Err(
            v("clip has no segments".into()).hint("add one with `rustler clip add-segment`")
        );
    }
    let c = clip.canvas;
    if c.width < 2 || c.height < 2 || !(c.fps.is_finite() && c.fps > 0.0) {
        return Err(v(format!(
            "invalid canvas {}x{} @ {} fps",
            c.width, c.height, c.fps
        )));
    }
    for seg in &clip.segments {
        let src = sources.get(&seg.source_id).ok_or_else(|| {
            v(format!(
                "segment {} references unknown source {}",
                seg.id, seg.source_id
            ))
        })?;
        if !src.probe.has_video || src.probe.width < 2 || src.probe.height < 2 {
            return Err(v(format!("source {} has no video", seg.source_id)));
        }
        if !(seg.start.is_finite() && seg.end.is_finite()) || seg.end <= seg.start {
            return Err(v(format!(
                "segment {}: end ({}) must be after start ({})",
                seg.id, seg.end, seg.start
            )));
        }
        if seg.start < 0.0 || seg.end > src.probe.duration + END_TOLERANCE {
            return Err(v(format!(
                "segment {} [{:.3}, {:.3}] is outside source {} duration {:.3}",
                seg.id, seg.start, seg.end, seg.source_id, src.probe.duration
            )));
        }
        for r in &seg.layout.regions {
            for (name, rect) in [("src", &r.src), ("dst", &r.dst)] {
                let ok = [rect.x, rect.y, rect.w, rect.h]
                    .iter()
                    .all(|x| x.is_finite())
                    && rect.w > 0.0
                    && rect.h > 0.0;
                if !ok {
                    return Err(v(format!(
                        "segment {} region {}: {name} rect must have positive w/h",
                        seg.id, r.id
                    )));
                }
            }
        }
    }
    if let Some(m) = &extras.music
        && !(m.volume_db.is_finite() && m.fade_in >= 0.0 && m.fade_out >= 0.0)
    {
        return Err(v("music: volume_db must be finite and fades >= 0".into()));
    }
    Ok(())
}

fn stage_ass(ass: &Path, workdir: &Path) -> rustler_core::Result<()> {
    let dest = workdir.join(ASS_FILE);
    if same_file(ass, &dest) {
        return Ok(());
    }
    std::fs::copy(ass, &dest).map_err(|e| io_err("copy captions", ass, e))?;
    Ok(())
}

fn stage_fonts(fonts_dir: &Path, workdir: &Path) -> rustler_core::Result<()> {
    let dest = workdir.join(FONTS_DIR);
    if same_file(fonts_dir, &dest) {
        return Ok(());
    }
    std::fs::create_dir_all(&dest).map_err(|e| io_err("create fonts dir", &dest, e))?;
    let Ok(rd) = std::fs::read_dir(fonts_dir) else {
        return Ok(()); // no fonts: libass falls back to system fonts
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() {
            let to = dest.join(entry.file_name());
            if !to.exists() {
                std::fs::copy(&p, &to).map_err(|e| io_err("copy font", &p, e))?;
            }
        }
    }
    Ok(())
}

/// ffmpeg runs with cwd = workdir, so every argv path must be absolute.
fn abs(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn write_file(p: &Path, body: &str) -> rustler_core::Result<()> {
    std::fs::write(p, body).map_err(|e| io_err("write", p, e))
}

fn io_err(what: &str, p: &Path, e: std::io::Error) -> RustlerError {
    RustlerError::new(
        ErrorCode::Internal,
        STAGE,
        format!("{what} {}: {e}", p.display()),
    )
}

/// Rounds down to an even integer.
fn even(v: f64) -> u32 {
    let v = if v.is_finite() && v > 0.0 {
        v.floor() as u64
    } else {
        0
    };
    (v - v % 2) as u32
}

fn frame_at(t: f64, fps: f64) -> u64 {
    (t * fps).round() as u64
}

fn sample_at(t: f64) -> u64 {
    (t * SR as f64).round() as u64
}

/// Seconds for argv: millisecond precision.
fn secs(t: f64) -> String {
    format!("{t:.3}")
}

/// Compact number: up to 3 decimals, trailing zeros trimmed (-14.0 -> "-14").
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

const NTSC: &[(u32, u32)] = &[(24000, 1001), (30000, 1001), (60000, 1001)];

/// Frame rate as an ffmpeg rational string ("30", "30000/1001", "25.5").
pub fn fps_str(fps: f64) -> String {
    for &(n, d) in NTSC {
        if (fps - n as f64 / d as f64).abs() < 0.005 {
            return format!("{n}/{d}");
        }
    }
    if (fps - fps.round()).abs() < 1e-6 {
        format!("{}", fps.round() as u64)
    } else {
        num(fps)
    }
}

fn tb_str(fps: f64) -> String {
    let f = fps_str(fps);
    match f.split_once('/') {
        Some((n, d)) => format!("{d}/{n}"),
        None => format!("1/{f}"),
    }
}
