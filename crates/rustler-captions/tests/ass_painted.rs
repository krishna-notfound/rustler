//! Per-word colour + size (caption word overrides): `write_ass_painted_with`.
use rustler_captions::{CaptionWord, PRESETS, WordPaint, write_ass, write_ass_colored, write_ass_painted_with};
use rustler_core::{Canvas, CaptionTrack, ErrorCode};

fn six() -> Vec<CaptionWord> {
    ["hello", "big", "world", "this", "is", "rust"]
        .iter()
        .enumerate()
        .map(|(i, t)| CaptionWord { text: t.to_string(), out_start: i as f64 * 0.5, out_end: i as f64 * 0.5 + 0.45 })
        .collect()
}

fn track(preset: &str) -> CaptionTrack {
    CaptionTrack { preset: preset.into(), max_words_per_line: 3, ..CaptionTrack::default() }
}

#[test]
fn no_paint_and_colour_only_match_existing_writers() {
    for p in PRESETS {
        let plain = write_ass(&six(), &track(p), &Canvas::default(), "Anton").unwrap();
        let none = write_ass_painted_with(&six(), &vec![WordPaint::default(); 6], &track(p), &Canvas::default(), "Anton", &[]).unwrap();
        assert_eq!(plain, none, "{p}");
        let mut colours = vec![None; 6];
        colours[1] = Some("#FF2020".to_string());
        let coloured = write_ass_colored(&six(), &colours, &track(p), &Canvas::default(), "Anton").unwrap();
        let mut paints = vec![WordPaint::default(); 6];
        paints[1] = WordPaint { colour: Some("#FF2020".into()), scale: Some(1.0) };
        let painted = write_ass_painted_with(&six(), &paints, &track(p), &Canvas::default(), "Anton", &[]).unwrap();
        assert_eq!(coloured, painted, "{p}: scale 1 is a no-op");
    }
}

#[test]
fn keyword_gets_colour_and_size_in_every_preset() {
    let mut paints = vec![WordPaint::default(); 6];
    paints[1] = WordPaint { colour: Some("#FF2020".into()), scale: Some(1.3) };
    for p in PRESETS {
        let ass = write_ass_painted_with(&six(), &paints, &track(p), &Canvas::default(), "Anton", &[]).unwrap();
        // "big" (index 1) carries the colour (BGR 2020FF) and 130% size; other words do not.
        assert!(ass.contains(r"\fscx130\fscy130"), "{p}:\n{ass}");
        assert!(ass.contains("&H002020FF&"), "{p}:\n{ass}");
        for line in ass.lines().filter(|l| l.starts_with("Dialogue:")) {
            let text = line.splitn(10, ',').nth(9).unwrap();
            if text.contains("big") {
                let at = text.find("big").unwrap();
                assert!(text[..at].contains(r"\fscx130"), "{p}: big is sized on every layer: {text}");
            }
            if text.contains("world") && !text.contains("big") {
                assert!(!text.contains(r"\fscx130"), "{p}: {text}");
            }
        }
    }
}

#[test]
fn bad_paint_is_validation() {
    let mut paints = vec![WordPaint::default(); 6];
    paints[0].colour = Some("red".into());
    let e = write_ass_painted_with(&six(), &paints, &track("bold-yellow"), &Canvas::default(), "", &[]).unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
    paints[0] = WordPaint { colour: None, scale: Some(9.0) };
    let e = write_ass_painted_with(&six(), &paints, &track("bold-yellow"), &Canvas::default(), "", &[]).unwrap_err();
    assert_eq!(e.code, ErrorCode::Validation);
}
