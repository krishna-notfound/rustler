"""Speaker diarization (``speakers.diarize``) and active-speaker evidence (``speakers.active``).

``speakers.diarize`` answers "who speaks when" for one audio file. Preferred path: sherpa-onnx
(Apache-2.0) offline speaker diarization with two local ONNX models, both license-checked:

* segmentation: ``pyannote/segmentation-3.0`` (MIT) as converted by the sherpa-onnx project,
* embedding: ``wespeaker_en_voxceleb_resnet34`` (WeSpeaker, Apache-2.0), also from the sherpa-onnx
  release assets.

Both are downloaded on first use into ``$RUSTLER_HOME/models/diarization/`` and verified by sha256
(see ``engine/MODELS.md``). When sherpa-onnx or the models are unavailable (``model: "auto"`` and
offline, say) the engine falls back to a much weaker numpy-only diarizer (MFCC statistics +
agglomerative clustering) and reports ``"model": "heuristic"`` plus a warning.

``speakers.active`` samples a video and returns, per sample, the YuNet face boxes with a
mouth-motion score (frame difference inside the lower part of each face box, normalized per
tracked face). The Rust side correlates that with the diarized turns.
"""

from __future__ import annotations

import hashlib
import logging
import os
import shutil
import tarfile
import tempfile
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

from .rpc import RpcError

log = logging.getLogger("rustler_engine.speakers")

SAMPLE_RATE = 16000
MODEL_NAME = "sherpa-onnx:pyannote-segmentation-3.0+wespeaker-resnet34"

SEG_TARBALL_URL = (
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/"
    "sherpa-onnx-pyannote-segmentation-3-0.tar.bz2"
)
SEG_TARBALL_SHA256 = "24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488"
SEG_MEMBER = "sherpa-onnx-pyannote-segmentation-3-0/model.onnx"
SEG_MODEL_SHA256 = "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079"
SEG_LICENSE_MEMBER = "sherpa-onnx-pyannote-segmentation-3-0/LICENSE"
EMB_FILENAME = "wespeaker_en_voxceleb_resnet34.onnx"
EMB_URL = (
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/" + EMB_FILENAME
)
EMB_SHA256 = "5ef208a9da1453335308a6b6f4e6dfbd7e183a38b604de0a57664f45d257fe94"

#: Agglomerative clustering threshold of sherpa's FastClustering (cosine distance on embeddings).
CLUSTER_THRESHOLD = 0.5
MIN_DURATION_ON = 0.3
MIN_DURATION_OFF = 0.5


def rustler_home() -> Path:
    env = os.environ.get("RUSTLER_HOME")
    return Path(env) if env else Path.home() / ".rustler"


def model_dir() -> Path:
    return rustler_home() / "models" / "diarization"


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def _download(url: str, dest_dir: Path, timeout: int = 180) -> Path:
    fd, tmp = tempfile.mkstemp(dir=str(dest_dir), suffix=".part")
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "rustler-engine/0.1"})
        with os.fdopen(fd, "wb") as out, urllib.request.urlopen(req, timeout=timeout) as resp:
            shutil.copyfileobj(resp, out)
    except Exception:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise
    return Path(tmp)


