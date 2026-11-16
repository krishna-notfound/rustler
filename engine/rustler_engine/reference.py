"""Light style measures for the style-reference feature (``reference.measure``).

Everything here is OpenCV + numpy on a low-res proxy of one example video; no model is downloaded or
run except the YuNet face detector the engine already ships (skipped with a note when unavailable).
The numbers are *measurements to combine with looking at the contact sheets*, not ground truth, and
each block says how it was measured. Methods, in short:

* **cuts** - consecutive frames are compared after luma normalisation (zero mean, unit variance, so a
  dark or fading shot does not hide a cut), threshold is adaptive (a multiple of the local median
  difference, with a floor) and a candidate must be a local maximum. A candidate whose two frames still
  match geometrically (ORB + similarity transform, same scene only larger or smaller) is a *scale jump*
  (punch-in cut), not a cut.
* **zoom** - the similarity-transform scale between frames ~0.4 s apart (ORB features, RANSAC, features
  inside caption/hook text masked out; left and right frame strips must agree, so a moving subject is not a zoom). Needs texture: ``coverage`` says how many pairs could be measured.
* **text bands** - rows whose horizontal-edge density stands out from the rows around them
  (Sobel-x, rolling-median baseline), kept when a run is 1.2-10 percent of the frame tall. Bands are
  clustered by position over time; a *caption* is a recurring band whose content keeps changing inside a
  shot, a *hook* is text that is up in the first 3 s and gone afterwards, a *static overlay* (logo, mug
  rim, watermark) does not change.
* **words per line** - word blobs along the caption band (gaps wider than a fraction of the text height).
* **colours** - k-means over bright or saturated pixels inside the caption band's word blobs.
* **music** - spectral/temporal heuristic on a 16 kHz mono wav (how deep the pauses go, rhythmic pulse).
"""

from __future__ import annotations

import logging
import os
import wave
from typing import Any, Callable, Dict, List, Optional, Tuple

from .rpc import RpcError

log = logging.getLogger("rustler_engine.reference")

MAX_SECONDS_DEFAULT = 180.0
CUT_FLOOR = 0.22
TEXT_STEP_S = 0.25
ZOOM_GAP_S = 0.4
ZOOM_THRESH = 0.035  # |log scale| per ZOOM_GAP_S that counts as a zoom


# ------------------------------------------------------------------------------------------------
# small helpers
# ------------------------------------------------------------------------------------------------


def _r(x: float, n: int = 3) -> float:
    return round(float(x), n)


def _stats(values: List[float]) -> Dict[str, Any]:
    import numpy as np

    if not values:
        return {"count": 0}
    a = np.asarray(values, dtype=float)
    return {
        "count": int(a.size),
        "mean": _r(a.mean()),
        "median": _r(np.median(a)),
        "min": _r(a.min()),
        "max": _r(a.max()),
        "p25": _r(np.percentile(a, 25)),
        "p75": _r(np.percentile(a, 75)),
    }


# ------------------------------------------------------------------------------------------------
# text bands
# ------------------------------------------------------------------------------------------------


def edge_mask(gray):
    import cv2
    import numpy as np

    gx = np.abs(cv2.Sobel(gray, cv2.CV_16S, 1, 0, ksize=3))
    return (gx > 160).astype(np.uint8)


def find_bands(gray) -> List[Dict[str, Any]]:
    """Text-like horizontal bands of one grayscale frame (pixel boxes plus a small edge-mask signature)."""
    import cv2
    import numpy as np

    h, w = gray.shape
    m = edge_mask(gray)
    d = cv2.dilate(m, np.ones((1, 7), np.uint8))
    p = d.mean(axis=1)
    k = max(3, int(h * 0.006) | 1)
    ps = np.convolve(p, np.ones(k) / k, mode="same")
    win = max(3, int(h * 0.10))
    base = np.empty_like(ps)
    for y in range(h):
        base[y] = np.median(ps[max(0, y - win) : y + win + 1])
    rows = (ps - base > 0.04) & (ps > 0.07)
    gap = max(1, int(h * 0.012))
    runs: List[Tuple[int, int]] = []
    y = 0
    while y < h:
        if rows[y]:
            y0 = y
            while y < h and (rows[y] or rows[min(h - 1, y + gap)]):
                y += 1
            runs.append((y0, y))
        y += 1
    out = []
    for y0, y1 in runs:
        bh = y1 - y0
        if not (h * 0.012 <= bh <= h * 0.10):
            continue
        cols = d[y0:y1].mean(axis=0) > 0.15
        xs = np.where(cols)[0]
        if len(xs) == 0 or (xs[-1] - xs[0]) < w * 0.10:
            continue
        sig = cv2.resize(d[y0:y1].astype(np.float32), (64, 8), interpolation=cv2.INTER_AREA)
        out.append({"y0": int(y0), "y1": int(y1), "x0": int(xs[0]), "x1": int(xs[-1]), "sig": sig, "dens": float(d[y0:y1].mean())})
    return out


