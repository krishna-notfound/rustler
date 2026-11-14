# Slice 1 — "First Real Clip" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An agent with only a shell can run `rustler setup`, import a local file or a YouTube URL (scan-first), transcribe it, look at frames, choose a moment by transcript unit ids, render a 9:16 clip (crop / blur-fit / split-vertical layout + karaoke captions + loudnorm), and get a QC report with a contact sheet — all through discoverable JSON CLI commands.

**Architecture:** Rust workspace; one command registry is the only way to call functionality (CLI is a generic, schema-driven front over it). FFmpeg/ffprobe/yt-dlp run as subprocesses from `~/.rustler/bin`. A Python worker (`rustler_engine`, uv-managed venv) does transcription (faster-whisper), scene detection (PySceneDetect) and Devanagari→Latin transliteration over line-delimited JSON-RPC on stdio. Project state is a folder of JSON files with revision-checked atomic saves and an append-only event log.

**Tech Stack:** Rust (edition 2024, rustc ≥ 1.85; installed 1.100), clap 4, tokio, serde/serde_json, schemars, thiserror/anyhow, tracing, reqwest (rustls), image/imageproc/ab_glyph, include_dir, ulid, chrono, sha2, zip/tar/xz2, strsim. Python 3.11 via uv: faster-whisper, scenedetect + opencv-python-headless, indic-transliteration, pytest. FFmpeg (BtbN static GPL build), yt-dlp release binary.

**Spec:** `docs/superpowers/specs/2026-11-15-rustler-design.md` (approved 2026-11-15). Research refs: `findings/00-comparison-matrix.md` §2–§3.

## Global Constraints

- Binary name `rustler`; tools home `~/.rustler/` overridable by env `RUSTLER_HOME`; project dir from `--project <dir>`, else env `RUSTLER_PROJECT`, else current dir if it contains `project.json`.
- Every capability is a registry command first; the CLI contains **no** feature logic.
- stdout carries exactly one JSON envelope per invocation: `{"ok":true,"schema":"rustler.<command>@1","data":{…},"warnings":[…]}` or `{"ok":false,"error":{"code","stage","message","hint","retryable"}}`. Progress/logs go to stderr only (NDJSON progress lines `{"event":"progress",…}`).
- Exit codes: 0 ok · 2 usage/validation · 3 not found · 4 dependency missing · 5 provider/network error · 6 media/ffmpeg error · 7 QC failed · 8 cancelled · 9 revision conflict · 10 internal.
- Times are **seconds as f64 in source time** unless a field name says `out_` (output/clip time). LLMs/agents choose **unit ids**, never raw timestamps, when creating clips from speech.
- Default canvas 1080×1920 @ 30 fps; captions default script `latin`; loudness target I = −14 LUFS, TP = −2 dBTP, LRA = 11.
- One encode per deliverable (one ffmpeg invocation produces the final mp4).
- Never pass secrets/cookies on argv logs; external processes get arguments as argv arrays (never through a shell).
- No admin rights, no interactive prompts in `setup`. Must work on Windows 11 x64 and Linux x64 without a GPU; NVENC used only after a real 1-frame probe succeeds.
- Code may be copied only from MIT repos (openshorts, autoclip, bridgeclip, opensource-clipping) with an attribution line in `THIRD_PARTY_NOTICES.md`; supoclip/ViralCutter/hypit are ideas-only.
- Fonts fetched only from upstream google/fonts with their OFL.txt.
- Parallel agents share one working tree but use their own `CARGO_TARGET_DIR=target/<agent-name>` and edit only files their task lists.

## Review Focus

1. **Awkward file names** (spaces, brackets, unicode/Devanagari, e.g. `my clip (1) – टेस्ट.mp4`) on Windows → import, see, render all work. Pinned in Task 9 (`render_handles_awkward_filename`).
2. **Sources missing a stream** (video with no audio track; audio-only podcast) → import succeeds, render of a video-without-audio produces a silent AAC track, audio-only import is accepted but `clip.create` errors `validation` "source has no video". Pinned in Task 6 (`probe_audio_only`), Task 9 (`render_video_without_audio_gets_silent_track`) and Task 10 (`audio_only_source_rejected`).
3. **Rotated / portrait / odd-size sources** (phone 1080×1920 with rotation metadata, 4K, odd widths) → probe reports display dims after rotation; layouts use display dims; output always even dims. Pinned in Task 6 (`probe_reports_display_dims_for_rotated`).
4. **Interrupted work** → `project.json`/`clip.json` written atomically (temp + rename); transcription rerun resumes from saved chunk parts. Pinned in Task 3 (`atomic_write_survives_partial`), Task 5 (`test_jfk_words` parts reuse) and Task 10 (`transcribe_resumes_from_parts`).
5. **Empty or wrong-language speech** (music, silence) → transcript with 0 words saves fine, `units` is empty, `clip.create --start-unit` errors `validation` with hint to use `--start/--end`, captions auto-disabled with a warning. Pinned in Task 10 (`empty_transcript_ok`, `create_without_units_hints_transcribe`).

---

## File Structure

```
Cargo.toml                         workspace (members = crates/*)
.gitignore                         target/, repos/, **/__pycache__, .venv
THIRD_PARTY_NOTICES.md             attributions for copied MIT code + fonts
crates/
  rustler-core/src/
    lib.rs                         module decls + re-exports
    ids.rs                         SourceId, ClipId, RenderId, ProjectId (prefixed ULIDs)
    error.rs                       ErrorCode, RustlerError, exit-code mapping
    model/project.rs               Project, Permissions, ProjectSettings
    model/source.rs                Source, SourceOrigin, MediaProbe, TimeRange
    model/transcript.rs            Transcript, Word, Unit, UnitKind
    model/analysis.rs              Scenes, AudioEvents, LoudnessPoint, Silence
    model/clip.rs                  Clip, Segment, Layout, Region, Rect, Fit, Background, CaptionTrack, AudioSettings, Canvas
    model/qc.rs                    QcReport, QcCheck, QcStatus
    events.rs                      Event, Actor
    store.rs                       ProjectStore (paths, atomic json io, revisions, events)
    timemap.rs                     TimeMap (source time <-> output time)
    units.rs                       build_units(), find_quote()
    layout.rs                      layout presets
  rustler-registry/src/
    lib.rs, spec.rs, ctx.rs, envelope.rs, registry.rs, progress.rs
  rustler-tools/src/
    lib.rs, paths.rs, download.rs, ffmpeg_install.rs, ytdlp_install.rs, uv_install.rs,
    engine_install.rs, fonts.rs, doctor.rs, run.rs (subprocess helper)
  rustler-engine-bridge/src/
    lib.rs, client.rs, protocol.rs
  rustler-media/src/
    lib.rs, probe.rs, proxy.rs, frames.rs (see.*), tiles.rs, audio_analysis.rs,
    encoders.rs, compile.rs (filtergraph), render.rs, qc.rs
  rustler-captions/src/
    lib.rs, ass.rs, presets.rs, chunk.rs
  rustler-commands/src/
    lib.rs (register_all), setup_cmds.rs, project_cmds.rs, source_cmds.rs, analyze_cmds.rs,
    see_cmds.rs, transcript_cmds.rs, clip_cmds.rs, render_cmds.rs, qc_cmds.rs
  rustler-cli/src/
    main.rs, argmap.rs (generic flag -> JSON), output.rs
engine/                            Python package (embedded into the binary via include_dir)
  pyproject.toml                   project rustler-engine, requires-python ">=3.11,<3.13"
  rustler_engine/__main__.py       stdio JSON-RPC loop
  rustler_engine/rpc.py            framing, progress notifications
  rustler_engine/transcribe.py     faster-whisper wrapper, chunked + resumable
  rustler_engine/scenes.py         PySceneDetect wrapper
  rustler_engine/translit.py       Devanagari -> Hinglish Latin
  tests/test_*.py
tests/e2e/slice1.rs (in rustler-cli/tests/)   end-to-end flow
```

---

## Wave 0 — Frozen contracts (sequential, Opus, one agent)

### Task 0: Repo init and workspace skeleton

**Files:**
- Create: `.gitignore`, `Cargo.toml`, `THIRD_PARTY_NOTICES.md`, every crate's `Cargo.toml` + `src/lib.rs` (or `main.rs`) with the module declarations listed in File Structure.

