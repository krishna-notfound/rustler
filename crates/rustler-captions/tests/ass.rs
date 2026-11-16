use rustler_captions::presets::{PRESET_TABLE, ActiveStyle};
use rustler_captions::{CaptionWord, PRESETS, chunk_words, list_presets, write_ass, write_srt};
use rustler_core::{Canvas, CaptionTrack, ErrorCode};

fn w(text: &str, s: f64, e: f64) -> CaptionWord {
    CaptionWord { text: text.into(), out_start: s, out_end: e }
}

/// 6 contiguous words, 0.5 s each.
fn six() -> Vec<CaptionWord> {
    ["hello", "big", "world", "this", "is", "rust"]
        .iter()
        .enumerate()
        .map(|(i, t)| w(t, i as f64 * 0.5, i as f64 * 0.5 + 0.45))
        .collect()
}

fn track(preset: &str) -> CaptionTrack {
    CaptionTrack { preset: preset.into(), ..CaptionTrack::default() }
}

fn dialogues(ass: &str) -> Vec<&str> {
    ass.lines().filter(|l| l.starts_with("Dialogue:")).collect()
}

#[test]
fn header_values() {
    let ass = write_ass(&six(), &track("bold-karaoke"), &Canvas::default(), "Anton").unwrap();
    assert!(ass.contains("PlayResX: 1080"));
    assert!(ass.contains("PlayResY: 1920"));
    assert!(ass.contains("Style: Default,Anton,97,"));
    // MarginV = round(1920 * 0.30) = 576, alignment 2
    assert!(ass.contains(",2,40,40,576,1"));
}

#[test]
fn one_event_per_word() {
    let ass = write_ass(&six(), &track("bold-karaoke"), &Canvas::default(), "Anton").unwrap();
    let d = dialogues(&ass);
    assert_eq!(d.len(), 6);
    assert!(d[1].contains(r"HELLO {\c&H0000E0FF&}BIG{\r} WORLD"), "{}", d[1]);
    // event 1 runs from word 1 start to word 2 start
    assert!(d[1].starts_with("Dialogue: 0,0:00:00.50,0:00:01.00,Default,"), "{}", d[1]);
    // last word of the last chunk: out_end + 0.15
    assert!(d[5].contains("0:00:02.50,0:00:03.10"), "{}", d[5]);
}

#[test]
fn gap_splits_chunk() {
    let words = vec![w("a", 0.0, 0.3), w("b", 1.3, 1.6), w("c", 1.7, 2.0)];
    let chunks = chunk_words(&words, 3, 0.6);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 1);
    assert_eq!(chunks[1].len(), 2);
}

#[test]
fn sentence_end_splits_chunk() {
    let words = vec![w("Hi.", 0.0, 0.3), w("there", 0.35, 0.6), w("you", 0.65, 0.9)];
    let chunks = chunk_words(&words, 5, 0.6);
    assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), vec![1, 2]);
}

/// YouTube caption speaker-change markers (`>>`) are timed words in the transcript; they must
/// not be burned in as captions.
#[test]
fn speaker_markers_are_not_captioned() {
    let words = vec![w(">>", 0.0, 0.6), w("Yeah.", 0.1, 0.6), w(">>", 0.5, 0.6), w("And", 0.7, 0.9), w("-", 0.9, 1.0), w("then", 1.0, 1.2)];
    let chunks = chunk_words(&words, 3, 0.6);
    let texts: Vec<Vec<&str>> = chunks.iter().map(|c| c.iter().map(|w| w.text.as_str()).collect()).collect();
    assert_eq!(texts, vec![vec!["Yeah."], vec!["And", "then"]]);
    let ass = write_ass(&words, &track("bold-karaoke"), &Canvas::default(), "Anton").unwrap();
    assert!(!ass.contains(">>"), "{ass}");
}

#[test]
fn time_format() {
    let words = vec![w("x", 3725.456, 3725.9)];
    let ass = write_ass(&words, &track("bold-karaoke"), &Canvas::default(), "Anton").unwrap();
    assert!(ass.contains("1:02:05.46"), "{ass}");
}

#[test]
fn unknown_preset_validation() {
    let err = write_ass(&six(), &track("nope"), &Canvas::default(), "Anton").unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    assert!(err.hint.unwrap().contains("bold-karaoke"));
}

#[test]
fn escaping_strips_override_chars() {
    let words = vec![w(r"a{\b1}b", 0.0, 0.5)];
    let ass = write_ass(&words, &track("bold-karaoke"), &Canvas::default(), "Anton").unwrap();
    let d = dialogues(&ass);
    assert!(d[0].ends_with(r"{\c&H0000E0FF&}AB1B{\r}") || d[0].contains("AB1B"), "{}", d[0]);
    assert!(!d[0].contains(r"\b1"));
}

#[test]
fn caption_position_follows_canvas_and_position_y() {
    let canvas = Canvas { width: 720, height: 1280, fps: 30.0 };
    let t = CaptionTrack { position_y: 0.5, ..CaptionTrack::default() };
    let ass = write_ass(&six(), &t, &canvas, "Anton").unwrap();
    assert!(ass.contains("PlayResX: 720"));
    assert!(ass.contains("Style: Default,Anton,65,")); // round(720*0.09)
    assert!(ass.contains(",2,40,40,640,1")); // round(1280*0.5)
}

#[test]
fn presets_table_matches_const_and_list() {
    assert_eq!(PRESET_TABLE.iter().map(|p| p.name).collect::<Vec<_>>(), PRESETS.to_vec());
    let l = list_presets();
    assert_eq!(l.len(), 4);
    assert!(l.iter().all(|p| !p.description.is_empty() && !p.font_family.is_empty()));
    for name in PRESETS {
        write_ass(&six(), &track(name), &Canvas::default(), "").unwrap();
    }
}

