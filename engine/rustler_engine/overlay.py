"""Overlay (facecam) rectangle snapping (``overlay.find_box``).

Given a face box that sits inside a picture-in-picture overlay (a streamer webcam), find the
overlay's axis-aligned rectangle from edges instead of guessing it from the face size:

1. For every sampled frame, a Canny edge map is computed per colour channel (OR-ed, so a
   colour-only discontinuity counts) in a window around the face.
2. The maps are averaged across frames. Borders of an overlay are static, so they survive the
   average; moving gameplay and the moving person wash out.
3. A border candidate for each side is a column/row that is an edge over most of the face's span.
   All combinations (left, right, top, bottom) are scored on the **full side length** of the whole
   rectangle, using prefix sums of the averaged map. Rectangles whose every border is supported
   (>= ``SIDE_MIN``) are valid; the tightest valid one wins. A frame edge can stand in for a
   border (an overlay flush with the corner of the frame has no edge there), at least two borders
   must be real edges.
4. A double border (a 4 px frame around the picture) snaps to the outer line so the frame is kept.

Confidence combines the mean border support with the share of individual frames that support the
rectangle on their own. Callers fall back to the face-size expansion when it is low.
Boxes are normalized 0..1 of the frame.
"""

from __future__ import annotations

import logging
import os
from typing import Any, Callable, Dict, List, Optional, Tuple

from .rpc import RpcError

log = logging.getLogger("rustler_engine.overlay")

MAX_FRAMES = 40
MAX_WIDTH = 1920
#: A border must be an edge over at least this share of its length (averaged over frames).
SIDE_MIN = 0.6
#: A border candidate must be an edge over this share of the face's span.
CAND_MIN = 0.5
#: A side counts for a single frame when at least this share of it is an edge in that frame.
FRAME_SIDE_MIN = 0.45
#: Search window around the face, in face sizes.
REACH_X = 4.0
REACH_UP = 3.0
REACH_DOWN = 4.0
#: Minimum gap between the face box and a border, in face sizes (the face is inside the overlay).
MIN_GAP = 0.12
#: A frame edge standing in for a border counts as this much support.
VIRTUAL_SUPPORT = 0.75
MAX_AREA = 0.5
MAX_CANDIDATES = 10
#: Outward search for the outer line of a double border, in px of a 1920-wide frame.
DOUBLE_BORDER_PX = 12


def _num(v: Any, name: str) -> float:
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise RpcError("validation", f"{name} must be a number")
    return float(v)


def _parse_near(params: Dict[str, Any]) -> Dict[str, float]:
    near = params.get("near")
    if not isinstance(near, dict):
        raise RpcError("validation", "near is required: {x, y, w, h} normalized face box")
    box = {k: _num(near.get(k), f"near.{k}") for k in ("x", "y", "w", "h")}
    if box["w"] <= 0 or box["h"] <= 0 or box["x"] < 0 or box["y"] < 0 or box["x"] + box["w"] > 1.0001 or box["y"] + box["h"] > 1.0001:
        raise RpcError("validation", "near must be a normalized box inside the frame with w, h > 0")
    return box


def load_frames(params: Dict[str, Any]) -> List[Any]:
    """BGR frames from ``image_path`` or ``video_path`` + ``times``."""
    import cv2
    import numpy as np

    image_path = params.get("image_path")
    video_path = params.get("video_path")
    if image_path is not None:
        if not isinstance(image_path, str) or not os.path.isfile(image_path):
            raise RpcError("validation", f"image not found: {image_path}")
        img = cv2.imdecode(np.fromfile(image_path, dtype=np.uint8), cv2.IMREAD_COLOR)
        if img is None:
            raise RpcError("media", f"cannot decode image: {image_path}")
        return [img]
    if not isinstance(video_path, str) or not video_path:
        raise RpcError("validation", "image_path or video_path (+ times) is required")
    times = params.get("times")
    if not isinstance(times, list) or not times or len(times) > MAX_FRAMES:
        raise RpcError("validation", f"times must list 1..{MAX_FRAMES} seconds")
    ts = sorted({round(_num(t, "times[]"), 4) for t in times if _num(t, "times[]") >= 0})
    if not ts:
        raise RpcError("validation", "times has no non-negative value")
    if not os.path.isfile(video_path):
        raise RpcError("validation", f"video not found: {video_path}")
    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        raise RpcError("media", f"cannot open video: {video_path}")
    frames = []
    try:
        for t in ts:
            cap.set(cv2.CAP_PROP_POS_MSEC, t * 1000.0)
            ok, frame = cap.read()
            if ok and frame is not None:
                frames.append(frame)
    finally:
        cap.release()
    if not frames:
        raise RpcError("media", f"no frame could be read at the given times: {video_path}")
    return frames