- [ ] **Step 1:** `git init` in `C:\Users\krishna\Desktop\rustler`; `.gitignore` contains `target/`, `repos/`, `**/__pycache__/`, `engine/.venv/`, `*.mp4` outside `tests/fixtures/`.
- [ ] **Step 2:** Workspace `Cargo.toml`: `resolver = "3"`, `members = ["crates/*"]`, `[workspace.package] edition = "2024"`, `[workspace.dependencies]` added with `cargo add --workspace`-equivalent entries for every dependency in Tech Stack (use the current versions `cargo add` resolves; do not hand-type versions).
- [ ] **Step 3:** Each module file exists; public items from the Interfaces blocks of Tasks 1–2 exist with `todo!()` bodies where Wave 1 fills them.
- [ ] **Step 4:** Run `cargo build --workspace` → compiles (warnings ok).
- [ ] **Step 5:** Commit `chore: workspace skeleton`.

### Task 1: Core data contracts (`rustler-core` models, ids, errors, events)

**Files:**
- Create: `crates/rustler-core/src/{ids.rs,error.rs,events.rs,model/*.rs}`
- Test: `crates/rustler-core/tests/contracts.rs`

**Interfaces (Produces — frozen after this task; changing them requires stopping and reporting):**
```rust
// ids.rs — newtypes over String, Display/FromStr/Serialize as plain string
pub struct ProjectId(String); pub struct SourceId(String); pub struct ClipId(String);
pub struct RenderId(String); pub struct SegmentId(String);
impl SourceId { pub fn new() -> Self } // "src_" + lowercase ULID; Clip "clp_", Render "rnd_", Project "prj_", Segment "seg_"

// error.rs
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)] #[serde(rename_all="snake_case")]
pub enum ErrorCode { Usage, Validation, NotFound, DependencyMissing, Provider, Media, QcFailed, Cancelled, RevisionConflict, Internal }
impl ErrorCode { pub fn exit_code(self) -> i32 } // 2,2,3,4,5,6,7,8,9,10
#[derive(thiserror::Error, Serialize, Debug, Clone)] #[error("{code:?}: {message}")]
pub struct RustlerError { pub code: ErrorCode, pub stage: String, pub message: String, pub hint: Option<String>, pub retryable: bool }
impl RustlerError { pub fn new(code: ErrorCode, stage: &str, message: impl Into<String>) -> Self; pub fn hint(self, h: impl Into<String>) -> Self; pub fn retryable(self) -> Self }
pub type Result<T> = std::result::Result<T, RustlerError>;

// model/project.rs
pub struct Project { pub schema: String /* "rustler.project@1" */, pub id: ProjectId, pub name: String, pub revision: u64,
  pub created_at: DateTime<Utc>, pub sources: Vec<SourceId>, pub clips: Vec<ClipId>, pub permissions: Permissions, pub settings: ProjectSettings }
pub struct Permissions { pub network: bool /*true*/, pub downloads: bool /*true*/, pub fs_roots: Vec<PathBuf> /*empty = any*/, pub cookies_file: Option<PathBuf>, pub max_spend_usd: f64 /*0.0*/ }
pub struct ProjectSettings { pub canvas: Canvas, pub caption_script: CaptionScript, pub default_whisper_model: String /*"small"*/ }

// model/source.rs
pub struct TimeRange { pub start: f64, pub end: f64 }
pub enum SourceOrigin { File { path: PathBuf }, Url { url: String, section: Option<TimeRange>, original_offset: f64 } }
pub struct MediaProbe { pub duration: f64, pub has_video: bool, pub has_audio: bool, pub width: u32, pub height: u32, /* display dims after rotation */
  pub rotation: i32, pub fps: f64, pub video_codec: Option<String>, pub audio_codec: Option<String>, pub sample_rate: Option<u32>, pub channels: Option<u32> }
pub struct Source { pub id: SourceId, pub name: String, pub origin: SourceOrigin, pub media_path: PathBuf, pub probe: MediaProbe,
  pub proxy: Option<PathBuf>, pub audio_wav: Option<PathBuf>, pub imported_at: DateTime<Utc> }   // paths relative to project dir when inside it

// model/transcript.rs
pub struct Word { pub i: u32, pub text: String, pub start: f64, pub end: f64, pub conf: Option<f32>, pub speaker: Option<String> }
pub struct Transcript { pub schema: String /* "rustler.transcript@1" */, pub source_id: SourceId, pub language: String, pub engine: String /* "faster-whisper:<model>" | "youtube:<kind>" */,
  pub script: CaptionScript, pub words: Vec<Word>, pub word_timing: WordTiming }
pub enum WordTiming { Exact, Approximate } // youtube captions = Approximate
pub enum UnitKind { Sentence, Clause }
pub struct Unit { pub id: u32, pub kind: UnitKind, pub word_start: u32, pub word_end: u32 /* inclusive */, pub start: f64, pub end: f64, pub text: String, pub boundary_strength: f32 }

// model/analysis.rs
pub struct Scene { pub start: f64, pub end: f64 }
pub struct LoudnessPoint { pub t: f64, pub lufs: f64 }
pub struct Silence { pub start: f64, pub end: f64 }
pub struct AudioEvents { pub curve: Vec<LoudnessPoint> /* 1 s buckets, momentary */, pub peaks: Vec<LoudnessPoint>, pub silences: Vec<Silence> }

// model/clip.rs
pub struct Canvas { pub width: u32 /*1080*/, pub height: u32 /*1920*/, pub fps: f64 /*30.0*/ }
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 } // normalized 0..1
pub enum Fit { Cover, Contain }
pub enum Background { Black, BlurFill }
pub struct Region { pub id: String, pub src: Rect, pub dst: Rect, pub z: i32, pub fit: Fit }
pub struct Layout { pub preset: Option<String>, pub regions: Vec<Region>, pub background: Background }
pub struct Segment { pub id: SegmentId, pub source_id: SourceId, pub start: f64, pub end: f64, pub layout: Layout }
pub enum CaptionScript { Latin, Devanagari, AsTranscribed }
pub struct CaptionTrack { pub enabled: bool, pub preset: String /*"bold-karaoke"*/, pub script: CaptionScript, pub position_y: f64 /*0.70*/, pub max_words_per_line: u32 /*3*/ }
pub struct AudioSettings { pub loudnorm: bool /*true*/, pub target_lufs: f64 /*-14*/, pub true_peak: f64 /*-2*/, pub lra: f64 /*11*/ }
pub struct Clip { pub schema: String /* "rustler.clip@1" */, pub id: ClipId, pub name: String, pub revision: u64, pub canvas: Canvas,
  pub segments: Vec<Segment>, pub captions: CaptionTrack, pub audio: AudioSettings, pub renders: Vec<RenderRecord>, pub created_at: DateTime<Utc> }
pub struct RenderRecord { pub id: RenderId, pub path: PathBuf, pub quality: RenderQuality, pub encoder: String, pub created_at: DateTime<Utc>, pub qc: Option<PathBuf> }
pub enum RenderQuality { Draft, Final }

// model/qc.rs
pub enum QcStatus { Pass, Warn, Fail }
pub struct QcCheck { pub name: String, pub status: QcStatus, pub value: serde_json::Value, pub expected: String, pub fix_hint: Option<String> }
pub struct QcReport { pub schema: String /* "rustler.qc@1" */, pub clip_id: ClipId, pub render_id: RenderId, pub passed: bool, pub checks: Vec<QcCheck>, pub contact_sheet: PathBuf }

// events.rs
pub enum Actor { Agent, Human, System }
pub struct Event { pub ts: DateTime<Utc>, pub actor: Actor, pub kind: String, pub project_revision: u64, pub data: serde_json::Value }
```
All types derive `Serialize, Deserialize, Clone, Debug, JsonSchema`; enums `#[serde(rename_all = "snake_case")]`; `SourceOrigin` is `#[serde(tag = "kind")]`; structs with defaults implement `Default` with the values in comments.

- [ ] **Step 1: Write failing tests** in `contracts.rs`:
  - `ids_have_prefixes`: `SourceId::new().to_string().starts_with("src_")` and length 30; same for clp_/rnd_/prj_/seg_.
  - `error_exit_codes`: `ErrorCode::Validation.exit_code()==2`, `NotFound==3`, `DependencyMissing==4`, `Provider==5`, `Media==6`, `QcFailed==7`, `Cancelled==8`, `RevisionConflict==9`, `Internal==10`.
  - `defaults_match_spec`: `Canvas::default()` = 1080/1920/30.0; `AudioSettings::default()` = true/−14/−2/11; `CaptionTrack::default()` = enabled, "bold-karaoke", Latin, 0.70, 3.
  - `clip_roundtrips_json`: build a Clip with one Segment and 2 regions → `serde_json::to_string` → back → equal; JSON has `"script":"latin"` and origin `"kind":"url"`.
  - `schema_generates`: `schemars::schema_for!(Clip)` serializes to JSON containing `"segments"`.
