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
  into the HF cache; progress `stage: "models.fetch"`).
