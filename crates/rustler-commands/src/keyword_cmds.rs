//! Command: caption.keywords. Marks the keywords of a clip for the `keyword-highlight` caption
//! preset: the agent names the words, or `auto` proposes them (emphasis words in English and
//! Hinglish, numbers, names in Title Case). A keyword is a caption word override with a colour
//! (`caption.word --color`), so `caption.words_list` shows it and `caption.word` /
//! `caption.words_clear` change it. Also home of the emoji check the render uses.

use crate::audio_cmds::change;
use crate::clip_cmds::{load_transcript, log_event};
use crate::layer_cmds::{check_revision, norm_word};
use rustler_captions::CaptionWord;
use rustler_core::{ClipId, ErrorCode, ProjectStore, RustlerError, SourceId, TimeMap};
use rustler_media::layers::{Anchor, CAPTION_WORDS_KEY, CaptionWordOverride};
use rustler_registry::{Annotations, Ctx, Example, Registry, typed};
use rustler_tools::ToolPaths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

type Result<T> = rustler_core::Result<T>;

/// Colour of a keyword when the caller gives none and the clip's preset has no karaoke colour to avoid.
const DEFAULT_COLOR: &str = "#2BFF5E";
/// The alternative when the preset's active-word colour is close to [`DEFAULT_COLOR`].
const ALT_COLOR: &str = "#FFD60A";
/// `auto` marks about one word in this many.
const WORDS_PER_KEYWORD: usize = 6;

fn validation(stage: &str, msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Validation, stage, msg)
}

// ---------------------------------------------------------------------------------------------
// Word lists and the proposal rules (pure)
// ---------------------------------------------------------------------------------------------

/// English emphasis words (lower case).
pub const EMPHASIS_EN: &[&str] = &[
    "never", "always", "secret", "secrets", "free", "money", "million", "millions", "billion", "billions", "thousand",
    "crazy", "insane", "best", "worst", "biggest", "only", "stop", "wrong", "truth", "fail", "failed", "win", "won",
    "hack", "mistake", "mistakes", "huge", "nobody", "everyone", "everything", "important", "shocking", "proof",
    "guaranteed", "zero", "fast", "easy", "impossible", "dollars", "rich", "broke", "fired", "quit", "lie", "lied",
    "illegal", "dangerous", "unbelievable", "incredible", "forever", "instantly", "percent", "overnight",
];

/// Hinglish (Roman) and a few Devanagari emphasis words.
pub const EMPHASIS_HINGLISH: &[&str] = &[
    "kabhi", "nahi", "nahin", "bilkul", "sabse", "zabardast", "jhooth", "sach", "raaz", "galti", "dhamaka", "pagal",
    "ekdum", "badhiya", "paisa", "paise", "lakh", "lakhs", "crore", "karod", "mast", "kamaal", "khatarnak", "dhoka",
    "fayda", "nuksan", "asli", "jaldi", "turant", "zaroor", "free", "kamyabi", "amir", "garib",
    "कभी", "नहीं", "सबसे", "पैसा", "पैसे", "लाख", "करोड़", "सच", "झूठ", "राज़", "गलती", "ज़बरदस्त", "बिल्कुल",
];

/// English number words (lower case). "one" is left out on purpose: it is mostly a pronoun or article.
pub const NUMBER_WORDS_EN: &[&str] = &[
    "zero", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen",
    "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty", "thirty", "forty", "fifty",
    "sixty", "seventy", "eighty", "ninety", "hundred", "hundreds", "dozen",
];

/// Hindi number words, Roman and Devanagari. Words that are also common English or Hindi words
/// ("do", "ek", "teen", "एक", "दो") are left out.
pub const NUMBER_WORDS_HINDI: &[&str] = &[
    "chaar", "paanch", "panch", "chhe", "chhah", "saat", "aath", "nau", "das", "gyarah", "barah", "bees", "tees",
    "chalis", "pachas", "sattar", "assi", "nabbe", "sau", "hazaar", "hazar",
    "तीन", "चार", "पाँच", "पांच", "छह", "छः", "सात", "आठ", "नौ", "दस", "ग्यारह", "बारह", "बीस", "तीस", "चालीस", "पचास",
    "साठ", "सत्तर", "अस्सी", "नब्बे", "सौ", "हज़ार", "हजार",
];

