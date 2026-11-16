//! ASS (Advanced SubStation Alpha) writer and SRT export. (Task 7)

use crate::chunk::{CaptionWord, chunk_words};
use crate::custom::find_preset_in;
use crate::presets::{ActiveStyle, PRESETS, Preset};
use rustler_core::{Canvas, CaptionTrack, ErrorCode, RustlerError};

/// Gap (seconds) between words that always starts a new chunk.
const MAX_GAP: f64 = 0.6;
/// How long the last word of a chunk lingers after it ends.
const TAIL: f64 = 0.15;
/// Pop animation: grow in 80 ms, settle in another 80 ms (relative to event start).
/// Height only: a wider word on the pop layer would shift that layer's centred line and cover the
/// spaces next to it (trial 3: "BIGGESTMISUNDERSTANDINGABOUT").
const POP: &str = r"\t(0,80,\fscy112)\t(80,160,\fscy100)";
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
    write_ass_with(words, track, canvas, font_family, &[])
}

/// [`write_ass`] that also resolves `track.preset` against `custom` presets (see
/// [`crate::load_custom_presets`]) when it is not a built-in name.
pub fn write_ass_with(
    words: &[CaptionWord],
    track: &CaptionTrack,
    canvas: &Canvas,
    font_family: &str,
    custom: &[Preset],
) -> rustler_core::Result<String> {
    write_ass_impl(words, None, track, canvas, font_family, custom)
}

/// `#RRGGBB` (or `RRGGBB`) to an ASS `0x00BBGGRR` colour; `None` for anything else.
pub fn parse_hex_colour(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    let (r, g, b) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
    Some((b << 16) | (g << 8) | r)
}

/// Like [`write_ass`], but `colors[i]` (`#RRGGBB`, parallel to `words`) overrides the text colour
/// of word `i` (speaker-coloured captions). `None` or a missing entry keeps the preset's colour.
/// The preset's active-word highlight is unchanged. `Validation` for an invalid colour.
pub fn write_ass_colored(
    words: &[CaptionWord],
    colors: &[Option<String>],
    track: &CaptionTrack,
    canvas: &Canvas,
    font_family: &str,
) -> rustler_core::Result<String> {
    write_ass_colored_with(words, colors, track, canvas, font_family, &[])
}

/// [`write_ass_colored`] that also resolves custom presets (like [`write_ass_with`]).
pub fn write_ass_colored_with(
    words: &[CaptionWord],
    colors: &[Option<String>],
    track: &CaptionTrack,
    canvas: &Canvas,
    font_family: &str,
    custom: &[Preset],
) -> rustler_core::Result<String> {
    let parsed = colors
        .iter()
        .map(|c| match c {
            None => Ok(None),
            Some(s) => parse_hex_colour(s).map(Some).ok_or_else(|| {
                RustlerError::new(ErrorCode::Validation, "captions", format!("invalid word colour '{s}'"))
                    .hint("use #RRGGBB, e.g. #FFE000")
            }),
        })
        .collect::<rustler_core::Result<Vec<Option<u32>>>>()?;
    let paints: Vec<Paint> = parsed.into_iter().map(|colour| Paint { colour, scale: None }).collect();
    write_ass_impl(words, Some(&paints), track, canvas, font_family, custom)
}

/// Per-word look for [`write_ass_painted_with`] (keyword captions): text colour `#RRGGBB` and a
/// size factor (1 = preset size).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WordPaint {
    pub colour: Option<String>,
    pub scale: Option<f64>,
}

