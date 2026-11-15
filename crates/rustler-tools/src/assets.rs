//! Asset library storage (`<root>/manifest.json` + files) and the setup step that fills the global
//! library (`~/.rustler/assets/`): built-in caption styles and layouts, the installed OFL fonts and a
//! curated subset of two CC0 Kenney sound packs and a curated set of FreePD CC0 music tracks. Every
//! entry carries license metadata; nothing without a verified license is ever registered.

use crate::download::{download, sha256_file};
use crate::paths::ToolPaths;
use rustler_core::{ErrorCode, RustlerError};
use rustler_registry::{ProgressEvent, ProgressSink};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

type Result<T> = rustler_core::Result<T>;

/// License string of entries Rustler itself ships (built-in caption styles and layouts).
pub const LICENSE_BUILTIN: &str = "built-in";

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Font,
    Music,
    Sfx,
    Image,
    #[serde(alias = "caption-style")]
    CaptionStyle,
    Layout,
    Template,
}

impl AssetKind {
    pub const ALL: [AssetKind; 7] = [
        AssetKind::Font,
        AssetKind::Music,
        AssetKind::Sfx,
        AssetKind::Image,
        AssetKind::CaptionStyle,
        AssetKind::Layout,
        AssetKind::Template,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AssetKind::Font => "font",
            AssetKind::Music => "music",
            AssetKind::Sfx => "sfx",
            AssetKind::Image => "image",
            AssetKind::CaptionStyle => "caption_style",
            AssetKind::Layout => "layout",
            AssetKind::Template => "template",
        }
    }

    /// Directory below the assets root that holds files of this kind.
    pub fn dir(self) -> &'static str {
        match self {
            AssetKind::Font => "fonts",
            AssetKind::Music => "music",
            AssetKind::Sfx => "sfx",
            AssetKind::Image => "images",
            AssetKind::CaptionStyle => "caption_styles",
            AssetKind::Layout => "layouts",
            AssetKind::Template => "templates",
        }
    }

    /// Short id prefix.
    fn prefix(self) -> &'static str {
        match self {
            AssetKind::CaptionStyle => "style",
            k => k.as_str(),
        }
    }

    /// Accepts `caption_style`, `caption-style` and `captionstyle`.
    pub fn parse(s: &str) -> Option<Self> {
        let n = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Self::ALL.into_iter().find(|k| k.as_str() == n || k.as_str().replace('_', "") == n)
    }
}

/// One manifest entry.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct AssetEntry {
    pub id: String,
    pub kind: AssetKind,
    pub name: String,
    /// Relative to the manifest's directory (forward slashes), or absolute (registered fonts).
    /// Built-in entries use `builtin:<kind>/<name>` and have no file.
    pub path: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// SPDX id or short license name (`CC0-1.0`, `OFL-1.1`, `built-in`, user-declared).
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// RFC 3339.
    pub added_at: String,
    /// sha256 of the file (empty for built-in entries).
    #[serde(default)]
    pub sha256: String,
}

impl AssetEntry {
    pub fn is_builtin(&self) -> bool {
        self.license == LICENSE_BUILTIN
    }
}

#[derive(Serialize, Deserialize, Default)]
struct ManifestFile {
    #[serde(default = "one")]
    version: u32,
    #[serde(default)]
    assets: Vec<AssetEntry>,
}

fn one() -> u32 {
    1
}

static LOCK: Mutex<()> = Mutex::new(());

fn err(msg: impl Into<String>) -> RustlerError {
    RustlerError::new(ErrorCode::Internal, "assets", msg)
}

/// Global library root.
pub fn assets_root(home: &Path) -> PathBuf {
    home.join("assets")
}

/// Per-project library root.
pub fn project_assets_root(project: &Path) -> PathBuf {
    project.join("assets")
}

pub fn manifest_path(root: &Path) -> PathBuf {
    root.join("manifest.json")
}

/// Entries of `<root>/manifest.json` (empty when missing; accepts a bare array too).
pub fn load_manifest(root: &Path) -> Vec<AssetEntry> {
    let Ok(bytes) = std::fs::read(manifest_path(root)) else { return Vec::new() };
    if let Ok(f) = serde_json::from_slice::<ManifestFile>(&bytes) {
        return f.assets;
    }
    serde_json::from_slice::<Vec<AssetEntry>>(&bytes).unwrap_or_default()
}

