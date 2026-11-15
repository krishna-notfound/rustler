# rustler-engine protocol (frozen)

Line-delimited JSON-RPC 2.0 over the worker's stdin/stdout. Start with
`<venv_python> -m rustler_engine` (cwd = the engine dir).

- **stdout carries protocol lines only** (one JSON object per line, UTF-8, `\n`). The worker
  redirects `sys.stdout` and OS file descriptor 1 to stderr at start-up, so library prints
  (tqdm, faster-whisper, native code) cannot corrupt the stream. Logs go to stderr.
- Requests are processed sequentially, one at a time. EOF on stdin = clean shutdown (exit 0).
  Cancellation = the client kills the process (chunk parts make the work resumable).
- Request: `{"jsonrpc":"2.0","id":N,"method":"...","params":{...}}`. `id` is an int or string.
- Success: `{"jsonrpc":"2.0","id":N,"result":{...}}`.
- Error: `{"jsonrpc":"2.0","id":N,"error":{"code":-32000,"message":"...","data":{"kind":"..."}}}`
  with `kind` one of `dependency_missing` | `media` | `validation` | `internal`.
  Protocol-level failures use the standard JSON-RPC codes: `-32700` parse error (`id` null),
  `-32600` invalid request, `-32601` method not found; these also carry `data.kind = "validation"`.
- Progress notifications (no `id` of their own, emitted before the response of request N):
  `{"jsonrpc":"2.0","method":"progress","params":{"id":N,"stage":"...","pct":0.0-100.0|null}}`
  (an optional `"message"` string may be present).

## Methods

- `engine.ping {}` -> `{"version": "<pkg version>", "python": "3.11.x"}`
- `engine.capabilities {}` -> `{"cuda": bool, "packages": {"faster_whisper": "x.y", ...}}`
- `transcribe.run {"audio_path", "model", "language": "auto"|"en"|"hi", "device": "auto"|"cpu"|"cuda", "parts_dir", "chunk_seconds": 600, "max_seconds": null|int}`
  -> `{"language", "words": [{"text","start","end","conf"}], "complete": bool}`
  - `language` / `device` / `chunk_seconds` / `max_seconds` are optional with the defaults shown.
  - Audio is split into `chunk_seconds` windows; each finished chunk is written to
    `parts_dir/NNNN.json`. Parts that match (same model, chunk_seconds, language request) are
    reused on the next call; if every chunk has a part, no audio decoding and no model load
    happens. Word times are absolute (seconds from the start of the file); `conf` is the word
    probability. `word_timestamps` and VAD are on.
  - `device: "auto"` = CUDA (float16) when ctranslate2 sees a GPU and it works, else CPU (int8).
  - `max_seconds` is a **wall-clock work budget** for this call. It is checked between chunks;
    at least one chunk is always processed per call. When it is exceeded with chunks remaining
    the call returns the words so far with `complete: false`; rerun the same call to continue.
  - Progress: `stage: "transcribe"`, `pct` = finished chunks / total chunks * 100, one per chunk.
  - Model downloads honour the `HF_HOME` environment variable.
- `scenes.detect {"video_path", "threshold": 27.0}` -> `{"scenes": [{"start","end"}]}`
  (ContentDetector; a video without cuts yields one scene covering the whole file).
- `translit.to_latin {"texts": [..]}` -> `{"texts": [..]}`  (Devanagari -> Hinglish Latin; other
  text is returned unchanged. Rules are documented in `rustler_engine/translit.py`.)
- `models.fetch {"whisper": ["small"]}` -> `{"fetched": [..]}`  (downloads faster-whisper models
  into the HF cache; progress `stage: "models.fetch"`). Also accepts `{"yunet": true}`: downloads
  the YuNet face model (MIT, opencv_zoo) to `$RUSTLER_HOME/models/yunet/face_detection_yunet_2023mar.onnx`
  (default `~/.rustler`) and returns `{"fetched": [.., "yunet"], "yunet_path": "..."}`; `whisper`
  may be omitted when only `yunet` is requested.