- [ ] **Step 2:** `cargo test -p rustler-core --test contracts` → FAIL (missing items).
- [ ] **Step 3:** Implement the types above.
- [ ] **Step 4:** Run tests → PASS.
- [ ] **Step 5:** Commit `feat(core): frozen data contracts`.

### Task 2: Registry contract + store/timemap/units/layout signatures

**Files:**
- Create: `crates/rustler-registry/src/*`, signatures-only in `crates/rustler-core/src/{store.rs,timemap.rs,units.rs,layout.rs}` (bodies `todo!()`), `engine/PROTOCOL.md`
- Test: `crates/rustler-registry/tests/registry.rs`

**Interfaces (Produces — frozen):**
```rust
// rustler-registry
pub struct Annotations { pub read_only: bool, pub destructive: bool, pub network: bool, pub costs_money: bool, pub long_running: bool }
pub struct Example { pub description: &'static str, pub cli: &'static str }
pub struct CommandSpec { pub name: &'static str /* "group.action" */, pub summary: &'static str, pub positional: &'static [&'static str],
  pub input_schema: serde_json::Value, pub output_schema: serde_json::Value, pub annotations: Annotations, pub examples: Vec<Example> }
#[async_trait] pub trait Command: Send + Sync { fn spec(&self) -> CommandSpec; async fn run(&self, ctx: &Ctx, input: serde_json::Value) -> rustler_core::Result<serde_json::Value>; }
/// Builds a Command from typed input/output; deserialization errors become ErrorCode::Validation with the serde message.
pub fn typed<I, O, F, Fut>(name: &'static str, summary: &'static str, positional: &'static [&'static str], ann: Annotations, examples: Vec<Example>, f: F) -> Box<dyn Command>
  where I: DeserializeOwned + JsonSchema + Send, O: Serialize + JsonSchema, F: Fn(Ctx, I) -> Fut + Send + Sync + 'static, Fut: Future<Output = rustler_core::Result<O>> + Send;
pub struct Registry { /* BTreeMap<&'static str, Box<dyn Command>> */ }
impl Registry { pub fn new() -> Self; pub fn register(&mut self, c: Box<dyn Command>); pub fn list(&self) -> Vec<CommandSpec>; pub fn describe(&self, name: &str) -> Option<CommandSpec>;
  pub async fn dispatch(&self, name: &str, input: serde_json::Value, ctx: Ctx) -> rustler_core::Result<serde_json::Value> }
#[derive(Clone)] pub struct Ctx { pub project_dir: Option<PathBuf>, pub home: PathBuf /* RUSTLER_HOME */, pub actor: Actor, pub dry_run: bool, pub yes: bool,
  pub progress: ProgressSink, pub cancel: tokio_util::sync::CancellationToken, pub max_seconds: Option<u64> }
impl Ctx { pub fn store(&self) -> rustler_core::Result<ProjectStore> /* NotFound "no project" with hint "run `rustler project create` or pass --project" */ }
#[derive(Clone)] pub struct ProgressSink { /* Arc<dyn Fn(ProgressEvent)> */ }
pub struct ProgressEvent { pub stage: String, pub pct: Option<f32>, pub message: Option<String> }
impl ProgressSink { pub fn stderr_ndjson() -> Self; pub fn noop() -> Self; pub fn emit(&self, e: ProgressEvent) }
pub enum Envelope { Ok { schema: String, data: serde_json::Value, warnings: Vec<String> }, Err { error: RustlerError } }
impl Envelope { pub fn ok(command: &str, data: serde_json::Value) -> Self /* schema = format!("rustler.{command}@1") */; pub fn to_json(&self) -> String; pub fn exit_code(&self) -> i32 }
// Warnings: commands put a "warnings": [..] array inside their output object; Envelope::ok moves it to the envelope level.

// rustler-core::store
pub struct ProjectStore { pub root: PathBuf }
impl ProjectStore {
  pub fn create(root: &Path, name: &str) -> Result<(Self, Project)>;          // makes dirs: sources/ analysis/ clips/ exports/ history/ ; fails Validation if project.json exists
  pub fn open(root: &Path) -> Result<Self>;                                    // NotFound if no project.json
  pub fn load_project(&self) -> Result<Project>;
  pub fn save_project(&self, p: &mut Project, base_revision: u64) -> Result<()>; // RevisionConflict if on-disk revision != base_revision; increments revision
  pub fn load_source(&self, id: &SourceId) -> Result<Source>; pub fn save_source(&self, s: &Source) -> Result<()>;
  pub fn load_clip(&self, id: &ClipId) -> Result<Clip>; pub fn save_clip(&self, c: &mut Clip, base_revision: u64) -> Result<()>;
  pub fn source_dir(&self, id: &SourceId) -> PathBuf; pub fn analysis_dir(&self, id: &SourceId) -> PathBuf; pub fn clip_dir(&self, id: &ClipId) -> PathBuf; pub fn exports_dir(&self) -> PathBuf;
  pub fn read_json<T: DeserializeOwned>(&self, rel: &Path) -> Result<T>; pub fn write_json<T: Serialize>(&self, rel: &Path, v: &T) -> Result<()>; // atomic: write tmp in same dir, fsync, rename
  pub fn append_event(&self, e: &Event) -> Result<()>;                         // one JSON line to events.ndjson
  pub fn resolve(&self, p: &Path) -> PathBuf; pub fn relativize(&self, p: &Path) -> PathBuf; // project-relative paths when inside root
}
// rustler-core::timemap
pub struct TimeMap { /* per segment: source_id, src_start, src_end, out_start */ }
impl TimeMap { pub fn from_segments(segs: &[Segment]) -> Self; pub fn duration(&self) -> f64;
  pub fn to_output(&self, source_id: &SourceId, t: f64) -> Option<f64>; pub fn to_source(&self, out_t: f64) -> Option<(SourceId, f64)> }
// rustler-core::units
pub fn build_units(words: &[Word]) -> Vec<Unit>;
pub fn find_quote(words: &[Word], from_word: u32, quote: &str) -> bool; // normalized (lowercase, strip punctuation, collapse spaces) prefix match of the words starting at from_word; strsim::normalized_levenshtein >= 0.8
// rustler-core::layout
pub fn preset(name: &str, src_w: u32, src_h: u32, canvas: &Canvas, params: &LayoutParams) -> Result<Layout>;
pub struct LayoutParams { pub focus_x: Option<f64> /* 0..1 center for center-crop, default 0.5 */, pub top: Option<Rect>, pub bottom: Option<Rect>, pub split: Option<f64> /* top height fraction, default 0.5 */ }
pub const PRESETS: &[&str] = &["center-crop", "blur-fit", "split-vertical"];
```

`engine/PROTOCOL.md` (frozen): line-delimited JSON-RPC 2.0 on stdin/stdout; stdout carries only protocol lines; logs to stderr. Requests `{"jsonrpc":"2.0","id":N,"method":…,"params":{…}}`. Notifications from engine: `{"jsonrpc":"2.0","method":"progress","params":{"id":N,"stage":…,"pct":…}}`. Errors: `{"code": -32000, "message": …, "data": {"kind": "dependency_missing"|"media"|"validation"|"internal"}}`. Methods:
- `engine.ping {}` → `{"version": "<pkg version>", "python": "3.11.x"}`
- `engine.capabilities {}` → `{"cuda": bool, "packages": {"faster_whisper": "x.y", …}}`
- `transcribe.run {"audio_path", "model", "language": "auto"|"en"|"hi", "device": "auto"|"cpu"|"cuda", "parts_dir", "chunk_seconds": 600, "max_seconds": null|int}` → `{"language", "words": [{"text","start","end","conf"}], "complete": bool}`
- `scenes.detect {"video_path", "threshold": 27.0}` → `{"scenes": [{"start","end"}]}`
- `translit.to_latin {"texts": [..]}` → `{"texts": [..]}`
- `models.fetch {"whisper": ["small"]}` → `{"fetched": [..]}`