/// Capitalised words that are not names (sentence openers, pronouns, weekdays are not listed).
const NOT_NAMES: &[&str] = &[
    "i", "i'm", "i'll", "i've", "i'd", "the", "a", "an", "and", "but", "so", "or", "if", "when", "what", "why", "how",
    "who", "this", "that", "these", "those", "it", "its", "it's", "we", "you", "he", "she", "they", "my", "your", "our",
    "yes", "no", "ok", "okay", "well", "now", "then", "there", "here", "just", "like", "also", "because", "mr", "mrs", "ms",
    "you're", "you've", "you'll", "you'd", "we're", "we've", "we'll", "they're", "they've", "they'll", "he's", "she's",
    "that's", "there's", "what's", "let's", "here's", "who's", "don't", "can't", "won't", "isn't", "wasn't", "didn't",
    "definitely", "actually", "basically", "honestly", "obviously", "maybe", "perhaps", "sometimes", "usually",
    "exactly", "absolutely", "certainly", "again", "still", "even", "every", "some", "many", "most", "all", "any",
    "one", "for", "with", "from", "not", "do", "does", "did", "are", "is", "was", "were", "have", "has", "had", "can",
    "could", "would", "should", "will", "let", "see", "look", "think", "imagine", "remember", "listen", "thank", "thanks",
];

/// Why a word was marked.
#[derive(Serialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Named by the agent.
    Agent,
    /// A number or a figure ("30", "3.5").
    Number,
    /// In the emphasis word list.
    Emphasis,
    /// A Title Case word inside a sentence.
    Name,
}

impl Reason {
    fn priority(self) -> u8 {
        match self {
            Self::Agent => 4,
            Self::Number => 3,
            Self::Emphasis => 2,
            Self::Name => 1,
        }
    }
}

/// One clip word as the proposer sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipWord {
    pub source_id: SourceId,
    pub word_i: u32,
    pub text: String,
    /// Edit-timeline start.
    pub at: f64,
}

/// True when the (trimmed) word ends a sentence, or is a bare marker with no letters or digits at all
/// (a speaker-turn ">>" or "-" starts a new sentence).
fn ends_sentence(t: &str) -> bool {
    let t = t.trim();
    if !t.chars().any(char::is_alphanumeric) {
        return true;
    }
    t.trim_end_matches(|c: char| matches!(c, '"' | '\'' | ')' | ']')).ends_with(['.', '?', '!', '…', '।'])
}

/// What the whole clip says about how its words are spelled: used so that a capital letter at a
/// sentence start is not taken for a name.
#[derive(Default, Clone, Debug)]
pub struct Spelling {
    /// Normalised words that also appear with a lower-case first letter.
    lower: HashSet<String>,
    /// Normalised words that appear with a capital first letter at least twice.
    repeated_caps: HashSet<String>,
}

impl Spelling {
    /// Counts the clip's words.
    pub fn of<'a>(texts: impl IntoIterator<Item = &'a str>) -> Self {
        let mut caps: std::collections::HashMap<String, u32> = Default::default();
        let mut out = Self::default();
        for t in texts {
            let n = norm_word(t);
            let core = t.trim().trim_matches(|c: char| !c.is_alphanumeric());
            match core.chars().next() {
                Some(c) if c.is_uppercase() => *caps.entry(n).or_default() += 1,
                Some(c) if c.is_lowercase() => {
                    out.lower.insert(n);
                }
                _ => {}
            }
        }
        out.repeated_caps = caps.into_iter().filter(|(_, c)| *c >= 2).map(|(n, _)| n).collect();
        out
    }
}

/// Classifies one word; `prev` is the word before it in the clip. Without the clip's spelling, see
/// [`classify_in`].
pub fn classify(word: &str, prev: Option<&str>) -> Option<Reason> {
    classify_in(word, prev, &Spelling::default())
}