def edge_map(img):
    """Binary edge map (uint8 0/1): Canny on each colour channel, OR-ed."""
    import cv2
    import numpy as np

    blur = cv2.GaussianBlur(img, (3, 3), 0)
    e = np.zeros(img.shape[:2], dtype=np.uint8)
    for c in range(blur.shape[2] if blur.ndim == 3 else 1):
        ch = blur[:, :, c] if blur.ndim == 3 else blur
        e |= (cv2.Canny(ch, 40, 120) > 0).astype(np.uint8)
    return e


def _dilate(a, axis: int):
    """Max over a +-1 neighbourhood along ``axis`` (axis 1: horizontal, for vertical borders)."""
    import numpy as np

    out = a.copy()
    if axis == 1:
        out[:, 1:] = np.maximum(out[:, 1:], a[:, :-1])
        out[:, :-1] = np.maximum(out[:, :-1], a[:, 1:])
    else:
        out[1:, :] = np.maximum(out[1:, :], a[:-1, :])
        out[:-1, :] = np.maximum(out[:-1, :], a[1:, :])
    return out


class _Prefix:
    """Prefix sums of a dilated edge map for O(1) border support lookups."""

    def __init__(self, e):
        import numpy as np

        f = e.astype(np.float32)
        dh, dv = _dilate(f, 1), _dilate(f, 0)
        h, w = f.shape
        self.rows = np.zeros((h + 1, w), dtype=np.float64)  # cumulative along y of dh: vertical borders
        self.rows[1:] = np.cumsum(dh, axis=0)
        self.cols = np.zeros((h, w + 1), dtype=np.float64)  # cumulative along x of dv: horizontal borders
        self.cols[:, 1:] = np.cumsum(dv, axis=1)

    def vert(self, x: int, y0: int, y1: int) -> float:
        """Share of rows y0..y1 (exclusive) that are an edge at column x."""
        return float((self.rows[y1, x] - self.rows[y0, x]) / max(1, y1 - y0))

    def horiz(self, y: int, x0: int, x1: int) -> float:
        return float((self.cols[y, x1] - self.cols[y, x0]) / max(1, x1 - x0))


def _candidates(profile, lo: int, hi: int, limit_virtual: Optional[int]) -> List[Tuple[int, float, bool]]:
    """Peaks of ``profile[lo:hi]`` above CAND_MIN as (index, support, virtual), best first (at most
    MAX_CANDIDATES, at least 4 px apart), plus an optional virtual candidate at the frame edge."""
    out: List[Tuple[int, float, bool]] = []
    if hi > lo:
        order = sorted(range(lo, hi), key=lambda i: -float(profile[i]))
        for i in order:
            v = float(profile[i])
            if v < CAND_MIN or len(out) >= MAX_CANDIDATES:
                break
            if all(abs(i - j) >= 4 for j, _, _ in out):
                out.append((i, v, False))
    if limit_virtual is not None:
        out.append((limit_virtual, VIRTUAL_SUPPORT, True))
    return out


