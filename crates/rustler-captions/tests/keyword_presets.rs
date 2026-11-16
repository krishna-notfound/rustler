//! `keyword-highlight` and `emoji-karaoke` presets.
use rustler_captions::{CaptionWord, WordPaint, write_ass, write_ass_painted_with};
use rustler_core::{Canvas, CaptionTrack};

fn six() -> Vec<CaptionWord> {
    ["this", "made", "me", "ten", "million", "dollars"]
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

fn text_of(line: &str) -> &str {
    line.splitn(10, ',').nth(9).unwrap()
}

#[test]
fn keyword_highlight_marked_words_get_colour_and_115_scale_others_plain() {
    let mut paints = vec![WordPaint::default(); 6];
    paints[4] = WordPaint { colour: Some("#FF2020".into()), scale: None }; // "million"
    let ass = write_ass_painted_with(&six(), &paints, &track("keyword-highlight"), &Canvas::default(), "", &[]).unwrap();
    let d = dialogues(&ass);
    // 2 chunks of 3 words -> one static event each (no per-word reveal, no active highlight).
    assert_eq!(d.len(), 2, "{ass}");
    assert!(!d[0].contains(r"\c&H") && !d[0].contains(r"\fsc"), "chunk 1 has no keyword: {}", d[0]);
    assert_eq!(text_of(d[0]), "this made me");
    let t = text_of(d[1]);
    assert!(t.contains(r"{\c&H002020FF&\fscx115\fscy115}million{\c&H00FFFFFF&\fscx100\fscy100}"), "{t}");
    assert!(t.starts_with("ten ") && t.ends_with(" dollars"), "{t}");
    // the event spans the whole chunk
    assert!(d[1].starts_with("Dialogue: 0,0:00:01.50,0:00:03.10,Default"), "{}", d[1]);
}

#[test]
fn keyword_highlight_explicit_scale_wins_and_no_overrides_is_plain() {
    let mut paints = vec![WordPaint::default(); 6];
    paints[3] = WordPaint { colour: Some("#00FF00".into()), scale: Some(1.4) };
    let ass = write_ass_painted_with(&six(), &paints, &track("keyword-highlight"), &Canvas::default(), "", &[]).unwrap();
    assert!(ass.contains(r"\fscx140\fscy140") && !ass.contains(r"\fscx115"));
    let plain = write_ass(&six(), &track("keyword-highlight"), &Canvas::default(), "").unwrap();
    for l in dialogues(&plain) {
        assert!(!text_of(l).contains('{'), "no overrides, no tags: {l}");
    }
}

#[test]
fn emoji_karaoke_keeps_the_emoji_with_its_word_and_recolours_the_active_word() {
    let mut w = six();
    w[4].text = "million 💰".into();
    let ass = write_ass(&w, &track("emoji-karaoke"), &Canvas::default(), "").unwrap();
    assert!(ass.contains("Style: Default,Poppins ExtraBold,97,"));
    let d = dialogues(&ass);
    assert_eq!(d.len(), 6, "one event per word like the other karaoke presets");
    assert!(text_of(d[4]).contains(r"{\c&H00A64DFF&}million 💰"), "{}", d[4]);
}