fn write_manifest(root: &Path, entries: &[AssetEntry]) -> Result<()> {
    std::fs::create_dir_all(root).map_err(|e| err(format!("create {}: {e}", root.display())))?;
    let path = manifest_path(root);
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(&ManifestFile { version: 1, assets: entries.to_vec() })
        .map_err(|e| err(format!("serialize manifest: {e}")))?;
    std::fs::write(&tmp, body).map_err(|e| err(format!("write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &path).map_err(|e| err(format!("write {}: {e}", path.display())))
}

/// Serialized read-modify-write of `<root>/manifest.json`; `f` may change the list and returns a value.
pub fn update_manifest<T>(root: &Path, f: impl FnOnce(&mut Vec<AssetEntry>) -> Result<T>) -> Result<T> {
    let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut entries = load_manifest(root);
    let out = f(&mut entries)?;
    write_manifest(root, &entries)?;
    Ok(out)
}

/// Absolute path of an entry's file (`None` for built-in entries without a file).
pub fn entry_file(root: &Path, e: &AssetEntry) -> Option<PathBuf> {
    if e.path.starts_with("builtin:") || e.path.is_empty() {
        return None;
    }
    let p = Path::new(&e.path);
    Some(if p.is_absolute() { p.to_path_buf() } else { p.components().fold(root.to_path_buf(), |acc, c| acc.join(c)) })
}

/// Lowercase ascii slug (`"Kenney Click 1"` -> `kenney-click-1`).
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() { "asset".into() } else { t }
}

/// `<prefix>-<slug>`, with `-2`, `-3`... when taken.
pub fn unique_id(kind: AssetKind, name: &str, existing: &[AssetEntry]) -> String {
    let base = format!("{}-{}", kind.prefix(), slug(name));
    let mut id = base.clone();
    let mut n = 2;
    while existing.iter().any(|e| e.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Inserts `e`, or replaces the entry with the same id (keeping its `added_at`).
pub fn upsert(entries: &mut Vec<AssetEntry>, mut e: AssetEntry) {
    if let Some(old) = entries.iter_mut().find(|x| x.id == e.id) {
        e.added_at = old.added_at.clone();
        *old = e;
    } else {
        entries.push(e);
    }
}

/// The built-in caption styles and layouts as (virtual) assets.
pub fn builtin_entries() -> Vec<AssetEntry> {
    let now = now_rfc3339();
    let mut out = Vec::new();
    for p in rustler_captions::list_presets() {
        out.push(AssetEntry {
            id: format!("style-{}", p.name),
            kind: AssetKind::CaptionStyle,
            name: p.name.clone(),
            path: format!("builtin:caption_style/{}", p.name),
            tags: vec!["caption".into(), "built-in".into(), p.font_family.to_ascii_lowercase()],
            license: LICENSE_BUILTIN.into(),
            license_url: None,
            source_url: None,
            attribution: None,
            added_at: now.clone(),
            sha256: String::new(),
        });
    }
    for name in rustler_core::layout::PRESETS {
        out.push(AssetEntry {
            id: format!("layout-{name}"),
            kind: AssetKind::Layout,
            name: (*name).to_string(),
            path: format!("builtin:layout/{name}"),
            tags: vec!["layout".into(), "built-in".into()],
            license: LICENSE_BUILTIN.into(),
            license_url: None,
            source_url: None,
            attribution: None,
            added_at: now.clone(),
            sha256: String::new(),
        });
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Setup step
// ---------------------------------------------------------------------------------------------

/// What [`install`] did.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq, Debug, Default)]
pub struct AssetsReport {
    pub builtins: usize,
    pub fonts: usize,
    pub sfx: usize,
    /// CC0 music tracks registered (FreePD).
    #[serde(default)]
    pub music: usize,
    /// Non-fatal problems (a pack that could not be fetched or verified, ...).
    pub warnings: Vec<String>,
}

const OFL_URL: &str = "https://openfontlicense.org/open-font-license-official-text/";
const CC0_URL: &str = "https://creativecommons.org/publicdomain/zero/1.0/";

/// Human name of a font file (`Poppins-ExtraBold` -> `Poppins ExtraBold`).
fn font_name(stem: &str) -> String {
    let base = stem.split('[').next().unwrap_or(stem);
    let mut s = base.replace('-', " ");
    if s == "NotoSansDevanagari" {
        s = "Noto Sans Devanagari".into();
    }
    s
}

/// Registers the OFL fonts that `setup` installed as `font` assets (files stay in `home/fonts`).
pub fn register_fonts(paths: &ToolPaths, root: &Path) -> Result<usize> {
    let mut found: Vec<AssetEntry> = Vec::new();
    for (local, remote, files) in crate::fonts::FAMILIES {
        for f in *files {
            let file = paths.fonts.join(local).join(f);
            if !file.is_file() {
                continue;
            }
            let stem = Path::new(f).file_stem().and_then(|s| s.to_str()).unwrap_or(f);
            let name = font_name(stem);
            let mut tags = vec!["font".to_string(), "ofl".to_string(), local.to_ascii_lowercase()];
            if local.contains("Devanagari") {
                tags.extend(["devanagari".to_string(), "hindi".to_string(), "hinglish".to_string()]);
            } else {
                tags.push("caption".into());
                tags.push("latin".into());
            }
            found.push(AssetEntry {
                id: format!("font-{}", slug(&name)),
                kind: AssetKind::Font,
                name,
                path: file.to_string_lossy().into_owned(),
                tags,
                license: "OFL-1.1".into(),
                license_url: Some(OFL_URL.into()),
                source_url: Some(format!("https://github.com/google/fonts/tree/main/ofl/{remote}")),
                attribution: None,
                added_at: now_rfc3339(),
                sha256: sha256_file(&file).unwrap_or_default(),
            });
        }
    }
    let n = found.len();
    update_manifest(root, |entries| {
        for e in found {
            upsert(entries, e);
        }
        Ok(())
    })?;
    Ok(n)
}

/// A curated sound: (pack, file inside the zip, display name, tags).
struct Sfx(&'static str, &'static str, &'static str, &'static [&'static str]);

/// (pack slug on kenney.nl, folder name below `sfx/`, source page).
const PACKS: &[(&str, &str)] = &[("interface-sounds", "Interface Sounds"), ("impact-sounds", "Impact Sounds")];

const SFX: &[Sfx] = &[
    Sfx("interface-sounds", "click_001.ogg", "Click 1", &["click", "ui"]),
    Sfx("interface-sounds", "click_003.ogg", "Click 2", &["click", "ui"]),
    Sfx("interface-sounds", "tick_001.ogg", "Tick", &["click", "tick"]),
    Sfx("interface-sounds", "select_001.ogg", "Select", &["click", "pop", "ui"]),
    Sfx("interface-sounds", "switch_001.ogg", "Switch", &["click", "switch"]),
    Sfx("interface-sounds", "toggle_001.ogg", "Toggle", &["pop", "toggle"]),
    Sfx("interface-sounds", "drop_001.ogg", "Drop pop", &["pop", "drop"]),
    Sfx("interface-sounds", "open_001.ogg", "Open pop", &["pop", "open"]),
    Sfx("interface-sounds", "confirmation_001.ogg", "Confirmation", &["ding", "success"]),
    Sfx("interface-sounds", "question_001.ogg", "Question", &["question", "ding"]),
    Sfx("interface-sounds", "error_001.ogg", "Error", &["error", "buzz"]),
    Sfx("interface-sounds", "glitch_001.ogg", "Glitch", &["glitch", "transition"]),
    Sfx("interface-sounds", "scratch_001.ogg", "Scratch", &["scratch", "transition"]),
    Sfx("interface-sounds", "maximize_001.ogg", "Swipe up", &["swipe", "whoosh", "transition"]),
    Sfx("interface-sounds", "maximize_004.ogg", "Swipe up long", &["swipe", "whoosh", "transition"]),
    Sfx("interface-sounds", "minimize_001.ogg", "Swipe down", &["swipe", "whoosh", "transition"]),
    Sfx("interface-sounds", "minimize_004.ogg", "Swipe down long", &["swipe", "whoosh", "transition"]),
    Sfx("interface-sounds", "scroll_001.ogg", "Scroll swipe", &["swipe", "scroll"]),
    Sfx("interface-sounds", "bong_001.ogg", "Bong", &["bong", "ding"]),
    Sfx("impact-sounds", "impactPunch_heavy_000.ogg", "Punch heavy", &["impact", "hit", "punch"]),
    Sfx("impact-sounds", "impactPunch_medium_000.ogg", "Punch medium", &["impact", "hit", "punch"]),
    Sfx("impact-sounds", "impactGeneric_light_000.ogg", "Impact light", &["impact", "hit"]),
    Sfx("impact-sounds", "impactSoft_heavy_000.ogg", "Impact soft heavy", &["impact", "thud"]),
    Sfx("impact-sounds", "impactMetal_heavy_000.ogg", "Impact metal", &["impact", "metal", "hit"]),
    Sfx("impact-sounds", "impactGlass_light_000.ogg", "Impact glass", &["impact", "glass"]),
    Sfx("impact-sounds", "impactBell_heavy_000.ogg", "Impact bell", &["impact", "bell"]),
];

fn kenney_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("rustler/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| RustlerError::new(ErrorCode::Internal, "setup.assets", format!("http client: {e}")))
}

fn hex_of(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// Asks kenney.nl for the (short-lived, signed) zip URL of a pack, the same call its download
/// button makes after "Continue without donating".
async fn kenney_zip_url(pack: &str) -> Result<String> {
    let page = format!("https://kenney.nl/assets/{pack}");
    let api = format!("https://kenney.nl/download-link/{}", hex_of(&format!("assets/{pack}")));
    let net = |e: &dyn std::fmt::Display| {
        RustlerError::new(ErrorCode::Provider, "setup.assets", format!("kenney.nl {pack}: {e}"))
            .hint("check your internet connection; `rustler setup --no-assets` skips the SFX pack")
            .retryable()
    };
    let text = kenney_client()?
        .get(&api)
        .header("Referer", &page)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| net(&e))?
        .text()
        .await
        .map_err(|e| net(&e))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| net(&format!("unexpected answer: {e}")))?;
    v.get("url")
        .and_then(|u| u.as_str())
        .map(str::to_string)
        .ok_or_else(|| net(&format!("no download url in answer: {}", text.chars().take(120).collect::<String>())))
}

/// Downloads one Kenney pack, checks its `License.txt` says CC0 and extracts the curated files.
async fn fetch_pack(
    paths: &ToolPaths,
    root: &Path,
    pack: &str,
    label: &str,
    progress: &ProgressSink,
) -> Result<Vec<AssetEntry>> {
    let url = kenney_zip_url(pack).await?;
    let zip_path = paths.home.join("dl").join(format!("kenney-{pack}.zip"));
    download(&url, &zip_path, None, progress, "setup.assets").await?;

    let dest_rel = format!("sfx/kenney-{pack}");
    let dest = root.join(&dest_rel);
    let wanted: Vec<&str> = SFX.iter().filter(|s| s.0 == pack).map(|s| s.1).collect();
    let mut names: Vec<&str> = wanted.clone();
    names.push("License.txt");
    let zp = zip_path.clone();
    let d = dest.clone();
    let extracted = tokio::task::spawn_blocking(move || crate::archive::extract_binaries(&zp, &names, &d))
        .await
        .map_err(|e| err(format!("extract task: {e}")))??;
    drop(extracted);

    let license = std::fs::read_to_string(dest.join("License.txt")).unwrap_or_default();
    let ok = {
        let l = license.to_ascii_lowercase();
        l.contains("creative commons zero") || l.contains("cc0")
    };
    if !ok {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(RustlerError::new(
            ErrorCode::Provider,
            "setup.assets",
            format!("the {label} zip does not state CC0 in its License.txt; not registered"),
        ));
    }

    let page = format!("https://kenney.nl/assets/{pack}");
    let mut out = Vec::new();
    for s in SFX.iter().filter(|s| s.0 == pack) {
        let file = dest.join(s.1);
        let name = format!("Kenney {} - {}", label, s.2);
        out.push(AssetEntry {
            id: format!("sfx-kenney-{}-{}", pack.trim_end_matches("-sounds"), slug(s.2)),
            kind: AssetKind::Sfx,
            name,
            path: format!("{dest_rel}/{}", s.1),
            tags: s.3.iter().map(|t| (*t).to_string()).chain(["kenney".to_string(), "cc0".to_string()]).collect(),
            license: "CC0-1.0".into(),
            license_url: Some(CC0_URL.into()),
            source_url: Some(page.clone()),
            attribution: Some("Kenney (www.kenney.nl), CC0; credit optional".into()),
            added_at: now_rfc3339(),
            sha256: sha256_file(&file).map_err(|e| err(format!("hash {}: {e}", file.display())))?,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// CC0 music (FreePD)
// ---------------------------------------------------------------------------------------------

/// License string of the FreePD music entries.
pub const LICENSE_FREEPD: &str = "CC0-1.0 / public domain (FreePD)";

/// FreePD closed in 2025 (the live site now only shows a closure notice), so the license statement and the
/// tracks are read from the Internet Archive capture of 2024-12-31, addressed by that exact timestamp
/// (`id_` = the original bytes without Wayback's page rewriting). The track bytes are pinned by sha256.
const FREEPD_SNAPSHOT: &str = "20241231";
/// Where the license statement is read (the FreePD home page, archived).
const FREEPD_LICENSE_PAGE: &str = "https://web.archive.org/web/20241231id_/https://freepd.com/";
/// The sentence FreePD's home page carried; registration is refused unless the page says it.
const FREEPD_LICENSE_TEXT: &str = "licensed cc0 1.0 universal public domain dedication";

/// A curated track: (file name on freepd.com/music/, display name, composer, mood tags, seconds, sha256).
struct Music(&'static str, &'static str, &'static str, &'static [&'static str], u32, &'static str);

const MUSIC: &[Music] = &[
    Music("Happy Whistling Ukulele", "Happy Whistling Ukulele", "Rafael Krux", &["upbeat", "happy", "ukulele"], 123, "ed4059c8fba8673d657e85175ff54eaa32cef9e43f688f164bef585e6e65cf15"),
    Music("Inventing Flight", "Inventing Flight", "Bryan Teoh", &["upbeat", "inspiring", "bright"], 142, "916e736efa3caed4ea8534e727670dbac41af0fb44f00fa667aebaab9b2b6407"),
    Music("3 am West End", "3 am West End", "statusq", &["chill", "lofi", "groove"], 291, "590bb9833cfb0d6610a6ac00093f9b3d062d604863d6e44bdac61d526d33cab8"),
    Music("Martini Sunset", "Martini Sunset", "Anonymous", &["chill", "lofi", "jazzy"], 174, "b9d12b5d59c187e07316cab61e82ccbea0f48f2f64a3e8062abbf1861acc7add"),
    Music("Soundtrack From the Starcourt Mall", "Starcourt Mall", "Bryan Teoh", &["chill", "lofi", "retro"], 217, "9199da36aefbe7974a75ad22084768ec77c1a98e14ac14ce36f193ece668b867"),
    Music("Night Attack", "Night Attack", "Rafael Krux", &["dramatic", "dark", "action"], 153, "aa52288cee980cbc8da8ffc087de6f92499a5a4130db6feab3f83d774501784c"),
    Music("Epic Boss Battle", "Epic Boss Battle", "Rafael Krux", &["epic", "dramatic", "battle"], 170, "9ea1753ae8bdf665e2b7011c9dd115bf764146d75004301a08a5efc7e6d7eaa7"),
    Music("Heroic Adventure", "Heroic Adventure", "Rafael Krux", &["epic", "triumphant", "uplifting"], 143, "41355a9625e8ee15d678c1c8c512636b6610a750f61b79820b53286a0dc3836a"),
    Music("Llama in Pajama", "Llama in Pajama", "Rafael Krux", &["funny", "quirky", "comedy"], 97, "3016ae15b98ea952cd0ccc1bd42c9596d4e78968fed8dc96c6ee6d375c1ed988"),
    Music("Rush", "Rush", "Rafael Krux", &["funny", "fast", "comedy"], 134, "060122428ce637a4ade01ba7b24178e7aff0bd97657ecb24aab2faf2c1ca4fc2"),
    Music("Busybody", "Busybody", "Bryan Teoh", &["funny", "frantic", "comedy"], 86, "5170e23bc9a24f870b896965832c268e87db18418bd1ca9aa361ef5f52d67f98"),
];

fn music_id(m: &Music) -> String {
    format!("music-freepd-{}", slug(m.1))
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// True when `page` (the FreePD home page text) carries the CC0 dedication sentence.
fn freepd_page_says_cc0(page: &str) -> bool {
    let flat = page.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    flat.contains(FREEPD_LICENSE_TEXT)
}

/// Checks the FreePD license statement, then downloads the curated tracks into `<root>/music/freepd/`
/// (each verified against its pinned sha256) and returns their manifest entries. A track that cannot be
/// fetched is skipped with a warning; nothing is registered if the license check fails.
pub async fn fetch_freepd_music(
    root: &Path,
    have: &[String],
    progress: &ProgressSink,
    warnings: &mut Vec<String>,
) -> Result<Vec<AssetEntry>> {
    let page = crate::download::fetch_text(FREEPD_LICENSE_PAGE).await.map_err(|e| {
        RustlerError::new(ErrorCode::Provider, "setup.assets", format!("FreePD license page: {e}"))
            .hint("the music pack is optional (`rustler setup run --music-pack`); add your own music with `rustler asset add`")
            .retryable()
    })?;
    if !freepd_page_says_cc0(&page) {
        return Err(RustlerError::new(
            ErrorCode::Provider,
            "setup.assets",
            "the FreePD home page does not state the CC0 1.0 dedication; music not registered",
        ));
    }
    let dest_rel = "music/freepd";
    let mut out = Vec::new();
    for m in MUSIC {
        let id = music_id(m);
        let rel = format!("{dest_rel}/{}.mp3", slug(m.1));
        let file = root.join(&rel);
        if have.contains(&id) && file.is_file() {
            continue;
        }
        let url = format!("https://web.archive.org/web/{FREEPD_SNAPSHOT}id_/https://freepd.com/music/{}.mp3", url_encode(m.0));
        if let Err(e) = download(&url, &file, Some(m.5), progress, "setup.assets").await {
            warnings.push(format!("FreePD {}: {e}", m.1));
            continue;
        }
        out.push(AssetEntry {
            id,
            kind: AssetKind::Music,
            name: format!("{} ({})", m.1, m.2),
            path: rel,
            tags: m.3.iter().map(|t| (*t).to_string()).chain(["freepd".to_string(), "cc0".to_string(), format!("{}s", m.4)]).collect(),
            license: LICENSE_FREEPD.into(),
            license_url: Some(CC0_URL.into()),
            source_url: Some(format!("https://freepd.com/music/{}.mp3", url_encode(m.0))),
            attribution: Some(format!(
                "{} via FreePD, CC0 1.0 (credit optional). freepd.com closed in 2025; file from the Internet Archive capture {FREEPD_SNAPSHOT}, sha256 pinned",
                m.2
            )),
            added_at: now_rfc3339(),
            sha256: m.5.to_string(),
        });
    }
    Ok(out)
}

/// `rustler setup` step without the optional music pack: see [`install_with`].
pub async fn install(paths: &ToolPaths, force: bool, progress: &ProgressSink) -> Result<AssetsReport> {
    install_with(paths, force, false, progress).await
}

/// `rustler setup` step: built-in styles/layouts, installed fonts, Kenney CC0 SFX and, only when `music_pack`
/// is true (`rustler setup run --music-pack`, off by default), the curated FreePD CC0 music. Rustler ships
/// no fixed music: the agent brings any track with `asset add`/`--path`. Idempotent; a network failure only
/// adds a warning.
pub async fn install_with(paths: &ToolPaths, force: bool, music_pack: bool, progress: &ProgressSink) -> Result<AssetsReport> {
    let root = assets_root(&paths.home);
    std::fs::create_dir_all(&root).map_err(|e| err(format!("create {}: {e}", root.display())))?;
    let mut report = AssetsReport::default();
    let emit = |m: String, pct: f32| {
        progress.emit(ProgressEvent { stage: "setup.assets".into(), pct: Some(pct), message: Some(m) })
    };

    let builtins = builtin_entries();
    report.builtins = builtins.len();
    update_manifest(&root, |entries| {
        for e in builtins {
            upsert(entries, e);
        }
        Ok(())
    })?;
    emit("registered built-in caption styles and layouts".into(), 0.1);

    report.fonts = register_fonts(paths, &root)?;
    emit(format!("registered {} installed fonts", report.fonts), 0.2);

    for (i, (pack, label)) in PACKS.iter().enumerate() {
        let have = load_manifest(&root)
            .iter()
            .filter(|e| e.kind == AssetKind::Sfx && e.id.starts_with(&format!("sfx-kenney-{}-", pack.trim_end_matches("-sounds"))))
            .count();
        let want = SFX.iter().filter(|s| s.0 == *pack).count();
        if !force && have >= want {
            report.sfx += have;
            continue;
        }
        match fetch_pack(paths, &root, pack, label, progress).await {
            Ok(entries) => {
                report.sfx += entries.len();
                update_manifest(&root, |m| {
                    for e in entries {
                        upsert(m, e);
                    }
                    Ok(())
                })?;
                emit(format!("registered Kenney {label} (CC0)"), 0.2 + 0.4 * (i + 1) as f32);
            }
            Err(e) => report.warnings.push(format!("Kenney {label}: {e}")),
        }
    }

    let have: Vec<String> =
        load_manifest(&root).iter().filter(|e| e.kind == AssetKind::Music).map(|e| e.id.clone()).collect();
    if music_pack && (force || have.len() < MUSIC.len()) {
        let mut warnings = Vec::new();
        match fetch_freepd_music(&root, if force { &[] } else { &have }, progress, &mut warnings).await {
            Ok(entries) => {
                update_manifest(&root, |m| {
                    for e in entries {
                        upsert(m, e);
                    }
                    Ok(())
                })?;
                emit("registered FreePD music (CC0)".into(), 0.95);
            }
            Err(e) => warnings.push(format!("FreePD music: {e}")),
        }
        report.warnings.extend(warnings);
    }
    report.music = load_manifest(&root).iter().filter(|e| e.kind == AssetKind::Music).count();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_ids() {
        assert_eq!(slug("Kenney  Click #1!"), "kenney-click-1");
        assert_eq!(slug("!!!"), "asset");
        let mut e = builtin_entries();
        assert!(e.iter().any(|x| x.id == "style-bold-karaoke" && x.is_builtin()));
        assert!(e.iter().any(|x| x.id == "layout-split-vertical"));
        let id = unique_id(AssetKind::CaptionStyle, "bold-karaoke", &e);
        assert_eq!(id, "style-bold-karaoke-2");
        e.clear();
        assert_eq!(unique_id(AssetKind::Sfx, "Whoosh 1", &e), "sfx-whoosh-1");
    }

    #[test]
    fn kinds_parse() {
        assert_eq!(AssetKind::parse("caption-style"), Some(AssetKind::CaptionStyle));
        assert_eq!(AssetKind::parse("Caption_Style"), Some(AssetKind::CaptionStyle));
        assert_eq!(AssetKind::parse("sfx"), Some(AssetKind::Sfx));
        assert_eq!(AssetKind::parse("video"), None);
    }

    #[test]
    fn manifest_round_trip_and_upsert() {
        let d = tempfile::tempdir().unwrap();
        assert!(load_manifest(d.path()).is_empty());
        update_manifest(d.path(), |m| {
            for e in builtin_entries() {
                upsert(m, e);
            }
            Ok(())
        })
        .unwrap();
        let first = load_manifest(d.path());
        assert_eq!(first.len(), 7);
        update_manifest(d.path(), |m| {
            for e in builtin_entries() {
                upsert(m, e);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(load_manifest(d.path()), first);
    }

    #[test]
    fn freepd_curation_is_sane() {
        assert!((10..=14).contains(&MUSIC.len()));
        let mut ids: Vec<String> = MUSIC.iter().map(music_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), MUSIC.len());
        for m in MUSIC {
            assert_eq!(m.5.len(), 64, "{}", m.0);
            assert!(m.3.len() >= 3);
        }
        assert_eq!(url_encode("3 am West End"), "3%20am%20West%20End");
        assert!(freepd_page_says_cc0("This music is all licensed\n CC0 1.0 Universal Public Domain Dedication."));
        assert!(!freepd_page_says_cc0("FreePD.com - Site Closed. Public Domain music"));
    }

    /// Network: license check + download of all curated tracks into a temp library, sha256 pinned.
    /// `cargo test -p rustler-tools freepd_music_network -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn freepd_music_network() {
        let tmp = tempfile::tempdir().unwrap();
        let root = assets_root(tmp.path());
        let mut warnings = Vec::new();
        let entries = fetch_freepd_music(&root, &[], &ProgressSink::noop(), &mut warnings).await.unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(entries.len(), MUSIC.len());
        for e in &entries {
            assert_eq!(e.kind, AssetKind::Music);
            assert_eq!(e.license, LICENSE_FREEPD);
            assert!(e.source_url.as_deref().unwrap().starts_with("https://freepd.com/music/"));
            let f = entry_file(&root, e).unwrap();
            assert_eq!(sha256_file(&f).unwrap(), e.sha256);
            assert!(std::fs::metadata(&f).unwrap().len() > 100_000);
        }
    }

    #[test]
    fn curated_sfx_cover_packs() {
        for s in SFX {
            assert!(PACKS.iter().any(|(p, _)| *p == s.0), "{}", s.1);
        }
        assert!((20..=30).contains(&SFX.len()));
    }
}