- `faces.track {"video_path", "start": 0.0, "end": null|float, "fps": 4.0, "model_path": null|string}`
  -> `{"width": int, "height": int, "samples": [{"t": float, "faces": [{"x","y","w","h","score"}]}]}`
  - Samples are taken at `start + i/fps` for `i` in `0 .. floor((end-start)*fps)-1`; `t` is **source
    time in seconds** (not clip-relative). `end` defaults to the video duration. `fps` in (0, 60].
    A sample whose frame cannot be read (past the end of the stream) ends the list early.
  - Boxes are normalized 0..1 of the full frame (`x`,`y` = top-left), clamped to the frame, and
    `faces` is sorted by area descending (empty list = no face). `width`/`height` are the source
    frame size.
  - Detector: OpenCV `cv2.FaceDetectorYN` + YuNet ONNX, score threshold 0.6, NMS 0.3, frames
    downscaled to at most 640 px wide before detection. Seeking uses `CAP_PROP_POS_MSEC`.
  - `model_path` overrides the model; otherwise the default path above is used and the model is
    downloaded on first use (error kind `media` if the download fails, `validation` if an explicit
    `model_path` is missing).
  - Progress: `stage: "faces.track"`, `pct` = samples done / total * 100 (about every 8 samples);
    `stage: "models.fetch"` notifications while the model downloads.
- `ocr.run {"video_path", "times": [..] | "every": 2.0, "start": 0.0, "end": null|float, "max_width": 1024}`
  -> `{"frames": [{"t", "t_end", "lines": [{"text", "box": {"x","y","w","h"}, "conf"}]}], "width", "height", "samples", "every", "engine"}`
  - Sample times: the explicit `times` list (seconds, sorted and de-duplicated), else
    `start + i*every` for every `i` while the time is `< end` (`end` defaults to the video duration;
    `every` > 0; at most 5000 samples, else `validation`). A sample whose frame cannot be read ends the run.
  - Frames are downscaled to at most `max_width` px wide (default 1024, min 160) and read with RapidOCR
    (PP-OCRv6 small, Apache-2.0; no text-angle classifier). Boxes are normalized 0..1 of the frame
    (`x`,`y` = top-left); `conf` in [0.5, 1]; lines are ordered top-to-bottom, then left-to-right.
    Lines without an alphanumeric character, or with `conf` < 0.5, are dropped.
  - **Dedupe:** consecutive samples that show the same text (compared case- and whitespace-insensitively)
    become ONE entry: `t` = first sample, `t_end` = last sample still showing it. Samples without any text
    produce no entry, so a gap between two identical captions yields two entries. A sample whose picture
    is essentially unchanged from the previous sample (thumbnail comparison) reuses the previous result.
    `samples` = frames actually read.
  - Errors: `validation` (missing/invalid params, video not found), `media` (cannot open video),
    `dependency_missing` (`rapidocr`/`onnxruntime` not installed). Progress: `stage: "ocr.run"`,
    `pct` = samples done / total * 100.
  - Devanagari text is not recognised by the bundled model (Latin and Chinese scripts are).
- `audio.events {"audio_path", "hop": 0.5, "model": "auto"|"yamnet"|"heuristic"}`
  -> `{"events": [{"start","end","label","score"}], "model": "yamnet"|"heuristic", "duration", "hop", "warnings"?: [..]}`
  - `audio_path` is a PCM WAV (the source's 16 kHz wav; other rates are resampled, stereo is mixed down;
    anything else = `media`). `hop` in [0.1, 5.0] seconds.
  - `label` is one of `laughter | applause | cheer | music | speech | silence`. Events are ordered,
    non-overlapping, and merge adjacent hops with the same label (a single unlabeled hop between two
    equal labels is bridged). Hops with no confident label produce no event. `score` is the mean label
    score 0..1. `end <= duration`.
  - `model: "auto"` (default) uses YAMNet (Apache-2.0 ONNX, downloaded to `$RUSTLER_HOME/models/yamnet/`
    on first use and sha256-verified, see `MODELS.md`) and falls back to a spectral heuristic when the model
    cannot be obtained (the result then says `"model": "heuristic"` and carries a `warnings` entry; the
    heuristic only distinguishes silence / speech / music / applause). `"yamnet"` fails with `media` /
    `dependency_missing` instead of falling back; `"heuristic"` never loads a model.
  - Hops are labelled from YAMNet frames (0.96 s window, 0.48 s hop) whose window contains the hop centre;
    reactions (laughter, applause, cheer, score >= 0.30) take priority over speech/music; digital quiet
    (RMS < -55 dBFS) is `silence`.
  - Progress: `stage: "audio.events"` per 600 s chunk; `stage: "models.fetch"` while the model downloads.
