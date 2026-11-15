"""Chunked, resumable faster-whisper transcription with word timestamps.

Audio is decoded once (``faster_whisper.decode_audio``, 16 kHz mono float32), cut into
``chunk_seconds`` windows and each finished window is saved to ``parts_dir/NNNN.json``.
A rerun reuses every part that matches (same model / chunk size / language request); when all
parts exist neither audio decoding nor model loading happens.
"""

from __future__ import annotations

import glob
import json
import logging
import os
import sys
import time
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

from .rpc import RpcError

log = logging.getLogger("rustler_engine.transcribe")

SR = 16000
PART_VERSION = 1
MIN_TAIL_SECONDS = 2.0  # a shorter trailing window is merged into the previous chunk

ModelFactory = Callable[[str, str, str], Any]
Decoder = Callable[[str], Any]

_MODELS: Dict[Tuple[str, str, str], Any] = {}


# --- device -----------------------------------------------------------------------------


def _register_cuda_libs() -> None:
    """Make pip-installed nvidia-cublas/cudnn libraries (the `cuda` extra) visible to ctranslate2."""
    try:
        import importlib.util

        spec = importlib.util.find_spec("nvidia")
        roots = list(spec.submodule_search_locations or []) if spec else []
    except Exception:
        return
    for root in roots:
        for libdir in glob.glob(os.path.join(root, "*", "bin" if sys.platform == "win32" else "lib")):
            if sys.platform == "win32":
                try:
                    os.add_dll_directory(libdir)
                except OSError:
                    pass
                os.environ["PATH"] = libdir + os.pathsep + os.environ.get("PATH", "")
            else:
                import ctypes

                for so in sorted(glob.glob(os.path.join(libdir, "lib*.so*"))):
                    try:
                        ctypes.CDLL(so, mode=ctypes.RTLD_GLOBAL)
                    except OSError:
                        pass


def cuda_libs_present() -> bool:
    """True when the CUDA runtime libraries faster-whisper needs can be found: the pip wheels of the
    `cuda` extra (`rustler setup run --with cuda`) or a system CUDA install. A lite install on an NVIDIA
    machine has the driver but not these, so it must keep using the CPU instead of failing at load time."""
    import importlib.util

    try:
        if importlib.util.find_spec("nvidia.cublas") is not None:
            return True
    except Exception:
        pass
    try:
        import ctypes.util

        names = ("cublas64_12", "cublas64_11") if sys.platform == "win32" else ("cublas",)
        return any(ctypes.util.find_library(n) for n in names)
    except Exception:
        return False


def cuda_available() -> bool:
    """A CUDA device exists *and* its runtime libraries are usable (see :func:`cuda_libs_present`)."""
    try:
        import ctranslate2

        return ctranslate2.get_cuda_device_count() > 0 and cuda_libs_present()
    except Exception:
        return False


def resolve_device(device: str) -> Tuple[str, str]:
    """Return (device, compute_type): cuda -> float16, cpu -> int8."""
    if device == "cuda" or (device == "auto" and cuda_available()):
        return "cuda", "float16"
    return "cpu", "int8"


def _default_factory(name: str, device: str, compute: str) -> Any:
    key = (name, device, compute)
    if key not in _MODELS:
        if device == "cuda":
            _register_cuda_libs()
        from faster_whisper import WhisperModel

        _MODELS.clear()  # keep at most one model in memory
        _MODELS[key] = WhisperModel(name, device=device, compute_type=compute)
    return _MODELS[key]


def _default_decode(path: str) -> Any:
    from faster_whisper import decode_audio

    return decode_audio(path, sampling_rate=SR)


# --- params -----------------------------------------------------------------------------


