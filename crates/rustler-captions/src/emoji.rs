//! Emoji in captions: detection, an emoji-font check and stripping.
//!
//! `caption.word --emoji` appends an emoji to a caption word. libass draws it only when some
//! installed font has the glyph (Segoe UI Emoji on Windows draws it as a one-colour outline in
//! the caption colour; a bitmap emoji font may not be drawn in colour). When no emoji font is
//! installed the render drops the emoji and says so (never a missing-glyph box).

use std::path::{Path, PathBuf};

/// True for characters drawn as emoji (pictographs, flags, keycaps, variation selectors, joiners).
pub fn is_emoji_char(c: char) -> bool {
    matches!(c as u32,
        0x1F300..=0x1FAFF   // pictographs, emoticons, transport, supplemental, extended-A
        | 0x1F1E6..=0x1F1FF // regional indicators (flags)
        | 0x2600..=0x27BF   // misc symbols, dingbats
        | 0x2B50 | 0x2B55 | 0x2B1B | 0x2B1C
        | 0x2300..=0x23FF   // misc technical (watch, hourglass, play buttons)
        | 0x2900..=0x297F
        | 0x3030 | 0x303D | 0x3297 | 0x3299
        | 0xFE0F | 0xFE0E   // variation selectors
        | 0x200D            // zero-width joiner
        | 0x20E3            // keycap
    )
}

/// True when `s` has an emoji (variation selectors and joiners alone do not count).
pub fn contains_emoji(s: &str) -> bool {
    s.chars().any(|c| is_emoji_char(c) && !matches!(c as u32, 0xFE0F | 0xFE0E | 0x200D | 0x20E3))
}

/// `s` without emoji, whitespace collapsed.
pub fn strip_emoji(s: &str) -> String {
    let t: String = s.chars().filter(|c| !is_emoji_char(*c)).collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// System font folders of this OS.
pub fn system_font_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(windows) {
        let win = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")).map(PathBuf::from);
        v.push(win.unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts"));
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        v.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
    } else {
        v.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join(".fonts"));
        }
    }
    v
}

fn has_emoji_font(dir: &Path, depth: u32) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 3 && has_emoji_font(&p, depth + 1) {
                return true;
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
            && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase().contains("emoj"))
        {
            return true;
        }
    }
    false
}

/// True when a font file with "emoji" in its name is in `dirs` or in the system font folders.
pub fn emoji_font_available(dirs: &[PathBuf]) -> bool {
    dirs.iter().chain(system_font_dirs().iter()).any(|d| has_emoji_font(d, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_strips() {
        assert!(contains_emoji("wow 🔥"));
        assert!(contains_emoji("❤️"));
        assert!(!contains_emoji("plain 123 #1"));
        assert!(!contains_emoji("नमस्ते"));
        assert_eq!(strip_emoji("money 💰"), "money");
        assert_eq!(strip_emoji("🔥"), "");
        assert_eq!(strip_emoji("fine"), "fine");
    }

    #[test]
    fn finds_an_emoji_font_in_a_folder() {
        let d = tempfile::tempdir().unwrap();
        assert!(!has_emoji_font(d.path(), 0));
        std::fs::create_dir_all(d.path().join("Noto")).unwrap();
        std::fs::write(d.path().join("Noto").join("NotoColorEmoji.ttf"), b"x").unwrap();
        assert!(has_emoji_font(d.path(), 0));
        assert!(emoji_font_available(&[d.path().to_path_buf()]));
    }
}