def ensure_models(progress: Optional[Callable[..., None]] = None) -> Tuple[Path, Path]:
    """Return ``(segmentation.onnx, embedding.onnx)``, downloading and verifying them if missing."""
    d = model_dir()
    d.mkdir(parents=True, exist_ok=True)
    seg = d / "pyannote-segmentation-3-0.onnx"
    emb = d / EMB_FILENAME
    try:
        if not (seg.is_file() and _sha256(seg) == SEG_MODEL_SHA256):
            if progress:
                progress("models.fetch", 0.0, "downloading pyannote segmentation 3.0 (sherpa-onnx, MIT)")
            log.info("downloading %s", SEG_TARBALL_URL)
            tmp = _download(SEG_TARBALL_URL, d)
            try:
                got = _sha256(tmp)
                if got != SEG_TARBALL_SHA256:
                    raise RpcError("media", f"segmentation archive has sha256 {got}, expected {SEG_TARBALL_SHA256}")
                with tarfile.open(tmp, "r:bz2") as tf:
                    for member, target in ((SEG_MEMBER, seg), (SEG_LICENSE_MEMBER, d / "pyannote-segmentation-3-0.LICENSE")):
                        f = tf.extractfile(member)  # read by exact member name: no path traversal
                        if f is None:
                            raise RpcError("media", f"segmentation archive lacks {member}")
                        part = target.with_suffix(target.suffix + ".part")
                        with open(part, "wb") as out:
                            shutil.copyfileobj(f, out)
                        os.replace(part, target)
            finally:
                if tmp.exists():
                    tmp.unlink()
            if _sha256(seg) != SEG_MODEL_SHA256:
                seg.unlink()
                raise RpcError("media", "segmentation model failed its sha256 check after extraction")
        if not (emb.is_file() and _sha256(emb) == EMB_SHA256):
            if progress:
                progress("models.fetch", 50.0, f"downloading {EMB_FILENAME} (WeSpeaker, Apache-2.0)")
            log.info("downloading %s", EMB_URL)
            tmp = _download(EMB_URL, d)
            try:
                got = _sha256(tmp)
                if got != EMB_SHA256:
                    raise RpcError("media", f"embedding model has sha256 {got}, expected {EMB_SHA256}")
                os.replace(tmp, emb)
            finally:
                if tmp.exists():
                    tmp.unlink()
    except RpcError:
        raise
    except Exception as e:
        raise RpcError("media", f"could not download the diarization models: {type(e).__name__}: {e}") from e
    if progress:
        progress("models.fetch", 100.0, "diarization models ready")
    return seg, emb


def fetch_models(progress: Callable[..., None]) -> str:
    return str(ensure_models(progress)[0].parent)


# --- helpers shared by both diarizers ---------------------------------------------------------


def relabel(raw: List[Tuple[float, float, int]]) -> List[Dict[str, Any]]:
    """Sort by start time, map raw speaker indices to ``S1, S2, ...`` in order of first appearance."""
    out: List[Dict[str, Any]] = []
    names: Dict[int, str] = {}
    for start, end, spk in sorted(raw, key=lambda r: (r[0], r[1])):
        if end <= start:
            continue
        if spk not in names:
            names[spk] = f"S{len(names) + 1}"
        out.append({"start": round(float(start), 3), "end": round(float(end), 3), "speaker": names[spk]})
    return out


def _int_param(params: Dict[str, Any], key: str, lo: int = 1, hi: int = 64) -> Optional[int]:
    v = params.get(key)
    if v is None:
        return None
    if isinstance(v, bool) or not isinstance(v, int) or not lo <= v <= hi:
        raise RpcError("validation", f"{key} must be an integer in [{lo}, {hi}] or null")
    return v


def count_speakers(segments: List[Dict[str, Any]]) -> int:
    return len({s["speaker"] for s in segments})


# --- sherpa-onnx diarizer ----------------------------------------------------------------------


def _sherpa_pass(x, seg: Path, emb: Path, num_clusters: int, progress: Callable[..., None]) -> List[Tuple[float, float, int]]:
    import sherpa_onnx as so

    threads = max(1, min(4, os.cpu_count() or 2))
    config = so.OfflineSpeakerDiarizationConfig(
        segmentation=so.OfflineSpeakerSegmentationModelConfig(
            pyannote=so.OfflineSpeakerSegmentationPyannoteModelConfig(model=str(seg)),
            num_threads=threads,
            provider="cpu",
        ),
        embedding=so.SpeakerEmbeddingExtractorConfig(model=str(emb), num_threads=threads, provider="cpu"),
        clustering=so.FastClusteringConfig(num_clusters=num_clusters, threshold=CLUSTER_THRESHOLD),
        min_duration_on=MIN_DURATION_ON,
        min_duration_off=MIN_DURATION_OFF,
    )
    if not config.validate():
        raise RpcError("internal", "sherpa-onnx rejected the diarization configuration")
    sd = so.OfflineSpeakerDiarization(config)
    if sd.sample_rate != SAMPLE_RATE:
        raise RpcError("internal", f"diarization model expects {sd.sample_rate} Hz, got {SAMPLE_RATE}")

    def cb(done: int, total: int) -> int:
        if total > 0:
            progress("speakers.diarize", round(done / total * 100.0, 1))
        return 0

    result = sd.process(x, callback=cb).sort_by_start_time()
    return [(float(s.start), float(s.end), int(s.speaker)) for s in result]


