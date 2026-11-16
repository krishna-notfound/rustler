"""``faces.active``: one light pass over a video range for smart layouts and the light active speaker.

Lite install only (OpenCV headless + the YuNet model, no diarization, no face recognition):

* **Faces** at ``fps`` (default 5) with YuNet boxes and the five landmarks.
* **Mouth motion** per face: three frames about 0.05 s apart are compared inside a mouth rectangle
  placed from the two mouth-corner landmarks. Head motion and codec noise are removed by
  subtracting the same measure in an eye band of the same face, and the result is divided by the
  local contrast so dark and bright footage are comparable (``mouth``, about 0..1).
* **Tracks**: a face in one shot (nearest centre within 0.15 of the frame width, width ratio
  <= 1.7, seen in the last 1.5 s). No identity across cuts (that is ``speakers.active`` with SFace).
* **Scan** (``scan: true``): every decoded frame is reduced to a 64x36 luma-normalised thumbnail;
  the frame-to-frame difference (histogram + grid of the normalised luma, so dark footage cuts are
  as visible as bright ones) gives frame-accurate cut candidates (``peaks``: local maxima with the
  median difference around them). Every ``stats_every``-th sample also gets graphics statistics
  (``g``): share of flat 8x8 blocks, edge density and the horizontal extent of the edges.

Frames are read sequentially (one seek to ``start``), so the burst frames and the scan are cheap.
"""

from __future__ import annotations

import logging
import os
from typing import Any, Callable, Dict, List, Optional, Tuple

from .rpc import RpcError

log = logging.getLogger("rustler_engine.active")

#: Frames are analysed at up to this width.
MAX_WIDTH = 960
MAX_SAMPLES = 20000
#: Track matching: nearest centre within this share of the frame width.
TRACK_MAX_DX = 0.15
TRACK_MAX_WIDTH_RATIO = 1.7
TRACK_HOLD_SECONDS = 1.5
#: Thumbnail for cut detection.
THUMB_W, THUMB_H = 64, 36
#: Luma std floor for the normalisation (near-black frames are not amplified into noise).
THUMB_STD_FLOOR = 6.0
#: Peaks below this difference are not reported.
PEAK_FLOOR = 0.12
#: Median window (seconds, either side) for a peak's local baseline.
PEAK_BASE_SECONDS = 1.0
#: Contrast floor (grey levels) for the mouth-motion normalisation.
CONTRAST_FLOOR = 8.0


def _clamp(v: float, lo: float, hi: float) -> float:
    return lo if v < lo else hi if v > hi else v


def mouth_rects(lm: List[List[float]], fw: int, fh: int) -> Optional[Tuple[Tuple[int, int, int, int], Tuple[int, int, int, int]]]:
    """Pixel rectangles ``(x0, y0, x1, y1)`` of the mouth (from the corners, extended down for the
    jaw) and of a reference eye band of the same face. ``None`` when the landmarks are degenerate."""
    (rex, rey), (lex, ley), _nose, (rmx, rmy), (lmx, lmy) = [(p[0] * fw, p[1] * fh) for p in lm]
    mw = ((lmx - rmx) ** 2 + (lmy - rmy) ** 2) ** 0.5
    ew = ((lex - rex) ** 2 + (ley - rey) ** 2) ** 0.5
    if mw < 3 or ew < 3:
        return None
    cx, cy = (rmx + lmx) / 2, (rmy + lmy) / 2
    mouth = (cx - 0.8 * mw, cy - 0.45 * mw, cx + 0.8 * mw, cy + 0.75 * mw)
    ex, ey = (rex + lex) / 2, (rey + ley) / 2
    eyes = (ex - 0.8 * ew, ey - 0.3 * ew, ex + 0.8 * ew, ey + 0.3 * ew)

    def px(r):
        x0, y0, x1, y1 = r
        x0, x1 = int(_clamp(round(x0), 0, fw - 1)), int(_clamp(round(x1), 1, fw))
        y0, y1 = int(_clamp(round(y0), 0, fh - 1)), int(_clamp(round(y1), 1, fh))
        return (x0, y0, max(x1, x0 + 1), max(y1, y0 + 1))

    return px(mouth), px(eyes)