- [ ] **Step 1: Failing tests** `registry.rs`:
  - `typed_command_validates_input`: register `typed::<In{a:u32},Out{b:u32}>("test.double", …)`; dispatch `{"a":"x"}` → Err code Validation; `{"a":2}` → `{"b":4}`.
  - `list_and_describe`: `list()` contains "test.double"; `describe("test.double").unwrap().input_schema["properties"]["a"]` exists; `describe("nope")` is None; `dispatch("nope")` → NotFound with hint containing "rustler commands list".
  - `envelope_shapes`: `Envelope::ok("source.import", json!({"x":1,"warnings":["w"]})).to_json()` parses to `{"ok":true,"schema":"rustler.source.import@1","data":{"x":1},"warnings":["w"]}`; error envelope `exit_code()` equals `code.exit_code()`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement registry (store/timemap/units/layout stay `todo!()` — Wave 1 Task 3). **Step 4:** Run → PASS; `cargo build --workspace` passes.
- [ ] **Step 5:** Commit `feat(registry): command registry contract + frozen signatures`.

---

## Wave 1 — parallel (each agent: own `CARGO_TARGET_DIR`, only its files)

### Task 3: Core implementations — store, timemap, units, layout  *(Sonnet)*

**Files:** Modify bodies in `crates/rustler-core/src/{store.rs,timemap.rs,units.rs,layout.rs}`. Test: `crates/rustler-core/tests/{store.rs,timemap.rs,units.rs,layout.rs}`.
**Interfaces:** Consumes Task 1 types; Produces the Task 2 signatures (no changes).

- [ ] **Step 1: Failing tests**
  - store: `create_makes_layout` (dirs sources/analysis/clips/exports/history exist, project.json revision 0); `create_twice_fails_validation`; `save_with_stale_revision_conflicts` (load p, save with base 0 → rev 1; second save with base 0 → Err RevisionConflict); `atomic_write_survives_partial` (a leftover `project.json.tmp*` file from a crash does not affect `load_project`, and next save succeeds); `events_append_lines` (2 appends → 2 parseable lines); `relativize_inside_root` (`root/sources/x.mp4` → `sources/x.mp4`; path outside root unchanged).
  - timemap: two segments A[10,20) and B[50,55) → `duration()==15`; `to_output(A,12)==Some(2)`; `to_output(B,51)==Some(11)`; `to_output(A,25)==None`; `to_source(12.0)==Some((B,51.0))`.
  - units: words "hello there. how are you?" with times → 2 Sentence units, unit 0 word_start 0 word_end 1, boundary_strength 1.0; a 0.8 s pause without punctuation splits a sentence (strength 0.8); a 30-word sentence with a comma at word 14 yields Clause units split at the comma (strength 0.5); Devanagari danda `।` ends a sentence; `build_units(&[])` is empty. `find_quote`: "So my fellow Americans" matches words "And so my fellow Americans," from index 1 (true) and fails at index 5 (false).
  - layout: source 1920×1080, canvas 1080×1920: `center-crop` → one region src w = 0.3164 (±0.001, i.e. 607.5/1920), h 1.0, x centered at focus 0.5, dst full; `blur-fit` → background BlurFill, region dst h = 0.3164 (1080·(1080/1920)/1920 ≈ 0.3164) centered, fit Contain; `split-vertical` with top=(0.75,0,0.25,0.33) bottom=(0,0,0.75,1) split 0.5 → two regions, dst (0,0,1,0.5) and (0,0.5,1,0.5), fit Cover; missing `top` → Err Validation "split-vertical needs --top and --bottom"; unknown preset → Err Validation listing `PRESETS`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. Unit rules: sentence ends on `. ? ! । …` at word end or gap ≥ 0.7 s to next word; sentences > 25 words split at the last `, ; :` before word 25 (else hard split at 25) as Clause units; `text` = words joined by single spaces. **Step 4:** PASS. **Step 5:** Commit `feat(core): store, timemap, units, layout presets`.

### Task 4: Tools — `setup`, `doctor`, subprocess helper  *(Sonnet)*

**Files:** `crates/rustler-tools/src/*`, `crates/rustler-commands/src/setup_cmds.rs`. Test: `crates/rustler-tools/tests/{paths.rs,doctor.rs}`, `crates/rustler-tools/tests/setup_live.rs` (`#[ignore]`, network).

**Interfaces (Produces):**
```rust
pub struct ToolPaths { pub home: PathBuf, pub bin: PathBuf, pub ffmpeg: PathBuf, pub ffprobe: PathBuf, pub ytdlp: PathBuf, pub uv: PathBuf,
  pub engine_dir: PathBuf /* home/engine */, pub venv_python: PathBuf, pub models: PathBuf, pub fonts: PathBuf }
impl ToolPaths { pub fn from_home(home: &Path) -> Self; pub fn default_home() -> PathBuf /* RUSTLER_HOME or ~/.rustler */; pub fn require(&self, which: Tool) -> rustler_core::Result<PathBuf> /* DependencyMissing, hint "run `rustler setup`" */ }
pub enum Tool { Ffmpeg, Ffprobe, Ytdlp, Uv, EnginePython }
pub async fn run(cmd: &Path, args: &[OsString], cwd: Option<&Path>, cancel: &CancellationToken) -> rustler_core::Result<Output>; // Media/DependencyMissing errors carry last 20 stderr lines in message
pub async fn run_streaming(cmd: &Path, args: &[OsString], cwd: Option<&Path>, cancel: &CancellationToken, on_stderr_line: impl FnMut(&str)) -> rustler_core::Result<ExitStatus>;
pub struct SetupOptions { pub profile: Profile /* Lite|Full */, pub gpu: GpuMode /* Auto|Cuda|None */, pub whisper_models: Vec<String> /* ["small"] */, pub force: bool }
pub async fn setup(paths: &ToolPaths, opts: &SetupOptions, progress: &ProgressSink) -> rustler_core::Result<SetupReport>;
pub struct SetupReport { pub installed: Vec<Component>, pub skipped: Vec<Component>, pub failed: Vec<(String, String)> }
pub struct Component { pub name: String, pub version: String, pub path: PathBuf, pub sha256: Option<String> }
pub async fn doctor(paths: &ToolPaths) -> DoctorReport;
pub struct DoctorReport { pub os: String, pub arch: String, pub ffmpeg: Option<ToolInfo>, pub ffprobe: Option<ToolInfo>, pub ytdlp: Option<ToolInfo>, pub uv: Option<ToolInfo>,
  pub libass: bool, pub encoders: Vec<EncoderStatus>, pub nvidia_gpu: Option<String>, pub engine: Option<serde_json::Value> /* engine.capabilities */, pub fonts: Vec<String>, pub ready: bool, pub missing: Vec<String> }
```
Commands registered: `setup.run` (input `{profile?, gpu?, whisper_models?, force?}`, annotations network+long_running), `doctor.run` (read_only).

Setup steps (idempotent; each skipped when manifest `home/manifest.json` shows it installed and the file exists):
1. FFmpeg: BtbN GitHub release `latest`, asset `ffmpeg-master-latest-win64-gpl.zip` (Windows) / `ffmpeg-master-latest-linux64-gpl.tar.xz` (Linux); verify against that release's `checksums.sha256`; extract `ffmpeg`/`ffprobe` to `home/bin`.
2. yt-dlp: GitHub `yt-dlp/yt-dlp` latest, `yt-dlp.exe` / `yt-dlp_linux`; verify against `SHA2-256SUMS`; `chmod +x` on Unix.
3. uv: GitHub `astral-sh/uv` latest standalone archive for the platform (`uv-x86_64-pc-windows-msvc.zip` / `uv-x86_64-unknown-linux-gnu.tar.gz`) with its `.sha256`; put `uv` in `home/bin`. Env for all uv calls: `UV_PYTHON_INSTALL_DIR=home/python`, `UV_CACHE_DIR=home/uv-cache`.
4. Engine: extract embedded `engine/` (via `include_dir!("$CARGO_MANIFEST_DIR/../../engine")`, excluding tests/.venv) to `home/engine`; `uv python install 3.11`; `uv sync --project home/engine` (+ `--extra full` for Full; `--extra cuda` when gpu Cuda/Auto-detected NVIDIA).
5. Models: `HF_HOME=home/models/hf`; call engine `models.fetch {"whisper": opts.whisper_models}`.
6. Fonts: download from `https://github.com/google/fonts/raw/main/ofl/<family>/…` — Anton-Regular.ttf, Montserrat (variable `Montserrat[wght].ttf`), NotoSansDevanagari (variable) — each with its `OFL.txt` into `home/fonts/<family>/`.
Downloads stream to `*.part` then rename; resume skip by size+sha.