def _parse(params: Dict[str, Any]) -> Dict[str, Any]:
    audio = params.get("audio_path")
    model = params.get("model")
    parts = params.get("parts_dir")
    if not isinstance(audio, str) or not audio:
        raise RpcError("validation", "audio_path is required")
    if not isinstance(model, str) or not model:
        raise RpcError("validation", "model is required")
    if not isinstance(parts, str) or not parts:
        raise RpcError("validation", "parts_dir is required")
    language = params.get("language") or "auto"
    if not isinstance(language, str):
        raise RpcError("validation", "language must be a string")
    device = params.get("device") or "auto"
    if device not in ("auto", "cpu", "cuda"):
        raise RpcError("validation", "device must be auto, cpu or cuda")
    chunk = params.get("chunk_seconds")
    chunk = 600 if chunk is None else chunk
    if isinstance(chunk, bool) or not isinstance(chunk, (int, float)) or chunk <= 0:
        raise RpcError("validation", "chunk_seconds must be a positive number")
    budget = params.get("max_seconds")
    if budget is not None and (isinstance(budget, bool) or not isinstance(budget, (int, float)) or budget < 0):
        raise RpcError("validation", "max_seconds must be null or a non-negative number")
    if not os.path.isfile(audio):
        raise RpcError("validation", f"audio not found: {audio}")
    return {
        "audio": audio,
        "model": model,
        "parts_dir": Path(parts),
        "language": language,
        "device": device,
        "chunk": chunk,
        "budget": budget,
    }


# --- parts ------------------------------------------------------------------------------


def _part_path(parts_dir: Path, i: int) -> Path:
    return parts_dir / f"{i:04d}.json"


def _load_part(cfg: Dict[str, Any], i: int) -> Optional[Dict[str, Any]]:
    try:
        part = json.loads(_part_path(cfg["parts_dir"], i).read_text(encoding="utf-8"))
        ok = (
            part["version"] == PART_VERSION
            and part["index"] == i
            and part["model"] == cfg["model"]
            and part["chunk_seconds"] == cfg["chunk"]
            and part["language_req"] == cfg["language"]
            and isinstance(part["total"], int)
            and isinstance(part["words"], list)
        )
        return part if ok else None
    except (OSError, ValueError, KeyError, TypeError):
        return None


def _save_part(cfg: Dict[str, Any], part: Dict[str, Any]) -> None:
    path = _part_path(cfg["parts_dir"], part["index"])
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(part, ensure_ascii=False), encoding="utf-8")
    os.replace(tmp, path)


def _chunk_bounds(n: int, chunk_seconds: float) -> List[Tuple[int, int]]:
    step = max(1, int(chunk_seconds * SR))
    bounds = [(s, min(s + step, n)) for s in range(0, n, step)]
    if len(bounds) > 1 and bounds[-1][1] - bounds[-1][0] < MIN_TAIL_SECONDS * SR:
        last = bounds.pop()
        bounds[-1] = (bounds[-1][0], last[1])
    return bounds


# --- transcription ----------------------------------------------------------------------


_CUDA_HINT = "CUDA libraries unavailable ({}); install the engine with the `cuda` extra or use device=cpu"


def _is_cuda_lib_error(e: BaseException) -> bool:
    msg = str(e).lower()
    return any(k in msg for k in ("cublas", "cudnn", "cuda driver", "libcudart", "cuda failed"))


class _Model:
    """Lazily loads the model; in device=auto a failing CUDA setup falls back to CPU int8."""

    def __init__(self, cfg: Dict[str, Any], factory: ModelFactory):
        self.cfg = cfg
        self.factory = factory
        self.device, self.compute = resolve_device(cfg["device"])
        self.model: Any = None

    def _load(self) -> Any:
        if self.model is None:
            try:
                self.model = self.factory(self.cfg["model"], self.device, self.compute)
            except ValueError as e:
                raise RpcError("validation", f"invalid model {self.cfg['model']!r}: {e}") from e
            except RpcError:
                raise
            except Exception as e:
                if self.device == "cuda" and self.cfg["device"] == "auto":
                    log.warning("CUDA model load failed (%s); falling back to CPU", e)
                    self.device, self.compute = "cpu", "int8"
                    return self._load()
                if _is_cuda_lib_error(e):
                    raise RpcError("dependency_missing", _CUDA_HINT.format(e)) from e
                raise
        return self.model

    def transcribe(self, audio: Any, language: Optional[str]) -> Tuple[List[Dict[str, Any]], str, float]:
        for attempt in (0, 1):
            model = self._load()
            try:
                segments, info = model.transcribe(
                    audio,
                    language=language,
                    word_timestamps=True,
                    vad_filter=True,
                    condition_on_previous_text=False,
                )
                words = []
                for seg in segments:  # generator: decoding happens here
                    for w in seg.words or []:
                        text = (w.word or "").strip()
                        if not text:
                            continue
                        start = float(w.start)
                        words.append(
                            {
                                "text": text,
                                "start": start,
                                "end": max(float(w.end), start),
                                "conf": float(w.probability),
                            }
                        )
                return words, info.language, float(getattr(info, "language_probability", 0.0) or 0.0)
            except RuntimeError as e:
                if attempt == 0 and self.device == "cuda" and self.cfg["device"] == "auto":
                    log.warning("CUDA transcription failed (%s); falling back to CPU", e)
                    self.device, self.compute = "cpu", "int8"
                    self.model = None
                    continue
                if _is_cuda_lib_error(e):
                    raise RpcError("dependency_missing", _CUDA_HINT.format(e)) from e
                raise
        raise AssertionError("unreachable")