def _sig_change(a, b) -> float:
    import numpy as np

    return float(np.abs(a - b).sum() / (a.sum() + b.sum() + 1e-6) * 2.0)


def cluster_bands(
    samples: List[Dict[str, Any]], height: int, width: int, cut_times: List[float], motion: Optional[List[float]] = None
) -> List[Dict[str, Any]]:
    """Group per-sample bands by vertical position; compute presence and intra-shot change."""
    import numpy as np

    items = []
    for si, s in enumerate(samples):
        for b in s["bands"]:
            items.append((si, (b["y0"] + b["y1"]) / 2.0 / height, b))
    if not items:
        return []
    items.sort(key=lambda it: it[1])
    groups: List[List[Tuple[int, float, Dict[str, Any]]]] = [[items[0]]]
    for it in items[1:]:
        if it[1] - groups[-1][-1][1] <= 0.02:
            groups[-1].append(it)
        else:
            groups.append([it])
    n = max(1, len(samples))
    clusters = []
    for g in groups:
        by_sample: Dict[int, Dict[str, Any]] = {}
        for si, _, b in g:
            # one band per sample per cluster: the widest
            if si not in by_sample or (b["x1"] - b["x0"]) > (by_sample[si]["x1"] - by_sample[si]["x0"]):
                by_sample[si] = b
        present = sorted(by_sample)
        if len(present) < 2:
            continue
        ys0 = np.median([by_sample[i]["y0"] for i in present]) / height
        ys1 = np.median([by_sample[i]["y1"] for i in present]) / height
        xs0 = np.percentile([by_sample[i]["x0"] for i in present], 10) / width
        xs1 = np.percentile([by_sample[i]["x1"] for i in present], 90) / width
        changes = []
        mot_changed: List[float] = []
        mot_still: List[float] = []
        for a, b in zip(present, present[1:]):
            if b != a + 1:
                continue
            ta, tb = samples[a]["t"], samples[b]["t"]
            if any(ta < c <= tb for c in cut_times):
                continue
            ch = _sig_change(by_sample[a]["sig"], by_sample[b]["sig"])
            changes.append(ch)
            if motion is not None:
                (mot_changed if ch > 0.35 else mot_still if ch < 0.2 else []).append(motion[a])
        # Text changes whether or not the picture moves; a noisy band changes only when the picture does.
        motion_ratio = 1.0
        if mot_changed and mot_still:
            motion_ratio = float(np.mean(mot_changed) / (np.mean(mot_still) + 0.02))
        change = float(np.mean([c > 0.35 for c in changes])) if changes else 0.0
        jump = float(np.mean([c > 0.6 for c in changes])) if changes else 0.0
        plateau = float(np.mean([c < 0.2 for c in changes])) if changes else 0.0
        cx = float(np.median([(by_sample[i]["x0"] + by_sample[i]["x1"]) / 2.0 for i in present])) / width
        clusters.append(
            {
                "y0": _r(ys0),
                "y1": _r(ys1),
                "x0": _r(xs0),
                "x1": _r(xs1),
                "presence": _r(len(present) / n),
                "change": _r(change),
                "jump": _r(jump),
                "plateau": _r(plateau),
                "cx": _r(cx),
                "motion_ratio": _r(motion_ratio, 2),
                "density": _r(float(np.median([by_sample[i]["dens"] for i in present])), 3),
                "present_samples": present,
                "by_sample": by_sample,
            }
        )
    return clusters


def pick_caption(clusters: List[Dict[str, Any]], duration: float) -> Optional[Dict[str, Any]]:
    best, best_score = None, 0.0
    for c in clusters:
        if c["presence"] < 0.2 or c["change"] < 0.15 or c["plateau"] < 0.2:
            continue
        if (c["x1"] - c["x0"]) < 0.15:
            continue
        dens_w = min(1.5, max(0.1, (c["density"] - 0.12) / 0.12))  # bold stroked captions are edge-dense
        score = c["presence"] * c["change"] * c["plateau"] * dens_w / max(1.0, c["motion_ratio"])
        if score > best_score:
            best, best_score = c, score
    return best