/// Like [`write_ass_colored_with`], with a per-word size factor as well (`paints[i]` parallel to
/// `words`). The active-word highlight keeps the word's size. `Validation` for an invalid colour
/// or a scale outside 0.2..4.
pub fn write_ass_painted_with(
    words: &[CaptionWord],
    paints: &[WordPaint],
    track: &CaptionTrack,
    canvas: &Canvas,
    font_family: &str,
    custom: &[Preset],
) -> rustler_core::Result<String> {
    let parsed = paints
        .iter()
        .map(|p| {
            let colour = match &p.colour {
                None => None,
                Some(s) => Some(parse_hex_colour(s).ok_or_else(|| {
                    RustlerError::new(ErrorCode::Validation, "captions", format!("invalid word colour '{s}'"))
                        .hint("use #RRGGBB, e.g. #FFE000")
                })?),
            };
            let scale = match p.scale {
                Some(f) if !(f.is_finite() && (0.2..=4.0).contains(&f)) => {
                    return Err(RustlerError::new(ErrorCode::Validation, "captions", format!("invalid word scale {f}")));
                }
                Some(f) if (f - 1.0).abs() < 1e-6 => None,
                other => other,
            };
            Ok(Paint { colour, scale })
        })
        .collect::<rustler_core::Result<Vec<Paint>>>()?;
    write_ass_impl(words, Some(&parsed), track, canvas, font_family, custom)
}

/// Parsed per-word look: ASS colour and size factor.
#[derive(Clone, Copy, Debug, Default)]
struct Paint {
    colour: Option<u32>,
    scale: Option<f64>,
}

/// `\fscx..\fscy..` for a size factor (percent).
fn fsc(f: f64) -> String {
    let p = (f * 100.0).round() as i64;
    format!(r"\fscx{p}\fscy{p}")
}