/// [`classify`] with the clip's spelling. A name is a Title Case word (not ALL CAPS, not a common
/// opener) that is never written in lower case elsewhere in the clip and that is either inside a
/// sentence (not the first word of the clip or of a sentence) or capitalised at least twice.
pub fn classify_in(word: &str, prev: Option<&str>, spelling: &Spelling) -> Option<Reason> {
    let n = norm_word(word);
    if n.is_empty() {
        return None;
    }
    if n.chars().any(|c| c.is_ascii_digit()) || NUMBER_WORDS_EN.contains(&n.as_str()) || NUMBER_WORDS_HINDI.contains(&n.as_str()) {
        return Some(Reason::Number);
    }
    if EMPHASIS_EN.contains(&n.as_str()) || EMPHASIS_HINGLISH.contains(&n.as_str()) {
        return Some(Reason::Emphasis);
    }
    let core = word.trim().trim_matches(|c: char| !c.is_alphanumeric());
    let mut chars = core.chars();
    let (Some(first), true) = (chars.next(), core.chars().count() >= 3) else { return None };
    let rest_has_upper = chars.clone().any(char::is_uppercase);
    if !first.is_uppercase() || rest_has_upper || NOT_NAMES.contains(&n.as_str()) || spelling.lower.contains(&n) {
        return None;
    }
    let mid_sentence = prev.is_some_and(|p| !ends_sentence(p));
    (mid_sentence || spelling.repeated_caps.contains(&n)).then_some(Reason::Name)
}

/// The auto proposal: numbers first, then emphasis words, then names; at most `max` words, none
/// next to another marked word. Returned in clip order as `(index, reason)`.
pub fn propose(words: &[ClipWord], max: usize) -> Vec<(usize, Reason)> {
    let spelling = Spelling::of(words.iter().map(|w| w.text.as_str()));
    let mut cands: Vec<(usize, Reason)> = words
        .iter()
        .enumerate()
        .filter_map(|(k, w)| {
            classify_in(&w.text, k.checked_sub(1).map(|p| words[p].text.as_str()), &spelling).map(|r| (k, r))
        })
        .collect();
    cands.sort_by(|a, b| b.1.priority().cmp(&a.1.priority()).then(a.0.cmp(&b.0)));
    let mut chosen: Vec<(usize, Reason)> = Vec::new();
    for (k, r) in cands {
        if chosen.len() >= max {
            break;
        }
        if chosen.iter().any(|(c, _)| c.abs_diff(k) <= 1) {
            continue;
        }
        chosen.push((k, r));
    }
    chosen.sort_by_key(|(k, _)| *k);
    chosen
}

/// Default keyword colour when the caller gives none: green, or yellow when the preset's
/// active-word (karaoke) colour is green, so a keyword never looks like the spoken-word highlight.
/// `active` is the preset's `0x00BBGGRR` active-word colour; `None` when the preset has none.
pub fn default_keyword_colour(active: Option<u32>) -> String {
    let Some(active) = active else { return DEFAULT_COLOR.to_string() };
    let ch = |c: u32, sh: u32| ((c >> sh) & 0xFF) as i32;
    // DEFAULT_COLOR #2BFF5E in 0x00BBGGRR order.
    let green = 0x005E_FF2B_u32;
    let d: i32 = [0, 8, 16].iter().map(|&sh| (ch(active, sh) - ch(green, sh)).pow(2)).sum();
    if d < 140 * 140 { ALT_COLOR.to_string() } else { DEFAULT_COLOR.to_string() }
}

