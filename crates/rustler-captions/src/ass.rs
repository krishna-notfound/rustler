//! ASS (Advanced SubStation Alpha) writer and SRT export. (Task 7)

use crate::chunk::{CaptionWord, chunk_words};
use crate::presets::{ActiveStyle, PRESETS, Preset, find_preset};
use rustler_core::{Canvas, CaptionTrack, ErrorCode, RustlerError};

/// Gap (seconds) between words that always starts a new chunk.
const MAX_GAP: f64 = 0.6;
/// How long the last word of a chunk lingers after it ends.
const TAIL: f64 = 0.15;
/// Pop animation: grow in 80 ms, settle in another 80 ms (relative to event start).
const POP: &str = r"\t(0,80,\fscx108\fscy112)\t(80,160,\fscx100\fscy100)";
const HIDE: &str = r"\alpha&HFF&";
const SHOW: &str = r"\alpha&H00&";

/// ASS time `H:MM:SS.cc` (centisecond rounding).
fn ass_time(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100)
}

/// SRT time `HH:MM:SS,mmm`.
fn srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Drop override-block / escape characters and normalise whitespace.
fn clean(text: &str) -> String {
    let s: String = text.chars().filter(|c| !matches!(c, '{' | '}' | '\\')).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn style_colour(c: u32) -> String {
    format!("&H{c:08X}")
}

fn tag_colour(c: u32) -> String {
    format!("&H{c:08X}&")
}

#[allow(clippy::too_many_arguments)]
fn style_line(name: &str, p: &Preset, font: &str, size: u32, border_style: u8, outline_colour: u32, outline: f64, shadow: f64, margin_v: u32) -> String {
    format!(
        "Style: {name},{font},{size},{},{},{},{},{},0,0,0,100,100,0,0,{border_style},{outline},{shadow},2,40,40,{margin_v},1\n",
        style_colour(p.primary),
        style_colour(p.primary),
        style_colour(outline_colour),
        style_colour(p.back_colour),
        if p.bold { -1 } else { 0 },
    )
}

/// Full `.ass` file contents; `Validation` for an unknown `track.preset`.
///
/// `font_family`, when non-empty, overrides the preset's own family (use
/// [`crate::presets::list_presets`] to learn the family a preset expects).
pub fn write_ass(
    words: &[CaptionWord],
    track: &CaptionTrack,
    canvas: &Canvas,
    font_family: &str,
) -> rustler_core::Result<String> {
    let p = find_preset(&track.preset).ok_or_else(|| {
        RustlerError::new(
            ErrorCode::Validation,
            "captions",
            format!("unknown caption preset '{}'", track.preset),
        )
        .hint(format!("available presets: {}", PRESETS.join(", ")))
    })?;
    let font = if font_family.trim().is_empty() { p.font_family } else { font_family.trim() };
    let size = (f64::from(canvas.width) * p.size_factor).round() as u32;
    let margin_v = (f64::from(canvas.height) * (1.0 - track.position_y)).round().max(0.0) as u32;

    let mut out = String::new();
    out.push_str("[Script Info]\nScriptType: v4.00+\n");
    out.push_str(&format!("PlayResX: {}\nPlayResY: {}\n", canvas.width, canvas.height));
    out.push_str("WrapStyle: 0\nScaledBorderAndShadow: yes\nYCbCr Matrix: None\n\n");
    out.push_str("[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    out.push_str(&style_line("Default", p, font, size, 1, p.outline_colour, p.outline, p.shadow, margin_v));
    if p.active == ActiveStyle::Box {
        // BorderStyle 3: the "outline" is an opaque box of the OutlineColour, padded by Outline px.
        out.push_str(&style_line("Box", p, font, size, 3, p.active_colour, p.box_padding, 0.0, margin_v));
    }
    out.push_str("\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");

    let chunks = chunk_words(words, track.max_words_per_line, MAX_GAP);
    let blur = if p.blur > 0.0 { format!(r"\blur{}", p.blur) } else { String::new() };
    let reset = format!(r"{{\r{blur}}}");
    let lead = if blur.is_empty() { String::new() } else { format!("{{{blur}}}") };

    for (ci, chunk) in chunks.iter().enumerate() {
        let disp: Vec<String> = chunk
            .iter()
            .map(|w| {
                let t = clean(&w.text);
                if p.uppercase { t.to_uppercase() } else { t }
            })
            .collect();
        let next_start = chunks.get(ci + 1).and_then(|c| c.first()).map(|w| w.out_start);
        for (i, w) in chunk.iter().enumerate() {
            if disp[i].is_empty() {
                continue;
            }
            let start = w.out_start;
            let mut end = match chunk.get(i + 1) {
                Some(n) => n.out_start,
                None => {
                    let e = w.out_end + TAIL;
                    next_start.map_or(e, |ns| e.min(ns))
                }
            };
            if end <= start {
                end = start + 0.01;
            }
            let (s, e) = (ass_time(start), ass_time(end));
            // Text of the whole chunk; `f(j, word)` renders word j.
            let join = |f: &dyn Fn(usize, &str) -> String| -> String {
                disp.iter().enumerate().map(|(j, d)| f(j, d)).collect::<Vec<_>>().join(" ")
            };
            let ac = tag_colour(p.active_colour);
            let mut ev = |layer: u8, style: &str, text: String| {
                out.push_str(&format!("Dialogue: {layer},{s},{e},{style},,0,0,0,,{text}\n"));
            };
            match p.active {
                ActiveStyle::Recolor => {
                    let t = join(&|j, d| {
                        if j == i { format!(r"{{\c{ac}}}{d}{reset}") } else { d.to_string() }
                    });
                    ev(0, "Default", format!("{lead}{t}"));
                }
                ActiveStyle::Pop => {
                    // Layer 0: line with the active word hidden. Layer 1: only the active word
                    // visible (same line layout, so nothing reflows) and popping.
                    let base = join(&|j, d| {
                        if j == i { format!(r"{{{HIDE}}}{d}{{{SHOW}}}") } else { d.to_string() }
                    });
                    ev(0, "Default", format!("{lead}{base}"));
                    let top = join(&|j, d| {
                        if j == i {
                            format!(r"{{{SHOW}\c{ac}{POP}}}{d}{{{HIDE}\fscx100\fscy100}}")
                        } else {
                            d.to_string()
                        }
                    });
                    ev(1, "Default", format!("{lead}{{{HIDE}}}{top}"));
                }
                ActiveStyle::Box => {
                    // Layer 0: only the active word's box visible. Layer 1: the normal line.
                    let boxed = join(&|j, d| {
                        if j == i { format!(r"{{{SHOW}\1a&HFF&}}{d}{{{HIDE}}}") } else { d.to_string() }
                    });
                    ev(0, "Box", format!("{lead}{{{HIDE}}}{boxed}"));
                    let line = join(&|_, d| d.to_string());
                    ev(1, "Default", format!("{lead}{line}"));
                }
            }
        }
    }
    Ok(out)
}

/// SRT sidecar: chunks of up to `max_words` words, original case, one cue per chunk.
pub fn write_srt(words: &[CaptionWord], max_words: u32) -> String {
    let chunks = chunk_words(words, max_words, MAX_GAP);
    let mut out = String::new();
    let mut n = 0;
    for (ci, chunk) in chunks.iter().enumerate() {
        let text = chunk.iter().map(|w| clean(&w.text)).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let start = chunk[0].out_start;
        let mut end = chunk[chunk.len() - 1].out_end;
        if let Some(ns) = chunks.get(ci + 1).and_then(|c| c.first()).map(|w| w.out_start) {
            end = end.min(ns);
        }
        if end <= start {
            end = start + 0.05;
        }
        n += 1;
        out.push_str(&format!("{n}\n{} --> {}\n{text}\n\n", srt_time(start), srt_time(end)));
    }
    out
}