def mouth_energy(burst, rects) -> float:
    """Mouth motion over a burst of grey frames: mean |frame difference| in the mouth rectangle
    minus the same in the eye band (head motion, codec noise), over the mouth's contrast."""
    import numpy as np

    (mx0, my0, mx1, my1), (ex0, ey0, ex1, ey1) = rects
    if len(burst) < 2:
        return 0.0
    vals = []
    for a, b in zip(burst, burst[1:]):
        ma = a[my0:my1, mx0:mx1].astype(np.float32)
        mb = b[my0:my1, mx0:mx1].astype(np.float32)
        ea = a[ey0:ey1, ex0:ex1].astype(np.float32)
        eb = b[ey0:ey1, ex0:ex1].astype(np.float32)
        if ma.size == 0 or ea.size == 0:
            continue
        dm = float(np.mean(np.abs(ma - mb)))
        de = float(np.mean(np.abs(ea - eb)))
        contrast = max(CONTRAST_FLOOR, float(np.std(ma)))
        vals.append(max(0.0, dm - de) / contrast)
    return float(sum(vals) / len(vals)) if vals else 0.0


def assign_tracks(samples: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Positional tracks (one face in one shot). Writes ``track`` into each face; returns a row per
    track with ``samples``, ``mean_x``, ``mean_w`` and the mouth percentiles ``mouth_p20``/``mouth_p80``."""
    active: Dict[int, Tuple[float, float, float]] = {}  # id -> (t, cx, w)
    next_id = 0
    for s in samples:
        used = set()
        for f in sorted(s["faces"], key=lambda f: f["w"] * f["h"], reverse=True):
            cx = f["x"] + f["w"] / 2
            best, best_d = None, None
            for tid, (tt, tcx, tw) in active.items():
                if tid in used or s["t"] - tt > TRACK_HOLD_SECONDS:
                    continue
                d = abs(cx - tcx)
                ratio = max(f["w"], tw) / max(1e-6, min(f["w"], tw))
                if d <= TRACK_MAX_DX and ratio <= TRACK_MAX_WIDTH_RATIO and (best_d is None or d < best_d):
                    best, best_d = tid, d
            if best is None:
                best = next_id
                next_id += 1
            used.add(best)
            active[best] = (s["t"], cx, f["w"])
            f["track"] = best
    rows: Dict[int, Dict[str, Any]] = {}
    mouths: Dict[int, List[float]] = {}
    for s in samples:
        for f in s["faces"]:
            r = rows.setdefault(f["track"], {"id": f["track"], "samples": 0, "sx": 0.0, "sw": 0.0})
            r["samples"] += 1
            r["sx"] += f["x"] + f["w"] / 2
            r["sw"] += f["w"]
            if "mouth" in f:
                mouths.setdefault(f["track"], []).append(f["mouth"])
    out = []
    for tid in sorted(rows):
        r = rows[tid]
        m = sorted(mouths.get(tid, []))
        row = {
            "id": tid,
            "samples": r["samples"],
            "mean_x": round(r["sx"] / r["samples"], 4),
            "mean_w": round(r["sw"] / r["samples"], 4),
        }
        if m:
            row["mouth_p20"] = round(m[int(0.2 * (len(m) - 1))], 5)
            row["mouth_p80"] = round(m[int(0.8 * (len(m) - 1))], 5)
        out.append(row)
    return out


def thumb(gray):
    """Luma-normalised 64x36 thumbnail (float32): zero mean, unit std (std floored)."""
    import cv2
    import numpy as np

    t = cv2.resize(gray, (THUMB_W, THUMB_H), interpolation=cv2.INTER_AREA).astype(np.float32)
    return (t - float(t.mean())) / max(THUMB_STD_FLOOR, float(t.std()))


def thumb_diff(a, b) -> float:
    """0..1 difference of two normalised thumbnails: half histogram distance, half grid distance."""
    import numpy as np

    ha, _ = np.histogram(np.clip(a, -3, 3), bins=24, range=(-3, 3))
    hb, _ = np.histogram(np.clip(b, -3, 3), bins=24, range=(-3, 3))
    hist = 0.5 * float(np.abs(ha / max(1, ha.sum()) - hb / max(1, hb.sum())).sum())
    ga = a.reshape(4, THUMB_H // 4, 8, THUMB_W // 8).mean(axis=(1, 3))
    gb = b.reshape(4, THUMB_H // 4, 8, THUMB_W // 8).mean(axis=(1, 3))
    grid = min(1.0, float(np.abs(ga - gb).mean()) / 1.5)
    return 0.5 * hist + 0.5 * grid


def peaks_of(diffs: List[Tuple[float, float]]) -> List[Dict[str, float]]:
    """Local maxima (±3 frames) of the frame differences with their local median baseline."""
    out = []
    n = len(diffs)
    for i, (t, d) in enumerate(diffs):
        if d < PEAK_FLOOR:
            continue
        lo, hi = max(0, i - 3), min(n, i + 4)
        if any(diffs[j][1] > d for j in range(lo, hi) if j != i):
            continue
        around = sorted(x for (tt, x) in diffs if abs(tt - t) <= PEAK_BASE_SECONDS and tt != t)
        base = around[len(around) // 2] if around else 0.0
        out.append({"t": round(t, 4), "diff": round(d, 4), "base": round(base, 4)})
    return out


def graphics_stats(gray) -> Dict[str, float]:
    """Flat-block share, edge density and horizontal edge extent of a frame (320 px wide)."""
    import cv2
    import numpy as np

    h, w = gray.shape[:2]
    sw = 320
    sh = max(8, round(h * sw / w))
    g = cv2.resize(gray, (sw, sh), interpolation=cv2.INTER_AREA)
    bh, bw = sh // 8, sw // 8
    blocks = g[: bh * 8, : bw * 8].reshape(bh, 8, bw, 8).astype(np.float32)
    flat = float((blocks.std(axis=(1, 3)) < 3.0).mean())
    edges = cv2.Canny(g, 60, 160)
    density = float((edges > 0).mean())
    cols = np.nonzero(edges.any(axis=0))[0]
    weights = (edges > 0).sum(axis=0)
    if cols.size:
        cum = np.cumsum(weights) / max(1, weights.sum())
        x_lo = float(np.searchsorted(cum, 0.05)) / sw
        x_hi = float(np.searchsorted(cum, 0.95)) / sw
    else:
        x_lo, x_hi = 0.5, 0.5
    return {"flat": round(flat, 4), "edges": round(density, 4), "x_lo": round(x_lo, 4), "x_hi": round(x_hi, 4)}


def active(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    from . import faces as faces_mod

    video_path = params.get("video_path")
    if not isinstance(video_path, str) or not video_path:
        raise RpcError("validation", "video_path is required")
    start = faces_mod._num(params, "start", 0.0)
    end = faces_mod._num(params, "end", None)
    fps = faces_mod._num(params, "fps", 5.0)
    motion = params.get("motion", True)
    scan = params.get("scan", False)
    stats_every = params.get("stats_every", 2)
    if not isinstance(motion, bool) or not isinstance(scan, bool):
        raise RpcError("validation", "motion and scan must be booleans")
    if isinstance(stats_every, bool) or not isinstance(stats_every, int) or stats_every < 1:
        raise RpcError("validation", "stats_every must be a positive integer")
    if start < 0:
        raise RpcError("validation", "start must be >= 0")
    if fps <= 0 or fps > 15:
        raise RpcError("validation", "fps must be in (0, 15]")
    if end is not None and end <= start:
        raise RpcError("validation", "end must be greater than start")
    if not os.path.isfile(video_path):
        raise RpcError("validation", f"video not found: {video_path}")

    import cv2

    model = faces_mod.ensure_model(params.get("model_path"), progress)
    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        raise RpcError("media", f"cannot open video: {video_path}")
    samples: List[Dict[str, Any]] = []
    diffs: List[Tuple[float, float]] = []
    try:
        width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
        height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
        src_fps = cap.get(cv2.CAP_PROP_FPS) or 25.0
        nframes = cap.get(cv2.CAP_PROP_FRAME_COUNT) or 0.0
        if end is None:
            if nframes <= 0:
                raise RpcError("validation", "end is required (video duration unknown)")
            end = nframes / src_fps
        if width <= 0 or height <= 0:
            raise RpcError("media", f"video has no frame size: {video_path}")
        n = int((end - start) * fps + 1e-9)
        if n < 1 or n > MAX_SAMPLES:
            raise RpcError("validation", f"range yields {n} samples (allowed 1..{MAX_SAMPLES})")
        scale = min(1.0, MAX_WIDTH / width)
        dw, dh = max(1, round(width * scale)), max(1, round(height * scale))
        try:
            detector = cv2.FaceDetectorYN.create(
                str(model), "", (dw, dh), faces_mod.SCORE_THRESHOLD, faces_mod.NMS_THRESHOLD, faces_mod.TOP_K
            )
        except cv2.error as e:
            raise RpcError("media", f"cannot load YuNet model {model}: {e}") from e
        burst_gap = max(1, round(src_fps * 0.05))
        burst_len = 3 if motion else 1

        cap.set(cv2.CAP_PROP_POS_MSEC, start * 1000.0)
        frame_i = 0
        prev_thumb = None
        sample_i = 0
        pending: Optional[Dict[str, Any]] = None  # sample collecting its burst
        burst: List[Any] = []
        next_burst_frame = 0
        first_t: Optional[float] = None
        while True:
            if pending is None and sample_i >= n and not scan:
                break
            if not cap.grab():
                break
            pos = cap.get(cv2.CAP_PROP_POS_MSEC) / 1000.0
            if first_t is None:
                first_t = pos if pos > 0 or start == 0 else start
            t = pos if pos > 0 else (first_t + frame_i / src_fps)
            if t >= end - 1e-6 and pending is None and sample_i >= n:
                break
            want_sample = sample_i < n and t + 0.5 / src_fps >= start + sample_i / fps
            want_burst = pending is not None and frame_i >= next_burst_frame
            if not (scan or want_sample or want_burst):
                frame_i += 1
                continue
            ok, frame = cap.retrieve()
            if not ok or frame is None:
                break
            if scale < 1.0:
                frame = cv2.resize(frame, (dw, dh), interpolation=cv2.INTER_AREA)
            gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
            if scan and t < end + 1e-6:
                th = thumb(gray)
                if prev_thumb is not None:
                    diffs.append((t, thumb_diff(prev_thumb, th)))
                prev_thumb = th
            if want_burst:
                burst.append(gray)
                next_burst_frame = frame_i + burst_gap
                if len(burst) >= burst_len:
                    _finish_sample(pending, burst, frame.shape[1], frame.shape[0], motion)
                    pending, burst = None, []
            if want_sample:
                if pending is not None:  # the next sample is due: close the burst with what it has
                    _finish_sample(pending, burst, frame.shape[1], frame.shape[0], motion)
                    pending, burst = None, []
                tq = start + sample_i / fps
                sample = {"t": round(tq, 4), "faces": _detect(detector, frame, faces_mod)}
                if scan and sample_i % stats_every == 0:
                    sample["g"] = graphics_stats(gray)
                samples.append(sample)
                sample_i += 1
                if motion and sample["faces"]:
                    pending, burst = sample, [gray]
                    next_burst_frame = frame_i + burst_gap
                if sample_i % 8 == 0 or sample_i == n:
                    progress("faces.active", round(sample_i / n * 100, 1))
            frame_i += 1
        if pending is not None:
            _finish_sample(pending, burst, dw, dh, motion)
    finally:
        cap.release()
    tracks = assign_tracks(samples)
    out: Dict[str, Any] = {"width": width, "height": height, "fps": fps, "samples": samples, "tracks": tracks}
    if scan:
        out["peaks"] = peaks_of(diffs)
        out["frames"] = len(diffs) + (1 if diffs else 0)
    return out


def _detect(detector, frame, faces_mod) -> List[Dict[str, Any]]:
    detector.setInputSize((frame.shape[1], frame.shape[0]))
    _, rows = detector.detect(frame)
    fh, fw = frame.shape[:2]
    out: List[Dict[str, Any]] = []
    if rows is None:
        return out
    for r in rows:
        x0 = _clamp(float(r[0]) / fw, 0.0, 1.0)
        y0 = _clamp(float(r[1]) / fh, 0.0, 1.0)
        x1 = _clamp((float(r[0]) + float(r[2])) / fw, 0.0, 1.0)
        y1 = _clamp((float(r[1]) + float(r[3])) / fh, 0.0, 1.0)
        if x1 <= x0 or y1 <= y0:
            continue
        out.append(
            {
                "x": round(x0, 5),
                "y": round(y0, 5),
                "w": round(x1 - x0, 5),
                "h": round(y1 - y0, 5),
                "score": round(float(r[-1]), 4),
                "lm": faces_mod.landmarks(r, fw, fh),
            }
        )
    out.sort(key=lambda f: f["w"] * f["h"], reverse=True)
    return out


def _finish_sample(sample: Dict[str, Any], burst, fw: int, fh: int, motion: bool) -> None:
    if not motion:
        return
    for f in sample["faces"]:
        rects = mouth_rects(f["lm"], fw, fh)
        f["mouth"] = round(mouth_energy(burst, rects), 5) if rects is not None and len(burst) >= 2 else 0.0
