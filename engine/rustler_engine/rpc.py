"""Line-delimited JSON-RPC 2.0 framing and dispatch (see PROTOCOL.md).

Handlers have the signature ``handler(params: dict, progress) -> dict`` where
``progress(stage, pct=None, message=None)`` emits a progress notification for the
request being served. Raising :class:`RpcError` selects the error kind; a bare
``ImportError`` becomes ``dependency_missing`` and anything else ``internal``.
"""

from __future__ import annotations

import json
import logging
import sys
import traceback
from typing import Any, Callable, Dict, TextIO

log = logging.getLogger("rustler_engine.rpc")

SERVER_ERROR = -32000
PARSE_ERROR = -32700
INVALID_REQUEST = -32600
METHOD_NOT_FOUND = -32601

KINDS = ("dependency_missing", "media", "validation", "internal")


class RpcError(Exception):
    """An error with a protocol kind (dependency_missing | media | validation | internal)."""

    def __init__(self, kind: str, message: str, code: int = SERVER_ERROR):
        super().__init__(message)
        assert kind in KINDS, kind
        self.kind = kind
        self.code = code
        self.message = message


Handler = Callable[[Dict[str, Any], Callable[..., None]], Dict[str, Any]]


def _error_obj(exc: BaseException) -> Dict[str, Any]:
    if isinstance(exc, RpcError):
        return {"code": exc.code, "message": exc.message, "data": {"kind": exc.kind}}
    if isinstance(exc, ImportError):
        return {
            "code": SERVER_ERROR,
            "message": f"missing Python dependency: {exc}",
            "data": {"kind": "dependency_missing"},
        }
    return {
        "code": SERVER_ERROR,
        "message": f"{type(exc).__name__}: {exc}",
        "data": {"kind": "internal"},
    }


def _write(out: TextIO, obj: Dict[str, Any]) -> None:
    out.write(json.dumps(obj, ensure_ascii=False, separators=(",", ":")) + "\n")
    out.flush()


def _protocol_error(out: TextIO, id_: Any, code: int, message: str) -> None:
    _write(
        out,
        {
            "jsonrpc": "2.0",
            "id": id_,
            "error": {"code": code, "message": message, "data": {"kind": "validation"}},
        },
    )


def serve(inp: TextIO, out: TextIO, methods: Dict[str, Handler]) -> None:
    """Serve requests from ``inp`` until EOF, writing only protocol lines to ``out``."""
    for raw in inp:
        line = raw.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except ValueError as e:
            _protocol_error(out, None, PARSE_ERROR, f"parse error: {e}")
            continue
        if not isinstance(msg, dict):
            _protocol_error(out, None, INVALID_REQUEST, "request must be a JSON object")
            continue
        id_ = msg.get("id")
        method = msg.get("method")
        params = msg.get("params", {})
        if params is None:
            params = {}
        if not isinstance(method, str) or not isinstance(params, dict):
            _protocol_error(out, id_, INVALID_REQUEST, "invalid request: need string method and object params")
            continue
        handler = methods.get(method)
        if handler is None:
            if "id" in msg:
                _protocol_error(out, id_, METHOD_NOT_FOUND, f"method not found: {method}")
            continue

        def progress(stage: str, pct: Any = None, message: Any = None, _id: Any = id_) -> None:
            p: Dict[str, Any] = {"id": _id, "stage": stage, "pct": pct}
            if message is not None:
                p["message"] = message
            _write(out, {"jsonrpc": "2.0", "method": "progress", "params": p})

        try:
            result = handler(params, progress)
            resp: Dict[str, Any] = {"jsonrpc": "2.0", "id": id_, "result": result}
        except Exception as e:  # every failure must become a protocol error
            if not isinstance(e, RpcError):
                log.error("handler %s failed:\n%s", method, traceback.format_exc())
            resp = {"jsonrpc": "2.0", "id": id_, "error": _error_obj(e)}
        if "id" in msg:  # requests without an id are notifications: no response
            _write(out, resp)


# --- built-in methods -------------------------------------------------------------------


def _ping(params, progress):
    from . import __version__

    return {"version": __version__, "python": sys.version.split()[0]}


def _capabilities(params, progress):
    from importlib import metadata

    from .transcribe import cuda_available

    packages = {}
    for dist, key in [
        ("faster-whisper", "faster_whisper"),
        ("ctranslate2", "ctranslate2"),
        ("scenedetect", "scenedetect"),
        ("opencv-python-headless", "opencv"),
        ("indic-transliteration", "indic_transliteration"),
        ("numpy", "numpy"),
    ]:
        try:
            packages[key] = metadata.version(dist)
        except metadata.PackageNotFoundError:
            pass
    return {"cuda": cuda_available(), "packages": packages}


def _translit(params, progress):
    from .translit import to_latin_many

    texts = params.get("texts")
    if not isinstance(texts, list) or not all(isinstance(t, str) for t in texts):
        raise RpcError("validation", "texts must be a list of strings")
    return {"texts": to_latin_many(texts)}


def _transcribe(params, progress):
    from . import transcribe

    return transcribe.run(params, progress)


def _scenes(params, progress):
    from . import scenes

    return scenes.detect(params, progress)


def _models_fetch(params, progress):
    from . import transcribe

    out: Dict[str, Any] = {"fetched": []}
    if "whisper" in params or not params.get("yunet"):
        out["fetched"] = transcribe.fetch_models(params, progress)["fetched"]
    if params.get("yunet"):
        from . import faces

        out["yunet_path"] = faces.fetch_yunet(progress)
        out["fetched"].append("yunet")
    return out


def _faces_track(params, progress):
    from . import faces

    return faces.track(params, progress)


def default_methods() -> Dict[str, Handler]:
    return {
        "engine.ping": _ping,
        "engine.capabilities": _capabilities,
        "transcribe.run": _transcribe,
        "scenes.detect": _scenes,
        "translit.to_latin": _translit,
        "models.fetch": _models_fetch,
        "faces.track": _faces_track,
    }
