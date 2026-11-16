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
  (``g``): share of flat 8x8 blocks, edge density, the horizontal extent of the edges and the
  content bounding box (``box``: textured/edged blocks as connected components, corner logos and
  specks left out) that smart layouts crop graphics to.

* **Pop-ups** (``popups: true``): source pop-ups burned into the video (subscribe / channel cards):
  a white, card-shaped rectangle that appears from one sample to the next in the lower or upper part
  of the frame, stays put for 1-12 s and goes away (``popups``: start, end, box, kind
  ``subscribe`` when it holds YouTube-red pixels else ``card``, and the colour that was there before).

Frames are read sequentially (one seek to ``start``), so the burst frames and the scan are cheap:
frames that only feed the scan are taken as the decoder's luma plane (no colour conversion), and
YuNet runs in a few threads (``RUSTLER_DETECT_THREADS``; default up to 3) while decoding goes on.
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
#: Content box: an 8x8 block (320 px wide frame) is content with this luma std ...
BOX_BLOCK_STD = 4.0
#: ... or this many Canny edge pixels.
BOX_BLOCK_EDGES = 3
#: Components smaller than this share of all content blocks are specks.
BOX_MIN_SHARE = 0.02
#: Components entirely inside this outer band of the frame (share of width/height) ...
BOX_EDGE_BAND = 0.18
#: ... and smaller than this share of the largest component are logos/watermarks.
BOX_EDGE_SMALL = 0.25


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


def thumb_feats(a):
    """(normalised 24-bin histogram over -3..3, 4x8 grid of means) of a normalised thumbnail."""
    import numpy as np

    idx = np.minimum(((np.clip(a, -3, 3) + 3.0) * 4.0).astype(np.int64), 23)  # = np.histogram's bins
    h = np.bincount(idx.ravel(), minlength=24).astype(np.float64)
    h /= max(1.0, float(h.sum()))
    return h, a.reshape(4, THUMB_H // 4, 8, THUMB_W // 8).mean(axis=(1, 3))


def feats_diff(fa, fb) -> float:
    import numpy as np

    hist = 0.5 * float(np.abs(fa[0] - fb[0]).sum())
    grid = min(1.0, float(np.abs(fa[1] - fb[1]).mean()) / 1.5)
    return 0.5 * hist + 0.5 * grid


def thumb_diff(a, b) -> float:
    """0..1 difference of two normalised thumbnails: half histogram distance, half grid distance."""
    return feats_diff(thumb_feats(a), thumb_feats(b))


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
    out = {"flat": round(flat, 4), "edges": round(density, 4), "x_lo": round(x_lo, 4), "x_hi": round(x_hi, 4)}
    box = content_box(blocks.std(axis=(1, 3)), edges, bh, bw)
    if box is not None:
        out["box"] = box
    return out


def content_box(block_std, edges, bh: int, bw: int) -> Optional[List[float]]:
    """Bounding box ``[x0, y0, x1, y1]`` (0..1) of the frame's content on a flat background: the
    8x8 blocks with texture or edges, closed by one block, as connected components. Small
    components near the frame edge (channel logos, watermarks) and specks are left out. ``None``
    when nothing qualifies."""
    import cv2
    import numpy as np

    if bh < 2 or bw < 2:
        return None
    ecount = (edges[: bh * 8, : bw * 8] > 0).reshape(bh, 8, bw, 8).sum(axis=(1, 3))
    mask = ((block_std >= BOX_BLOCK_STD) | (ecount >= BOX_BLOCK_EDGES)).astype(np.uint8)
    if not mask.any():
        return None
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, np.ones((3, 3), np.uint8))
    n, _labels, stats, _ = cv2.connectedComponentsWithStats(mask, connectivity=8)
    comps = []
    for k in range(1, n):
        x, y, w, h, area = (int(v) for v in stats[k])
        comps.append((x / bw, y / bh, (x + w) / bw, (y + h) / bh, area))
    if not comps:
        return None
    total = sum(c[4] for c in comps)
    largest = max(c[4] for c in comps)
    kept = []
    for x0, y0, x1, y1, area in comps:
        if area < BOX_MIN_SHARE * total:
            continue
        at_edge = x0 >= 1 - BOX_EDGE_BAND or x1 <= BOX_EDGE_BAND or y1 <= BOX_EDGE_BAND or y0 >= 1 - BOX_EDGE_BAND
        if at_edge and area < BOX_EDGE_SMALL * largest:
            continue
        kept.append((x0, y0, x1, y1))
    if not kept:
        return None
    return [
        round(min(c[0] for c in kept), 4),
        round(min(c[1] for c in kept), 4),
        round(max(c[2] for c in kept), 4),
        round(max(c[3] for c in kept), 4),
    ]


