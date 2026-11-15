"""On-screen text recognition (``ocr.run``).

Engine: RapidOCR (Apache-2.0) running the PP-OCRv6 *small* detection + recognition ONNX models that
ship inside the ``rapidocr`` wheel (Apache-2.0, PaddleOCR weights); no download is needed and no
text-angle classifier is used (video text is upright). See ``engine/MODELS.md``.

Frames are decoded with OpenCV, downscaled to at most ``max_width`` px and recognised one by one.
Identical consecutive results are collapsed into one entry whose ``t_end`` is the last sample that
still showed the same text. Boxes are normalized 0..1 of the frame.

Languages (``lang`` param): ``latin`` (default; the bundled model), ``devanagari`` (PaddleOCR PP-OCRv5
Devanagari recognition model in ONNX form, Apache-2.0, downloaded to ``$RUSTLER_HOME/models/ocr/`` and
sha256-verified; see ``engine/MODELS.md``) and ``auto`` (Latin first; on frames where it finds nothing
or only weak lines the Devanagari pass also runs and the higher-confidence reading of each text line wins).
"""

from __future__ import annotations

import hashlib
import logging
import math
import os
import re
import shutil
import tempfile
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

from .rpc import RpcError

log = logging.getLogger("rustler_engine.ocr")

MAX_SAMPLES = 5000
DEFAULT_EVERY = 2.0
DEFAULT_MAX_WIDTH = 1024
MIN_CONF = 0.5
#: A frame is treated as unchanged when fewer than this share of its 192x108 thumbnail pixels moved
#: by more than THUMB_DELTA gray levels (a swapped caption changes far more pixels than that).
STATIC_CHANGED_FRACTION = 0.0007
THUMB_DELTA = 24

LANGS = ("latin", "devanagari", "auto")
#: ``auto``: a Latin pass whose lines are all at least this confident (mean) is accepted as is.
AUTO_OK_MEAN = 0.85
AUTO_OK_MIN = 0.7
#: ``auto``: a Devanagari reading replaces an overlapping Latin one only when this much more confident.
AUTO_MARGIN = 0.03

DEVANAGARI_FILE = "devanagari_PP-OCRv5_rec_mobile.onnx"
DEVANAGARI_URL = (
    "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/" + DEVANAGARI_FILE
)
DEVANAGARI_SHA256 = "d6f0a906580e3fa6b324a318718f1f31f268b6ea8ef985f91c2012a37f52c91e"
ENGINE_LABELS = {
    "latin": "rapidocr/PP-OCRv6-small",
    "devanagari": "rapidocr/PP-OCRv6-small-det+PP-OCRv5-devanagari-mobile-rec",
    "auto": "rapidocr/PP-OCRv6-small+PP-OCRv5-devanagari-mobile-rec",
}

_ENGINES: Dict[str, Any] = {}


def rustler_home() -> Path:
    env = os.environ.get("RUSTLER_HOME")
    return Path(env) if env else Path.home() / ".rustler"


def devanagari_model_path() -> Path:
    return rustler_home() / "models" / "ocr" / DEVANAGARI_FILE


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def ensure_devanagari(progress: Optional[Callable[..., None]] = None) -> Path:
    """Return the Devanagari recognition model, downloading and verifying it when missing."""
    p = devanagari_model_path()
    if p.is_file() and _sha256(p) == DEVANAGARI_SHA256:
        return p
    p.parent.mkdir(parents=True, exist_ok=True)
    if progress:
        progress("models.fetch", 0.0, f"downloading {DEVANAGARI_FILE}")
    log.info("downloading %s -> %s", DEVANAGARI_URL, p)
    fd, tmp = tempfile.mkstemp(dir=str(p.parent), suffix=".part")
    try:
        with os.fdopen(fd, "wb") as out, urllib.request.urlopen(DEVANAGARI_URL, timeout=120) as resp:
            shutil.copyfileobj(resp, out)
        got = _sha256(Path(tmp))
        if got != DEVANAGARI_SHA256:
            raise RpcError("media", f"downloaded {DEVANAGARI_FILE} has sha256 {got}, expected {DEVANAGARI_SHA256}")
        os.replace(tmp, p)
    except RpcError:
        raise
    except Exception as e:
        raise RpcError("media", f"could not download the Devanagari OCR model: {type(e).__name__}: {e}") from e
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)
    if progress:
        progress("models.fetch", 100.0, "ocr devanagari ready")
    return p


def fetch_devanagari(progress: Callable[..., None]) -> str:
    return str(ensure_devanagari(progress))


