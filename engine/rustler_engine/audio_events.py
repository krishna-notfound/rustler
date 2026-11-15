"""Audio event detection (``audio.events``): laughter, applause, cheering, music, speech, silence.

Primary classifier: YAMNet (Google, Apache-2.0) as an ONNX export with the mel front-end baked in
(``audiomagic/yamnet-onnx`` on Hugging Face, pinned commit, Apache-2.0). It is cached at
``$RUSTLER_HOME/models/yamnet/`` (default ``~/.rustler``), downloaded on first use and verified by
SHA-256 (see ``engine/MODELS.md``). The 521 AudioSet classes are folded into our six labels.

Fallback: when the model cannot be obtained, a small spectral heuristic (RMS, spectral flatness,
envelope modulation, onset rate) is used and the result says ``"model": "heuristic"``. The heuristic
can only tell silence / speech / music / applause apart; it never reports laughter or cheering.
"""

from __future__ import annotations

import csv
import hashlib
import logging
import math
import os
import shutil
import tempfile
import urllib.request
import wave
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

from .rpc import RpcError, require_extra

log = logging.getLogger("rustler_engine.audio_events")

SAMPLE_RATE = 16000
YAMNET_REPO = "audiomagic/yamnet-onnx"
YAMNET_COMMIT = "f25b741c2f0bdc6d7e6db24b5fddda23347dbafd"
_BASE = f"https://huggingface.co/{YAMNET_REPO}/resolve/{YAMNET_COMMIT}/"
YAMNET_FILES = {
    "yamnet.onnx": (_BASE + "yamnet.onnx", "d3835ffbbd4a1bb3e777f0ca217b5007907f5171dd5d17c4236b95b2af8f908e"),
    "yamnet_class_map.csv": (_BASE + "yamnet_class_map.csv", "cdf24d193e196d9e95912a2667051ae203e92a2ba09449218ccb40ef787c6df2"),
}

LABELS = ("laughter", "applause", "cheer", "music", "speech", "silence")
#: YAMNet frame geometry: 0.96 s window, 0.48 s hop (15360 / 7680 samples at 16 kHz).
FRAME_WIN = 15360
FRAME_HOP = 7680
#: Frames per ONNX call (1250 frames = exactly 600 s of hop grid) to bound memory on long files.
CHUNK_FRAMES = 1250

SILENCE_DB = -55.0
#: Per-label score needed for a hop to take that label.
REACTION_MIN = 0.30
SPEECH_MIN = 0.35
MUSIC_MIN = 0.35


def rustler_home() -> Path:
    env = os.environ.get("RUSTLER_HOME")
    return Path(env) if env else Path.home() / ".rustler"


def model_dir() -> Path:
    return rustler_home() / "models" / "yamnet"


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def ensure_model(progress: Optional[Callable[..., None]] = None) -> Tuple[Path, Path]:
    """Return ``(yamnet.onnx, class_map.csv)``, downloading and verifying them when missing."""
    d = model_dir()
    d.mkdir(parents=True, exist_ok=True)
    paths = []
    for name, (url, sha) in YAMNET_FILES.items():
        p = d / name
        if p.is_file() and _sha256(p) == sha:
            paths.append(p)
            continue
        if progress:
            progress("models.fetch", 0.0, f"downloading yamnet {name}")
        log.info("downloading %s -> %s", url, p)
        fd, tmp = tempfile.mkstemp(dir=str(d), suffix=".part")
        try:
            with os.fdopen(fd, "wb") as out, urllib.request.urlopen(url, timeout=120) as resp:
                shutil.copyfileobj(resp, out)
            got = _sha256(Path(tmp))
            if got != sha:
                raise RpcError("media", f"downloaded {name} has sha256 {got}, expected {sha}")
            os.replace(tmp, p)
        except RpcError:
            raise
        except Exception as e:
            raise RpcError("media", f"could not download YAMNet {name}: {type(e).__name__}: {e}") from e
        finally:
            if os.path.exists(tmp):
                os.unlink(tmp)
        paths.append(p)
    if progress:
        progress("models.fetch", 100.0, "yamnet ready")
    return paths[0], paths[1]


def fetch_yamnet(progress: Callable[..., None]) -> str:
    return str(ensure_model(progress)[0])


# --- audio loading -----------------------------------------------------------------------


