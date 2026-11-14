//! Loudness curve, peaks and silences in one ffmpeg pass. (Task 6)

use crate::probe::probe;
use crate::proc::{self, argv};
use rustler_core::{AudioEvents, LoudnessPoint, Silence};
use rustler_registry::{ProgressEvent, ProgressSink};
use rustler_tools::ToolPaths;
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// Floor reported by ebur128 for digital silence.
const FLOOR_LUFS: f64 = -120.7;
const PEAK_ABOVE_MEDIAN_LU: f64 = 6.0;
const PEAK_WINDOW_S: usize = 5;

/// ebur128 momentary loudness (10 Hz) -> 1 s buckets (max M); peaks = buckets >= median + 6 LU
/// that are local maxima within +/-5 s; silences via `silencedetect=n=-35dB:d=0.6` in the same graph.
///
/// A source without an audio stream yields empty `AudioEvents`.
pub async fn analyze_audio(
    paths: &ToolPaths,
    media: &Path,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> rustler_core::Result<AudioEvents> {
    let info = probe(paths, media).await?;
    if !info.has_audio {
        return Ok(AudioEvents::default());
    }
    let total = info.duration;

    let mut args = argv!["-hide_banner", "-loglevel", "info", "-nostdin", "-nostats", "-i"];
    args.push(media.as_os_str().to_owned());
    args.extend(argv![
        "-map", "0:a:0", "-vn",
        "-af", "silencedetect=n=-35dB:d=0.6,ebur128",
        "-f", "null", "-",
    ]);

    let mut parser = Parser::default();
    let mut last_pct_bucket = -1i64;
    proc::run_streaming(&paths.ffmpeg, &args, None, cancel, |line| {
        parser.feed(line);
        if total > 0.0 {
            let bucket = parser.last_t.floor() as i64;
            if bucket != last_pct_bucket {
                last_pct_bucket = bucket;
                progress.emit(ProgressEvent {
                    stage: "analyze.audio".into(),
                    pct: Some((parser.last_t / total).clamp(0.0, 1.0) as f32),
                    message: None,
                });
            }
        }
    })
    .await
    .map_err(|mut e| {
        e.stage = "analyze_audio".into();
        e
    })?;

    Ok(parser.finish(total))
}

#[derive(Default)]
struct Parser {
    /// (t, momentary LUFS) at ~10 Hz
    samples: Vec<(f64, f64)>,
    silences: Vec<Silence>,
    open_silence: Option<f64>,
    last_t: f64,
}

fn after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.find(key).map(|i| &line[i + key.len()..])
}

fn first_f64(s: &str) -> Option<f64> {
    s.split_whitespace().next()?.parse().ok()
}

impl Parser {
    fn feed(&mut self, line: &str) {
        if line.contains("ebur128") {
            // "[Parsed_ebur128_1 @ 0x..] t: 0.099979  TARGET:-23 LUFS  M:-120.7 S:-120.7  I: -70.0 LUFS ..."
            if let (Some(t), Some(m)) = (
                after(line, "] t:").and_then(first_f64),
                after(line, " M:").and_then(first_f64),
            ) {
                self.last_t = t;
                self.samples.push((t, if m.is_finite() { m.max(FLOOR_LUFS) } else { FLOOR_LUFS }));
            }
        } else if line.contains("silencedetect") {
            if let Some(s) = after(line, "silence_start:").and_then(first_f64) {
                self.open_silence = Some(s.max(0.0));
            } else if let (Some(e), Some(s)) = (
                after(line, "silence_end:").and_then(first_f64),
                self.open_silence.take(),
            ) {
                self.silences.push(Silence { start: s, end: e.max(s) });
            }
        }
    }

    fn finish(mut self, total: f64) -> AudioEvents {
        if let Some(s) = self.open_silence.take() {
            let end = if total > s { total } else { self.last_t.max(s) };
            self.silences.push(Silence { start: s, end });
        }
        let curve = buckets(&self.samples);
        let peaks = find_peaks(&curve);
        AudioEvents { curve, peaks, silences: self.silences }
    }
}

/// 1 s buckets (bucket start time `t`), value = max momentary loudness within the bucket.
fn buckets(samples: &[(f64, f64)]) -> Vec<LoudnessPoint> {
    let Some(last) = samples.last() else { return Vec::new() };
    let n = last.0.floor() as usize + 1;
    let mut max = vec![f64::NEG_INFINITY; n];
    for &(t, m) in samples {
        let i = (t.floor().max(0.0) as usize).min(n - 1);
        if m > max[i] {
            max[i] = m;
        }
    }
    max.iter()
        .enumerate()
        .map(|(i, &m)| LoudnessPoint { t: i as f64, lufs: if m.is_finite() { m } else { FLOOR_LUFS } })
        .collect()
}

/// Buckets >= median + 6 LU that are local maxima within +/-5 buckets (first of equal plateau wins).
fn find_peaks(curve: &[LoudnessPoint]) -> Vec<LoudnessPoint> {
    if curve.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<f64> = curve.iter().map(|p| p.lufs).collect();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let threshold = median + PEAK_ABOVE_MEDIAN_LU;
    let mut peaks = Vec::new();
    for (i, p) in curve.iter().enumerate() {
        if p.lufs < threshold {
            continue;
        }
        let lo = i.saturating_sub(PEAK_WINDOW_S);
        let hi = (i + PEAK_WINDOW_S).min(curve.len() - 1);
        let is_max = (lo..i).all(|j| curve[j].lufs < p.lufs) && (i + 1..=hi).all(|j| curve[j].lufs <= p.lufs);
        if is_max {
            peaks.push(*p);
        }
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ebur128_and_silencedetect_lines() {
        let mut p = Parser::default();
        p.feed("[Parsed_ebur128_1 @ 000001] t: 0.099979    TARGET:-23 LUFS    M:-120.7 S:-120.7     I: -70.0 LUFS       LRA:   0.0 LU");
        p.feed("[Parsed_ebur128_1 @ 000001] t: 1.2    TARGET:-23 LUFS    M: -20.5 S: -30.0     I: -25.0 LUFS       LRA:   0.0 LU");
        p.feed("[silencedetect @ 0000] silence_start: 2.00012");
        p.feed("[silencedetect @ 0000] silence_end: 4.00012 | silence_duration: 2.00000");
        p.feed("[silencedetect @ 0000] silence_start: 6");
        let ev = p.finish(8.0);
        assert_eq!(ev.curve.len(), 2);
        assert_eq!(ev.curve[0].lufs, FLOOR_LUFS);
        assert_eq!(ev.curve[1].lufs, -20.5);
        assert_eq!(ev.silences, vec![Silence { start: 2.00012, end: 4.00012 }, Silence { start: 6.0, end: 8.0 }]);
    }

    #[test]
    fn peaks_need_to_clear_median_and_be_local_maxima() {
        let mut curve: Vec<LoudnessPoint> =
            (0..30).map(|i| LoudnessPoint { t: f64::from(i), lufs: -30.0 }).collect();
        curve[10].lufs = -10.0;
        curve[11].lufs = -10.0; // plateau: only the first counts
        curve[25].lufs = -27.0; // below median + 6
        let peaks = find_peaks(&curve);
        assert_eq!(peaks.len(), 1);
        assert_eq!(peaks[0].t, 10.0);
    }
}