def detect_workers() -> int:
    """Threads for YuNet (OpenCV releases the GIL while detecting): up to 3, half the CPUs."""
    try:
        env = int(os.environ.get("RUSTLER_DETECT_THREADS", "0"))
    except ValueError:
        env = 0
    if env > 0:
        return min(env, 8)
    return max(1, min(3, (os.cpu_count() or 2) // 2))


# --- source pop-ups ------------------------------------------------------------------------------

#: Pop-up analysis width (px).
POP_W = 480
#: A pop-up pixel: bright and unsaturated (white card) ...
POP_V_MIN = 200
POP_S_MAX = 40
#: ... that is new (absent this long before, s) and stays (at least this long, s).
POP_BACK_S = 0.4
POP_PERSIST_S = 0.6
#: Cards slide out over up to this long (s): a card going away is compared with how it was before.
POP_SLIDE_S = 1.0
#: Card size (share of the frame) and shape.
POP_MIN_W, POP_MAX_W = 0.10, 0.60
POP_MIN_H, POP_MAX_H = 0.035, 0.25
POP_MIN_ASPECT = 1.6
POP_MIN_FILL = 0.75
POP_MIN_WHITE = 0.45
#: Lower band starts / upper band ends here (share of the height).
POP_LOWER = 0.55
POP_UPPER = 0.35
#: A card counts when it stays this long (s).
POP_MIN_S, POP_MAX_S = 1.0, 12.0
#: Share of YouTube-red pixels (the SUBSCRIBE button) that makes a card a subscribe card.
POP_RED_MIN = 0.01


def popup_masks(bgr):
    """(white, red) boolean masks of a frame at ``POP_W`` px wide, plus the small BGR frame."""
    import cv2
    import numpy as np

    h0, w0 = bgr.shape[:2]
    sh = max(8, round(h0 * POP_W / w0))
    small = cv2.resize(bgr, (POP_W, sh), interpolation=cv2.INTER_AREA)
    hsv = cv2.cvtColor(small, cv2.COLOR_BGR2HSV)
    white = (hsv[..., 2] >= POP_V_MIN) & (hsv[..., 1] <= POP_S_MAX)
    red = ((hsv[..., 0] < 8) | (hsv[..., 0] > 172)) & (hsv[..., 1] > 120) & (hsv[..., 2] > 110)
    return white, red, small


def card_boxes(new, white) -> List[Tuple[int, int, int, int]]:
    """Card-shaped components ``(x, y, w, h)`` (px) of the ``new`` mask: a filled, wide rectangle of
    card size in the lower or upper band, mostly white inside."""
    import cv2
    import numpy as np

    sh, sw = new.shape
    m = cv2.morphologyEx(new.astype(np.uint8), cv2.MORPH_CLOSE, np.ones((5, 5), np.uint8))
    cnts, _ = cv2.findContours(m, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    out = []
    for c in cnts:
        x, y, w, h = cv2.boundingRect(c)
        if not (POP_MIN_W * sw <= w <= POP_MAX_W * sw and POP_MIN_H * sh <= h <= POP_MAX_H * sh):
            continue
        if w < POP_MIN_ASPECT * h:
            continue
        if cv2.contourArea(c) < POP_MIN_FILL * w * h:
            continue
        if not (y + h >= POP_LOWER * sh or y <= POP_UPPER * sh):
            continue
        if float(white[y : y + h, x : x + w].mean()) < POP_MIN_WHITE:
            continue
        out.append((x, y, w, h))
    return out


class PopupScan:
    """Streaming detector of source pop-ups (subscribe / channel cards burned into the video): a
    white, card-shaped rectangle that appears from one moment to the next in the lower or upper
    part of the frame, stays put for ``POP_MIN_S``..``POP_MAX_S`` and goes away. Fed one sample
    frame at a time (``fps``)."""

    def __init__(self, fps: float):
        self.fps = fps
        self.back = max(1, round(POP_BACK_S * fps))
        self.persist = max(2, round(POP_PERSIST_S * fps) + 1)
        self.slide = max(1, round(POP_SLIDE_S * fps))
        self.hist: List[Tuple[float, Any, Any, Any]] = []  # (t, white, red, small bgr)
        self.open: List[Dict[str, Any]] = []
        self.done: List[Dict[str, Any]] = []
        self.size: Optional[Tuple[int, int]] = None
        self.first: Optional[Tuple[float, Any]] = None

    def feed(self, t: float, bgr) -> None:
        import numpy as np

        white, red, small = popup_masks(bgr)
        self.size = white.shape
        self.hist.append((t, white, red, small))
        keep = max(self.back + self.persist + 1, self.slide + self.persist + 2)
        if len(self.hist) > keep:
            self.hist.pop(0)
        # Open cards: still there?
        for c in list(self.open):
            x, y, w, h = c["px"]
            share = float(white[y : y + h, x : x + w].mean())
            if share < 0.6 * c["white"]:
                c["miss"] += 1
                if c["miss"] == 1:
                    c["gone_t"] = t
                if c["miss"] >= 2:
                    c["end"] = c["gone_t"] + 1.0 / self.fps  # covers the slide-out
                    self.open.remove(c)
                    self.done.append(c)
            else:
                c["miss"] = 0
                c["red"] = max(c["red"], float(red[y : y + h, x : x + w].mean()))
                if t - c["start"] <= POP_SLIDE_S + POP_BACK_S:
                    # Still sliding in when it was first seen: follow it to where it settles.
                    c["px"] = _union(c["px"], _grow(c["px"], white, most_x=0.03, most_y=0.04))
        if self.first is None:
            self.first = (t, white)
        # A card already on screen when the range starts: its appearance is not seen, but its
        # going away is (white for `persist` samples, then gone for two). Kept when it was there
        # from the first sample on and is a subscribe card (`finish`).
        g = len(self.hist) - 2
        pre_end = g - self.slide  # compared with how it was before it started to slide out
        if pre_end >= 2:
            pre = np.logical_and.reduce([h[1] for h in self.hist[max(0, pre_end - self.persist) : pre_end]])
            gone = pre & ~self.hist[g][1] & ~self.hist[g + 1][1]
            self._find(pre_end - 1, gone, gone_at=g)
        # New cards: white now and for `persist` samples, not white `back` samples earlier.
        if len(self.hist) < self.back + self.persist:
            return
        j = len(self.hist) - self.persist
        run = self.hist[j:]
        self._find(j, np.logical_and.reduce([h[1] for h in run]) & ~self.hist[j - self.back][1])

    def _red(self, px, j: int) -> float:
        """Most YouTube-red share in box ``px`` over the samples up to ``j`` (a SUBSCRIBE button
        turns grey once it is clicked)."""
        x, y, w, h = px
        return max(float(r[y : y + h, x : x + w].mean()) for _, _, r, _ in self.hist[: j + 1])

    def _find(self, j: int, new, gone_at: Optional[int] = None) -> None:
        """Cards in mask ``new`` measured on sample ``j``: appearing ones (opened), or with
        ``gone_at`` (the first sample without it) ones that were there from the start (closed)."""
        import numpy as np

        from_start = gone_at is not None
        tj, wj, rj, _ = self.hist[j]
        for px in card_boxes(new, wj):
            # Parts of the card over something that was already white (a mug, a shirt) are not
            # "new": grow the box over the white columns/rows that continue it.
            px = _grow(px, wj)
            x, y, w, h = px
            # Part of a card already followed (or one that just went: its slide-out)?
            recent = self.open + [c for c in self.done if c["end"] >= tj - 1.0]
            hit = [c for c in recent if _overlap(px, c["px"]) > 0.5]
            if from_start and hit and all(c["from_start"] for c in hit):
                # The same card seen going away a sample later, often whole where it was first
                # seen in pieces (something white behind it): one card, the union of the boxes.
                c = hit[0]
                c["px"] = _union(c["px"], px)
                c["red"] = max(c["red"], self._red(px, j))
                continue
            if hit:
                continue
            share = float(wj[y : y + h, x : x + w].mean())
            card = {
                "px": px,
                # Absent `back` samples before `j`: it came in (slid in) after that.
                "start": self.hist[max(0, j - self.back)][0],
                "from_start": from_start,
                "white": share,
                "red": float(rj[y : y + h, x : x + w].mean()),
                "miss": 0,
            }
            if from_start:
                # `j` is its last sample: it was there from the first sample on, else it is not a
                # card this pass can place in time.
                t0, w0 = self.first
                if float(w0[y : y + h, x : x + w].mean()) < 0.6 * share:
                    continue
                under = self.hist[gone_at][3][y : y + h, x : x + w].reshape(-1, 3)  # after it went
                card.update(
                    start=t0,
                    end=self.hist[gone_at][0] + 1.0 / self.fps,  # covers the slide-out
                    red=self._red(px, j),
                    fill=[int(v) for v in np.median(under, axis=0)][::-1],
                )
                self.done.append(card)
                continue
            under = self.hist[j - self.back][3][y : y + h, x : x + w].reshape(-1, 3)  # before it came
            card["fill"] = [int(v) for v in np.median(under, axis=0)][::-1]  # RGB
            self.open.append(card)

    def finish(self, end_t: float) -> List[Dict[str, Any]]:
        for c in self.open:
            c["end"] = end_t
            c["open_end"] = True
        cards = self.done + self.open
        self.open = []
        out = []
        if self.size is None:
            return out
        sh, sw = self.size
        for c in sorted(cards, key=lambda c: c["start"]):
            dur = c["end"] - c["start"]
            subscribe = c["red"] >= POP_RED_MIN
            if c["from_start"]:
                # Seen only going away: a subscribe card that leaves inside the range (any length
                # up to the usual maximum; its start is the range start).
                if c.get("open_end") or not subscribe or dur > POP_MAX_S:
                    continue
            elif not (POP_MIN_S <= dur <= POP_MAX_S):
                continue
            elif c.get("open_end") and not subscribe:
                continue  # appears and stays until the range ends: only a subscribe card counts
            x, y, w, h = c["px"]
            out.append(
                {
                    "start": round(c["start"], 3),
                    "end": round(c["end"], 3),
                    "box": [round(x / sw, 4), round(y / sh, 4), round((x + w) / sw, 4), round((y + h) / sh, 4)],
                    "kind": "subscribe" if subscribe else "card",
                    "red": round(c["red"], 4),
                    "white": round(c["white"], 3),
                    "fill": "#%02X%02X%02X" % tuple(c["fill"]),
                    "open_end": bool(c.get("open_end", False)),
                    "from_start": bool(c["from_start"]),
                }
            )
        return out


def _reach(profile) -> int:
    """How many entries of ``profile`` (white shares going outward from a box side) still belong
    to the card: the farthest point where the running mean is >= 0.5 and the share itself
    >= 0.3 (an avatar or a line of text inside the card lowers single columns, the background
    beyond the card's edge does not come back)."""
    best, total = 0, 0.0
    for k, v in enumerate(profile, 1):
        total += v
        if total / k >= 0.5 and v >= 0.3:
            best = k
    return best


def _grow(px, white, most_x: float = 0.12, most_y: float = 0.04) -> Tuple[int, int, int, int]:
    """Extends box ``px`` (``x, y, w, h``) over the white columns/rows that continue it, by at
    most ``most_x`` / ``most_y`` of the frame per side."""
    sh, sw = white.shape
    x0, y0, x1, y1 = px[0], px[1], px[0] + px[2], px[1] + px[3]
    lx, ly = int(most_x * sw), int(most_y * sh)
    col = white[y0:y1].mean(axis=0)
    x0 -= _reach(col[max(0, x0 - lx) : x0][::-1])
    x1 += _reach(col[x1 : min(sw, x1 + lx)])
    row = white[:, x0:x1].mean(axis=1)
    y0 -= _reach(row[max(0, y0 - ly) : y0][::-1])
    y1 += _reach(row[y1 : min(sh, y1 + ly)])
    return (x0, y0, x1 - x0, y1 - y0)


def _union(a, b) -> Tuple[int, int, int, int]:
    x0, y0 = min(a[0], b[0]), min(a[1], b[1])
    x1, y1 = max(a[0] + a[2], b[0] + b[2]), max(a[1] + a[3], b[1] + b[3])
    return (x0, y0, x1 - x0, y1 - y0)


def _overlap(a, b) -> float:
    """Intersection over the smaller of two ``(x, y, w, h)`` boxes."""
    ax, ay, aw, ah = a
    bx, by, bw, bh = b
    ix = max(0, min(ax + aw, bx + bw) - max(ax, bx))
    iy = max(0, min(ay + ah, by + bh) - max(ay, by))
    small = min(aw * ah, bw * bh)
    return ix * iy / small if small > 0 else 0.0


# --- the pass ------------------------------------------------------------------------------------


def _scan_cuts(video_path: str, start: float, end: float, size: Tuple[int, int], diffs: List[Tuple[float, float]], box: Dict[str, Any]) -> None:
    """Frame-difference scan over ``start..end`` with its own capture, run in a thread next to the
    sample pass (decoding, colour conversion and resizing release the GIL): every frame's grey
    thumbnail, exactly as the single pass made it. Appends ``(t, diff)`` to ``diffs``.
    (The decoder's raw luma plane would be cheaper, but OpenCV hands it out without reordering
    B-frames, which moved cuts by a few frames.)"""
    import cv2

    try:
        cap = cv2.VideoCapture(video_path)
        if not cap.isOpened():
            raise RpcError("media", f"cannot open video: {video_path}")
        try:
            src_fps = cap.get(cv2.CAP_PROP_FPS) or 25.0
            cap.set(cv2.CAP_PROP_POS_MSEC, start * 1000.0)
            prev = None
            first_t: Optional[float] = None
            frame_i = 0
            while not box.get("stop"):
                if not cap.grab():
                    break
                pos = cap.get(cv2.CAP_PROP_POS_MSEC) / 1000.0
                if first_t is None:
                    first_t = pos if pos > 0 or start == 0 else start
                t = pos if pos > 0 else (first_t + frame_i / src_fps)
                frame_i += 1
                if t >= end + 1e-6:
                    break
                ok, img = cap.retrieve()
                if not ok or img is None:
                    break
                if (img.shape[1], img.shape[0]) != size:
                    img = cv2.resize(img, size, interpolation=cv2.INTER_AREA)
                th = thumb_feats(thumb(cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)))
                if prev is not None:
                    diffs.append((t, feats_diff(prev, th)))
                prev = th
        finally:
            cap.release()
    except Exception as e:  # reported by the caller
        box["error"] = e


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
    popups = params.get("popups", False)
    stats_every = params.get("stats_every", 2)
    if not isinstance(motion, bool) or not isinstance(scan, bool) or not isinstance(popups, bool):
        raise RpcError("validation", "motion, scan and popups must be booleans")
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

    import threading
    from collections import deque
    from concurrent.futures import ThreadPoolExecutor

    import cv2

    model = faces_mod.ensure_model(params.get("model_path"), progress)
    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        raise RpcError("media", f"cannot open video: {video_path}")
    samples: List[Dict[str, Any]] = []
    diffs: List[Tuple[float, float]] = []
    pool: Optional[ThreadPoolExecutor] = None
    pop = PopupScan(fps) if popups else None
    scan_box: Dict[str, Any] = {}
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

        def new_detector():
            return cv2.FaceDetectorYN.create(
                str(model), "", (dw, dh), faces_mod.SCORE_THRESHOLD, faces_mod.NMS_THRESHOLD, faces_mod.TOP_K
            )

        try:
            detector = new_detector()
        except cv2.error as e:
            raise RpcError("media", f"cannot load YuNet model {model}: {e}") from e
        workers = detect_workers()
        local = threading.local()

        def detect_in_thread(frame):
            d = getattr(local, "det", None)
            if d is None:
                d = local.det = new_detector()
            return _detect(d, frame, faces_mod)

        if workers > 1:
            pool = ThreadPoolExecutor(workers, thread_name_prefix="yunet")
        max_pending = 4 * workers
        burst_gap = max(1, round(src_fps * 0.05))
        burst_len = 3 if motion else 1

        # The cut scan reads every frame through its own capture in a thread; this pass only
        # converts the sample and burst frames.
        scan_thread = None
        if scan:
            scan_thread = threading.Thread(target=_scan_cuts, args=(video_path, start, end, (dw, dh), diffs, scan_box), daemon=True)
            scan_thread.start()

        pend: deque = deque()  # samples waiting for their faces and burst, in order
        open_rec: Optional[Dict[str, Any]] = None

        def finish(rec: Dict[str, Any]) -> None:
            faces = rec["fut"].result() if rec["fut"] is not None else rec["faces"]
            rec["sample"]["faces"] = faces
            if motion and faces:
                _finish_sample(rec["sample"], rec["burst"], dw, dh, motion)
            rec["burst"] = None

        def drain(force: bool = False) -> None:
            while pend:
                head = pend[0]
                if head is open_rec and not force:
                    return
                done = head["fut"] is None or head["fut"].done()
                if not (done or force or len(pend) > max_pending):
                    return
                finish(pend.popleft())

        cap.set(cv2.CAP_PROP_POS_MSEC, start * 1000.0)
        frame_i = 0
        sample_i = 0
        next_burst_frame = 0
        first_t: Optional[float] = None
        last_t = start
        while True:
            if open_rec is None and sample_i >= n:
                break
            if not cap.grab():
                break
            pos = cap.get(cv2.CAP_PROP_POS_MSEC) / 1000.0
            if first_t is None:
                first_t = pos if pos > 0 or start == 0 else start
            t = pos if pos > 0 else (first_t + frame_i / src_fps)
            last_t = t
            want_sample = sample_i < n and t + 0.5 / src_fps >= start + sample_i / fps
            want_burst = open_rec is not None and frame_i >= next_burst_frame
            if not (want_sample or want_burst):
                frame_i += 1
                continue
            ok, frame = cap.retrieve()
            if not ok or frame is None:
                break
            if scale < 1.0:
                frame = cv2.resize(frame, (dw, dh), interpolation=cv2.INTER_AREA)
            gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
            if want_burst:
                open_rec["burst"].append(gray)
                next_burst_frame = frame_i + burst_gap
                if len(open_rec["burst"]) >= burst_len:
                    open_rec = None
            if want_sample:
                open_rec = None  # the next sample is due: the burst closes with what it has
                tq = start + sample_i / fps
                sample: Dict[str, Any] = {"t": round(tq, 4), "faces": []}
                rec: Dict[str, Any] = {"sample": sample, "burst": [gray] if motion else [], "fut": None, "faces": None}
                if pool is not None:
                    rec["fut"] = pool.submit(detect_in_thread, frame)
                else:
                    rec["faces"] = _detect(detector, frame, faces_mod)
                if scan and sample_i % stats_every == 0:
                    sample["g"] = graphics_stats(gray)
                if pop is not None:
                    pop.feed(tq, frame)
                samples.append(sample)
                pend.append(rec)
                sample_i += 1
                if motion and burst_len > 1:
                    open_rec = rec
                    next_burst_frame = frame_i + burst_gap
                if sample_i % 8 == 0 or sample_i == n:
                    progress("faces.active", round(sample_i / n * 100, 1))
            drain()
            frame_i += 1
        open_rec = None
        drain(force=True)
        if scan_thread is not None:
            scan_thread.join()
            if "error" in scan_box:
                raise scan_box["error"]
    finally:
        scan_box["stop"] = True
        cap.release()
        if pool is not None:
            pool.shutdown(wait=True)
    tracks = assign_tracks(samples)
    out: Dict[str, Any] = {"width": width, "height": height, "fps": fps, "samples": samples, "tracks": tracks}
    if scan:
        out["peaks"] = peaks_of(diffs)
        out["frames"] = len(diffs) + (1 if diffs else 0)
    if pop is not None:
        out["popups"] = pop.finish(min(end, last_t + 1.0 / fps))
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
