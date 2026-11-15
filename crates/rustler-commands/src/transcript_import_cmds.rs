//! Command: transcript.import — hand Rustler a transcript the agent already has.
//!
//! Agents can often get a transcript faster with their own tools (a web fetch of YouTube's
//! captions, a transcript API, a browser) than through yt-dlp, whose YouTube lookups can take
//! minutes. This command accepts that transcript in common formats, stores it exactly like
//! `analyze transcribe` / `source fetch-transcript` do, and builds units so `transcript get|search`,
//! `highlights candidates` and `clip create --start-unit` all work on it.

use crate::project_cmds::validation;
use crate::source_cmds::{parse_json3, pending_id};
use crate::transcript_cmds::{analysis_rel, save_transcript, units_preview};
use rustler_core::{CaptionScript, SourceId, SourceOrigin, Transcript, Word, WordTiming};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const STAGE: &str = "transcript.import";

#[derive(Deserialize, JsonSchema, Debug)]
#[serde(deny_unknown_fields)]
pub struct ImportIn {
    /// Where the transcript belongs: a source id (`src_...`) or a video URL (stored as a pending
    /// transcript, usable with `transcript get|search` before importing the video).
    pub target: String,
    /// Path of the transcript file. Give this or `text`.
    pub file: Option<std::path::PathBuf>,
    /// The transcript content inline. Give this or `file`.
    pub text: Option<String>,
    /// `auto` (default), `vtt`, `srt`, `json3` (YouTube), `words` (JSON `[{text,start,end}]`, exact
    /// timing) or `lines` (one `[mm:ss] text` or `mm:ss text` per line).
    pub format: Option<String>,
    /// Times in the file are `original` (full-video time) or `source` (already relative to this
    /// source). Default: `original` for a section imported with `--sections`, else `source`.
    pub times: Option<String>,
    /// Language code; default guessed (`hi` when Devanagari is present, else `en`).
    pub language: Option<String>,
    /// Overwrite an existing transcript for the target (default false).
    pub replace: Option<bool>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct ImportOut {
    /// Id to use with `transcript get|search` (the source id, or `url_...` for a URL).
    pub transcript_id: String,
    pub format: String,
    pub language: String,
    pub word_timing: String,
    pub word_count: usize,
    pub unit_count: usize,
    pub duration_covered: f64,
    pub units_preview: Vec<String>,
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

/// `hh:mm:ss.mmm`, `mm:ss.mmm`, `mm:ss,mmm` or plain seconds → seconds.
fn parse_clock(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let mut total = 0.0;
    for p in &parts {
        total = total * 60.0 + p.trim().parse::<f64>().ok()?;
    }
    (total.is_finite() && total >= 0.0).then_some(total)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'").replace("&quot;", "\"").replace("&nbsp;", " ")
}

/// Spreads a cue's words evenly over its time span (approximate timing).
fn spread(text: &str, start: f64, end: f64, out: &mut Vec<(String, f64, f64)>) {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || end <= start {
        return;
    }
    let step = (end - start) / words.len() as f64;
    for (k, w) in words.iter().enumerate() {
        out.push((w.to_string(), start + step * k as f64, start + step * (k + 1) as f64));
    }
}

/// WebVTT or SRT cues. YouTube auto-caption VTT carries per-word `<00:00:01.230>` stamps; those are
/// used when present, and its rolling duplicate lines are dropped.
fn parse_cues(src: &str) -> Vec<(String, f64, f64)> {
    let mut out: Vec<(String, f64, f64)> = Vec::new();
    let mut last_text = String::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if let Some((a, b)) = line.split_once("-->") {
            let start = parse_clock(a.split_whitespace().next().unwrap_or(""));
            let end = parse_clock(b.split_whitespace().next().unwrap_or(""));
            i += 1;
            let mut body: Vec<&str> = Vec::new();
            while i < lines.len() && !lines[i].trim().is_empty() {
                body.push(lines[i]);
                i += 1;
            }
            let (Some(start), Some(end)) = (start, end) else { continue };
            // Per-word stamps (YouTube auto captions): "word<00:00:01.230><c> next</c>…".
            if let Some(stamped) = body.iter().find(|l| l.contains("><c>") || l.contains("<c>")) {
                let mut t = start;
                let mut current = String::new();
                let mut rest = *stamped;
                while !rest.is_empty() {
                    if let Some(open) = rest.find('<') {
                        current.push_str(&rest[..open]);
                        let close = rest[open..].find('>').map(|c| open + c).unwrap_or(rest.len() - 1);
                        let tag = &rest[open + 1..close];
                        if let Some(ts) = parse_clock(tag).filter(|_| tag.contains(':')) {
                            let w = strip_tags(&current);
                            spread(&w, t, ts.max(t + 0.01), &mut out);
                            current.clear();
                            t = ts;
                        }
                        rest = &rest[(close + 1).min(rest.len())..];
                    } else {
                        current.push_str(rest);
                        rest = "";
                    }
                }
                spread(&strip_tags(&current), t, end.max(t + 0.01), &mut out);
                // YouTube follows each stamped cue with a short cue repeating the finished line.
                last_text = strip_tags(stamped).split_whitespace().collect::<Vec<_>>().join(" ");
                continue;
            }
            let text = strip_tags(&body.join(" ")).split_whitespace().collect::<Vec<_>>().join(" ");
            // Rolling captions repeat the previous line; keep only the new tail.
            let new_part = text.strip_prefix(&last_text).map(str::trim).unwrap_or(&text).to_string();
            if !text.is_empty() && text != last_text {
                spread(&new_part, start, end, &mut out);
                last_text = text;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// `[mm:ss] text` / `mm:ss text` / `(hh:mm:ss) text` per line; each line lasts until the next.
fn parse_lines(src: &str) -> Vec<(String, f64, f64)> {
    let mut stamped: Vec<(f64, String)> = Vec::new();
    for line in src.lines() {
        let l = line.trim().trim_start_matches(['[', '(']);
        let Some(split) = l.find(|c: char| c == ']' || c == ')' || c.is_whitespace()) else { continue };
        let (clock, text) = l.split_at(split);
        let Some(t) = parse_clock(clock).filter(|_| clock.contains(':')) else { continue };
        let text = text.trim_start_matches([']', ')', ' ', '-', '–']).trim();
        if !text.is_empty() {
            stamped.push((t, text.to_string()));
        }
    }
    let mut out = Vec::new();
    for k in 0..stamped.len() {
        let (t, text) = &stamped[k];
        let end = stamped.get(k + 1).map(|n| n.0).unwrap_or(t + 0.35 * text.split_whitespace().count() as f64 + 0.5);
        spread(text, *t, end.max(t + 0.2), &mut out);
    }
    out
}

/// JSON `[{text|word, start, end}]` (or `{words:[…]}`) with exact word times.
fn parse_words_json(src: &str) -> Option<Vec<(String, f64, f64)>> {
    let v: Value = serde_json::from_str(src).ok()?;
    let arr = v.as_array().cloned().or_else(|| v.get("words").and_then(Value::as_array).cloned())?;
    let mut out = Vec::new();
    for w in arr {
        let text = w.get("text").or_else(|| w.get("word")).and_then(Value::as_str)?.trim().to_string();
        let start = w.get("start").and_then(Value::as_f64)?;
        let end = w.get("end").and_then(Value::as_f64).unwrap_or(start + 0.3);
        if !text.is_empty() {
            out.push((text, start, end.max(start)));
        }
    }
    Some(out)
}

fn detect_format(src: &str) -> &'static str {
    let t = src.trim_start();
    if t.starts_with("WEBVTT") {
        "vtt"
    } else if t.starts_with('{') && t.contains("\"events\"") {
        "json3"
    } else if t.starts_with('[') && t.contains("\"start\"") || t.starts_with('{') && t.contains("\"words\"") {
        "words"
    } else if src.contains("-->") {
        "srt"
    } else {
        "lines"
    }
}

async fn import(ctx: Ctx, i: ImportIn) -> rustler_core::Result<ImportOut> {
    let store = ctx.store()?;
    let content = match (&i.file, &i.text) {
        (Some(p), None) => std::fs::read_to_string(p)
            .map_err(|e| validation(STAGE, format!("cannot read {}: {e}", p.display()), "pass an existing UTF-8 text file"))?,
        (None, Some(t)) => t.clone(),
        _ => return Err(validation(STAGE, "give exactly one of --file or --text", "e.g. `rustler transcript import src_x --file captions.vtt`")),
    };
    let format = match i.format.as_deref().unwrap_or("auto") {
        "auto" => detect_format(&content).to_string(),
        f @ ("vtt" | "srt" | "json3" | "words" | "lines") => f.to_string(),
        other => return Err(validation(STAGE, format!("unknown format `{other}`"), "use auto, vtt, srt, json3, words or lines")),
    };
    let mut warnings = Vec::new();
    let (raw, exact): (Vec<(String, f64, f64)>, bool) = match format.as_str() {
        "json3" => (parse_json3(&content)?.into_iter().map(|w| (w.text, w.start, w.end)).collect(), false),
        "words" => (
            parse_words_json(&content)
                .ok_or_else(|| validation(STAGE, "not a words JSON array", "expected [{\"text\":\"hi\",\"start\":0.0,\"end\":0.3}, …]"))?,
            true,
        ),
        "lines" => (parse_lines(&content), false),
        _ => (parse_cues(&content), false),
    };
    if raw.is_empty() {
        return Err(validation(STAGE, format!("no timed words found ({format})"), "the transcript needs timestamps: VTT/SRT cues, YouTube json3, words JSON, or `[mm:ss] text` lines"));
    }

    // Resolve the target and the time base.
    let target = i.target.trim();
    let (transcript_id, source_id, section_offset, section_len) = if target.starts_with("src_") {
        let sid: SourceId = target.parse().map_err(|_| validation(STAGE, format!("bad source id `{target}`"), "see `rustler source list`"))?;
        let src = store.load_source(&sid)?;
        let (off, len) = match &src.origin {
            SourceOrigin::Url { section: Some(_), original_offset, .. } => (*original_offset, Some(src.probe.duration)),
            _ => (0.0, None),
        };
        (target.to_string(), sid, off, len)
    } else if target.starts_with("http://") || target.starts_with("https://") {
        (pending_id(target), SourceId::new(), 0.0, None)
    } else {
        return Err(validation(STAGE, format!("target `{target}` is neither a source id nor a URL"), "pass `src_...` or the video URL"));
    };
    let times_original = match i.times.as_deref() {
        Some("original") => true,
        Some("source") => false,
        Some(o) => return Err(validation(STAGE, format!("unknown times `{o}`"), "use original or source")),
        None => section_len.is_some(),
    };
    let shift = if times_original { section_offset } else { 0.0 };
    let limit = section_len.map(|l| l + 0.05).unwrap_or(f64::INFINITY);
    let mut words: Vec<Word> = Vec::new();
    for (text, s, e) in raw {
        let (s, e) = (s - shift, e - shift);
        if e <= 0.0 || s >= limit {
            continue;
        }
        words.push(Word { i: words.len() as u32, text, start: s.max(0.0), end: e.min(limit), conf: None, speaker: None });
    }
    if words.is_empty() {
        return Err(validation(
            STAGE,
            "no words fall inside this source's time range",
            if times_original { "if the file is already relative to this section, pass --times source" } else { "if the file uses full-video times, pass --times original" },
        ));
    }
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    for (k, w) in words.iter_mut().enumerate() {
        w.i = k as u32;
    }

    let rel = analysis_rel(&transcript_id)?.join("transcript.json");
    if store.resolve(&rel).is_file() && !i.replace.unwrap_or(false) && !ctx.dry_run {
        return Err(validation(STAGE, format!("{transcript_id} already has a transcript"), "pass --replace true to overwrite it"));
    }
    let has_devanagari = words.iter().any(|w| w.text.chars().any(|c| ('\u{0900}'..='\u{097F}').contains(&c)));
    let language = i.language.clone().unwrap_or_else(|| if has_devanagari { "hi".into() } else { "en".into() });
    if has_devanagari {
        warnings.push("contains Devanagari; captions render it as-is (set the clip's caption script, or import a romanized transcript for Latin Hinglish captions)".into());
    }
    if !exact {
        warnings.push("word times are approximate (spread across each caption cue); for frame-exact cuts run `rustler analyze transcribe` on the imported section".into());
    }
    let duration_covered = words.last().map(|w| w.end).unwrap_or(0.0) - words.first().map(|w| w.start).unwrap_or(0.0);
    let t = Transcript {
        schema: "rustler.transcript@1".into(),
        source_id,
        language: language.clone(),
        engine: format!("agent:{format}"),
        script: if has_devanagari { CaptionScript::AsTranscribed } else { CaptionScript::Latin },
        words,
        word_timing: if exact { WordTiming::Exact } else { WordTiming::Approximate },
    };
    let word_count = t.words.len();
    if ctx.dry_run {
        return Ok(ImportOut {
            transcript_id,
            format,
            language,
            word_timing: if exact { "exact" } else { "approximate" }.into(),
            word_count,
            unit_count: 0,
            duration_covered,
            units_preview: vec![],
            warnings,
            next: vec!["dry run: nothing written".into()],
        });
    }
    let units = save_transcript(&store, &transcript_id, &t)?;
    Ok(ImportOut {
        next: vec![
            format!("rustler transcript get {transcript_id}"),
            format!("rustler highlights candidates {transcript_id}"),
        ],
        transcript_id,
        format,
        language,
        word_timing: if exact { "exact" } else { "approximate" }.into(),
        word_count,
        unit_count: units.len(),
        duration_covered,
        units_preview: units_preview(&units, 15),
        warnings,
    })
}

/// Registers `transcript.import`.
pub fn register(reg: &mut Registry) {
    reg.register(typed(
        "transcript.import",
        "Import a transcript the agent already has (VTT/SRT, YouTube json3, words JSON, or `[mm:ss] text` lines) for a source or a URL — skips yt-dlp/whisper entirely",
        &["target"],
        Annotations::default(),
        vec![
            Example { description: "Captions you fetched yourself, for an imported source", cli: "rustler transcript import src_01k... --file captions.vtt" },
            Example { description: "Before downloading anything: store it against the URL", cli: "rustler transcript import https://youtu.be/ID --file transcript.txt --format lines" },
        ],
        import,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks() {
        assert_eq!(parse_clock("00:01:02.500"), Some(62.5));
        assert_eq!(parse_clock("01:02,250"), Some(62.25));
        assert_eq!(parse_clock("1:00:00"), Some(3600.0));
        assert_eq!(parse_clock("x"), None);
    }

    #[test]
    fn srt_cues_spread_words() {
        let srt = "1\n00:00:01,000 --> 00:00:03,000\nask not what\n\n2\n00:00:03,000 --> 00:00:04,000\nyour country\n";
        let w = parse_cues(srt);
        assert_eq!(w.len(), 5);
        assert_eq!(w[0].0, "ask");
        assert!((w[0].1 - 1.0).abs() < 1e-9 && (w[2].2 - 3.0).abs() < 1e-9);
        assert_eq!(w[3].0, "your");
    }

    #[test]
    fn youtube_vtt_word_stamps_and_rolling_dupes() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nask<00:00:01.500><c> not</c><00:00:02.000><c> what</c>\n\n00:00:03.000 --> 00:00:03.010\nask not what\n\n00:00:03.010 --> 00:00:05.000\nask not what\nyour<00:00:04.000><c> country</c>\n";
        let w = parse_cues(vtt);
        let texts: Vec<&str> = w.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(texts, ["ask", "not", "what", "your", "country"]);
        assert!((w[1].1 - 1.5).abs() < 1e-9 && (w[4].1 - 4.0).abs() < 1e-9);
    }

    #[test]
    fn plain_rolling_vtt_keeps_only_new_tail() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nhello there\n\n00:00:02.000 --> 00:00:04.000\nhello there general kenobi\n";
        let texts: Vec<String> = parse_cues(vtt).into_iter().map(|x| x.0).collect();
        assert_eq!(texts, ["hello", "there", "general", "kenobi"]);
    }

    #[test]
    fn timestamped_lines() {
        let w = parse_lines("[00:05] So my fellow Americans\n00:09 ask not\n(1:02:03) late line\nno stamp here\n");
        assert_eq!(w.first().unwrap().0, "So");
        assert!((w[0].1 - 5.0).abs() < 1e-9);
        assert!(w.iter().any(|x| x.0 == "ask" && (x.1 - 9.0).abs() < 1e-9));
        assert!(w.iter().any(|x| x.0 == "late" && (x.1 - 3723.0).abs() < 1e-9));
    }

    #[test]
    fn words_json_and_detection() {
        let j = r#"[{"text":"hi","start":0.1,"end":0.3},{"word":"there","start":0.3,"end":0.6}]"#;
        assert_eq!(detect_format(j), "words");
        assert_eq!(parse_words_json(j).unwrap().len(), 2);
        assert_eq!(detect_format("WEBVTT\n"), "vtt");
        assert_eq!(detect_format("1\n00:00:01,000 --> 00:00:02,000\nx"), "srt");
        assert_eq!(detect_format("[00:01] hi"), "lines");
    }
}