def run(
    params: Dict[str, Any],
    progress: Callable[..., None],
    *,
    model_factory: Optional[ModelFactory] = None,
    decode: Optional[Decoder] = None,
    clock: Callable[[], float] = time.monotonic,
) -> Dict[str, Any]:
    cfg = _parse(params)
    cfg["parts_dir"].mkdir(parents=True, exist_ok=True)
    started = clock()
    decode = decode or _default_decode
    engine = _Model(cfg, model_factory or _default_factory)
    language_arg = None if cfg["language"] == "auto" else cfg["language"]

    audio = None
    bounds: List[Tuple[int, int]] = []
    total: Optional[int] = None
    parts: List[Dict[str, Any]] = []
    complete = True
    i = 0
    while total is None or i < total:
        part = _load_part(cfg, i)
        if part is not None and (total is None or part["total"] == total):
            total = part["total"]
            parts.append(part)
            progress("transcribe", round((i + 1) / total * 100, 1))
            i += 1
            continue

        if audio is None:
            try:
                audio = decode(cfg["audio"])
            except RpcError:
                raise
            except Exception as e:
                raise RpcError("media", f"cannot decode audio {cfg['audio']}: {type(e).__name__}: {e}") from e
            bounds = _chunk_bounds(len(audio), cfg["chunk"])
            if total is not None and total != len(bounds):
                parts, i = [], 0  # stale parts from a different audio file: start over
            total = len(bounds)
            if total == 0:
                break
            if i >= total:
                break
            continue

        s, e = bounds[i]
        words, lang, prob = engine.transcribe(audio[s:e], language_arg)
        offset = s / SR
        for w in words:
            w["start"] = round(offset + w["start"], 3)
            w["end"] = round(max(offset + w["end"], w["start"]), 3)
            w["conf"] = round(w["conf"], 4)
        part = {
            "version": PART_VERSION,
            "index": i,
            "total": total,
            "offset": offset,
            "model": cfg["model"],
            "chunk_seconds": cfg["chunk"],
            "language_req": cfg["language"],
            "language": lang,
            "language_probability": prob,
            "words": words,
        }
        _save_part(cfg, part)
        parts.append(part)
        progress("transcribe", round((i + 1) / total * 100, 1))
        i += 1
        budget = cfg["budget"]
        if budget is not None and i < total and clock() - started >= budget:
            complete = False
            break

    words_out = [w for p in parts for w in p["words"]]
    words_out.sort(key=lambda w: (w["start"], w["end"]))
    if cfg["language"] != "auto":
        language = cfg["language"]
    elif parts:
        language = max(parts, key=lambda p: p.get("language_probability", 0.0))["language"]
    else:
        language = "und"
    return {"language": language, "words": words_out, "complete": complete}


# --- models.fetch -----------------------------------------------------------------------


def fetch_models(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    names = params.get("whisper", [])
    if not isinstance(names, list) or not all(isinstance(n, str) and n for n in names):
        raise RpcError("validation", "whisper must be a list of model names")
    from faster_whisper.utils import download_model

    fetched: List[str] = []
    for idx, name in enumerate(names):
        progress("models.fetch", round(idx / max(len(names), 1) * 100, 1), f"downloading whisper {name}")
        try:
            download_model(name)
        except ValueError as e:
            raise RpcError("validation", f"invalid whisper model {name!r}: {e}") from e
        fetched.append(name)
        progress("models.fetch", round((idx + 1) / len(names) * 100, 1), f"fetched whisper {name}")
    return {"fetched": fetched}