def diarize_sherpa(
    x, num_speakers: Optional[int], min_speakers: Optional[int], max_speakers: Optional[int], progress: Callable[..., None]
) -> List[Dict[str, Any]]:
    seg, emb = ensure_models(progress)
    raw = _sherpa_pass(x, seg, emb, num_speakers if num_speakers else -1, progress)
    segs = relabel(raw)
    if num_speakers:
        return segs
    n = count_speakers(segs)
    target = n
    if max_speakers is not None and n > max_speakers:
        target = max_speakers
    if min_speakers is not None and n < min_speakers:
        target = min_speakers
    if target != n and len(x) > 0:
        # Threshold clustering is outside the requested range: redo the clustering with a fixed count.
        segs = relabel(_sherpa_pass(x, seg, emb, target, progress))
    return segs


# --- numpy heuristic diarizer (fallback) ---------------------------------------------------------


def _mel_filterbank(n_mels: int, n_fft: int, sr: int, fmin: float = 100.0, fmax: float = 7000.0):
    import numpy as np

    def hz2mel(f):
        return 2595.0 * np.log10(1.0 + f / 700.0)

    def mel2hz(m):
        return 700.0 * (10.0 ** (m / 2595.0) - 1.0)

    mels = np.linspace(hz2mel(fmin), hz2mel(fmax), n_mels + 2)
    hz = mel2hz(mels)
    bins = np.floor((n_fft + 1) * hz / sr).astype(int)
    fb = np.zeros((n_mels, n_fft // 2 + 1), dtype=np.float32)
    for i in range(n_mels):
        l, c, r = bins[i], bins[i + 1], bins[i + 2]
        for k in range(l, c):
            if c > l:
                fb[i, k] = (k - l) / (c - l)
        for k in range(c, r):
            if r > c:
                fb[i, k] = (r - k) / (r - c)
    return fb


def mfcc_frames(x, sr: int = SAMPLE_RATE, n_mfcc: int = 13):
    """``(mfcc[frames, n_mfcc], energy_db[frames])`` with 25 ms frames every 10 ms."""
    import numpy as np

    flen, hop, n_fft = int(0.025 * sr), int(0.010 * sr), 512
    if len(x) < flen:
        return np.zeros((0, n_mfcc), dtype=np.float32), np.zeros(0, dtype=np.float32)
    n = 1 + (len(x) - flen) // hop
    idx = np.arange(flen)[None, :] + hop * np.arange(n)[:, None]
    frames = x[idx] * np.hamming(flen).astype(np.float32)
    energy = 10.0 * np.log10(np.maximum(1e-10, (frames**2).mean(axis=1)))
    spec = np.abs(np.fft.rfft(frames, n=n_fft, axis=1)) ** 2
    mel = np.log(np.maximum(1e-10, spec @ _mel_filterbank(24, n_fft, sr).T))
    k = np.arange(24)
    dct = np.cos(np.pi / 24 * (k[None, :] + 0.5) * np.arange(n_mfcc)[:, None]).astype(np.float32)
    return (mel @ dct.T).astype(np.float32), energy.astype(np.float32)


def _voiced_regions(energy, hop: float = 0.01) -> List[Tuple[int, int]]:
    """Frame index ranges of speech-like energy (relative threshold, small gaps closed)."""
    import numpy as np

    if len(energy) == 0:
        return []
    thr = max(float(np.percentile(energy, 95)) - 30.0, -55.0)
    v = energy > thr
    k = 5
    v = np.convolve(v.astype(np.float32), np.ones(k) / k, mode="same") > 0.5
    regions: List[List[int]] = []
    i = 0
    while i < len(v):
        if v[i]:
            j = i
            while j < len(v) and v[j]:
                j += 1
            regions.append([i, j])
            i = j
        else:
            i += 1
    max_gap = int(0.4 / hop)
    merged: List[List[int]] = []
    for r in regions:
        if merged and r[0] - merged[-1][1] <= max_gap:
            merged[-1][1] = r[1]
        else:
            merged.append(r)
    return [(a, b) for a, b in merged if (b - a) * hop >= 0.3]


def _agglomerate(feats, k: Optional[int], max_k: int):
    """Average-linkage agglomerative clustering on cosine distance; returns ``(labels, k)``."""
    import numpy as np

    n = len(feats)
    if n == 1:
        return np.zeros(1, dtype=int), 1
    f = feats / np.maximum(1e-9, np.linalg.norm(feats, axis=1, keepdims=True))
    dist = 1.0 - f @ f.T
    np.fill_diagonal(dist, np.inf)
    members = [[i] for i in range(n)]
    active = np.ones(n, dtype=bool)
    sizes = np.ones(n)
    heights: List[float] = []
    history: List[Tuple[int, int]] = []
    for _ in range(n - 1):
        i, j = np.unravel_index(np.argmin(dist), dist.shape)
        heights.append(float(dist[i, j]))
        history.append((int(i), int(j)))
        # average linkage update into row/col i, retire j
        new = (dist[i] * sizes[i] + dist[j] * sizes[j]) / (sizes[i] + sizes[j])
        dist[i, :] = new
        dist[:, i] = new
        dist[i, i] = np.inf
        dist[j, :] = np.inf
        dist[:, j] = np.inf
        sizes[i] += sizes[j]
        members[i].extend(members[j])
        members[j] = []
        active[j] = False
    if k is None:
        # Largest jump between consecutive merge heights picks the cut; a flat dendrogram = one speaker.
        k = 1
        best = 0.0
        for kk in range(2, min(max_k, n) + 1):
            gap = heights[n - kk] - heights[n - kk - 1] if n - kk - 1 >= 0 else 0.0
            if gap > best:
                best, k = gap, kk
        if heights[-1] < 0.35 or best < 0.08:
            k = 1
    k = max(1, min(k, n))
    # Replay merges until k clusters remain.
    groups = {i: [i] for i in range(n)}
    for (i, j) in history[: n - k]:
        groups[i].extend(groups.pop(j))
    labels = np.zeros(n, dtype=int)
    for c, (_, idxs) in enumerate(sorted(groups.items(), key=lambda kv: min(kv[1]))):
        labels[idxs] = c
    return labels, k


def diarize_heuristic(
    x, num_speakers: Optional[int], min_speakers: Optional[int], max_speakers: Optional[int], progress: Callable[..., None]
) -> List[Dict[str, Any]]:
    import numpy as np

    mfcc, energy = mfcc_frames(x)
    hop = 0.01
    regions = _voiced_regions(energy, hop)
    if not regions:
        return []
    voiced_total = sum((b - a) * hop for a, b in regions)
    win = 1.0
    step = max(0.5, voiced_total / 600.0)
    if step > 0.5:
        win = 2.0 * step
    wins: List[Tuple[int, int, int]] = []  # (frame_a, frame_b, region_index)
    for ri, (a, b) in enumerate(regions):
        wf, sf = int(win / hop), int(step / hop)
        if b - a <= wf + sf // 2:
            wins.append((a, b, ri))
            continue
        s = a
        while s + wf <= b:
            wins.append((s, s + wf, ri))
            s += sf
        if wins[-1][1] < b - sf // 2:
            wins.append((b - wf, b, ri))
    feats = np.stack([np.concatenate([mfcc[a:b, 1:].mean(axis=0), mfcc[a:b, 1:].std(axis=0)]) for a, b, _ in wins])
    feats = (feats - feats.mean(axis=0)) / np.maximum(1e-6, feats.std(axis=0))
    progress("speakers.diarize", 50.0)
    k_req = num_speakers
    lo = min_speakers or 1
    hi = max_speakers or 6
    labels, k = _agglomerate(feats, k_req, hi)
    if k_req is None and k < lo:
        labels, k = _agglomerate(feats, lo, hi)
    if k_req is None and k > hi:
        labels, k = _agglomerate(feats, hi, hi)
    raw: List[Tuple[float, float, int]] = []
    for (a, b, _), lab in zip(wins, labels):
        s, e = a * hop, b * hop
        # Cut overlapping windows in half at the boundary with the previous one.
        if raw and s < raw[-1][1]:
            mid = (s + raw[-1][1]) / 2.0
            if lab == raw[-1][2]:
                raw[-1] = (raw[-1][0], max(e, raw[-1][1]), lab)
                continue
            raw[-1] = (raw[-1][0], mid, raw[-1][2])
            s = mid
        elif raw and lab == raw[-1][2] and s - raw[-1][1] <= MIN_DURATION_OFF:
            raw[-1] = (raw[-1][0], e, lab)
            continue
        raw.append((s, e, int(lab)))
    progress("speakers.diarize", 100.0)
    return relabel(raw)


# --- speakers.diarize ---------------------------------------------------------------------------


def diarize(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    from .audio_events import load_wav

    audio_path = params.get("audio_path")
    if not isinstance(audio_path, str) or not audio_path:
        raise RpcError("validation", "audio_path is required")
    num_speakers = _int_param(params, "num_speakers")
    min_speakers = _int_param(params, "min_speakers")
    max_speakers = _int_param(params, "max_speakers")
    if min_speakers and max_speakers and min_speakers > max_speakers:
        raise RpcError("validation", "min_speakers must be <= max_speakers")
    model = params.get("model", "auto")
    if model not in ("auto", "sherpa", "heuristic"):
        raise RpcError("validation", 'model must be "auto", "sherpa" or "heuristic"')
    x = load_wav(audio_path)
    duration = len(x) / SAMPLE_RATE
    warnings: List[str] = []
    segments: List[Dict[str, Any]]
    used = MODEL_NAME
    if model == "heuristic":
        used = "heuristic"
        segments = diarize_heuristic(x, num_speakers, min_speakers, max_speakers, progress)
    else:
        try:
            if len(x) < SAMPLE_RATE // 2:
                segments = []
            else:
                segments = diarize_sherpa(x, num_speakers, min_speakers, max_speakers, progress)
        except (RpcError, ImportError) as e:
            if model == "sherpa":
                raise
            if isinstance(e, RpcError) and e.kind == "validation":
                raise
            msg = e.message if isinstance(e, RpcError) else f"sherpa-onnx is not installed: {e}"
            log.warning("falling back to the heuristic diarizer: %s", msg)
            used = "heuristic"
            warnings.append(f"fell back to the heuristic diarizer (much less accurate): {msg}")
            segments = diarize_heuristic(x, num_speakers, min_speakers, max_speakers, progress)
    if used == "heuristic" and "heuristic" not in " ".join(warnings):
        warnings.append("heuristic diarizer (MFCC statistics + clustering): speaker changes are approximate")
    out: Dict[str, Any] = {
        "segments": segments,
        "num_speakers": count_speakers(segments),
        "duration": round(duration, 3),
        "model": used,
    }
    if warnings:
        out["warnings"] = warnings
    return out


# --- speakers.active ---------------------------------------------------------------------------

MAX_ACTIVE_WIDTH = 640
MAX_ACTIVE_SAMPLES = 20000
#: Mouth motion below this spread (p95 - p10, in 0..1 pixel units) is treated as sensor/codec noise.
MIN_MOTION_SPREAD = 0.008
TRACK_MAX_DX = 0.22
TRACK_HOLD_SECONDS = 3.0


def mouth_region(box: Tuple[float, float, float, float], width: int, height: int) -> Tuple[int, int, int, int]:
    """Pixel rectangle ``(x0, y0, x1, y1)`` of the lower face (mouth + chin) of a normalized box."""
    x, y, w, h = box
    x0 = int(round((x + 0.15 * w) * width))
    x1 = int(round((x + 0.85 * w) * width))
    y0 = int(round((y + 0.55 * h) * height))
    y1 = int(round((y + 1.0 * h) * height))
    x0, x1 = max(0, min(x0, width - 1)), max(1, min(x1, width))
    y0, y1 = max(0, min(y0, height - 1)), max(1, min(y1, height))
    if x1 <= x0:
        x1 = min(width, x0 + 1)
    if y1 <= y0:
        y1 = min(height, y0 + 1)
    return x0, y0, x1, y1


def motion_between(a, b, rect: Tuple[int, int, int, int]) -> float:
    """Mean absolute grey difference (0..1) of one rectangle between two frames."""
    import cv2
    import numpy as np

    x0, y0, x1, y1 = rect
    pa = cv2.resize(a[y0:y1, x0:x1], (24, 12), interpolation=cv2.INTER_AREA).astype(np.float32)
    pb = cv2.resize(b[y0:y1, x0:x1], (24, 12), interpolation=cv2.INTER_AREA).astype(np.float32)
    return float(np.abs(pa - pb).mean() / 255.0)


def assign_tracks(samples: List[Dict[str, Any]]) -> None:
    """Give every face a ``id`` by greedy nearest-centre matching to tracks seen in the last 3 s;
    ids are then renumbered left to right (by mean centre x)."""
    tracks: List[Dict[str, Any]] = []
    for s in samples:
        pairs = []
        for fi, f in enumerate(s["faces"]):
            cx = f["x"] + f["w"] / 2.0
            for ti, tr in enumerate(tracks):
                if s["t"] - tr["last_t"] > TRACK_HOLD_SECONDS:
                    continue
                d = abs(cx - tr["cx"])
                if d <= TRACK_MAX_DX:
                    pairs.append((d, fi, ti))
        pairs.sort()
        used_f, used_t = set(), set()
        for d, fi, ti in pairs:
            if fi in used_f or ti in used_t:
                continue
            used_f.add(fi)
            used_t.add(ti)
            f = s["faces"][fi]
            f["_track"] = ti
            tracks[ti]["cx"] = f["x"] + f["w"] / 2.0
            tracks[ti]["last_t"] = s["t"]
            tracks[ti]["xs"].append(f["x"] + f["w"] / 2.0)
        for fi, f in enumerate(s["faces"]):
            if fi in used_f:
                continue
            cx = f["x"] + f["w"] / 2.0
            f["_track"] = len(tracks)
            tracks.append({"cx": cx, "last_t": s["t"], "xs": [cx]})
    order = sorted(range(len(tracks)), key=lambda i: sum(tracks[i]["xs"]) / len(tracks[i]["xs"]))
    rank = {old: new for new, old in enumerate(order)}
    for s in samples:
        for f in s["faces"]:
            f["id"] = rank[f.pop("_track")]


def normalize_motion(samples: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Add ``speak`` (0..1) per face, normalized within its own track; returns per-track stats."""
    import numpy as np

    by_id: Dict[int, List[float]] = {}
    for s in samples:
        for f in s["faces"]:
            by_id.setdefault(f["id"], []).append(f["motion"])
    stats: Dict[int, Dict[str, float]] = {}
    for tid, vals in by_id.items():
        v = np.asarray(vals)
        lo, hi = float(np.percentile(v, 10)), float(np.percentile(v, 95))
        stats[tid] = {"lo": lo, "hi": hi, "spread": hi - lo}
    for s in samples:
        for f in s["faces"]:
            st = stats[f["id"]]
            if st["spread"] < MIN_MOTION_SPREAD:
                f["speak"] = 0.0
            else:
                f["speak"] = round(min(1.0, max(0.0, (f["motion"] - st["lo"]) / st["spread"])), 4)
    tracks = []
    for tid in sorted(by_id):
        areas = [f["w"] * f["h"] for s in samples for f in s["faces"] if f["id"] == tid]
        cxs = [f["x"] + f["w"] / 2.0 for s in samples for f in s["faces"] if f["id"] == tid]
        tracks.append(
            {
                "id": tid,
                "samples": len(by_id[tid]),
                "mean_x": round(sum(cxs) / len(cxs), 4),
                "mean_area": round(sum(areas) / len(areas), 5),
                "motion_p10": round(stats[tid]["lo"], 5),
                "motion_p95": round(stats[tid]["hi"], 5),
                "static": stats[tid]["spread"] < MIN_MOTION_SPREAD,
            }
        )
    return tracks


def active(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    from . import faces as faces_mod

    video_path = params.get("video_path")
    if not isinstance(video_path, str) or not video_path:
        raise RpcError("validation", "video_path is required")
    start = faces_mod._num(params, "start", 0.0)
    end = faces_mod._num(params, "end", None)
    fps = faces_mod._num(params, "fps", 5.0)
    if start < 0:
        raise RpcError("validation", "start must be >= 0")
    if fps <= 0 or fps > 15:
        raise RpcError("validation", "fps must be in (0, 15]")
    if end is not None and end <= start:
        raise RpcError("validation", "end must be greater than start")
    if not os.path.isfile(video_path):
        raise RpcError("validation", f"video not found: {video_path}")

    import cv2

    model = faces_mod.ensure_model(None, progress)
    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        raise RpcError("media", f"cannot open video: {video_path}")
    samples: List[Dict[str, Any]] = []
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
        scale = min(1.0, MAX_ACTIVE_WIDTH / width)
        dw, dh = max(1, round(width * scale)), max(1, round(height * scale))
        try:
            detector = cv2.FaceDetectorYN.create(
                str(model), "", (dw, dh), faces_mod.SCORE_THRESHOLD, faces_mod.NMS_THRESHOLD, faces_mod.TOP_K
            )
        except cv2.error as e:
            raise RpcError("media", f"cannot load YuNet model {model}: {e}") from e
        n = int((end - start) * fps + 1e-9)
        if n < 1 or n > MAX_ACTIVE_SAMPLES:
            raise RpcError("validation", f"range yields {n} samples (allowed 1..{MAX_ACTIVE_SAMPLES})")
        gap = max(1, round(src_fps * 0.05))  # frames between the 3 frames of a motion burst

        def prep(fr):
            if scale < 1.0:
                fr = cv2.resize(fr, (dw, dh), interpolation=cv2.INTER_AREA)
            return fr

        for i in range(n):
            t = start + i / fps
            cap.set(cv2.CAP_PROP_POS_MSEC, t * 1000.0)
            ok, f0 = cap.read()
            if not ok or f0 is None:
                break
            f0 = prep(f0)
            burst = [cv2.cvtColor(f0, cv2.COLOR_BGR2GRAY)]
            for _ in range(2):
                for _ in range(gap - 1):
                    cap.grab()
                ok, fr = cap.read()
                if not ok or fr is None:
                    break
                burst.append(cv2.cvtColor(prep(fr), cv2.COLOR_BGR2GRAY))
            detector.setInputSize((f0.shape[1], f0.shape[0]))
            _, rows = detector.detect(f0)
            fh, fw = f0.shape[:2]
            fl: List[Dict[str, Any]] = []
            if rows is not None:
                for r in rows:
                    x0 = min(max(float(r[0]) / fw, 0.0), 1.0)
                    y0 = min(max(float(r[1]) / fh, 0.0), 1.0)
                    x1 = min(max((float(r[0]) + float(r[2])) / fw, 0.0), 1.0)
                    y1 = min(max((float(r[1]) + float(r[3])) / fh, 0.0), 1.0)
                    if x1 <= x0 or y1 <= y0:
                        continue
                    box = (x0, y0, x1 - x0, y1 - y0)
                    rect = mouth_region(box, fw, fh)
                    diffs = [motion_between(burst[k], burst[k + 1], rect) for k in range(len(burst) - 1)]
                    motion = sum(diffs) / len(diffs) if diffs else 0.0
                    fl.append(
                        {
                            "x": round(box[0], 5),
                            "y": round(box[1], 5),
                            "w": round(box[2], 5),
                            "h": round(box[3], 5),
                            "score": round(float(r[-1]), 4),
                            "motion": round(motion, 5),
                        }
                    )
            fl.sort(key=lambda f: f["w"] * f["h"], reverse=True)
            samples.append({"t": round(t, 4), "faces": fl})
            if i % 4 == 0 or i == n - 1:
                progress("speakers.active", round((i + 1) / n * 100, 1))
    finally:
        cap.release()
    assign_tracks(samples)
    tracks = normalize_motion(samples)
    for s in samples:
        for f in s["faces"]:
            f["motion"] = round(f["motion"], 5)
    return {"width": width, "height": height, "fps": fps, "samples": samples, "tracks": tracks}