/// Words the agent named: `a, b` and repeated entries both work; matched ignoring case and punctuation.
pub fn named_words(raw: &[String]) -> Vec<String> {
    raw.iter()
        .flat_map(|s| s.split(','))
        .map(norm_word)
        .filter(|s| !s.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Clip words
// ---------------------------------------------------------------------------------------------

/// Transcript words that fall inside the clip, in edit-timeline order.
fn clip_words(store: &ProjectStore, tm: &TimeMap) -> Result<Vec<ClipWord>> {
    let mut out = Vec::new();
    let mut seen: Vec<SourceId> = Vec::new();
    for s in tm.segments() {
        if seen.contains(&s.source_id) {
            continue;
        }
        seen.push(s.source_id.clone());
    }
    for sid in &seen {
        let Some(t) = load_transcript(store, sid)? else { continue };
        for w in &t.words {
            let mid = (w.start + w.end) / 2.0;
            for s in tm.segments().iter().filter(|s| &s.source_id == sid && mid >= s.src_start && mid < s.src_end) {
                if w.text.trim().is_empty() || w.text.trim().starts_with('[') {
                    continue;
                }
                out.push(ClipWord {
                    source_id: sid.clone(),
                    word_i: w.i,
                    text: w.text.trim().to_string(),
                    at: s.out_start + (w.start.max(s.src_start) - s.src_start),
                });
            }
        }
    }
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// caption.keywords
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
pub struct KeywordsIn {
    pub clip_id: ClipId,
    /// Words to mark (comma list ok; case and punctuation ignored); every occurrence in the clip.
    pub words: Option<Vec<String>>,
    /// Propose keywords by the rules: numbers, emphasis words (English + Hinglish), names in Title Case inside a sentence (never a sentence opener), number words.
    /// Review with `--dry-run` first; change single words with `caption word`.
    pub auto: Option<bool>,
    /// With `auto`: at most this many words (default: one per about 6 words of the clip).
    pub max: Option<u32>,
    /// Keyword colour `#RRGGBB` (default: green #2BFF5E, or yellow #FFD60A when the caption preset's active-word colour is green; keyword-highlight keeps its own accent).
    pub color: Option<String>,
    pub base_revision: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct KeywordMark {
    pub source_id: SourceId,
    pub word_i: u32,
    pub text: String,
    /// Edit-timeline start of the word.
    pub at: f64,
    pub reason: Reason,
    /// The colour the word has now.
    pub color: String,
    /// The word already had a colour (set earlier with `caption.word`); it is left alone.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub kept_existing_color: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct KeywordsOut {
    pub clip_id: ClipId,
    pub marked: Vec<KeywordMark>,
    /// Caption word overrides of the clip now (keywords and other per-word styles).
    pub total_overrides: usize,
    /// The clip's caption preset.
    pub preset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub next: Vec<String>,
}

async fn caption_keywords(ctx: Ctx, i: KeywordsIn) -> Result<KeywordsOut> {
    const STAGE: &str = "caption.keywords";
    let store = ctx.store()?;
    let clip = store.load_clip(&i.clip_id)?;
    check_revision(STAGE, &clip, i.base_revision)?;
    let named = named_words(i.words.as_deref().unwrap_or(&[]));
    let auto = i.auto.unwrap_or(false);
    if named.is_empty() && !auto {
        return Err(validation(STAGE, "give words (the keywords) or auto")
            .hint("e.g. --words million,quit  or  --auto --dry-run to see what the rules propose"));
    }
    let color = match &i.color {
        Some(c) => {
            rustler_captions::parse_hex_colour(c).ok_or_else(|| validation(STAGE, format!("invalid colour '{c}'")).hint("use #RRGGBB, e.g. #FFE000"))?;
            c.clone()
        }
        None => {
            let paths = ToolPaths::from_home(&ctx.home);
            let custom = crate::asset_cmds::custom_presets(&paths.home, Some(&store.root));
            match rustler_captions::find_preset_in(&clip.captions.preset, &custom) {
                // `static` presets (keyword-highlight) have no active-word colour: the preset accent is the default.
                Some(p) if p.active == rustler_captions::presets::ActiveStyle::Static => {
                    rustler_captions::custom::hex_colour(p.active_colour & 0x00FF_FFFF)
                }
                Some(p) => default_keyword_colour(Some(p.active_colour & 0x00FF_FFFF)),
                None => default_keyword_colour(None),
            }
        }
    };
    let tm = TimeMap::from_segments(&clip.segments);
    let words = clip_words(&store, &tm)?;
    let mut warnings = Vec::new();
    if words.is_empty() {
        return Err(RustlerError::new(ErrorCode::NotFound, STAGE, format!("clip {} has no transcript words", clip.id))
            .hint("transcribe the source first: `rustler transcript run <source_id>`"));
    }

    let mut picks: Vec<(usize, Reason)> = Vec::new();
    for n in &named {
        let hits: Vec<usize> = (0..words.len()).filter(|k| &norm_word(&words[*k].text) == n).collect();
        if hits.is_empty() {
            warnings.push(format!("'{n}' is not in the clip's words"));
        }
        picks.extend(hits.into_iter().map(|k| (k, Reason::Agent)));
    }
    if auto {
        let max = i.max.map_or_else(|| (words.len() / WORDS_PER_KEYWORD).max(1), |m| m as usize);
        let proposed = propose(&words, max);
        if proposed.is_empty() {
            warnings.push("auto found no numbers, emphasis words or names; name the words yourself with --words".into());
        }
        for (k, r) in proposed {
            if !picks.iter().any(|(p, _)| *p == k) {
                picks.push((k, r));
            }
        }
    }
    picks.sort_by_key(|(k, _)| *k);

    let anchors: Vec<Anchor> =
        picks.iter().map(|(k, _)| Anchor { source_id: words[*k].source_id.clone(), word_i: words[*k].word_i }).collect();
    let (existing, total) = change(&ctx, &store, &clip.id, CAPTION_WORDS_KEY, Vec::is_empty, |list: &mut Vec<CaptionWordOverride>| {
        let mut kept = Vec::new();
        for a in &anchors {
            match list.iter_mut().find(|o| &o.word_anchor == a) {
                Some(o) if o.color.is_some() => kept.push(true),
                Some(o) => {
                    o.color = Some(color.clone());
                    kept.push(false);
                }
                None => {
                    list.push(CaptionWordOverride { word_anchor: a.clone(), color: Some(color.clone()), scale: None, emoji: None, hide: false });
                    kept.push(false);
                }
            }
        }
        Ok((kept, list.len()))
    })?;
    if !ctx.dry_run {
        log_event(&ctx, &store, STAGE, serde_json::json!({"clip_id": clip.id, "keywords": picks.len(), "auto": auto}))?;
    }
    let marked: Vec<KeywordMark> = picks
        .iter()
        .zip(&existing)
        .map(|((k, r), kept)| KeywordMark {
            source_id: words[*k].source_id.clone(),
            word_i: words[*k].word_i,
            text: words[*k].text.clone(),
            at: words[*k].at,
            reason: *r,
            color: color.clone(),
            kept_existing_color: *kept,
        })
        .collect();
    let mut next = Vec::new();
    if clip.captions.preset != "keyword-highlight" {
        warnings.push(format!(
            "the clip's caption style is '{}': keywords only get their colour; the 1.15x size and plain look come with keyword-highlight",
            clip.captions.preset
        ));
        next.push(format!("rustler caption set {} --style keyword-highlight", clip.id));
    }
    if !clip.captions.enabled {
        warnings.push(format!("captions are off for this clip; turn them on with `rustler caption set {} --enabled true`", clip.id));
    }
    if let Some(m) = marked.first() {
        next.push(format!("rustler render preview-frame {} --out-t {:.2}", clip.id, m.at + 0.1));
    }
    next.push(format!("rustler caption words-list {}", clip.id));
    Ok(KeywordsOut {
        clip_id: clip.id.clone(),
        marked,
        total_overrides: total,
        preset: clip.captions.preset.clone(),
        dry_run: ctx.dry_run.then_some(true),
        warnings,
        next,
    })
}

// ---------------------------------------------------------------------------------------------
// Emoji at render time
// ---------------------------------------------------------------------------------------------

/// Drops the emoji from caption words (with one warning) when no emoji font is installed, so the
/// captions never show missing-glyph boxes. Called by the render path after word overrides.
pub(crate) fn drop_emoji_without_font(paths: &ToolPaths, store: &ProjectStore, words: &mut [CaptionWord], warnings: &mut Vec<String>) {
    if !words.iter().any(|w| rustler_captions::emoji::contains_emoji(&w.text)) {
        return;
    }
    let mut dirs = vec![paths.fonts.clone()];
    dirs.extend(crate::asset_cmds::asset_font_dirs(&paths.home, Some(&store.root)));
    drop_emoji(&dirs, words, warnings);
}

/// [`drop_emoji_without_font`] with the font folders given.
fn drop_emoji(dirs: &[std::path::PathBuf], words: &mut [CaptionWord], warnings: &mut Vec<String>) {
    if !words.iter().any(|w| rustler_captions::emoji::contains_emoji(&w.text)) {
        return;
    }
    if rustler_captions::emoji::emoji_font_available(dirs) {
        return;
    }
    let mut n = 0;
    for w in words.iter_mut().filter(|w| rustler_captions::emoji::contains_emoji(&w.text)) {
        w.text = rustler_captions::emoji::strip_emoji(&w.text);
        n += 1;
    }
    warnings.push(format!(
        "{n} caption word(s) had an emoji; no emoji font is installed (Segoe UI Emoji on Windows, or put Noto Color Emoji into ~/.rustler/fonts), so the emoji were left out"
    ));
}

// ---------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------

/// Registers this module's commands.
pub fn register(reg: &mut Registry) {
    let write = Annotations { read_only: false, destructive: false, network: false, costs_money: false, long_running: false };
    reg.register(typed(
        "caption.keywords",
        "Mark the keywords of a clip for the keyword-highlight caption style: name the words, or auto proposes numbers, emphasis words (English + Hinglish) and names. Each keyword is a caption word colour you can review with caption.words_list and change with caption.word",
        &["clip_id"],
        write,
        vec![
            Example { description: "see what the rules would mark", cli: "rustler caption keywords clp_x --auto --dry-run" },
            Example { description: "mark the words you chose", cli: "rustler caption keywords clp_x --words million,quit --color #FFE000" },
        ],
        caption_keywords,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cw(texts: &[&str]) -> Vec<ClipWord> {
        texts
            .iter()
            .enumerate()
            .map(|(k, t)| ClipWord { source_id: SourceId::new(), word_i: k as u32, text: (*t).to_string(), at: k as f64 * 0.4 })
            .collect()
    }

    #[test]
    fn classifies_numbers_emphasis_and_names() {
        assert_eq!(classify("30", Some("in")), Some(Reason::Number));
        assert_eq!(classify("$3.5", Some("made")), Some(Reason::Number));
        assert_eq!(classify("Never,", Some("you")), Some(Reason::Emphasis));
        assert_eq!(classify("million", None), Some(Reason::Emphasis));
        assert_eq!(classify("bilkul", None), Some(Reason::Emphasis));
        assert_eq!(classify("पैसा", None), Some(Reason::Emphasis));
        assert_eq!(classify("Elon", Some("with")), Some(Reason::Name));
        // sentence start, ALL CAPS, short, pronoun: not names
        assert_eq!(classify("Then", Some("done.")), None);
        assert_eq!(classify("Elon", None), None);
        assert_eq!(classify("NASA", Some("at")), None);
        assert_eq!(classify("Ok", Some("said")), None);
        assert_eq!(classify("I", Some("so")), None);
        assert_eq!(classify("plain", Some("a")), None);
    }

    #[test]
    fn propose_orders_by_priority_caps_and_skips_neighbours() {
        let w = cw(&["I", "met", "Elon", "and", "made", "10", "million", "dollars", "never", "again", "so", "nice"]);
        // max 2: the number (10) first, then an emphasis word that is not adjacent to it.
        let p = propose(&w, 2);
        // ("million" next to the 10 is skipped; "dollars" is the first emphasis word with a gap)
        assert_eq!(p.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec![5, 7]);
        assert_eq!(p[0].1, Reason::Number);
        assert_eq!(p[1].1, Reason::Emphasis);
        // no cap: neighbours of marked words (million, never) are skipped, the name fits
        let all = propose(&w, 10);
        let idx: Vec<usize> = all.iter().map(|(k, _)| *k).collect();
        assert_eq!(idx, vec![2, 5, 7]);
        assert!(propose(&cw(&["just", "plain", "words"]), 3).is_empty());
    }

    #[test]
    fn sentence_openers_and_speaker_markers_are_not_names() {
        // the trial-2 clip 04 words: ">>" markers start sentences without a full stop
        let w = cw(&[">>", "Three.", ">>", "Definitely", "three.", ">>", "You're", "as", "good", "as", "you", "can", "communicate."]);
        let p = propose(&w, 10);
        assert!(p.iter().all(|(_, r)| *r != Reason::Name), "{p:?}");
        // "Three." is a number word, "Definitely" and "You're" are nothing
        assert_eq!(classify("Three.", Some(">>")), Some(Reason::Number));
        assert_eq!(classify("Definitely", Some(">>")), None);
        assert_eq!(classify("You're", Some(">>")), None);
        // a capital mid-sentence is still a name, but not when the clip also writes the word in lower case
        assert_eq!(classify("Vietnamese", Some("the")), Some(Reason::Name));
        let w = cw(&["we", "love", "Honestly", "it", "and", "honestly", "more"]);
        assert!(propose(&w, 5).iter().all(|(_, r)| *r != Reason::Name));
        let w = cw(&["we", "met", "Priyanka", "and", "then", "priyanka", "left"]);
        assert!(propose(&w, 5).iter().all(|(_, r)| *r != Reason::Name));
        // a repeated capitalised word that is never lower case is a name even at sentence starts
        let w = cw(&["Raj", "spoke.", "Then", "Raj", "left."]);
        let sp = Spelling::of(w.iter().map(|x| x.text.as_str()));
        assert_eq!(classify_in("Raj", None, &sp), Some(Reason::Name));
        assert_eq!(classify_in("Raj", Some("spoke."), &sp), Some(Reason::Name));
        assert_eq!(classify_in("Then", Some("spoke."), &sp), None);
        // a word capitalised once at a sentence start is not a name
        assert_eq!(classify_in("Raj", Some("spoke."), &Spelling::default()), None);
    }

    #[test]
    fn number_words_count_as_numbers() {
        for t in ["three", "Ten", "hundred,", "twenty", "सौ", "paanch", "hazaar"] {
            assert_eq!(classify(t, Some("a")), Some(Reason::Number), "{t}");
        }
        // too common to be numbers
        for t in ["one", "do", "teen", "एक"] {
            assert_eq!(classify(t, Some("a")), None, "{t}");
        }
        let w = cw(&["it", "took", "three", "years"]);
        assert_eq!(propose(&w, 3), vec![(2, Reason::Number)]);
    }

    #[test]
    fn default_colour_differs_from_the_active_word_colour() {
        // hormozi-pop is green (0x005EFF2B): keywords go yellow; yellow and other presets get green
        assert_eq!(default_keyword_colour(Some(0x005E_FF2B)), "#FFD60A");
        assert_eq!(default_keyword_colour(Some(0x0000_E0FF)), "#2BFF5E");
        assert_eq!(default_keyword_colour(None), "#2BFF5E");
        for p in rustler_captions::presets::PRESET_TABLE.iter().filter(|p| p.active != rustler_captions::presets::ActiveStyle::Static) {
            let hex = default_keyword_colour(Some(p.active_colour & 0x00FF_FFFF));
            assert_ne!(hex, rustler_captions::custom::hex_colour(p.active_colour & 0x00FF_FFFF), "{}", p.name);
        }
    }

    #[test]
    fn named_words_split_commas_and_ignore_case() {
        let mut n = named_words(&["Million, QUIT".to_string(), "job!".to_string()]);
        n.sort();
        assert_eq!(n, vec!["job", "million", "quit"]);
        assert!(named_words(&[" , ".to_string()]).is_empty());
    }

    #[test]
    fn emoji_words_are_stripped_when_no_font_is_known() {
        let mut ws = vec![
            CaptionWord { text: "money 💰".into(), out_start: 0.0, out_end: 0.4 },
            CaptionWord { text: "plain".into(), out_start: 0.4, out_end: 0.8 },
        ];
        let d = tempfile::tempdir().unwrap();
        let dirs = vec![d.path().join("nofonts")];
        let mut warnings = Vec::new();
        drop_emoji(&dirs, &mut ws, &mut warnings);
        let font_here = rustler_captions::emoji::emoji_font_available(&dirs);
        if font_here {
            // this machine has a system emoji font: nothing is dropped
            assert_eq!(ws[0].text, "money 💰");
            assert!(warnings.is_empty());
        } else {
            assert_eq!(ws[0].text, "money");
            assert_eq!(warnings.len(), 1);
        }
        assert_eq!(ws[1].text, "plain");
    }
}
