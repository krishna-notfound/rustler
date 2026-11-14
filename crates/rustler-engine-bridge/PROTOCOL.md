# Rustler engine protocol (frozen, Slice 1 Task 2)

> Canonical copy of the engine wire protocol. It lives here (not in `engine/PROTOCOL.md`)
> because `engine/` was being built in parallel when the contracts were frozen; the
> engine side (Task 5) should link to or copy this file verbatim.

Line-delimited JSON-RPC 2.0 on stdin/stdout; stdout carries only protocol lines; logs to stderr.

Requests:

```json
{"jsonrpc":"2.0","id":N,"method":…,"params":{…}}
```

Notifications from engine:

```json
{"jsonrpc":"2.0","method":"progress","params":{"id":N,"stage":…,"pct":…}}
```

Errors:

```json
{"code": -32000, "message": …, "data": {"kind": "dependency_missing"|"media"|"validation"|"internal"}}
```

Methods:

- `engine.ping {}` → `{"version": "<pkg version>", "python": "3.11.x"}`
- `engine.capabilities {}` → `{"cuda": bool, "packages": {"faster_whisper": "x.y", …}}`
- `transcribe.run {"audio_path", "model", "language": "auto"|"en"|"hi", "device": "auto"|"cpu"|"cuda", "parts_dir", "chunk_seconds": 600, "max_seconds": null|int}` → `{"language", "words": [{"text","start","end","conf"}], "complete": bool}`
- `scenes.detect {"video_path", "threshold": 27.0}` → `{"scenes": [{"start","end"}]}`
- `translit.to_latin {"texts": [..]}` → `{"texts": [..]}`
- `models.fetch {"whisper": ["small"]}` → `{"fetched": [..]}`
