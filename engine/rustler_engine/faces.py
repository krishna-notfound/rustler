"""Face tracking samples for the face-reframe crop path (``faces.track``).

Detector: OpenCV ``cv2.FaceDetectorYN`` with the YuNet ONNX model
(``face_detection_yunet_2023mar.onnx``, MIT, opencv_zoo). The model is cached at
``$RUSTLER_HOME/models/yunet/`` (default ``~/.rustler``) and downloaded on first use.
Output boxes are normalized to 0..1 of the frame; smoothing happens in Rust
(``rustler_media::facepath``).
"""

from __future__ import annotations

import logging
import os
import shutil
import tempfile
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

from .rpc import RpcError

log = logging.getLogger("rustler_engine.faces")

YUNET_FILENAME = "face_detection_yunet_2023mar.onnx"
YUNET_URL = (
    "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/" + YUNET_FILENAME
)
SCORE_THRESHOLD = 0.6
NMS_THRESHOLD = 0.3
TOP_K = 50
#: Frames are downscaled to at most this width before detection (boxes are normalized anyway).
MAX_DETECT_WIDTH = 640
MAX_SAMPLES = 20000


def rustler_home() -> Path:
    env = os.environ.get("RUSTLER_HOME")
    return Path(env) if env else Path.home() / ".rustler"


def default_model_path() -> Path:
    return rustler_home() / "models" / "yunet" / YUNET_FILENAME


def ensure_model(model_path: Optional[str] = None, progress: Optional[Callable[..., None]] = None) -> Path:
    """Return a usable YuNet model path, downloading the default model when missing."""
    if model_path:
        p = Path(model_path)
        if not p.is_file():
            raise RpcError("validation", f"model_path not found: {model_path}")
        return p
    p = default_model_path()
    if p.is_file() and p.stat().st_size > 0:
        return p
    p.parent.mkdir(parents=True, exist_ok=True)
    if progress:
        progress("models.fetch", 0.0, f"downloading yunet {YUNET_FILENAME}")
    log.info("downloading %s -> %s", YUNET_URL, p)
    fd, tmp = tempfile.mkstemp(dir=str(p.parent), suffix=".part")
    try:
        with os.fdopen(fd, "wb") as out, urllib.request.urlopen(YUNET_URL, timeout=60) as resp:
            shutil.copyfileobj(resp, out)
        if os.path.getsize(tmp) < 10_000:
            raise RpcError("media", "downloaded YuNet model is implausibly small")
        os.replace(tmp, p)
    except RpcError:
        raise
    except Exception as e:
        raise RpcError("media", f"could not download YuNet model: {type(e).__name__}: {e}") from e
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)
    if progress:
        progress("models.fetch", 100.0, "fetched yunet")
    return p


def fetch_yunet(progress: Callable[..., None]) -> str:
    return str(ensure_model(None, progress))


def _num(params: Dict[str, Any], key: str, default: Optional[float]) -> Optional[float]:
    v = params.get(key, default)
    if v is None:
        return None
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise RpcError("validation", f"{key} must be a number")
    return float(v)


def track(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    video_path = params.get("video_path")
    if not isinstance(video_path, str) or not video_path:
        raise RpcError("validation", "video_path is required")
    start = _num(params, "start", 0.0)
    end = _num(params, "end", None)
    fps = _num(params, "fps", 4.0)
    if start < 0:
        raise RpcError("validation", "start must be >= 0")
    if fps <= 0 or fps > 60:
        raise RpcError("validation", "fps must be in (0, 60]")
    if end is not None and end <= start:
        raise RpcError("validation", "end must be greater than start")
    model_path = params.get("model_path")
    if model_path is not None and not isinstance(model_path, str):
        raise RpcError("validation", "model_path must be a string")
    if not os.path.isfile(video_path):
        raise RpcError("validation", f"video not found: {video_path}")

    import cv2

    model = ensure_model(model_path, progress)
    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        raise RpcError("media", f"cannot open video: {video_path}")
    try:
        width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
        height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
        src_fps = cap.get(cv2.CAP_PROP_FPS) or 0.0
        nframes = cap.get(cv2.CAP_PROP_FRAME_COUNT) or 0.0
        duration = nframes / src_fps if src_fps > 0 and nframes > 0 else None
        if end is None:
            if duration is None:
                raise RpcError("validation", "end is required (video duration unknown)")
            end = duration
        if width <= 0 or height <= 0:
            raise RpcError("media", f"video has no frame size: {video_path}")

        scale = min(1.0, MAX_DETECT_WIDTH / width)
        dw, dh = max(1, round(width * scale)), max(1, round(height * scale))
        try:
            detector = cv2.FaceDetectorYN.create(
                str(model), "", (dw, dh), SCORE_THRESHOLD, NMS_THRESHOLD, TOP_K
            )
        except cv2.error as e:
            raise RpcError("media", f"cannot load YuNet model {model}: {e}") from e

        step = 1.0 / fps
        n = int((end - start) / step + 1e-9)
        if n < 1 or n > MAX_SAMPLES:
            raise RpcError("validation", f"range yields {n} samples (allowed 1..{MAX_SAMPLES})")
        samples: List[Dict[str, Any]] = []
        for i in range(n):
            t = start + i * step
            cap.set(cv2.CAP_PROP_POS_MSEC, t * 1000.0)
            ok, frame = cap.read()
            if not ok or frame is None:
                break  # past the end of the stream
            if scale < 1.0:
                frame = cv2.resize(frame, (dw, dh), interpolation=cv2.INTER_AREA)
            detector.setInputSize((frame.shape[1], frame.shape[0]))
            _, rows = detector.detect(frame)
            faces = []
            if rows is not None:
                fh, fw = frame.shape[:2]
                for r in rows:
                    x0 = min(max(float(r[0]) / fw, 0.0), 1.0)
                    y0 = min(max(float(r[1]) / fh, 0.0), 1.0)
                    x1 = min(max((float(r[0]) + float(r[2])) / fw, 0.0), 1.0)
                    y1 = min(max((float(r[1]) + float(r[3])) / fh, 0.0), 1.0)
                    if x1 <= x0 or y1 <= y0:
                        continue
                    faces.append(
                        {
                            "x": round(x0, 5),
                            "y": round(y0, 5),
                            "w": round(x1 - x0, 5),
                            "h": round(y1 - y0, 5),
                            "score": round(float(r[-1]), 4),
                        }
                    )
            faces.sort(key=lambda f: f["w"] * f["h"], reverse=True)
            samples.append({"t": round(t, 4), "faces": faces})
            if i % 8 == 0 or i == n - 1:
                progress("faces.track", round((i + 1) / n * 100, 1))
    finally:
        cap.release()
    return {"width": width, "height": height, "samples": samples}