- [ ] **Step 1: Failing tests:** `paths_respect_rustler_home` (env RUSTLER_HOME=tmp → `ffmpeg == tmp/bin/ffmpeg(.exe)`); `require_missing_is_dependency_missing` (code DependencyMissing, hint contains "rustler setup"); `doctor_on_empty_home_not_ready` (ready=false, missing contains "ffmpeg","engine"); `run_captures_stderr_tail_on_failure` (run `ffprobe`-like failing command — use `cargo`'s own binary path `std::env::current_exe()` with bad args is not portable; use `rustc --definitely-bad-flag` → Err whose message contains "error"); `setup_live` (`#[ignore]`): full lite setup into tmp home then `doctor` ready=true, libass=true.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. Encoder probe (used by doctor and Task 8 via `rustler_media::encoders`, but doctor calls it): `ffmpeg -hide_banner -f lavfi -i color=c=black:s=256x256:d=0.1 -frames:v 1 -c:v <enc> -f null -` for `h264_nvenc, hevc_nvenc, h264_qsv, h264_amf, h264_videotoolbox, libx264`; libass = `ffmpeg -hide_banner -filters` contains ` ass `. **Step 4:** PASS (+ run `setup_live` manually once with `--ignored` on this machine; record result in progress log). **Step 5:** Commit `feat(tools): setup, doctor, subprocess runner`.

### Task 5: Python engine + bridge  *(Sonnet)*

**Files:** `engine/**`, `crates/rustler-engine-bridge/src/*`. Test: `engine/tests/test_rpc.py`, `engine/tests/test_translit.py`, `engine/tests/test_transcribe.py` (marked slow), `crates/rustler-engine-bridge/tests/bridge.rs`.

**Interfaces (Produces):**
```rust
pub struct EngineClient { /* child process, pending map, reader task */ }
impl EngineClient {
  pub async fn spawn(paths: &ToolPaths) -> rustler_core::Result<Self>; // `<venv_python> -m rustler_engine`, cwd home/engine, env HF_HOME; DependencyMissing if venv missing
  pub async fn spawn_with(python: &Path, engine_dir: &Path, env: &[(String,String)]) -> rustler_core::Result<Self>; // used by tests with `uv run`
  pub async fn call<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P, progress: &ProgressSink, cancel: &CancellationToken) -> rustler_core::Result<R>;
  pub async fn shutdown(self);
}
// engine error kinds map: dependency_missing→DependencyMissing, media→Media, validation→Validation, other→Internal. Child exit mid-call → Internal "engine crashed" + last stderr lines, retryable.
```
Python (`pyproject.toml`): deps `faster-whisper`, `scenedetect`, `opencv-python-headless`, `indic-transliteration`, `numpy`; optional `full = []` (reserved), `cuda = ["nvidia-cublas-cu12", "nvidia-cudnn-cu12"]`; dev `pytest`. Versions: let `uv add` resolve current.

Transcribe behavior: split audio into `chunk_seconds` windows by reading with faster-whisper's decode on offsets (or ffmpeg-free slicing via `faster_whisper.decode_audio` + numpy slicing); each finished chunk written to `parts_dir/NNNN.json`; existing parts are skipped (resume); `word_timestamps=True`, `vad_filter=True`; device auto → cuda if `ctranslate2.get_cuda_device_count()>0` (compute_type float16) else cpu int8; stops early when `max_seconds` exceeded and returns `complete: false`. Words from all parts concatenated in time order.

Translit: Devanagari tokens → Hinglish Latin via indic-transliteration (DEVANAGARI→ITRANS) + normalization table (lowercase; `aa→a` only word-internal where common spelling expects it; drop final inherent schwa; nasal `M`/`.n`→`n`; `I/ii→i`, `U/uu→u`, `RRi→ri`, `~N`→`n`). Non-Devanagari text passes through unchanged.

- [ ] **Step 1: Failing tests:**
  - `test_rpc.py::test_ping_and_unknown_method`: run the loop with in-memory streams → `engine.ping` returns version; unknown method → error code -32601.
  - `test_translit.py::test_common_hinglish`: `क्या→kya`, `है→hai`, `नहीं→nahi`, `भाई→bhai`, `मैं→main`, `ज़िंदगी→zindagi`, `"bro क्या scene है"→"bro kya scene hai"`.
  - `test_transcribe.py::test_jfk_words` (`@pytest.mark.slow`): download `https://github.com/ggml-org/whisper.cpp/raw/master/samples/jfk.wav` (MIT repo sample) to tmp → `transcribe.run` model `tiny` → words contain "Americans", first word start < 1.0, all `end >= start`; second call with same parts_dir does no re-decoding (parts reused — assert part file mtime unchanged).
  - `bridge.rs::ping_roundtrip` (skips with message if `uv` not on PATH or `RUSTLER_HOME` engine missing): `spawn_with(uv run --project engine python)` → `call("engine.ping")` ok; `call("nope")` → Err; kill child mid-call → Err Internal retryable.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** `cd engine && uv run pytest -q` (and `-m slow` once) + `cargo test -p rustler-engine-bridge` → PASS. **Step 5:** Commit `feat(engine): python worker + rust bridge`.

### Task 6: Media basics — probe, proxy/audio, see.*, audio analysis  *(Sonnet)*

**Files:** `crates/rustler-media/src/{probe.rs,proxy.rs,frames.rs,tiles.rs,audio_analysis.rs}`, `crates/rustler-media/tests/{probe.rs,see.rs,audio.rs}`, `crates/rustler-media/tests/support/mod.rs` (fixture generator).

**Interfaces (Produces):**
```rust
pub async fn probe(paths: &ToolPaths, media: &Path) -> rustler_core::Result<MediaProbe>; // ffprobe -v error -print_format json -show_streams -show_format; rotation from side_data/tags; display dims swap on ±90
pub async fn make_proxy(paths: &ToolPaths, src: &Path, out: &Path, cancel: &CancellationToken, progress: &ProgressSink) -> rustler_core::Result<()>; // 540p max height, libx264 veryfast crf 30, aac 96k, keyint 30; skipped if out newer than src
pub async fn extract_audio_wav(paths: &ToolPaths, src: &Path, out: &Path, cancel: &CancellationToken) -> rustler_core::Result<()>; // 16 kHz mono s16 wav
pub async fn frame(paths: &ToolPaths, video: &Path, t: f64, width: u32, out: &Path) -> rustler_core::Result<()>; // -ss before -i, 1 frame, jpg q 3
pub struct TileSpec { pub start: f64, pub end: f64, pub count: u32 /* default 12 */, pub cols: u32 /* 4 */, pub tile_width: u32 /* 480 */ }
pub struct TileLabel { pub t: f64, pub text: String } // "00:47:05.2 · words spoken around t"
pub async fn tiles(paths: &ToolPaths, video: &Path, spec: &TileSpec, labels: &dyn Fn(f64) -> String, font: &Path, out: &Path) -> rustler_core::Result<Vec<TileLabel>>; // evenly spaced times; compose with image+imageproc, label strip 36 px under each tile, ab_glyph font
pub async fn preview(paths: &ToolPaths, video: &Path, start: f64, end: f64, height: u32, out: &Path) -> rustler_core::Result<()>; // 360p mp4, max 60 s → Validation if longer
pub async fn analyze_audio(paths: &ToolPaths, media: &Path, cancel: &CancellationToken, progress: &ProgressSink) -> rustler_core::Result<AudioEvents>;
  // one ffmpeg pass: -af "ebur128=metadata=1,ametadata=print:key=lavfi.r128.M:file=-" style or parse ebur128 stderr "t: … M: …" lines (10 Hz) → 1 s buckets (max M);
  // peaks = buckets ≥ median + 6 LU that are local maxima within ±5 s; silences via a second filter in the same graph: silencedetect=n=-35dB:d=0.6 parsed from stderr
```
Test fixtures (`support/mod.rs`, generated with the installed ffmpeg into a tmp dir; tests skip with a printed reason if `rustler doctor` tools are missing): `make_video(path, secs, w, h, with_audio)` = `testsrc2` + `sine=frequency=440` (audio: 2 s tone, 2 s silence, repeat); `make_rotated(path)` = 1920×1080 testsrc with `-metadata:s:v rotate=90` (or `-display_rotation 90` on new ffmpeg); `make_audio_only(path)`.

- [ ] **Step 1: Failing tests:** `probe_basic` (1280×720 10 s with audio → duration 10±0.1, has_audio, fps 25±0.1); `probe_reports_display_dims_for_rotated` (width 1080, height 1920, rotation ±90); `probe_audio_only` (has_video false); `frame_and_tiles` (frame jpg exists >1 KB; tiles 12 count, cols 4 → image width 1920, returned labels times ascending within [start,end]); `preview_rejects_over_60s` (Validation); `audio_events_find_silences` (fixture alternating tone/silence 20 s → ≥ 4 silences each ≈2 s ±0.3; curve length ≈20).
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS. **Step 5:** Commit `feat(media): probe, proxy, see, audio analysis`.

### Task 7: Captions v1 — ASS writer  *(Sonnet)*

**Files:** `crates/rustler-captions/src/{ass.rs,presets.rs,chunk.rs}`, test `crates/rustler-captions/tests/ass.rs`.

**Interfaces (Produces):**
```rust
pub struct CaptionWord { pub text: String, pub out_start: f64, pub out_end: f64 } // already in output time, already in target script
pub fn chunk_words(words: &[CaptionWord], max_words: u32, max_gap: f64 /* 0.6 */) -> Vec<Vec<CaptionWord>>; // new chunk at max_words, gap > max_gap, or sentence-final punctuation
pub fn write_ass(words: &[CaptionWord], track: &CaptionTrack, canvas: &Canvas, font_family: &str) -> rustler_core::Result<String>; // Validation for unknown preset
pub const PRESETS: &[&str] = &["bold-karaoke"];
```
Preset `bold-karaoke` (values copied into code): PlayResX = canvas.width, PlayResY = canvas.height; font `Anton`; size = round(canvas.width × 0.072) (78 at 1080); primary `&H00FFFFFF`; outline `&H00000000` width 6; shadow 0; bold 0; alignment 2 (bottom-center); MarginV = round(canvas.height × (1 − position_y)); uppercase text. Karaoke: for each chunk, one Dialogue per word interval: the whole chunk text with the active word wrapped `{\c&H0000E0FF&}WORD{\r}` (yellow #FFE000 in ASS BGR), event start = word.out_start, end = next word start (last word: chunk end + 0.15 capped by next chunk start). No size change on the active word. Times formatted `H:MM:SS.cc`. Text escaping: `{`,`}` and `\` removed; line breaks never inserted inside a chunk (≤ max_words_per_line words).

- [ ] **Step 1: Failing tests:** `header_values` (contains `PlayResX: 1080`, `PlayResY: 1920`, `Style: Default,Anton,78,`); `one_event_per_word` (6 words, max 3 → 2 chunks → 6 Dialogue lines; 2nd line text contains `{\c&H0000E0FF&}` around the 2nd word, uppercased); `gap_splits_chunk` (gap 1.0 s between word 1 and 2 → chunks of 1 and rest); `time_format` (`3725.456` → `1:02:05.46`); `unknown_preset_validation`.
- [ ] **Step 2–5:** FAIL → implement → PASS → commit `feat(captions): bold-karaoke ASS writer`.

### Task 8: CLI front — generic arg mapping and output  *(Sonnet)*

**Files:** `crates/rustler-cli/src/{main.rs,argmap.rs,output.rs}`, test `crates/rustler-cli/tests/argmap.rs`, `crates/rustler-cli/tests/cli.rs`.

**Interfaces:**
- Consumes: `Registry`, `Ctx`, `Envelope`; `rustler_commands::register_all(&mut Registry)` (Task 10 provides; until then a stub registering nothing is in Task 0 skeleton).
- Produces: CLI grammar (frozen):
  - `rustler <group> <action> [positional…] [--key value]… [--input-json <json | @file>]` → command `group.action` (multiword actions use `-` in CLI, `_` in names: `rustler source fetch-transcript` → `source.fetch_transcript`).
  - Global flags (anywhere): `--project <dir>`, `--pretty`, `--dry-run`, `--yes`, `--actor agent|human` (default agent), `--max-seconds N`, `-q`.
  - Built-ins: `rustler commands list` → `{"commands":[{name,summary,annotations}]}`; `rustler commands describe <name>` → full CommandSpec; `rustler help` → same as list plus a short "start here" text field: `"start": "rustler doctor → rustler setup → rustler project create --name X → rustler source import <file|url> → rustler analyze quick <source-id> → rustler transcript get <source-id> → rustler see tiles … → rustler clip create … → rustler render run <clip-id> → rustler qc run <clip-id>"`.
```rust
pub fn args_to_json(spec: &CommandSpec, positional: &[String], flags: &[(String, Option<String>)]) -> rustler_core::Result<serde_json::Value>;
// --kebab-key → snake_key; value parsed as JSON if it parses (numbers, bools, arrays, objects) else string; flag without value → true;
// repeated key → array; positional values assigned to spec.positional in order (extra → Usage error); --input-json merged first, flags override.
```
- [ ] **Step 1: Failing tests:** `kebab_to_snake_and_types` (`--start-unit 12 --pretty-name "a b" --loud` → `{"start_unit":12,"pretty_name":"a b","loud":true}`); `repeated_becomes_array` (`--sections 1:00-2:00 --sections 5:00-6:00` → array of 2 strings); `positional_mapping` (spec positional ["source_id"] + ["src_x"] → `{"source_id":"src_x"}`; two positionals → Usage); `input_json_file_merge` (`@file` with `{"a":1,"b":2}` + `--b 3` → `{"a":1,"b":3}`). `cli.rs` (uses `assert_cmd`): `rustler commands list` exits 0 and stdout parses with `ok:true`; `rustler nope run` exits 3 with error code `not_found`; stderr has no JSON envelope.
- [ ] **Step 2–5:** FAIL → implement (clap with `allow_external_subcommands`/trailing var args, then `args_to_json`) → PASS → commit `feat(cli): generic schema-driven CLI`.

---

## Wave 2 — parallel (after Wave 1 merged and `cargo test --workspace` green)

### Task 9: Render compiler + render  *(Opus)*

**Files:** `crates/rustler-media/src/{encoders.rs,compile.rs,render.rs}`, tests `crates/rustler-media/tests/{compile.rs,render.rs}`.

**Interfaces (Produces):**
```rust
pub struct SourceMedia { pub id: SourceId, pub path: PathBuf, pub probe: MediaProbe }
pub struct EncoderChoice { pub name: String /* "h264_nvenc" | "libx264" | … */, pub args: Vec<String> }
pub async fn pick_encoder(paths: &ToolPaths, quality: RenderQuality, prefer_gpu: bool) -> EncoderChoice;
  // order: h264_nvenc, h264_qsv, h264_amf, h264_videotoolbox, libx264; each must pass the 1-frame probe (cache result in home/encoders.json)
  // libx264 final: -preset medium -crf 20; draft: -preset veryfast -crf 28. nvenc final: -preset p5 -rc vbr -cq 23 -b:v 0; draft: -preset p2 -cq 30. Always -pix_fmt yuv420p.
pub struct CompiledRender { pub args: Vec<OsString> /* full ffmpeg argv */, pub filter_script: String, pub workdir: PathBuf }
pub fn compile(clip: &Clip, sources: &HashMap<SourceId, SourceMedia>, ass_file: Option<&Path>, fonts_dir: &Path, enc: &EncoderChoice, out: &Path, workdir: &Path) -> rustler_core::Result<CompiledRender>;
pub async fn render(paths: &ToolPaths, compiled: &CompiledRender, expected_duration: f64, cancel: &CancellationToken, progress: &ProgressSink) -> rustler_core::Result<()>;
  // runs ffmpeg with cwd=workdir and `-progress pipe:2`; pct = out_time / expected_duration; on failure delete partial output
pub async fn preview_frame(paths: &ToolPaths, compiled: &CompiledRender, out_t: f64, out_jpg: &Path) -> rustler_core::Result<()>; // same graph, `-ss out_t` after inputs on output side, -frames:v 1
```
Compiler rules (pure function — the heart of Slice 1):
- One input per segment: `-ss <start> -to <end> -i <path>` (input seeking). Inputs referenced by relative file names after copying/hard-linking? **No** — use absolute paths in argv (argv avoids escaping); only the ASS file and fonts dir are referenced inside the filter script, so they are written into `workdir` and referenced by bare relative names (`captions.ass`, `fontsdir=fonts`) with ffmpeg cwd = workdir (avoids Windows drive-colon escaping — autoclip trick, MIT).
- Filter script written to `workdir/graph.txt`, passed with `-filter_complex_script graph.txt` (supported in current ffmpeg; if `doctor` reports a version where it is removed, use `-/filter_complex graph.txt`).
- Per segment i: base canvas `color=c=black:s=WxH:r=FPS:d=<seg dur>` or, for BlurFill, `[i:v]scale=W/4:H/4:force_original_aspect_ratio=increase,crop=W/4:H/4,gblur=sigma=10,lutyuv=y=val*0.6,scale=W:H,setsar=1`; each region (sorted by z): `[i:v]crop=<src px>,scale=<dst px>:force_original_aspect_ratio=<increase|decrease>,crop=<dst px> (Cover) or pad (Contain)` overlaid at dst px; `fps=FPS,setsar=1,format=yuv420p`. Pixel values: src rect × display dims, dst rect × canvas, all rounded **down to even**.
- Audio per segment: `[i:a]aresample=48000,aformat=channel_layouts=stereo` or, if source has no audio, `anullsrc=r=48000:cl=stereo,atrim=duration=<seg dur>`; `asetpts=N/SR/TB`.
- Concat all: `concat=n=K:v=1:a=1`; then captions `ass=captions.ass:fontsdir=fonts` when `ass_file` is Some; audio `loudnorm=I=<target>:TP=<tp>:LRA=<lra>,aresample=48000` when enabled.
- Output: encoder args, `-c:a aac -b:a 160k -ar 48000`, `-movflags +faststart`, `-r FPS`, out path.
- Validation errors: no segments; segment end ≤ start; segment outside source duration (+0.05 tolerance); source has no video.

- [ ] **Step 1: Failing tests** (`compile.rs`, pure, no ffmpeg needed): `single_segment_center_crop_graph` (graph contains `crop=606:1080` for 1920×1080 source and canvas 1080×1920 center-crop — even rounding — and `concat=n=1:v=1:a=1` and `loudnorm=I=-14:TP=-2:LRA=11`); `blur_fill_graph_has_gblur`; `split_vertical_two_overlays` (two `overlay=` with y 0 and y 960); `no_audio_source_uses_anullsrc`; `captions_reference_relative_ass` (contains `ass=captions.ass:fontsdir=fonts`, no drive letter); `rejects_segment_out_of_bounds` (Validation). `render.rs` (needs tools; skip otherwise): `render_center_crop_10s` (fixture 1920×1080 10 s → output probe 1080×1920, duration 10±0.1, has_audio, h264+aac); `render_handles_awkward_filename` (source file `my clip (1) – टेस्ट.mp4`, ASS present → succeeds); `render_video_without_audio_gets_silent_track`; `preview_frame_matches_size` (jpg 1080×1920).
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS. **Step 5:** Commit `feat(media): filtergraph compiler and renderer`.

### Task 10: Commands — project, source, analyze, transcript, see, clip, render  *(Sonnet ×2: 10a = project/source/analyze/transcript; 10b = see/clip/render; disjoint files)*

**Files:** `crates/rustler-commands/src/{lib.rs,project_cmds.rs,source_cmds.rs,analyze_cmds.rs,transcript_cmds.rs}` (10a); `{see_cmds.rs,clip_cmds.rs,render_cmds.rs}` (10b); `lib.rs` `register_all` lists all modules (written in Task 0; 10a/10b only add their own `register(reg)` call lines — 10a owns lib.rs, 10b sends its lines to 10a / orchestrator). Tests: `crates/rustler-commands/tests/{source.rs,analyze.rs,clip.rs}`.

**Commands (input → output; every mutating command appends an Event and honors `--dry-run` by returning `{"dry_run":true,"would":…}` without writing):**
- `project.create {name, dir?}` → `{project_dir, project}`; `project.show {}` → project + counts.
- `source.import {input /*path or url*/, sections?: ["MM:SS-MM:SS"|"H:MM:SS-H:MM:SS"], name?}` → `{sources: [Source]}`. File: checks `permissions.fs_roots`; media stays in place (absolute path recorded); probe; proxy + wav into `sources/<id>/`. URL: requires `permissions.network && downloads`; yt-dlp `-f "bv*[height<=1080]+ba/b[height<=1080]" --merge-output-format mp4 -o sources/<id>/media.%(ext)s` (+ `--download-sections "*<range>"` per section → one Source per section, `original_offset` = section start; + `--cookies <file>` only if `permissions.cookies_file`).
- `source.info {url}` → `{title, duration, uploader, chapters:[{start,end,title}], heatmap:[{start,end,value}], subtitles:{manual:[langs], auto:[langs]}}` from `yt-dlp -J --skip-download`.
- `source.fetch_transcript {url, lang?: "auto"}` → saves a pending transcript under `analysis/url_<sha8>/transcript.json` (word_timing Approximate) and returns `{transcript_path, language, word_count, text_preview}`; picks manual subs before auto, prefers `en`, then `hi` (romanized via engine `translit.to_latin` when caption script latin). Uses `--write-subs --write-auto-subs --sub-format json3 --skip-download`; json3 parsing: each event's segs → words, word start = event tStartMs + seg tOffsetMs, end = next word start (or event end).
- `source.search {query, limit: 10}` → `{results:[{url,title,duration,channel}]}` (`ytsearchN:` + `--flat-playlist -J`).
- `source.list {}`, `source.show {source_id}`.
- `analyze.transcribe {source_id, model?, language?: "auto", device?: "auto"}` → runs engine on `audio_wav`, parts in `analysis/<id>/transcript.parts/`; romanizes Devanagari when project caption_script is latin (keeps original in a `raw` field per word? → no: original kept in `transcript.raw.json`); writes `transcript.json` + `units.json`; honors `--max-seconds` → `{complete:false, resume:"rerun the same command"}`.
- `analyze.scenes {source_id}` → engine on proxy → `scenes.json`; `analyze.audio {source_id}` → `audio_events.json`; `analyze.quick {source_id}` → audio + scenes + transcribe(model from settings) → summary `{duration, words, units, scenes, peaks:[top 10 by lufs], silences_total}`.
- `transcript.get {source_id, start?, end?, format?: "units"}` → units as lines `"#12 [00:47:05.2–00:47:09.8] text"` plus structured list; `transcript.search {source_id, query, limit: 20}` → matches `{unit_id, start, end, text}` (case-insensitive, accent/romanization-insensitive substring; then fuzzy ≥0.8).
- `see.frame {source_id, t, width?: 640}` → `{image}`; `see.tiles {source_id, start?, end?, around?: "<phrase>", window?: 20, count?: 12}` → `{image, tiles:[{t,label}]}` (`around` → first `transcript.search` hit centre ± window/2; labels = words overlapping t±1 s); `see.preview {source_id, start, end}` → `{video}`. Images go to `analysis/<id>/see/<timestamp>.jpg`.
- `clip.create {name, source_id, start_unit?, end_unit?, start?, end?, expect_text?, layout?: "center-crop", layout_params?: LayoutParams, captions?: true}` → Clip. Unit mode: start = max(unit.start − 0.08, prev word end), end = min(unit.end + 0.15, next word start); `expect_text` checked with `find_quote` → Validation "opening words do not match: transcript says '<…>'" on mismatch. Errors: units missing → Validation hint "run `rustler analyze transcribe <source>` or pass --start/--end"; no video → Validation. Empty transcript → captions disabled + warning.
- `clip.add_segment {clip_id, source_id, …same range args, layout?}`, `clip.set_layout {clip_id, segment_id?, preset, params?}` (all segments if no segment_id), `clip.set_captions {clip_id, enabled?, preset?, position_y?, max_words_per_line?, script?}`, `clip.show {clip_id}` (includes TimeMap summary: segments with out_start/out_end, total duration), `clip.list {}`.
- `render.run {clip_id, quality?: "final", prefer_gpu?: true}` → builds CaptionWords (words overlapping segments → `TimeMap::to_output`, romanized per script), writes `clips/<id>/renders/<rid>/captions.ass`, symlink-or-copies `home/fonts` needed into workdir `fonts/`, compiles, renders to `clips/<id>/renders/<rid>.mp4`, appends RenderRecord → `{render_id, path, encoder, duration}`. `render.preview_frame {clip_id, out_t}` → `{image}`.

- [ ] **Step 1: Failing tests** (tool-dependent tests skip without tools): `source.rs::import_local_creates_proxy_and_wav`; `source.rs::import_respects_fs_roots` (path outside roots → Validation); `source.rs::url_import_requires_permission` (network false → Validation, no subprocess); `source.rs::parse_json3_words` (fixture json3 string in test → words with start = tStartMs+tOffsetMs/1000); `analyze.rs::transcribe_resumes_from_parts` (uses jfk.wav fixture with model tiny; first call `--max-seconds 1` → complete false or true; second call completes; parts reused); `analyze.rs::empty_transcript_ok` (tone-only fixture → 0 words, units []); `clip.rs::create_by_units_pads_and_snaps` (synthetic transcript written directly to analysis dir → start = unit.start−0.08 clamp); `clip.rs::expect_text_mismatch_validation`; `clip.rs::create_without_units_hints_transcribe`; `clip.rs::audio_only_source_rejected`; `clip.rs::stale_revision_conflict` (two `clip.set_captions` with same base revision → second Err RevisionConflict).
- [ ] **Step 2–5:** FAIL → implement → PASS → commit `feat(commands): slice-1 command set` (10a and 10b commit separately).

### Task 11: QC v1 + export  *(Sonnet)*

**Files:** `crates/rustler-media/src/qc.rs`, `crates/rustler-commands/src/qc_cmds.rs`, tests `crates/rustler-media/tests/qc.rs`.

**Interfaces (Produces):**
```rust
pub async fn run_qc(paths: &ToolPaths, clip: &Clip, render: &RenderRecord, render_path: &Path, font: &Path, out_dir: &Path) -> rustler_core::Result<QcReport>;
```
Checks (name → rule → status):
- `streams` → has h264 video + aac audio → else Fail.
- `dimensions` → equals clip.canvas → else Fail.
- `duration` → |probe − TimeMap.duration| ≤ 0.10 s → else Fail; `av_sync` → |video stream dur − audio stream dur| ≤ 0.10 → else Fail.
- `loudness` → ebur128 integrated within target ±1.5 LU → else Warn; true peak ≤ −1.0 dBTP → else Warn (only when audio.loudnorm).
- `black_frames` → `blackdetect=d=0.5:pix_th=0.10` any interval → Fail with fix_hint "check segment/layout covering black area".
- `frozen_video` → `freezedetect=n=0.003:d=2` any → Warn.
- `long_silence` → `silencedetect=n=-45dB:d=1.5` any → Warn, fix_hint "consider cutting pause at <t>".
- `contact_sheet` → 12 frames grid of the render (reuse `tiles` with labels = output time) saved `qc/<rid>.jpg`; always Pass; report includes path and the note "look at this image before declaring the clip done".
`passed` = no Fail. Commands: `qc.run {clip_id, render_id?}` (latest render default) → report JSON; CLI exit 7 when `passed=false` (command returns Err QcFailed carrying the report path in hint). `export.clip {clip_id, render_id?, name?}` → copies render to `exports/<name or clip name>.mp4` + `<name>.report.json` = `{clip, render, qc, source_refs, created_at}`; refuses (Validation) if latest QC failed unless `--yes`.

- [ ] **Step 1: Failing tests:** `qc_passes_good_render` (render fixture → passed, contact sheet exists); `qc_flags_black` (render with a segment over a black-only fixture region → black_frames Fail); `qc_duration_mismatch_fails` (fake clip whose TimeMap says 12 s vs 10 s file → duration Fail). **Step 2–5:** FAIL → implement → PASS → commit `feat(qc): qc v1 and export`.

---

## Wave 3 — integration (sequential)

### Task 12: End-to-end test + docs  *(Opus for e2e; Haiku for docs/feature files)*

**Files:** `crates/rustler-cli/tests/e2e_slice1.rs`, `skills/rustler/SKILL.md` (Slice-1 minimal version), `features/f01…f09_*.md`, `docs/progress-log.md`, `features/README.md` statuses.

- [ ] **Step 1: e2e test** (`#[ignore]`, run explicitly; needs tools): in a tmp project — `project create` → generate a 60 s fixture video whose audio is `jfk.wav` looped ×5 over `testsrc2` 1920×1080 → `source import` → `analyze quick` → `transcript search --query "fellow Americans"` → `clip create --start-unit <hit> --end-unit <hit+1> --expect-text "<first 3 words of that unit>" --layout blur-fit` → `render run` → `qc run` (exit 0) → `export clip`. Assert each envelope `ok:true`, exported mp4 1080×1920, report has `qc.passed=true`, `events.ndjson` has ≥ 6 lines with actor "agent".
- [ ] **Step 2:** Run `cargo test -p rustler-cli --test e2e_slice1 -- --ignored` → PASS (record real output summary in progress log).
- [ ] **Step 3:** Manual check on this laptop: `rustler doctor` shows NVENC ok; repeat the e2e render with NVENC; watch the exported clip; look at the QC contact sheet. Record findings.
- [ ] **Step 4:** Haiku agent: write `features/f01…f09_*.md` from this plan using `features/_feature-template.md` (owned files, interfaces, acceptance = the tests above, status Built/Verified per real results) and a minimal `skills/rustler/SKILL.md` (discovery-first, the start-here chain, "look at the QC contact sheet before done", unit-ids-not-timestamps, never auto-publish). Update `features/README.md` statuses and `docs/progress-log.md`.
- [ ] **Step 5:** Commit `test: slice-1 e2e + docs`.

---

## Execution notes

- **Model tiering (user rule):** Wave 0 + Task 9 + Task 12 e2e = Opus; Tasks 3–8, 10, 11 = Sonnet; doc/feature-file writing and mechanical fixes = Haiku. Each implementer gets: this plan's Global Constraints + Review Focus + their task block + the Interfaces of the tasks they consume.
- **Parallelism:** Wave 1 Tasks 3–8 run concurrently (6 agents). Wave 2 Tasks 9, 10a, 10b, 11 run concurrently (4 agents) after Wave 1 is green. A reviewer agent checks each task against its tests and the Global Constraints before the next wave starts.
- **Deferred to Slice 2 (not in this plan):** MCP server (f10), job registry commands, provider endpoints (STT endpoint, vision describe), OCR, faces.

---

## 5-HOUR SCOPE (user constraint, 2026-11-15) — overrides the slice ordering above

User: "keep GUI simple but CLI strong", **5 hours max**. Delivered in this session:
1. Slice 1 as planned (Tasks 0–12), tests kept but `#[ignore]` live/network tests run once only.
2. Pulled forward, strong-CLI extras (Wave 2b, parallel with Wave 2):
   - **MCP server** `rustler mcp serve` (stdio) generated from the registry (tools/list from CommandSpec; images returned as image content).
   - **Edit ops**: `edit.trim {clip_id, segment_id, start_delta?, end_delta?}`, `edit.split {clip_id, out_t}`, `edit.delete_segment`, `edit.remove_pauses {clip_id, min_gap: 0.5}` (splits segments at word gaps), `history.undo` (snapshot-based: `history/<rev>.clip.json`).
   - **Face reframe v0**: engine `faces.track` (OpenCV YuNet at 4 fps on proxy → per-sample box) + Rust smoothing (median + One-Euro + dead zone) → `center-crop` focus driven by `sendcmd` x-keyframes; command `crop.track_face {clip_id, segment_id?}`.
   - **Caption presets ×4**: `bold-karaoke`, `clean-white`, `boxed-highlight`, `hormozi-pop` (data-driven in presets.rs) + `caption.list_styles`.
   - **Music bed**: `audio.add_music {clip_id, path, volume_db: -18, duck: true}` (sidechaincompress threshold 0.08 ratio 5 attack 100 release 1000, amix normalize=0, fades).
3. **Simple GUI = local web page, not Tauri** (Claude's call for the time budget): `rustler gui` starts an axum server on 127.0.0.1 (random port, token in URL) serving one static page: projects → sources (transcript, tiles) → clips (segments, layout, captions) → renders (video player, QC report + contact sheet) + buttons that call registry commands through `POST /api/run {command, input}`. Same registry = same capabilities. Tauri Premiere-style editor stays in the backlog (Slice 5).
Deferred: provider layer, highlight helpers, brief/variants, asset library, transitions/effects, diarization, Tauri editor.
