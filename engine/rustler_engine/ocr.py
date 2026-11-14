"""On-screen text recognition (``ocr.run``).

Engine: RapidOCR (Apache-2.0) running the PP-OCRv6 *small* detection + recognition ONNX models that
ship inside the ``rapidocr`` wheel (Apache-2.0, PaddleOCR weights); no download is needed and no
text-angle classifier is used (video text is upright). See ``engine/MODELS.md``.

Frames are decoded with OpenCV, downscaled to at most ``max_width`` px and recognised one by one.
Identical consecutive results are collapsed into one entry whose ``t_end`` is the last sample that
still showed the same text. Boxes are normalized 0..1 of the frame.
"""

from __future__ import annotations

import logging
import math
import os
import re
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

_ENGINE: Any = None


def _engine():
    global _ENGINE
    if _ENGINE is None:
        try:
            from rapidocr import RapidOCR
        except ImportError as e:
            raise RpcError("dependency_missing", f"rapidocr is not installed: {e}") from e
        threads = max(1, min(8, os.cpu_count() or 4))
        try:
            _ENGINE = RapidOCR(
                params={
                    "Global.use_cls": False,
                    "Global.log_level": "warning",
                    "EngineConfig.onnxruntime.intra_op_num_threads": threads,
                    "EngineConfig.onnxruntime.inter_op_num_threads": 1,
                }
            )
        except ImportError as e:
            raise RpcError("dependency_missing", f"rapidocr backend missing: {e}") from e
        except Exception as e:
            raise RpcError("internal", f"cannot start RapidOCR: {type(e).__name__}: {e}") from e
    return _ENGINE


def normalize_text(t: str) -> str:
    return re.sub(r"\s+", " ", t).strip().casefold()


def _keep(text: str, conf: float) -> bool:
    if not any(ch.isalnum() for ch in text):
        return False
    if conf < MIN_CONF:
        return False
    return len(text) >= 2 or conf >= 0.8


def recognize(img) -> List[Dict[str, Any]]:
    """OCR one BGR image -> lines (top-to-bottom) with normalized boxes."""
    h, w = img.shape[:2]
    res = _engine()(img)
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
        _engine()  # fail early (and load the models once) before decoding

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
                lines = recognize(small)
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
    return {
        "frames": frames,
        "width": width,
        "height": height,
        "samples": done,
        "every": every,
        "engine": "rapidocr/PP-OCRv6-small",
    }