def pick_hook(
    clusters: List[Dict[str, Any]], samples: List[Dict[str, Any]], caption: Optional[Dict[str, Any]]
) -> Optional[Dict[str, Any]]:
    """Text up in the first 3 s that is gone afterwards (not the caption cluster)."""
    first = [i for i, s in enumerate(samples) if s["t"] < 3.0]
    later = [i for i, s in enumerate(samples) if s["t"] >= 3.5]
    if len(first) < 3:
        return None
    cand = []
    for c in clusters:
        if c is caption:
            continue
        if (c["x1"] - c["x0"]) < 0.15:
            continue
        ps = set(c["present_samples"])
        in_first = sum(1 for i in first if i in ps) / len(first)
        after = (sum(1 for i in later if i in ps) / len(later)) if later else 0.0
        if in_first >= 0.5 and after <= 0.25:
            cand.append((c, in_first, after))
    if not cand:
        return None
    cand.sort(key=lambda t: (t[0]["y0"]))
    # merge lines stacked within 7 percent of the frame height of the first one
    base = cand[0][0]
    group = [t for t in cand if t[0]["y0"] - base["y0"] <= 0.12]
    y0 = min(t[0]["y0"] for t in group)
    y1 = max(t[0]["y1"] for t in group)
    x0 = min(t[0]["x0"] for t in group)
    x1 = max(t[0]["x1"] for t in group)
    idx = sorted({i for t in group for i in t[0]["present_samples"] if samples[i]["t"] < 6.0})
    t_from = samples[idx[0]]["t"] if idx else 0.0
    # contiguous visibility from the first present sample
    t_to = t_from
    for a, b in zip(idx, idx[1:]):
        if b - a <= 2:
            t_to = samples[b]["t"]
        else:
            break
    return {
        "present": True,
        "y0": y0,
        "y1": y1,
        "x0": x0,
        "x1": x1,
        "lines": len(group),
        "visible_from": _r(t_from, 2),
        "visible_to": _r(t_to + TEXT_STEP_S, 2),
        "coverage_first_3s": _r(float(np_mean([t[1] for t in group])), 2),
    }


def np_mean(xs):
    return sum(xs) / len(xs) if xs else 0.0


def _runs(mask, close_gap: int, min_w: int) -> List[Tuple[int, int]]:
    xs = [int(i) for i in range(len(mask)) if mask[i]]
    if not xs:
        return []
    runs = []
    s = p = xs[0]
    for x in xs[1:]:
        if x - p - 1 > close_gap:
            runs.append((s, p))
            s = x
        p = x
    runs.append((s, p))
    return [(a, b) for a, b in runs if b - a + 1 >= min_w]