#[test]
fn clean_white_recolors_only_active_word() {
    let ass = write_ass(&six(), &track("clean-white"), &Canvas::default(), "").unwrap();
    assert!(ass.contains("Style: Default,Poppins ExtraBold,97,"));
    let d = dialogues(&ass);
    assert_eq!(d.len(), 6);
    assert!(d[0].contains(r"{\c&H0000D7FF&}hello{\r\blur1.5}"), "{}", d[0]);
}

#[test]
fn boxed_highlight_has_box_layer_behind_text() {
    let ass = write_ass(&six(), &track("boxed-highlight"), &Canvas::default(), "").unwrap();
    assert!(ass.contains("Style: Box,Poppins ExtraBold,"));
    let d = dialogues(&ass);
    assert_eq!(d.len(), 12);
    assert!(d[0].starts_with("Dialogue: 0,") && d[0].contains(",Box,"), "{}", d[0]);
    assert!(d[1].starts_with("Dialogue: 1,") && d[1].contains(",Default,"), "{}", d[1]);
    // in the box layer only the active word has a visible box
    assert!(d[0].contains(r"{\alpha&H00&\1a&HFF&}hello"), "{}", d[0]);
}

#[test]
fn hormozi_pop_pops_only_active_word_on_own_layer() {
    let ass = write_ass(&six(), &track("hormozi-pop"), &Canvas::default(), "").unwrap();
    let d = dialogues(&ass);
    assert_eq!(d.len(), 12);
    assert!(d[0].starts_with("Dialogue: 0,"), "{}", d[0]);
    assert!(!d[0].contains(r"\t("), "base layer must not animate");
    assert!(d[1].starts_with("Dialogue: 1,"), "{}", d[1]);
    assert_eq!(d[1].matches(r"\t(").count(), 2);
    assert!(d[1].contains(r"\t(0,80,\fscx108\fscy112)\t(80,160,\fscx100\fscy100)"));
    assert!(d[1].contains(r"\c&H005EFF2B&"));
    assert!(d[1].contains("HELLO"));
    assert_eq!(PRESET_TABLE.iter().filter(|p| p.active == ActiveStyle::Pop).count(), 1);
}

#[test]
fn srt_export() {
    let srt = write_srt(&six(), 3);
    assert!(srt.starts_with("1\n00:00:00,000 --> 00:00:01,450\nhello big world\n\n"), "{srt}");
    assert!(srt.contains("2\n00:00:01,500 --> 00:00:02,950\nthis is rust\n"), "{srt}");
}

/// Visual check with the real ffmpeg + libass. Run:
/// `cargo test -p rustler-captions --test ass -- --ignored --nocapture`
/// Output PNGs are written to `<temp>/rustler-captions-visual/`.
#[test]
#[ignore = "needs ffmpeg in ~/.rustler/bin and network for fonts (cached after first run)"]
fn render_preset_frames_with_real_ffmpeg() {
    use std::path::PathBuf;
    use std::process::Command;

    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap();
    let rhome = std::env::var("RUSTLER_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(home).join(".rustler"));
    let ffmpeg = ["ffmpeg.exe", "ffmpeg"].iter().map(|n| rhome.join("bin").join(n)).find(|p| p.exists());
    let Some(ffmpeg) = ffmpeg else {
        eprintln!("SKIP: no ffmpeg in {}", rhome.join("bin").display());
        return;
    };
    let dir = std::env::temp_dir().join("rustler-captions-visual");
    let fonts = dir.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (dest, url) in [
        (fonts.join("Anton-Regular.ttf"), "https://github.com/google/fonts/raw/main/ofl/anton/Anton-Regular.ttf"),
        (fonts.join("Poppins-ExtraBold.ttf"), "https://github.com/google/fonts/raw/main/ofl/poppins/Poppins-ExtraBold.ttf"),
    ] {
        if !dest.exists() || std::fs::metadata(&dest).unwrap().len() < 10_000 {
            let st = Command::new("curl").args(["-L", "-sS", "-f", "-o"]).arg(&dest).arg(url).status().expect("curl");
            assert!(st.success(), "font download failed: {url}");
        }
    }
    let _ = &src;

    let words: Vec<CaptionWord> = [("Stop", 0.0, 0.4), ("scrolling", 0.5, 1.0), ("right", 1.1, 1.4), ("now", 1.5, 1.9)]
        .iter()
        .map(|&(t, s, e)| w(t, s, e))
        .collect();
    for name in PRESETS {
        let ass = write_ass(&words, &track(name), &Canvas::default(), "").unwrap();
        let ass_name = format!("{name}.ass");
        std::fs::write(dir.join(&ass_name), ass).unwrap();
        // t=0.55 (active word 2, just after the pop starts), t=0.85 (settled), t=1.6 (word 4 of next chunk)
        for (tag, t) in [("pop", 0.58), ("settled", 0.85)] {
            let png = format!("{name}-{tag}.png");
            let st = Command::new(&ffmpeg)
                .current_dir(&dir)
                .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=gray:s=1080x1920:d=2:r=30", "-vf"])
                .arg(format!("ass={ass_name}:fontsdir=fonts"))
                .args(["-ss", &t.to_string(), "-frames:v", "1", &png])
                .status()
                .unwrap();
            assert!(st.success(), "ffmpeg failed for {name}");
            assert!(std::fs::metadata(dir.join(&png)).unwrap().len() > 1000);
        }
    }
    eprintln!("frames written to {}", dir.display());
}
