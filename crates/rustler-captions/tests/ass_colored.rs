//! Per-word colours (speaker-coloured captions): `write_ass_colored` and `parse_hex_colour`.
use rustler_captions::{CaptionWord, PRESETS, parse_hex_colour, write_ass, write_ass_colored};
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

fn dialogues(ass: &str) -> Vec<&str> {
    ass.lines().filter(|l| l.starts_with("Dialogue:")).collect()
}

/// S1 yellow for the first three words, S2 cyan for the last three.
fn speaker_colors() -> Vec<Option<String>> {
    ["#FFE000", "#FFE000", "#FFE000", "#5EE0FF", "#5EE0FF", "#5EE0FF"].iter().map(|c| Some(c.to_string())).collect()
}

#[test]
fn hex_colours_become_ass_bgr() {
    assert_eq!(parse_hex_colour("#FFE000"), Some(0x0000_E0FF)); // R=FF G=E0 B=00 -> 00 BB GG RR
    assert_eq!(parse_hex_colour("5ee0ff"), Some(0x00FF_E05E));
    assert_eq!(parse_hex_colour(" #000000 "), Some(0));
    for bad in ["", "#FFF", "#GGGGGG", "#FFE0001", "yellow"] {
        assert_eq!(parse_hex_colour(bad), None, "{bad}");
    }
}

#[test]
fn no_colours_matches_plain_write_ass() {
    for p in PRESETS {
        let plain = write_ass(&six(), &track(p), &Canvas::default(), "Anton").unwrap();
        let none = write_ass_colored(&six(), &vec![None; 6], &track(p), &Canvas::default(), "Anton").unwrap();
        let empty = write_ass_colored(&six(), &[], &track(p), &Canvas::default(), "Anton").unwrap();
        assert_eq!(plain, none, "{p}");
        assert_eq!(plain, empty, "{p}");
    }
}

#[test]
fn speaker_colours_appear_on_the_right_words() {
    let ass = write_ass_colored(&six(), &speaker_colors(), &track("clean-white"), &Canvas::default(), "Anton").unwrap();
    let d = dialogues(&ass);
    assert_eq!(d.len(), 6);
    // Chunk 1 (hello big world): every event paints the non-active words yellow (&H0000E0FF&).
    let first = d[0];
    assert!(first.contains(r"{\c&H0000E0FF&}big{"), "{first}");
    assert!(first.contains(r"{\c&H0000E0FF&}world{"), "{first}");
    assert!(!first.contains("&H00FFE05E&"), "chunk 1 must not use the second speaker's colour: {first}");
    // Chunk 2 (this is rust): cyan = &H00FFE05E&.
    let fourth = d[3];
    assert!(fourth.contains(r"{\c&H00FFE05E&}is{"), "{fourth}");
    assert!(fourth.contains(r"{\c&H00FFE05E&}rust{"), "{fourth}");
    assert!(!fourth.contains("&H0000E0FF&"), "{fourth}");
}

#[test]
fn colours_follow_the_global_word_index_across_chunks() {
    // Only word 4 ("is", second chunk, index 1 inside it) is coloured.
    let mut colors = vec![None; 6];
    colors[4] = Some("#7CFF6B".to_string());
    let ass = write_ass_colored(&six(), &colors, &track("clean-white"), &Canvas::default(), "Anton").unwrap();
    let d = dialogues(&ass);
    let painted: Vec<usize> = d.iter().enumerate().filter(|(_, l)| l.contains("&H006BFF7C&")).map(|(i, _)| i).collect();
    // "is" is painted in the events where it is not the active word: events of "this" (3) and "rust" (5).
    assert_eq!(painted, vec![3, 5]);
}

#[test]
fn every_preset_paints_visible_words_but_never_the_hidden_layers() {
    for p in PRESETS {
        let ass = write_ass_colored(&six(), &speaker_colors(), &track(p), &Canvas::default(), "").unwrap();
        assert!(ass.contains("&H0000E0FF&") && ass.contains("&H00FFE05E&"), "{p}");
        // hormozi-pop's layer 1 hides every word except the active one: no colour tags there
        if *p == "hormozi-pop" {
            for l in dialogues(&ass).iter().filter(|l| l.starts_with("Dialogue: 1,")) {
                assert!(!l.contains("&H0000E0FF&") && !l.contains("&H00FFE05E&"), "{l}");
            }
        }
    }
}

#[test]
fn invalid_colour_is_a_validation_error() {
    let mut colors = speaker_colors();
    colors[2] = Some("not-a-colour".to_string());
    let err = write_ass_colored(&six(), &colors, &track("bold-karaoke"), &Canvas::default(), "").unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.hint.unwrap().contains("#RRGGBB"));
}