def _engine(lang: str = "latin"):
    """The RapidOCR instance for ``latin`` or ``devanagari`` (created once per process)."""
    if lang not in ("latin", "devanagari"):
        raise RpcError("validation", f"unknown OCR engine language: {lang}")
    if lang not in _ENGINES:
        try:
            from rapidocr import LangRec, OCRVersion, RapidOCR
        except ImportError as e:
            raise RpcError("dependency_missing", f"rapidocr is not installed: {e}") from e
        threads = max(1, min(8, os.cpu_count() or 4))
        params: Dict[str, Any] = {
            "Global.use_cls": False,
            "Global.log_level": "warning",
            "EngineConfig.onnxruntime.intra_op_num_threads": threads,
            "EngineConfig.onnxruntime.inter_op_num_threads": 1,
        }
        if lang == "devanagari":
            params["Rec.model_path"] = str(ensure_devanagari())
            params["Rec.lang_type"] = LangRec.DEVANAGARI
            params["Rec.ocr_version"] = OCRVersion.PPOCRV5
        try:
            _ENGINES[lang] = RapidOCR(params=params)
        except ImportError as e:
            raise RpcError("dependency_missing", f"rapidocr backend missing: {e}") from e
        except Exception as e:
            raise RpcError("internal", f"cannot start RapidOCR ({lang}): {type(e).__name__}: {e}") from e
    return _ENGINES[lang]


def normalize_text(t: str) -> str:
    return re.sub(r"\s+", " ", t).strip().casefold()


def _keep(text: str, conf: float) -> bool:
    if not any(ch.isalnum() for ch in text):
        return False
    if conf < MIN_CONF:
        return False
    return len(text) >= 2 or conf >= 0.8


def _recognize_one(img, lang: str) -> List[Dict[str, Any]]:
    """OCR one BGR image with one engine -> lines (top-to-bottom) with normalized boxes."""
    h, w = img.shape[:2]
    res = _engine(lang)(img)
    if res is None or getattr(res, "txts", None) is None or res.boxes is None:
        return []
    lines: List[Dict[str, Any]] = []
    for box, text, score in zip(res.boxes, res.txts, res.scores):
        text = str(text).strip()
        conf = float(score)
        if not _keep(text, conf):
            continue
        xs = [float(p[0]) for p in box]
        ys = [float(p[1]) for p in box]
        x0, x1 = min(max(min(xs) / w, 0.0), 1.0), min(max(max(xs) / w, 0.0), 1.0)
        y0, y1 = min(max(min(ys) / h, 0.0), 1.0), min(max(max(ys) / h, 0.0), 1.0)
        if x1 <= x0 or y1 <= y0:
            continue
        lines.append(
            {
                "text": text,
                "box": {"x": round(x0, 5), "y": round(y0, 5), "w": round(x1 - x0, 5), "h": round(y1 - y0, 5)},
                "conf": round(conf, 4),
            }
        )
    lines.sort(key=lambda l: (round(l["box"]["y"] + l["box"]["h"] / 2, 2), l["box"]["x"]))
    return lines


def has_devanagari(text: str) -> bool:
    return any("ऀ" <= ch <= "ॿ" for ch in text)


def _overlap(a: Dict[str, float], b: Dict[str, float]) -> float:
    """Intersection area over the smaller box area."""
    iw = min(a["x"] + a["w"], b["x"] + b["w"]) - max(a["x"], b["x"])
    ih = min(a["y"] + a["h"], b["y"] + b["h"]) - max(a["y"], b["y"])
    if iw <= 0 or ih <= 0:
        return 0.0
    return iw * ih / max(1e-9, min(a["w"] * a["h"], b["w"] * b["h"]))


def _latin_is_weak(lines: List[Dict[str, Any]]) -> bool:
    if not lines:
        return True
    confs = [l["conf"] for l in lines]
    return sum(confs) / len(confs) < AUTO_OK_MEAN or min(confs) < AUTO_OK_MIN