def caption_details(frames: List[Tuple[float, Any]], band: Dict[str, Any], by_sample: Dict[int, Dict[str, Any]], idxs: List[int]):
    """Words per line and fill/outline colours from decoded frames where the caption band is present."""
    import cv2
    import numpy as np

    wpl: List[int] = []
    fill_px: List[Any] = []
    dark_px: List[Any] = []
    for si in idxs:
        img = frames[si][1]
        b = by_sample[si]
        h, w = img.shape[:2]
        y0, y1 = max(0, b["y0"] - 2), min(h, b["y1"] + 2)
        crop = img[y0:y1]
        gray = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)
        m = edge_mask(gray)
        colmask = m.any(axis=0)
        bh = y1 - y0
        runs = _runs(colmask, max(2, int(round(bh * 0.28))), max(3, int(round(bh * 0.2))))
        if runs:
            wpl.append(len(runs))
        sel = np.zeros(gray.shape, dtype=bool)
        for a, c in runs:
            sel[:, a : c + 1] = True
        hsv = cv2.cvtColor(crop, cv2.COLOR_BGR2HSV)
        v = hsv[..., 2].astype(np.int32)
        s = hsv[..., 1].astype(np.int32)
        luma = gray.astype(np.int32)
        fill = sel & ((luma > 190) | ((s > 140) & (v > 110)))
        dark = sel & (luma < 45)
        if fill.any():
            fill_px.append(crop[fill][:4000])
        if dark.any():
            dark_px.append(crop[dark][:2000])
    out: Dict[str, Any] = {}
    if wpl:
        out["words_per_line"] = {"median": float(np.median(wpl)), "mean": _r(np.mean(wpl), 2), "max": int(max(wpl)), "samples": len(wpl)}
    colors: List[Dict[str, Any]] = []
    if fill_px:
        px = np.concatenate(fill_px).astype(np.float32)
        if len(px) > 20000:
            px = px[np.random.RandomState(0).choice(len(px), 20000, replace=False)]
        k = int(min(3, max(1, len(px) // 50)))
        crit = (cv2.TERM_CRITERIA_EPS + cv2.TERM_CRITERIA_MAX_ITER, 30, 1.0)
        _, labels, centers = cv2.kmeans(px, k, None, crit, 3, cv2.KMEANS_PP_CENTERS)
        counts = np.bincount(labels.ravel(), minlength=k)
        for ci in np.argsort(-counts):
            bgr = centers[ci]
            colors.append({"hex": "#%02x%02x%02x" % (int(bgr[2]), int(bgr[1]), int(bgr[0])), "share": _r(counts[ci] / counts.sum(), 2)})
    out["fill_colors"] = colors
    if dark_px:
        d = np.concatenate(dark_px)
        bgr = np.median(d, axis=0)
        out["outline_or_box_color"] = "#%02x%02x%02x" % (int(bgr[2]), int(bgr[1]), int(bgr[0]))
        total_fill = sum(len(x) for x in fill_px) or 1
        out["dark_to_fill_pixel_ratio"] = _r(len(d) / total_fill, 2)
    return out


# ------------------------------------------------------------------------------------------------
# cuts, zoom
# ------------------------------------------------------------------------------------------------


def _norm(g):
    import numpy as np

    g = g.astype(np.float32)
    return (g - g.mean()) / (g.std() + 4.0)


def detect_cut_candidates(diffs: List[float], times: List[float], fps: float) -> List[int]:
    """Indices j (difference between frame j and j+1) that are cut candidates."""
    import numpy as np

    n = len(diffs)
    if n == 0:
        return []
    d = np.asarray(diffs)
    half = max(3, int(round(fps * 1.0)))
    cands = []
    for j in range(n):
        if d[j] < CUT_FLOOR:
            continue
        lo, hi = max(0, j - half), min(n, j + half + 1)
        window = np.concatenate([d[lo : max(lo, j - 2)], d[min(hi, j + 3) : hi]])
        med = float(np.median(window)) if len(window) else 0.0
        if d[j] < 3.0 * max(med, 0.04):
            continue
        near = d[max(0, j - 2) : min(n, j + 3)]
        if d[j] < near.max():
            continue
        cands.append(j)
    # one per 0.2 s: the strongest
    merged: List[int] = []
    for j in cands:
        if merged and times[j + 1] - times[merged[-1] + 1] <= 0.2:
            if d[j] > d[merged[-1]]:
                merged[-1] = j
        else:
            merged.append(j)
    return merged


def _match_scale(a, b, mask_a=None, mask_b=None):
    """Similarity-transform scale between two ORB feature sets: (scale, inliers, ratio) or None."""
    import cv2
    import numpy as np

    pa, da = a
    pb, db = b
    if da is None or db is None or len(pa) < 12 or len(pb) < 12:
        return None
    if mask_a is not None:
        keep = mask_a
        pa, da = pa[keep], da[keep]
    if mask_b is not None:
        keep = mask_b
        pb, db = pb[keep], db[keep]
    if len(pa) < 12 or len(pb) < 12:
        return None
    bf = cv2.BFMatcher(cv2.NORM_HAMMING)
    try:
        knn = bf.knnMatch(da, db, k=2)
    except cv2.error:
        return None
    good = [m[0] for m in knn if len(m) == 2 and m[0].distance < 0.75 * m[1].distance]
    if len(good) < 12:
        return None
    src = pa[[m.queryIdx for m in good]].astype(np.float32)
    dst = pb[[m.trainIdx for m in good]].astype(np.float32)
    M, inl = cv2.estimateAffinePartial2D(src, dst, method=cv2.RANSAC, ransacReprojThreshold=3.0)
    if M is None or inl is None:
        return None
    ni = int(inl.sum())
    if ni < 10 or ni / len(good) < 0.4:
        return None
    s = float((M[0, 0] ** 2 + M[1, 0] ** 2) ** 0.5)
    if not (0.3 < s < 3.0):
        return None
    return s, ni, ni / len(good)


def _scale_dual(fa, fb, keep_a, keep_b):
    """Zoom scale that BOTH side strips of the frame agree on (a moving subject only moves the middle).

    Returns ``(scale, inliers, ratio)`` or ``None`` when either side is too flat to measure or the
    two sides disagree by more than 3 percent.
    """
    import numpy as np

    pa, pb = fa[0], fb[0]
    res = []
    for lo, hi in ((0.0, 0.36), (0.64, 1.0)):
        ma = keep_a & (pa[:, 0] >= lo) & (pa[:, 0] <= hi)
        mb = keep_b & (pb[:, 0] >= lo) & (pb[:, 0] <= hi)
        r = _match_scale(fa, fb, ma, mb)
        if r is None:
            return None
        res.append(r)
    la, lb = np.log(res[0][0]), np.log(res[1][0])
    if abs(la - lb) > 0.03:
        return None
    return float(np.exp((la + lb) / 2)), res[0][1] + res[1][1], min(res[0][2], res[1][2])


# ------------------------------------------------------------------------------------------------
# audio
# ------------------------------------------------------------------------------------------------


def music_heuristic(wav_path: str) -> Dict[str, Any]:
    """Is there music under the speech? Pause depth + rhythmic pulse on a 16 kHz mono wav."""
    import numpy as np

    try:
        with wave.open(wav_path, "rb") as wf:
            sr = wf.getframerate()
            n = wf.getnframes()
            ch = wf.getnchannels()
            raw = wf.readframes(n)
    except Exception as e:  # noqa: BLE001
        return {"available": False, "note": f"cannot read audio: {type(e).__name__}: {e}"}
    x = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0
    if ch > 1:
        x = x.reshape(-1, ch).mean(axis=1)
    if len(x) < sr * 3:
        return {"available": False, "note": "audio shorter than 3 s"}
    win, hop = int(sr * 0.025), int(sr * 0.010)
    nfr = 1 + (len(x) - win) // hop
    idx = np.arange(win)[None, :] + hop * np.arange(nfr)[:, None]
    fr = x[idx] * np.hanning(win)[None, :]
    rms = np.sqrt((fr**2).mean(axis=1) + 1e-12)
    db = 20 * np.log10(rms)
    live = db[db > -75]
    if len(live) < 100:
        return {"available": True, "music_present": False, "likelihood": 0.0, "note": "audio is (nearly) silent"}
    p10, p90 = np.percentile(live, 10), np.percentile(live, 90)
    gap = float(p90 - p10)
    # onset envelope in three bands -> autocorrelation at 0.35-1.0 s
    spec = np.abs(np.fft.rfft(fr, axis=1))
    freqs = np.fft.rfftfreq(win, 1.0 / sr)
    env = np.zeros(nfr - 1)
    for lo, hi in ((60, 400), (400, 2000), (2000, 6000)):
        band = np.log(spec[:, (freqs >= lo) & (freqs < hi)].sum(axis=1) + 1e-6)
        env += np.maximum(np.diff(band), 0)
    env = env - env.mean()
    beat = 0.0
    if env.std() > 1e-6 and len(env) > 300:
        ac = np.correlate(env, env, mode="full")[len(env) - 1 :]
        ac = ac / (ac[0] + 1e-9)
        lo_l, hi_l = int(0.35 / 0.010), int(1.0 / 0.010)
        beat = float(ac[lo_l:hi_l].max())
    s1 = float(np.clip((32.0 - gap) / 18.0, 0, 1))
    s2 = float(np.clip((beat - 0.15) / 0.35, 0, 1))
    likelihood = 0.65 * s1 + 0.35 * s2
    return {
        "available": True,
        "music_present": bool(likelihood >= 0.5),
        "likelihood": _r(likelihood, 2),
        "pause_depth_db": _r(gap, 1),
        "pulse_strength": _r(beat, 2),
        "method": "heuristic: shallow pauses (p90-p10 frame level, dB; speech alone has deep pauses) and a periodic onset pulse (0.35-1.0 s); run `analyze events` on a source for a model-based check",
    }


# ------------------------------------------------------------------------------------------------
# layout / faces
# ------------------------------------------------------------------------------------------------


def horizontal_structure(gray) -> Dict[str, Any]:
    """Full-width horizontal edges: a split screen has one near the middle, inset/blur-fit bars two."""
    import cv2
    import numpy as np

    h, w = gray.shape
    gy = np.abs(cv2.Sobel(gray, cv2.CV_16S, 0, 1, ksize=3))
    cov = (gy > 120).mean(axis=1)
    k = 3
    cov = np.convolve(cov, np.ones(k) / k, mode="same")
    lines: List[float] = []
    y = int(h * 0.05)
    while y < h * 0.95:
        if cov[y] > 0.8:
            y0 = y
            while y < h * 0.95 and cov[y] > 0.8:
                y += 1
            lines.append(((y0 + y) / 2.0) / h)
        y += 1
    mids = [v for v in lines if 0.25 <= v <= 0.75]
    if len(mids) == 1 and len(lines) == 1:
        return {"structure": "split_screen", "split_y": _r(mids[0], 2)}
    tops = [v for v in lines if v < 0.4]
    bots = [v for v in lines if v > 0.6]
    if tops and bots:
        return {"structure": "inset_bars", "top_edge": _r(tops[0], 2), "bottom_edge": _r(bots[-1], 2)}
    return {"structure": "full_frame"}


def _make_detector(width: int, height: int):
    import cv2

    from . import faces

    model = faces.ensure_model(None, None)
    scale = min(1.0, faces.MAX_DETECT_WIDTH / width)
    dw, dh = max(1, round(width * scale)), max(1, round(height * scale))
    det = cv2.FaceDetectorYN.create(str(model), "", (dw, dh), faces.SCORE_THRESHOLD, faces.NMS_THRESHOLD, faces.TOP_K)
    return det, (dw, dh)


def _faces_in(det, size, img) -> List[Tuple[float, float, float, float]]:
    import cv2

    dw, dh = size
    h, w = img.shape[:2]
    if (w, h) != (dw, dh):
        img = cv2.resize(img, (dw, dh), interpolation=cv2.INTER_AREA)
    _, rows = det.detect(img)
    out = []
    if rows is not None:
        for r in rows:
            out.append((float(r[0]) / dw, float(r[1]) / dh, float(r[2]) / dw, float(r[3]) / dh))
    return out


def classify_shot(face_counts: List[int], face_heights: List[float], face_cys: List[float]) -> str:
    if not face_counts:
        return "unknown"
    n = int(round(sorted(face_counts)[len(face_counts) // 2]))
    if max(face_counts) >= 2 and n >= 2:
        return "multi_face"
    if n == 0:
        return "no_face"
    fh = max(face_heights) if face_heights else 0.0
    if fh >= 0.22:
        return "face_closeup"
    if fh >= 0.09:
        return "face_medium"
    return "face_small"


# ------------------------------------------------------------------------------------------------
# main entry
# ------------------------------------------------------------------------------------------------


def measure(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    import cv2
    import numpy as np

    video = params.get("video_path")
    if not isinstance(video, str) or not video:
        raise RpcError("validation", "video_path is required")
    if not os.path.isfile(video):
        raise RpcError("validation", f"video not found: {video}")
    max_s = params.get("max_seconds", MAX_SECONDS_DEFAULT)
    if isinstance(max_s, bool) or not isinstance(max_s, (int, float)) or max_s <= 0:
        raise RpcError("validation", "max_seconds must be a positive number")
    want_faces = bool(params.get("faces", True))
    audio = params.get("audio_path")

    cap = cv2.VideoCapture(video)
    if not cap.isOpened():
        raise RpcError("media", f"cannot open video: {video}")
    fps = cap.get(cv2.CAP_PROP_FPS) or 0.0
    if fps <= 1:
        fps = 30.0
    width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    if width <= 0 or height <= 0:
        raise RpcError("media", f"video has no frame size: {video}")
    step = max(1, int(round(fps / 15.0)))
    eff_fps = fps / step
    text_every = max(1, int(round(TEXT_STEP_S * eff_fps)))
    orb = cv2.ORB_create(nfeatures=400, fastThreshold=10)
    sw = min(160, width)
    sh = max(8, int(round(height * sw / width)))
    ow = min(320, width)
    oh = max(8, int(round(height * ow / width)))

    times: List[float] = []
    smalls: List[Any] = []
    hists: List[Any] = []
    feats: List[Any] = []
    samples: List[Dict[str, Any]] = []
    text_frames: List[Tuple[float, Any]] = []
    n_read = 0
    kept = 0
    limit_t = float(max_s)
    truncated = False
    progress("reference.measure", 0.0, "reading frames")
    while True:
        ok, frame = cap.read()
        if not ok or frame is None:
            break
        t = n_read / fps
        n_read += 1
        if (n_read - 1) % step != 0:
            continue
        if t > limit_t:
            truncated = True
            break
        gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
        small = cv2.resize(gray, (sw, sh), interpolation=cv2.INTER_AREA)
        smalls.append(_norm(small))
        hsv = cv2.cvtColor(cv2.resize(frame, (sw, sh), interpolation=cv2.INTER_AREA), cv2.COLOR_BGR2HSV)
        hh = cv2.calcHist([hsv], [0, 1, 2], None, [8, 4, 4], [0, 180, 0, 256, 0, 256])
        hists.append(cv2.normalize(hh, hh, 1.0, 0.0, cv2.NORM_L1).flatten())
        g2 = cv2.resize(gray, (ow, oh), interpolation=cv2.INTER_AREA) if (ow, oh) != (width, height) else gray
        kp, des = orb.detectAndCompute(g2, None)
        pts = np.array([k.pt for k in kp], dtype=np.float32).reshape(-1, 2)
        pts[:, 0] /= ow
        pts[:, 1] /= oh
        feats.append((pts, des))
        times.append(t)
        if kept % text_every == 0:
            samples.append({"t": t, "bands": find_bands(gray), "kept": kept})
            ok2, buf = cv2.imencode(".jpg", frame, [cv2.IMWRITE_JPEG_QUALITY, 92])
            text_frames.append((t, buf))
        kept += 1
        if kept % 60 == 0:
            progress("reference.measure", min(0.9, t / max(1.0, limit_t)), None)
    cap.release()
    nframes = len(times)
    if nframes < 3:
        raise RpcError("media", f"video too short to measure ({nframes} frames): {video}")
    duration = times[-1] + 1.0 / eff_fps

    # ---- cuts
    diffs: List[float] = []
    for i in range(nframes - 1):
        ds = float(np.abs(smalls[i] - smalls[i + 1]).mean())
        dh_ = float(cv2.compareHist(hists[i].astype(np.float32), hists[i + 1].astype(np.float32), cv2.HISTCMP_BHATTACHARYYA))
        # flat or very dark footage has almost no structure to compare, so a big colour-histogram jump counts too
        diffs.append(max(ds, dh_ if dh_ > 0.45 else 0.0))
    cand = detect_cut_candidates(diffs, times, eff_fps)

    # ---- text bands (before zoom, whose feature matching masks text out)
    cut_guess = [times[j + 1] for j in cand]
    motion = []
    for a, b in zip(samples, samples[1:]):
        motion.append(float(np.abs(smalls[a["kept"]] - smalls[b["kept"]]).mean()))
    motion.append(0.0)
    clusters = cluster_bands(samples, height, width, cut_guess, motion)
    caption = pick_caption(clusters, duration)
    hook = pick_hook(clusters, samples, caption)
    text_rows: List[Tuple[float, float]] = []
    for c in clusters:
        if c["presence"] >= 0.1:
            text_rows.append((c["y0"] - 0.01, c["y1"] + 0.01))

    def feature_mask(i: int):
        pts = feats[i][0]
        keep = np.ones(len(pts), dtype=bool)
        for y0, y1 in text_rows:
            keep &= ~((pts[:, 1] >= y0) & (pts[:, 1] <= y1))
        return keep

    # ---- classify candidates: cut vs scale jump
    cuts: List[Dict[str, Any]] = []
    scale_jumps: List[Dict[str, Any]] = []
    for j in cand:
        res = _scale_dual(feats[j], feats[j + 1], feature_mask(j), feature_mask(j + 1))
        t_cut = times[j + 1]
        if t_cut < 0.3 or t_cut > duration - 0.3:
            continue  # a fade in/out at the very ends is not a cut
        if res is not None and abs(np.log(res[0])) >= 0.04:
            scale_jumps.append({"t": _r(t_cut, 2), "scale": _r(res[0], 3), "kind": "punch_in" if res[0] > 1 else "punch_out"})
        else:
            cuts.append({"t": _r(t_cut, 2), "strength": _r(diffs[j], 2)})
    cut_ts = [c["t"] for c in cuts]

    # ---- shots
    bounds = [0.0] + cut_ts + [duration]
    shots = [{"start": _r(a, 2), "end": _r(b, 2)} for a, b in zip(bounds, bounds[1:]) if b - a > 0.05]
    lens = [s["end"] - s["start"] for s in shots]

    # ---- zoom (smooth) from pairs ~0.4 s apart
    k = max(2, int(round(ZOOM_GAP_S * eff_fps)))
    pair_logs: List[Tuple[float, float, float]] = []  # (t_start, t_end, log scale)
    tried = 0
    for i in range(0, nframes - k, 2):
        ta, tb = times[i], times[i + k]
        if any(ta < c <= tb for c in cut_ts):
            continue
        if any(ta < sj["t"] <= tb for sj in scale_jumps):
            continue
        tried += 1
        res = _scale_dual(feats[i], feats[i + k], feature_mask(i), feature_mask(i + k))
        if res is not None:
            pair_logs.append((ta, tb, float(np.log(res[0]))))
    events: List[Dict[str, Any]] = []
    cur: Optional[Dict[str, Any]] = None
    for ta, tb, lg in pair_logs:
        if abs(lg) >= ZOOM_THRESH:
            sign = 1 if lg > 0 else -1
            if cur and cur["sign"] == sign and ta - cur["end"] <= 0.45:
                cur["end"] = tb
                cur["logs"].append(lg)
            else:
                if cur:
                    events.append(cur)
                cur = {"sign": sign, "start": ta, "end": tb, "logs": [lg]}
        elif cur and ta - cur["end"] > 0.45:
            events.append(cur)
            cur = None
    if cur:
        events.append(cur)
    zoom_events = []
    for e in events:
        span = e["end"] - e["start"]
        net = float(np.exp(np.mean(e["logs"]) * max(1.0, span / ZOOM_GAP_S)))
        zoom_events.append({"start": _r(e["start"], 2), "end": _r(e["end"], 2), "factor": _r(net, 3), "kind": "zoom_in" if net > 1 else "zoom_out"})
    for sj in scale_jumps:
        zoom_events.append({"start": sj["t"], "end": sj["t"], "factor": sj["scale"], "kind": sj["kind"]})
    zoom_events.sort(key=lambda e: e["start"])
    minutes = duration / 60.0
    zoom = {
        "events": zoom_events,
        "per_minute": _r(len(zoom_events) / minutes, 2) if minutes > 0 else 0.0,
        "coverage": _r(len(pair_logs) / tried, 2) if tried else 0.0,
        "method": f"ORB features + similarity transform between frames {ZOOM_GAP_S} s apart (text areas masked); smooth zoom = |scale change| >= {ZOOM_THRESH * 100:.1f}% per {ZOOM_GAP_S} s, punch = a scale jump at a visual cut. `coverage` is the fraction of frame pairs with enough texture to measure; low coverage (dark/flat footage) means zoom may be under-counted",
    }

    # ---- caption details
    text: Dict[str, Any] = {"samples": len(samples), "static_overlays": [], "method": "Sobel-x edge density rows vs rolling-median baseline, clustered by position"}
    if caption:
        decoded: List[Tuple[float, Any]] = [(t, cv2.imdecode(b, cv2.IMREAD_COLOR)) for t, b in text_frames]
        present = caption["present_samples"]
        pick = present[:: max(1, len(present) // 24)][:24]
        det = caption_details(decoded, caption, caption["by_sample"], pick)
        text["caption"] = {
            "present": True,
            "y0": caption["y0"],
            "y1": caption["y1"],
            "y_center": _r((caption["y0"] + caption["y1"]) / 2, 3),
            "height": _r(caption["y1"] - caption["y0"], 3),
            "x0": caption["x0"],
            "x1": caption["x1"],
            "presence": caption["presence"],
            "change_rate": caption["change"],
            **det,
        }
    else:
        text["caption"] = {"present": False}
    text["hook"] = hook or {"present": False}
    for c in clusters:
        if c is caption:
            continue
        if hook and c["y0"] >= hook["y0"] - 0.001 and c["y1"] <= hook["y1"] + 0.001:
            continue
        if c["presence"] >= 0.3:
            text["static_overlays"].append(
                {"y0": c["y0"], "y1": c["y1"], "x0": c["x0"], "x1": c["x1"], "presence": c["presence"], "change_rate": c["change"]}
            )
    if params.get("debug"):
        text["clusters_debug"] = [{k: v for k, v in c.items() if k not in ("by_sample", "present_samples")} for c in clusters]
    # text visible in the first 3 s at all
    first = [s for s in samples if s["t"] < 3.0]
    text["any_text_first_3s"] = bool(first) and sum(1 for s in first if s["bands"]) / len(first) >= 0.5

    # ---- faces and per-shot layout
    faces_block: Dict[str, Any] = {"available": False}
    decoded_all: Optional[List[Tuple[float, Any]]] = None
    shot_layouts: List[Dict[str, Any]] = []
    det = None
    if want_faces:
        try:
            det, dsize = _make_detector(width, height)
            faces_block = {"available": True, "model": "YuNet"}
        except Exception as e:  # noqa: BLE001
            faces_block = {"available": False, "note": f"face detector unavailable ({type(e).__name__}: {e}); layout class limited to frame structure"}
    else:
        faces_block = {"available": False, "note": "faces disabled for this call"}
    decoded_all = [(t, cv2.imdecode(b, cv2.IMREAD_COLOR)) for t, b in text_frames]
    shot_classes: Dict[str, int] = {}
    for sh in shots:
        a, b = sh["start"], sh["end"]
        inside = [i for i, (t, _) in enumerate(decoded_all) if a + 0.1 <= t <= b - 0.1] or [
            min(range(len(decoded_all)), key=lambda i: abs(decoded_all[i][0] - (a + b) / 2))
        ]
        pick_i = [inside[len(inside) // 4], inside[len(inside) // 2], inside[(3 * len(inside)) // 4]] if len(inside) >= 3 else inside
        pick_i = sorted(set(pick_i))
        counts: List[int] = []
        heights: List[float] = []
        cys: List[float] = []
        structs: List[str] = []
        for i in pick_i:
            img = decoded_all[i][1]
            structs.append(horizontal_structure(cv2.cvtColor(img, cv2.COLOR_BGR2GRAY))["structure"])
            if det is not None:
                fs = _faces_in(det, dsize, img)
                counts.append(len(fs))
                if fs:
                    big = max(fs, key=lambda f: f[3])
                    heights.append(big[3])
                    cys.append(big[1] + big[3] / 2)
        struct = max(set(structs), key=structs.count) if structs else "full_frame"
        if det is not None:
            cls = classify_shot(counts, heights, cys)
        else:
            cls = "unknown"
        entry = {"start": a, "end": b, "layout": cls, "structure": struct}
        if det is not None:
            entry["faces"] = int(sorted(counts)[len(counts) // 2]) if counts else 0
            if cys:
                entry["face_center_y"] = _r(float(np.median(cys)), 2)
                entry["face_height"] = _r(float(np.median(heights)), 2)
        shot_layouts.append(entry)
        key = cls if struct == "full_frame" else f"{cls}+{struct}"
        shot_classes[key] = shot_classes.get(key, 0) + 1

    out: Dict[str, Any] = {
        "duration": _r(duration, 2),
        "analysed_seconds": _r(duration, 2),
        "truncated": truncated,
        "size": {"w": width, "h": height},
        "analysis_fps": _r(eff_fps, 1),
        "cuts": {
            "count": len(cuts),
            "per_minute": _r(len(cuts) / minutes, 2) if minutes > 0 else 0.0,
            "times": cut_ts,
            "shot_length": _stats(lens),
            "method": "luma-normalised frame differences, adaptive threshold (>= 3x local median, floor 0.22), local maxima; scale jumps (punch-in cuts) are counted under zoom, not here",
        },
        "shots": shot_layouts,
        "layout_distribution": shot_classes,
        "zoom": zoom,
        "text": text,
        "faces": faces_block,
    }
    if audio:
        out["music"] = music_heuristic(str(audio))
    progress("reference.measure", 1.0, None)
    return out