fn write_ass_impl(
    words: &[CaptionWord],
    colors: Option<&[Paint]>,
    track: &CaptionTrack,
    canvas: &Canvas,
    font_family: &str,
    custom: &[Preset],
) -> rustler_core::Result<String> {
    let p = find_preset_in(&track.preset, custom).ok_or_else(|| {
        let mut names: Vec<&str> = PRESETS.to_vec();
        names.extend(custom.iter().map(|c| c.name));
        RustlerError::new(
            ErrorCode::Validation,
            "captions",
            format!("unknown caption preset '{}'", track.preset),
        )
        .hint(format!("available presets: {}", names.join(", ")))
    })?;
    let font = if font_family.trim().is_empty() { p.font_family } else { font_family.trim() };
    let size = (f64::from(canvas.width) * p.size_factor).round() as u32;
    let margin_v = (f64::from(canvas.height) * (1.0 - track.position_y)).round().max(0.0) as u32;

    // Keyword presets: a word with its own colour and no explicit size gets the preset's keyword size.
    let keyword_paints: Option<Vec<Paint>> = (p.keyword_scale > 0.0).then(|| {
        colors
            .unwrap_or_default()
            .iter()
            .map(|c| if c.colour.is_some() && c.scale.is_none() { Paint { scale: Some(p.keyword_scale), ..*c } } else { *c })
            .collect()
    });
    let colors: Option<&[Paint]> = keyword_paints.as_deref().or(colors);

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

    let mut base_index = 0usize;
    let back_to_primary = tag_colour(p.primary);
    for (ci, chunk) in chunks.iter().enumerate() {
        let chunk_base = base_index;
        base_index += chunk.len();
        let paint_of = |j: usize| -> Paint { colors.and_then(|c| c.get(chunk_base + j)).copied().unwrap_or_default() };
        // Word j at its own size only (hidden layers keep the same line layout).
        let sized = |j: usize, d: &str| -> String {
            match paint_of(j).scale {
                Some(f) => format!(r"{{{}}}{d}{{\fscx100\fscy100}}", fsc(f)),
                None => d.to_string(),
            }
        };
        // A visible non-active word, painted with its per-word colour and size when set.
        let paint = |j: usize, d: &str| -> String {
            let p = paint_of(j);
            let mut open = String::new();
            let mut close = String::new();
            if let Some(c) = p.colour {
                open.push_str(&format!(r"\c{}", tag_colour(c)));
                close.push_str(&format!(r"\c{back_to_primary}"));
            }
            if let Some(f) = p.scale {
                open.push_str(&fsc(f));
                close.push_str(r"\fscx100\fscy100");
            }
            if open.is_empty() { d.to_string() } else { format!("{{{open}}}{d}{{{close}}}") }
        };
        // Extra tags for the active word: its size.
        let active_size = |j: usize| -> String { paint_of(j).scale.map(fsc).unwrap_or_default() };
        let disp: Vec<String> = chunk
            .iter()
            .map(|w| {
                let t = clean(&w.text);
                if p.uppercase { t.to_uppercase() } else { t }
            })
            .collect();
        let next_start = chunks.get(ci + 1).and_then(|c| c.first()).map(|w| w.out_start);
        if p.active == ActiveStyle::Static {
            // One event per chunk: the whole line at once, only keyword words painted.
            let (Some(first), Some(last)) = (chunk.first(), chunk.last()) else { continue };
            let start = first.out_start;
            let tail = last.out_end + TAIL;
            let end = next_start.map_or(tail, |ns| tail.min(ns)).max(start + 0.01);
            let line = disp
                .iter()
                .enumerate()
                .filter(|(_, d)| !d.is_empty())
                .map(|(j, d)| paint(j, d))
                .collect::<Vec<_>>()
                .join(" ");
            if !line.is_empty() {
                out.push_str(&format!("Dialogue: 0,{},{},Default,,0,0,0,,{lead}{line}
", ass_time(start), ass_time(end)));
            }
            continue;
        }
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
                ActiveStyle::Static => unreachable!("static captions are written per chunk above"),
                ActiveStyle::Recolor => {
                    let t = join(&|j, d| {
                        if j == i { format!(r"{{\c{ac}{}}}{d}{reset}", active_size(j)) } else { paint(j, d) }
                    });
                    ev(0, "Default", format!("{lead}{t}"));
                }
                ActiveStyle::Pop => {
                    // Layer 0: line with the active word hidden. Layer 1: only the active word
                    // visible (same line layout, so nothing reflows) and popping.
                    let base = join(&|j, d| {
                        if j == i {
                            match paint_of(j).scale {
                                Some(f) => format!(r"{{{HIDE}{}}}{d}{{{SHOW}\fscx100\fscy100}}", fsc(f)),
                                None => format!(r"{{{HIDE}}}{d}{{{SHOW}}}"),
                            }
                        } else {
                            paint(j, d)
                        }
                    });
                    ev(0, "Default", format!("{lead}{base}"));
                    let top = join(&|j, d| {
                        if j == i {
                            let pop = match paint_of(j).scale {
                                None => POP.to_string(),
                                Some(f) => {
                                    let p = |v: f64| (v * f).round() as i64;
                                    // Width stays at the word's own size (same layout as layer 0).
                                    format!(r"{}\t(0,80,\fscy{})\t(80,160,\fscy{})", fsc(f), p(112.0), p(100.0))
                                }
                            };
                            format!(r"{{{SHOW}\c{ac}{pop}}}{d}{{{HIDE}\fscx100\fscy100}}")
                        } else {
                            sized(j, d)
                        }
                    });
                    ev(1, "Default", format!("{lead}{{{HIDE}}}{top}"));
                }
                ActiveStyle::Box => {
                    // Layer 0: only the active word's box visible. Layer 1: the normal line.
                    let boxed = join(&|j, d| {
                        if j == i {
                            match paint_of(j).scale {
                                Some(f) => format!(r"{{{SHOW}\1a&HFF&{}}}{d}{{{HIDE}\fscx100\fscy100}}", fsc(f)),
                                None => format!(r"{{{SHOW}\1a&HFF&}}{d}{{{HIDE}}}"),
                            }
                        } else {
                            sized(j, d)
                        }
                    });
                    ev(0, "Box", format!("{lead}{{{HIDE}}}{boxed}"));
                    let line = join(&|j, d| paint(j, d));
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