def merge_readings(latin: List[Dict[str, Any]], deva: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Per text line keep the more confident reading. A Devanagari reading must contain Devanagari
    to replace an overlapping Latin one (the model also reads Latin letters, which the Latin model
    does better); a line only the Devanagari pass found is added."""
    out: List[Dict[str, Any]] = [dict(l, lang="latin") for l in latin]
    for d in deva:
        hits = [i for i, l in enumerate(out) if _overlap(l["box"], d["box"]) > 0.4]
        if not hits:
            out.append(dict(d, lang="devanagari"))
            continue
        if not has_devanagari(d["text"]):
            continue
        best_old = max(out[i]["conf"] for i in hits)
        if d["conf"] > best_old + AUTO_MARGIN:
            for i in sorted(hits, reverse=True):
                del out[i]
            out.append(dict(d, lang="devanagari"))
    out.sort(key=lambda l: (round(l["box"]["y"] + l["box"]["h"] / 2, 2), l["box"]["x"]))
    return out


def recognize(img, lang: str = "latin") -> List[Dict[str, Any]]:
    """OCR one BGR image -> lines (top-to-bottom) with normalized boxes.

    ``lang``: ``latin`` | ``devanagari`` | ``auto`` (the latter two tag each line with ``lang``)."""
    if lang == "latin":
        return _recognize_one(img, "latin")
    if lang == "devanagari":
        return [dict(l, lang="devanagari") for l in _recognize_one(img, "devanagari")]
    latin = _recognize_one(img, "latin")
    if not _latin_is_weak(latin):
        return [dict(l, lang="latin") for l in latin]
    return merge_readings(latin, _recognize_one(img, "devanagari"))


def _num(params: Dict[str, Any], key: str, default: Optional[float]) -> Optional[float]:
    v = params.get(key, default)
    if v is None:
        return None
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise RpcError("validation", f"{key} must be a number")
    return float(v)


def sample_times(params: Dict[str, Any], duration: Optional[float]) -> Tuple[List[float], float]:
    times = params.get("times")
    every = _num(params, "every", DEFAULT_EVERY)
    if times is not None:
        if not isinstance(times, list) or not all(isinstance(t, (int, float)) and not isinstance(t, bool) for t in times):
            raise RpcError("validation", "times must be a list of numbers (seconds)")
        ts = sorted({round(float(t), 4) for t in times if t >= 0})
        if not ts:
            raise RpcError("validation", "times is empty")
        return ts, every
    if every <= 0:
        raise RpcError("validation", "every must be > 0")
    start = _num(params, "start", 0.0)
    end = _num(params, "end", None)
    if start < 0:
        raise RpcError("validation", "start must be >= 0")
    if end is None:
        if duration is None:
            raise RpcError("validation", "end is required (video duration unknown)")
        end = duration
    if end <= start:
        raise RpcError("validation", "end must be greater than start")
    n = max(1, math.ceil((end - start) / every - 1e-9))
    if n > MAX_SAMPLES:
        raise RpcError("validation", f"range yields {n} samples (allowed 1..{MAX_SAMPLES}); raise `every`")
    return [round(start + i * every, 4) for i in range(n)], every


def run(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    video_path = params.get("video_path")
    if not isinstance(video_path, str) or not video_path:
        raise RpcError("validation", "video_path is required")
    max_width = _num(params, "max_width", float(DEFAULT_MAX_WIDTH))
    if max_width < 160:
        raise RpcError("validation", "max_width must be >= 160")
    lang = params.get("lang", "latin")
    if lang is None:
        lang = "latin"
    if lang not in LANGS:
        raise RpcError("validation", f"lang must be one of {', '.join(LANGS)}")
    if not os.path.isfile(video_path):
        raise RpcError("validation", f"video not found: {video_path}")

    import cv2
    import numpy as np

    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        raise RpcError("media", f"cannot open video: {video_path}")
    try:
        width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
        height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
        src_fps = cap.get(cv2.CAP_PROP_FPS) or 0.0
        nframes = cap.get(cv2.CAP_PROP_FRAME_COUNT) or 0.0
        duration = nframes / src_fps if src_fps > 0 and nframes > 0 else None
        if width <= 0 or height <= 0:
            raise RpcError("media", f"video has no frame size: {video_path}")
        times, every = sample_times(params, duration)
        warnings: List[str] = []
        if lang == "auto":
            try:
                ensure_devanagari(progress)
            except RpcError as e:
                warnings.append(f"Devanagari model unavailable, reading Latin only: {e.message}")
                lang = "latin"
        # fail early (and load the models once) before decoding
        _engine("latin" if lang != "devanagari" else "devanagari")
        if lang == "auto":
            _engine("devanagari")

        scale = min(1.0, max_width / width)
        dw, dh = max(1, round(width * scale)), max(1, round(height * scale))
        frames: List[Dict[str, Any]] = []
        prev_key: Optional[Tuple[str, ...]] = None
        prev_thumb = None
        prev_lines: List[Dict[str, Any]] = []
        done = 0
        for i, t in enumerate(times):
            cap.set(cv2.CAP_PROP_POS_MSEC, t * 1000.0)
            ok, frame = cap.read()
            if not ok or frame is None:
                break  # past the end of the stream
            done += 1
            thumb = cv2.cvtColor(cv2.resize(frame, (192, 108), interpolation=cv2.INTER_AREA), cv2.COLOR_BGR2GRAY)
            if prev_thumb is not None and float(np.mean(cv2.absdiff(thumb, prev_thumb) > THUMB_DELTA)) < STATIC_CHANGED_FRACTION:
                lines = prev_lines  # unchanged picture: reuse the last result
            else:
                small = cv2.resize(frame, (dw, dh), interpolation=cv2.INTER_AREA) if scale < 1.0 else frame
                lines = recognize(small, lang)
            prev_thumb, prev_lines = thumb, lines
            key = tuple(normalize_text(l["text"]) for l in lines)
            if key and key == prev_key and frames:
                frames[-1]["t_end"] = round(t, 4)
            elif key:
                frames.append({"t": round(t, 4), "t_end": round(t, 4), "lines": lines})
            prev_key = key
            if i % 2 == 0 or i == len(times) - 1:
                progress("ocr.run", round((i + 1) / len(times) * 100, 1))
    finally:
        cap.release()
    out: Dict[str, Any] = {
        "frames": frames,
        "width": width,
        "height": height,
        "samples": done,
        "every": every,
        "engine": ENGINE_LABELS[lang],
        "lang": lang,
    }
    if warnings:
        out["warnings"] = warnings
    return out