def load_wav(path: str):
    """Read a PCM WAV as mono float32 at 16 kHz (linear resampling when needed)."""
    import numpy as np

    if not os.path.isfile(path):
        raise RpcError("validation", f"audio not found: {path}")
    try:
        with wave.open(path, "rb") as w:
            ch, width, rate, n = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
            raw = w.readframes(n)
    except (wave.Error, EOFError) as e:
        raise RpcError("media", f"audio_path must be a PCM WAV file (the source's 16 kHz wav): {e}") from e
    if width == 2:
        x = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    elif width == 1:
        x = (np.frombuffer(raw, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    elif width == 4:
        x = np.frombuffer(raw, dtype="<i4").astype(np.float32) / 2147483648.0
    elif width == 3:
        b = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
        v = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
        v = np.where(v >= 1 << 23, v - (1 << 24), v)
        x = v.astype(np.float32) / 8388608.0
    else:
        raise RpcError("media", f"unsupported WAV sample width {width}")
    if ch > 1:
        x = x[: len(x) // ch * ch].reshape(-1, ch).mean(axis=1)
    if rate != SAMPLE_RATE and len(x) > 1:
        n_out = int(round(len(x) * SAMPLE_RATE / rate))
        x = np.interp(np.linspace(0, len(x) - 1, n_out), np.arange(len(x)), x).astype(np.float32)
    return np.ascontiguousarray(x, dtype=np.float32)


# --- label groups ------------------------------------------------------------------------


def group_indices(names: List[str]) -> Dict[str, List[Tuple[int, float]]]:
    """Fold AudioSet display names into our labels as ``label -> [(class index, weight)]``."""
    g: Dict[str, List[Tuple[int, float]]] = {k: [] for k in LABELS}
    for i, raw in enumerate(names):
        n = raw.strip().lower()
        if "laugh" in n or n in ("giggle", "snicker") or n.startswith("chuckle"):
            g["laughter"].append((i, 1.0))
        elif n in ("applause", "clapping"):
            g["applause"].append((i, 1.0))
        elif n == "cheering":
            g["cheer"].append((i, 1.0))
        elif n == "crowd":
            g["cheer"].append((i, 0.7))
        elif n == "silence":
            g["silence"].append((i, 1.0))
        elif n in ("speech synthesizer",) or n.startswith("hubbub"):
            continue
        elif n.startswith(("speech", "male speech", "female speech", "child speech", "conversation", "narration")):
            g["speech"].append((i, 1.0))
        elif n == "music" or n == "musical instrument" or n.endswith(" music") or n.startswith("music of") or n == "singing":
            g["music"].append((i, 1.0))
    return g


def read_class_names(csv_path: Path) -> List[str]:
    with open(csv_path, newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    return [r["display_name"] for r in sorted(rows, key=lambda r: int(r["index"]))]


# --- YAMNet inference --------------------------------------------------------------------


def yamnet_group_scores(x, progress: Callable[..., None]):
    """Return ``(frame_scores[frames, 6], model_name)``; frame i covers [0.48 i, 0.48 i + 0.96] s."""
    import numpy as np

    require_extra("audio-events", "YAMNet sound-event detection")
    import onnxruntime as ort

    model, class_map = ensure_model(progress)
    names = read_class_names(class_map)
    groups = group_indices(names)
    so = ort.SessionOptions()
    so.log_severity_level = 3
    sess = ort.InferenceSession(str(model), so, providers=["CPUExecutionProvider"])
    in_name = sess.get_inputs()[0].name
    chunk_len = (CHUNK_FRAMES - 1) * FRAME_HOP + FRAME_WIN
    stride = CHUNK_FRAMES * FRAME_HOP
    out: List[Any] = []
    n = len(x)
    pos, total = 0, max(1, n)
    while pos < n or not out:
        seg = x[pos : pos + chunk_len]
        if len(seg) < FRAME_WIN:
            seg = np.pad(seg, (0, FRAME_WIN - len(seg)))
        scores = sess.run(None, {in_name: seg})[0]
        out.append(scores)
        pos += stride
        progress("audio.events", round(min(pos, total) / total * 100, 1))
    s = np.concatenate(out, axis=0)
    cols = []
    for label in LABELS:
        idx = groups[label]
        if not idx:
            cols.append(np.zeros(len(s), dtype=np.float32))
            continue
        ii = [i for i, _ in idx]
        ww = np.array([w for _, w in idx], dtype=np.float32)
        cols.append((s[:, ii] * ww).max(axis=1))
    return np.stack(cols, axis=1), "yamnet"


def frames_to_bins(frame_scores, n_bins: int, hop: float):
    """Average the (at most two) YAMNet frames whose window contains each bin's centre."""
    import numpy as np

    nf = len(frame_scores)
    out = np.zeros((n_bins, frame_scores.shape[1]), dtype=np.float32)
    step = FRAME_HOP / SAMPLE_RATE
    win = FRAME_WIN / SAMPLE_RATE
    for b in range(n_bins):
        c = (b + 0.5) * hop
        hi = int(math.floor(c / step))
        lo = int(math.ceil((c - win) / step))
        lo, hi = max(lo, 0), min(hi, nf - 1)
        if lo > hi:
            lo = hi = min(max(int(round((c - win / 2) / step)), 0), nf - 1)
        out[b] = frame_scores[lo : hi + 1].mean(axis=0)
    return out


# --- heuristic fallback ------------------------------------------------------------------


def heuristic_features(x, hop: float, n_bins: int):
    """Per-hop ``(rms_db, flatness, mod_ratio, onsets_per_s)`` arrays."""
    import numpy as np

    hs = int(round(hop * SAMPLE_RATE))
    rms_db = np.full(n_bins, -120.0, dtype=np.float32)
    flat = np.zeros(n_bins, dtype=np.float32)
    mod = np.zeros(n_bins, dtype=np.float32)
    onsets = np.zeros(n_bins, dtype=np.float32)
    fl, fh = 512, 160  # 32 ms frames, 10 ms hop for the envelope
    win = np.hanning(fl).astype(np.float32)
    for b in range(n_bins):
        seg = x[b * hs : (b + 1) * hs]
        if len(seg) < fl:
            continue
        rms = float(np.sqrt(np.mean(seg.astype(np.float64) ** 2)))
        rms_db[b] = 20 * math.log10(max(rms, 1e-9))
        n_fr = 1 + (len(seg) - fl) // fh
        idx = np.arange(fl)[None, :] + fh * np.arange(n_fr)[:, None]
        frames = seg[idx] * win
        env = np.sqrt(np.mean(frames**2, axis=1)) + 1e-9
        spec = np.abs(np.fft.rfft(frames, axis=1)) ** 2 + 1e-12
        gm = np.exp(np.mean(np.log(spec), axis=1))
        am = np.mean(spec, axis=1)
        flat[b] = float(np.mean(gm / am))
        e = env - env.mean()
        if len(e) >= 8 and float(np.sum(e**2)) > 0:
            ps = np.abs(np.fft.rfft(e * np.hanning(len(e)))) ** 2
            freqs = np.fft.rfftfreq(len(e), d=fh / SAMPLE_RATE)
            band = (freqs >= 2.0) & (freqs <= 8.0)
            mod[b] = float(ps[band].sum() / max(ps[1:].sum(), 1e-12))
        d = np.diff(env)
        thr = max(0.5 * float(np.std(env)), 0.02 * float(env.max()))
        peaks = (d[:-1] > thr) & (d[1:] <= thr)
        onsets[b] = float(np.sum(peaks)) / (len(seg) / SAMPLE_RATE)
    return rms_db, flat, mod, onsets


def heuristic_labels(x, hop: float, n_bins: int) -> List[Tuple[Optional[str], float]]:
    rms_db, flat, mod, onsets = (a.astype(float) for a in heuristic_features(x, hop, n_bins))
    out: List[Tuple[Optional[str], float]] = []
    for b in range(n_bins):
        if rms_db[b] < SILENCE_DB:
            out.append(("silence", 1.0))
        elif flat[b] > 0.25 and onsets[b] >= 8.0:
            out.append(("applause", float(round(min(1.0, 0.4 + onsets[b] / 40.0), 3))))
        elif flat[b] < 0.12 and mod[b] < 0.25:
            out.append(("music", float(round(min(1.0, 0.5 + (0.12 - flat[b]) * 3), 3))))
        elif mod[b] >= 0.15 and flat[b] < 0.5:
            out.append(("speech", float(round(min(1.0, 0.4 + mod[b]), 3))))
        else:
            out.append((None, 0.0))
    return out


# --- decisions and merging ---------------------------------------------------------------


def decide(scores, rms_db: float) -> Tuple[Optional[str], float]:
    """Pick one label for a hop from its group scores (order of ``LABELS``)."""
    s = {k: float(scores[i]) for i, k in enumerate(LABELS)}
    if rms_db < SILENCE_DB:
        return "silence", 1.0
    react = [(s[k], k) for k in ("laughter", "applause", "cheer") if s[k] >= REACTION_MIN]
    if react:
        v, k = max(react)
        return k, v
    if s["speech"] >= SPEECH_MIN and s["speech"] >= s["music"]:
        return "speech", s["speech"]
    if s["music"] >= MUSIC_MIN:
        return "music", s["music"]
    if s["speech"] >= 0.25:
        return "speech", s["speech"]
    if s["silence"] >= 0.5:
        return "silence", s["silence"]
    return None, 0.0


def merge_bins(labels: List[Tuple[Optional[str], float]], hop: float, duration: float, max_gap_bins: int = 1) -> List[Dict[str, Any]]:
    """Merge adjacent hops with the same label; bridge gaps of up to ``max_gap_bins`` hops."""
    n = len(labels)
    lab = [l for l, _ in labels]
    # bridge short runs of unlabeled hops between two hops that carry the same label
    i = 0
    while i < n:
        if lab[i] is None:
            j = i
            while j < n and lab[j] is None:
                j += 1
            if 0 < i and j < n and j - i <= max_gap_bins and lab[i - 1] == lab[j]:
                for k in range(i, j):
                    lab[k] = lab[i - 1]
            i = j
        else:
            i += 1
    events: List[Dict[str, Any]] = []
    i = 0
    while i < n:
        if lab[i] is None:
            i += 1
            continue
        j = i
        while j + 1 < n and lab[j + 1] == lab[i]:
            j += 1
        sc = [labels[k][1] for k in range(i, j + 1) if labels[k][0] == lab[i]] or [0.0]
        events.append(
            {
                "start": round(i * hop, 3),
                "end": round(min((j + 1) * hop, duration), 3),
                "label": lab[i],
                "score": round(sum(sc) / len(sc), 3),
            }
        )
        i = j + 1
    return [e for e in events if e["end"] > e["start"]]


def _rms_db_per_bin(x, hop: float, n_bins: int):
    import numpy as np

    hs = int(round(hop * SAMPLE_RATE))
    out = np.full(n_bins, -120.0, dtype=np.float32)
    for b in range(n_bins):
        seg = x[b * hs : (b + 1) * hs]
        if len(seg):
            out[b] = 20 * math.log10(max(float(np.sqrt(np.mean(seg.astype(np.float64) ** 2))), 1e-9))
    return out


def run(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    audio_path = params.get("audio_path")
    if not isinstance(audio_path, str) or not audio_path:
        raise RpcError("validation", "audio_path is required")
    hop = params.get("hop", 0.5)
    if isinstance(hop, bool) or not isinstance(hop, (int, float)) or not (0.1 <= hop <= 5.0):
        raise RpcError("validation", "hop must be a number in [0.1, 5.0] seconds")
    hop = float(hop)
    which = params.get("model", "auto")
    if which not in ("auto", "yamnet", "heuristic"):
        raise RpcError("validation", "model must be auto | yamnet | heuristic")

    x = load_wav(audio_path)
    duration = len(x) / SAMPLE_RATE
    if duration <= 0:
        return {"events": [], "model": "none", "duration": 0.0, "hop": hop}
    n_bins = max(1, int(math.ceil(duration / hop - 1e-9)))
    warnings: List[str] = []

    labels: Optional[List[Tuple[Optional[str], float]]] = None
    model_name = "heuristic"
    if which in ("auto", "yamnet"):
        try:
            frame_scores, model_name = yamnet_group_scores(x, progress)
            bins = frames_to_bins(frame_scores, n_bins, hop)
            rms_db = _rms_db_per_bin(x, hop, n_bins)
            labels = [decide(bins[b], float(rms_db[b])) for b in range(n_bins)]
        except RpcError as e:
            if which == "yamnet" or e.kind in ("validation",):
                raise
            hint = f"; to enable: {e.hint}" if getattr(e, "hint", None) else ""
            warnings.append(f"YAMNet unavailable ({e.message}); used the spectral heuristic instead{hint}")
            log.warning("yamnet unavailable: %s", e.message)
        except ImportError as e:
            if which == "yamnet":
                raise
            warnings.append(f"YAMNet unavailable ({e}); used the spectral heuristic instead")
    if labels is None:
        model_name = "heuristic"
        labels = heuristic_labels(x, hop, n_bins)
        progress("audio.events", 100.0)
    events = merge_bins(labels, hop, duration)
    res: Dict[str, Any] = {"events": events, "model": model_name, "duration": round(duration, 3), "hop": hop}
    if warnings:
        res["warnings"] = warnings
    return res