def find_rect(frames: List[Any], near: Dict[str, float]) -> Dict[str, Any]:
    """Snap the overlay rectangle around ``near`` (normalized face box). See the module doc."""
    import cv2
    import numpy as np

    h0, w0 = frames[0].shape[:2]
    scale = min(1.0, MAX_WIDTH / w0)
    if scale < 1.0:
        frames = [cv2.resize(f, (round(w0 * scale), round(h0 * scale)), interpolation=cv2.INTER_AREA) for f in frames]
    H, W = frames[0].shape[:2]
    fx0, fy0 = near["x"] * W, near["y"] * H
    fx1, fy1 = (near["x"] + near["w"]) * W, (near["y"] + near["h"]) * H
    fw, fh = fx1 - fx0, fy1 - fy0
    rx0, rx1 = max(0, int(fx0 - REACH_X * fw)), min(W, int(fx1 + REACH_X * fw) + 1)
    ry0, ry1 = max(0, int(fy0 - REACH_UP * fh)), min(H, int(fy1 + REACH_DOWN * fh) + 1)
    # face in window coordinates
    ax0, ay0, ax1, ay1 = int(round(fx0 - rx0)), int(round(fy0 - ry0)), int(round(fx1 - rx0)), int(round(fy1 - ry0))
    ww, wh = rx1 - rx0, ry1 - ry0
    if ww < 8 or wh < 8 or ax1 <= ax0 or ay1 <= ay0:
        return {"rect": None, "confidence": 0.0, "reason": "face box leaves no search window"}

    maps = [edge_map(f[ry0:ry1, rx0:rx1]) for f in frames]
    mean = np.mean([m.astype(np.float32) for m in maps], axis=0)
    pre = _Prefix(mean)
    frame_pre = [_Prefix(m) for m in maps] if len(maps) > 1 else [pre]

    gx, gy = max(2, int(round(MIN_GAP * fw))), max(2, int(round(MIN_GAP * fh)))
    dh, dv = _dilate(mean, 1), _dilate(mean, 0)
    col_prof = dh[ay0:ay1, :].mean(axis=0)  # vertical border candidates, support over the face rows
    row_prof = dv[:, ax0:ax1].mean(axis=1)  # horizontal border candidates, support over the face columns
    left = _candidates(col_prof, 0, ax0 - gx, 0 if rx0 == 0 else None)
    right = _candidates(col_prof, ax1 + gx, ww, ww - 1 if rx1 == W else None)
    top = _candidates(row_prof, 0, ay0 - gy, 0 if ry0 == 0 else None)
    bottom = _candidates(row_prof, ay1 + gy, wh, wh - 1 if ry1 == H else None)
    if not (left and right and top and bottom):
        return {"rect": None, "confidence": 0.0, "reason": "no border candidates on every side of the face"}

    best = None
    for L, _, lv in left:
        for R, _, rv in right:
            for T, _, tv in top:
                for B, _, bv in bottom:
                    if sum((lv, rv, tv, bv)) > 2:
                        continue  # at least two borders must be real edges
                    if (R - L) * (B - T) > MAX_AREA * W * H:
                        continue
                    s_l = VIRTUAL_SUPPORT if lv else pre.vert(L, T, B + 1)
                    s_r = VIRTUAL_SUPPORT if rv else pre.vert(R, T, B + 1)
                    s_t = VIRTUAL_SUPPORT if tv else pre.horiz(T, L, R + 1)
                    s_b = VIRTUAL_SUPPORT if bv else pre.horiz(B, L, R + 1)
                    if min(s_l, s_r, s_t, s_b) < SIDE_MIN:
                        continue
                    area = (R - L) * (B - T)
                    mean_s = (s_l + s_r + s_t + s_b) / 4
                    key = (area, -mean_s)
                    if best is None or key < best[0]:
                        best = (key, (L, R, T, B), (lv, rv, tv, bv))
    if best is None:
        return {"rect": None, "confidence": 0.0, "reason": "no rectangle with supported borders on all four sides"}
    _, (L, R, T, B), virt = best

    # A double border (frame around the picture): move each real side to the outermost supported line.
    reach = max(2, int(round(DOUBLE_BORDER_PX * W / 1920)))

    def outward(pos: int, step: int, limit: int, support) -> int:
        best_pos = pos
        for k in range(1, reach + 1):
            q = pos + step * k
            if not (0 <= q < limit):
                break
            if support(q) >= SIDE_MIN:
                best_pos = q
        return best_pos

    if not virt[0]:
        L = outward(L, -1, ww, lambda q: pre.vert(q, T, B + 1))
    if not virt[1]:
        R = outward(R, 1, ww, lambda q: pre.vert(q, T, B + 1))
    if not virt[2]:
        T = outward(T, -1, wh, lambda q: pre.horiz(q, L, R + 1))
    if not virt[3]:
        B = outward(B, 1, wh, lambda q: pre.horiz(q, L, R + 1))

    def supports(p: _Prefix) -> List[float]:
        return [
            VIRTUAL_SUPPORT if virt[0] else p.vert(L, T, B + 1),
            VIRTUAL_SUPPORT if virt[1] else p.vert(R, T, B + 1),
            VIRTUAL_SUPPORT if virt[2] else p.horiz(T, L, R + 1),
            VIRTUAL_SUPPORT if virt[3] else p.horiz(B, L, R + 1),
        ]

    sup = supports(pre)
    ok_frames = sum(1 for p in frame_pre if min(supports(p)) >= FRAME_SIDE_MIN)
    frac = ok_frames / len(frame_pre)
    confidence = max(0.0, min(1.0, 0.5 * (sum(sup) / 4) + 0.5 * frac))

    # Sub-edge placement: the Canny line can sit on either side of the true colour step, so put each
    # real border on the strongest step between neighbouring pixels along its whole length.
    wins = [f[ry0:ry1, rx0:rx1].astype(np.int16) for f in frames]

    def step(axis: int, k: int, lo: int, hi: int) -> float:
        """Mean colour step between line k and k+1 (axis 1: columns, 0: rows) over span lo..hi."""
        tot = 0.0
        for w_ in wins:
            if axis == 1:
                tot += float(np.abs(w_[lo:hi, k + 1] - w_[lo:hi, k]).sum(axis=-1).mean())
            else:
                tot += float(np.abs(w_[k + 1, lo:hi] - w_[k, lo:hi]).sum(axis=-1).mean())
        return tot

    def boundary(axis: int, pos: int, lo: int, hi: int, limit: int) -> int:
        ks = [k for k in range(pos - 2, pos + 2) if 0 <= k < limit - 1]
        return max(ks, key=lambda k: step(axis, k, lo, hi)) + 1 if ks else pos

    xl = L if virt[0] else boundary(1, L, T, B + 1, ww)
    xr = R + 1 if virt[1] else boundary(1, R, T, B + 1, ww)
    yt = T if virt[2] else boundary(0, T, L, R + 1, wh)
    yb = B + 1 if virt[3] else boundary(0, B, L, R + 1, wh)
    x0 = (rx0 + xl) / W
    x1 = min(1.0, (rx0 + xr) / W)
    y0 = (ry0 + yt) / H
    y1 = min(1.0, (ry0 + yb) / H)
    if virt[0]:
        x0 = 0.0
    if virt[1]:
        x1 = 1.0
    if virt[2]:
        y0 = 0.0
    if virt[3]:
        y1 = 1.0
    names = ("left", "right", "top", "bottom")
    return {
        "rect": {"x": round(x0, 5), "y": round(y0, 5), "w": round(x1 - x0, 5), "h": round(y1 - y0, 5)},
        "confidence": round(confidence, 4),
        "sides": {n: {"support": round(s, 3), "frame_edge": bool(v)} for n, s, v in zip(names, sup, virt)},
        "frames_supporting": ok_frames,
    }


def find_box(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    near = _parse_near(params)
    frames = load_frames(params)
    progress("overlay.find_box", 50.0)
    res = find_rect(frames, near)
    progress("overlay.find_box", 100.0)
    h, w = frames[0].shape[:2]
    out: Dict[str, Any] = {
        "rect": res["rect"],
        "confidence": res["confidence"],
        "source": "edges" if res["rect"] else "none",
        "frames": len(frames),
        "width": w,
        "height": h,
    }
    for k in ("sides", "frames_supporting", "reason"):
        if k in res:
            out[k] = res[k]
    return out