- `models.fetch` also accepts `{"yamnet": true}` -> adds `"yamnet"` to `fetched` and `"yamnet_path"`.
- `engine.capabilities` `packages` now also lists `rapidocr` and `onnxruntime` when installed.
- `speakers.diarize {"audio_path", "num_speakers": null|int, "min_speakers": null|int, "max_speakers": null|int, "model": "auto"|"sherpa"|"heuristic"}`
  -> `{"segments": [{"start","end","speaker":"S1"}], "num_speakers": int, "duration": float, "model": "sherpa-onnx:pyannote-segmentation-3.0+wespeaker-resnet34"|"heuristic", "warnings"?: [..]}`
  - `audio_path` is a PCM WAV (same rules as `audio.events`: the source's 16 kHz wav; other rates are
    resampled, stereo mixed down). Segments are sorted by start, one speaker at a time, `start`/`end`
    in seconds of the file. Speaker ids are `S1`, `S2`, ... in order of first appearance.
  - `model: "auto"` (default) uses sherpa-onnx (Apache-2.0) offline speaker diarization with the
    pyannote segmentation-3.0 ONNX (MIT) and the WeSpeaker ResNet34 embedding model (Apache-2.0); both
    are downloaded to `$RUSTLER_HOME/models/diarization/` on first use and sha256-verified (`MODELS.md`).
    When sherpa-onnx or the models are unavailable it falls back to a numpy heuristic (MFCC statistics
    + average-linkage clustering) and says `"model": "heuristic"` with a `warnings` entry; the
    heuristic is much less accurate (it can split one speaker in two). `"sherpa"` fails instead of
    falling back (`media` / `dependency_missing`); `"heuristic"` never loads a model.
  - `num_speakers` fixes the cluster count. Otherwise the count is estimated with a distance
    threshold; `min_speakers` / `max_speakers` re-cluster with a fixed count when the estimate lies
    outside the range. All three are integers in 1..64 (else `validation`).
  - A file shorter than 0.5 s or without speech returns `segments: []`.
  - Progress: `stage: "speakers.diarize"`, `pct` from the chunk callback; `stage: "models.fetch"` while
    the models download. Speed on this laptop (CPU): about 3 s for 29 s of audio once the models are cached.
- `speakers.active {"video_path", "start": 0.0, "end": null|float, "fps": 5.0}`
  -> `{"width", "height", "fps", "samples": [{"t", "faces": [{"x","y","w","h","score","id","track","motion","speak"}]}], "tracks": [{"id","samples","mean_x","mean_area","motion_p10","motion_p95","static","shots"}], "people", "warnings"?}`
  - Same sampling and face boxes as `faces.track` (YuNet, normalized 0..1, `t` = source time), `fps` in
    (0, 15]. Per sample three consecutive frames (about 0.05 s apart) are read; `motion` is the mean
    absolute grey difference (0..1) inside the lower 45 % of each face box (mouth and chin, middle 70 %
    of the width), averaged over the burst.
  - `track` is a face in one shot (greedy matching to a track seen in the last 3 s: nearest centre within
    0.22 of the frame width, width ratio <= 1.7, and, when both faces have an appearance embedding, cosine
    >= 0.25 so a cut to a close-up does not glue the close-up onto a wide-shot face). `id` is the *person*:
    tracks are linked by appearance (SFace embeddings of faces >= 24 px wide, average-linkage cosine >= 0.40,
    never two tracks that share a sample), so one person keeps one `id` across cuts; numbered left to right
    by mean x. `people` is the number of persons. Without the SFace model (offline) `id` = `track` and
    `warnings` says so. Frames are analysed at up to 960 px wide.
  - `speak` (0..1) is `motion` normalized within the face's own *track* ((motion - p10) / (p95 - p10),
    clipped; a close-up and a wide shot of one person are normalized separately), so a quiet face and an
    animated face are comparable. A track whose p95 - p10 spread is below 0.008 is `static` (a photo, a
    poster, someone who never speaks) and gets `speak: 0`. `tracks[]` has one row per person (`shots` =
    number of tracks merged into it).
  - Errors: `validation` (missing params, video not found, bad range), `media` (cannot open video,
    cannot load the YuNet model). Progress: `stage: "speakers.active"`.
  - `speakers.diarize` is windowed since 2026-11-15: sherpa-onnx finds the speech regions, then 1.5 s windows
    (every 0.5 s) are embedded and clustered (average linkage; debris clusters such as applause are folded into
    the nearest speaker; auto speaker count only when clusters are clearly apart), labels are smoothed with a
    switch penalty, runs under 1 s are absorbed and switches snap to the quietest moment within 0.7 s. The
    result shape is unchanged (non-overlapping segments).
- `models.fetch` also accepts `{"diarization": true}` -> adds `"diarization"` to `fetched` and `"diarization_dir"`.
- `engine.capabilities` `packages` now also lists `sherpa_onnx` when installed.
