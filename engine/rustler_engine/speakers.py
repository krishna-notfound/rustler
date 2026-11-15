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
    """sherpa-onnx segmentation for the speech regions, then windowed-embedding re-segmentation.

    sherpa's own clustering embeds whole (often long) segments. On real two-person footage with
    similar voices and room reverb it merged a host and a guest into one speaker. The windowed
    pass (:func:`resegment`) embeds 1.5 s windows, clusters them and smooths the labels, which
    separated the same voices. If the windowed pass cannot run, sherpa's own labels are used.
    """
    seg, emb = ensure_models(progress)
    raw = _sherpa_pass(x, seg, emb, num_speakers if num_speakers else -1, progress)
    own = relabel(raw)
    try:
        regions = merge_regions([(s["start"], s["end"]) for s in own], gap=MIN_DURATION_OFF)
        segs = resegment(x, regions, emb, num_speakers, min_speakers, max_speakers, progress)
    except ImportError:
        segs = []
    if segs:
        return segs
    if num_speakers:
        return own
    n = count_speakers(own)
    target = n
    if max_speakers is not None and n > max_speakers:
        target = max_speakers
    if min_speakers is not None and n < min_speakers:
        target = min_speakers
    if target != n and len(x) > 0:
        # Threshold clustering is outside the requested range: redo the clustering with a fixed count.
        own = relabel(_sherpa_pass(x, seg, emb, target, progress))
    return own


# --- windowed embedding re-segmentation ---------------------------------------------------------

EMBED_WINDOW = 1.5
EMBED_HOP = 0.5
#: Auto speaker count: k clusters are believed when every pair is at least this far apart (mean cosine
#: distance between their windows) and at least MIN_SPLIT_RATIO times the spread inside any cluster.
MIN_SPLIT_HEIGHT = 0.33
MIN_SPLIT_RATIO = 1.3
MAX_AUTO_SPEAKERS = 8
#: A speaker needs at least this share of the embedded windows and this many seconds of speech.
MIN_SPEAKER_SHARE = 0.08
MIN_SPEAKER_SECONDS = 3.0
#: Cost (in cosine-similarity units) of switching speaker between two adjacent windows.
SWITCH_PENALTY = 0.25
#: Switch boundaries snap to the quietest moment within this many seconds.
SNAP_SECONDS = 0.7
#: Speaker runs shorter than this are absorbed by the neighbours (back-channels, breaths).
MIN_RUN_SECONDS = 1.0


def merge_regions(spans: List[Tuple[float, float]], gap: float = 0.0) -> List[Tuple[float, float]]:
    out: List[List[float]] = []
    for a, b in sorted(spans):
        if out and a <= out[-1][1] + gap:
            out[-1][1] = max(out[-1][1], b)
        else:
            out.append([a, b])
    return [(a, b) for a, b in out]


def embedding_windows(
    regions: List[Tuple[float, float]], window: float = EMBED_WINDOW, hop: float = EMBED_HOP
) -> List[Tuple[float, float, int]]:
    """``(start, end, region_index)`` windows every ``hop`` s inside each speech region; a region
    shorter than the window gets one window of its own length (regions under 0.5 s are skipped)."""
    wins: List[Tuple[float, float, int]] = []
    for ri, (a, b) in enumerate(regions):
        if b - a < 0.5:
            continue
        if b - a <= window + hop / 2:
            wins.append((a, b, ri))
            continue
        s = a
        while s + window <= b + 1e-9:
            wins.append((s, s + window, ri))
            s += hop
        if wins[-1][1] < b - hop / 2:
            wins.append((b - window, b, ri))
    return wins


def _embed(x, wins: List[Tuple[float, float, int]], emb: Path, progress: Callable[..., None]):
    import numpy as np
    import sherpa_onnx as so

    threads = max(1, min(4, os.cpu_count() or 2))
    ex = so.SpeakerEmbeddingExtractor(so.SpeakerEmbeddingExtractorConfig(model=str(emb), num_threads=threads, provider="cpu"))
    out = []
    for i, (a, b, _) in enumerate(wins):
        st = ex.create_stream()
        st.accept_waveform(sample_rate=SAMPLE_RATE, waveform=x[int(a * SAMPLE_RATE) : int(b * SAMPLE_RATE)])
        st.input_finished()
        e = np.asarray(ex.compute(st), dtype=np.float32)
        out.append(e / max(1e-9, float(np.linalg.norm(e))))
        if i % 25 == 0:
            progress("speakers.diarize", round(i / len(wins) * 100.0, 1))
    return np.stack(out)


def cluster_embeddings(E, k: Optional[int], lo: Optional[int] = None, hi: Optional[int] = None, mass=None):
    """Average-linkage agglomerative clustering of unit vectors on cosine distance.

    ``k`` fixes the cluster count. Otherwise the count is the largest one whose clusters are clearly apart
    (:data:`MIN_SPLIT_HEIGHT`, :data:`MIN_SPLIT_RATIO`), clusters holding too little speech are folded into the nearest one,
    and the ``lo``/``hi`` bounds are applied. ``mass`` is the speech time (s) each window stands
    for (windows overlap, so this is not the window length); default 0.5 s. Returns integer labels
    (0 = first window's cluster).
    """
    import numpy as np

    n = len(E)
    if n <= 1:
        return np.zeros(n, dtype=int)
    dist0 = np.maximum(0.0, 1.0 - E @ E.T)
    dist = dist0.copy()
    np.fill_diagonal(dist, np.inf)
    sizes = np.ones(n)
    history: List[Tuple[int, int, float]] = []
    for _ in range(n - 1):
        flat = int(np.argmin(dist))
        i, j = divmod(flat, n)
        if i > j:
            i, j = j, i
        history.append((i, j, float(dist[i, j])))
        new = (dist[i] * sizes[i] + dist[j] * sizes[j]) / (sizes[i] + sizes[j])
        dist[i, :] = new
        dist[:, i] = new
        dist[i, i] = np.inf
        dist[j, :] = np.inf
        dist[:, j] = np.inf
        sizes[i] += sizes[j]

    def cut(target: int) -> Dict[int, List[int]]:
        g: Dict[int, List[int]] = {i: [i] for i in range(n)}
        for i, j, _ in history[: n - target]:
            g[i].extend(g.pop(j))
        return g

    w = np.full(n, EMBED_HOP) if mass is None else np.asarray(mass, dtype=float)
    min_mass = max(MIN_SPEAKER_SECONDS, MIN_SPEAKER_SHARE * float(w.sum()))

    def fold(g: Dict[int, List[int]]) -> Dict[int, List[int]]:
        """Fold clusters with too little speech (applause, laughter, noise) into the nearest one."""
        while len(g) > 1:
            small = [c for c, m in g.items() if float(w[m].sum()) < min_mass]
            if not small:
                break
            c = min(small, key=lambda c: len(g[c]))
            tgt = min((o for o in g if o != c), key=lambda o: float(dist0[np.ix_(g[c], g[o])].mean()))
            g[tgt].extend(g.pop(c))
        return g

    def avg(a: List[int], b: List[int]) -> float:
        return float(dist0[np.ix_(a, b)].mean())

    def forced(kk: int) -> Dict[int, List[int]]:
        """``kk`` clusters of real speech: the smallest cut that leaves kk clusters with enough
        speech once the debris (applause, laughter, noise) is folded into its nearest cluster."""
        g = cut(kk)
        for t in range(kk, min(n, kk + 12) + 1):
            cand = cut(t)
            if sum(1 for m in cand.values() if float(w[m].sum()) >= min_mass) >= kk:
                g = cand
                break
        g = fold(g)
        while len(g) > kk:  # more big clusters than asked: merge the closest pair
            ids = list(g)
            i, j = min(((a, b) for ai, a in enumerate(ids) for b in ids[ai + 1 :]), key=lambda p: avg(g[p[0]], g[p[1]]))
            g[i].extend(g.pop(j))
        return g

    def within(m: List[int]) -> float:
        if len(m) < 2:
            return 0.0
        sub = dist0[np.ix_(m, m)]
        return float(sub.sum() / (len(m) * (len(m) - 1)))

    def believable(g: Dict[int, List[int]]) -> bool:
        ids = list(g)
        if len(ids) < 2:
            return True
        between = min(avg(g[a], g[b]) for ai, a in enumerate(ids) for b in ids[ai + 1 :])
        return between >= MIN_SPLIT_HEIGHT and between >= MIN_SPLIT_RATIO * max(within(g[a]) for a in ids)

    if k:
        k = max(1, min(k, n))
        g = forced(k)
        if len(g) < k:  # not enough distinct speech to fill k clusters: honour the exact count anyway
            g = cut(k)
    else:
        # Speaker count: the largest k whose clusters are all clearly apart (average cosine distance
        # between clusters vs the spread inside them); one speaker's windows also spread out a little.
        g = forced(1)
        for kk in range(2, min(MAX_AUTO_SPEAKERS, n) + 1):
            cand = forced(kk)
            if len(cand) < kk or not believable(cand):
                break
            g = cand
        if lo and len(g) < lo:
            g = forced(min(lo, n))
        if hi and len(g) > hi:
            g = forced(hi)
    labels = np.zeros(n, dtype=int)
    for lab, (_, idxs) in enumerate(sorted(g.items(), key=lambda kv: min(kv[1]))):
        labels[idxs] = lab
    return labels


def smooth_labels(E, labels, penalty: float = SWITCH_PENALTY):
    """Viterbi over windows in time order: emission = cosine similarity to the cluster centroid,
    a fixed cost for every speaker switch (so a lone odd window does not flip the speaker)."""
    import numpy as np

    n = len(E)
    k = int(labels.max()) + 1 if n else 0
    if k <= 1 or n < 2:
        return labels
    cent = np.stack([E[labels == c].mean(axis=0) for c in range(k)])
    cent /= np.maximum(1e-9, np.linalg.norm(cent, axis=1, keepdims=True))
    sim = E @ cent.T
    score = np.zeros((n, k))
    back = np.zeros((n, k), dtype=int)
    score[0] = sim[0]
    for i in range(1, n):
        for c in range(k):
            sw = score[i - 1].copy()
            sw[c] = -np.inf
            j = int(np.argmax(sw))
            if sw[j] - penalty > score[i - 1, c]:
                score[i, c] = sw[j] - penalty + sim[i, c]
                back[i, c] = j
            else:
                score[i, c] = score[i - 1, c] + sim[i, c]
                back[i, c] = c
    out = np.zeros(n, dtype=int)
    out[-1] = int(np.argmax(score[-1]))
    for i in range(n - 1, 0, -1):
        out[i - 1] = back[i, out[i]]
    return out


def window_slots(wins: List[Tuple[float, float, int]]) -> List[Tuple[float, float]]:
    """The non-overlapping slot each window owns: from the midpoint to its previous neighbour's
    centre to the midpoint to the next one's; the first and last window of a speech region own
    that region's edge. One entry per window (a zero-length slot is possible)."""
    centres = [(a + b) / 2.0 for a, b, _ in wins]
    out: List[Tuple[float, float]] = []
    for i, (a, b, ri) in enumerate(wins):
        first = i == 0 or wins[i - 1][2] != ri
        last = i + 1 == len(wins) or wins[i + 1][2] != ri
        lo = a if first else (centres[i - 1] + centres[i]) / 2.0
        hi = b if last else (centres[i] + centres[i + 1]) / 2.0
        out.append((lo, max(lo, hi)))
    return out


def window_spans(wins: List[Tuple[float, float, int]], labels) -> List[Tuple[float, float, int]]:
    """``(start, end, label)`` per window slot (see :func:`window_slots`), empty slots dropped."""
    return [(lo, hi, int(labels[i])) for i, (lo, hi) in enumerate(window_slots(wins)) if hi > lo]


def merge_runs(spans: List[Tuple[float, float, int]], min_run: float = MIN_RUN_SECONDS) -> List[List[Any]]:
    """Join adjacent same-speaker spans, then let neighbours absorb runs shorter than ``min_run``."""
    runs: List[List[Any]] = []

    def push(r: List[Any]) -> None:
        if runs and runs[-1][2] == r[2] and r[0] - runs[-1][1] <= 1e-6:
            runs[-1][1] = r[1]
        else:
            runs.append(list(r))

    for s in spans:
        push(list(s))
    while len(runs) > 1:
        short = [i for i, r in enumerate(runs) if r[1] - r[0] < min_run]
        victim = None
        for i in sorted(short, key=lambda i: runs[i][1] - runs[i][0]):
            nbs = [j for j in (i - 1, i + 1) if 0 <= j < len(runs) and (abs(runs[j][1] - runs[i][0]) <= 1e-6 or abs(runs[j][0] - runs[i][1]) <= 1e-6)]
            if nbs:
                victim = (i, max(nbs, key=lambda j: runs[j][1] - runs[j][0]))
                break
        if victim is None:
            break
        i, j = victim
        if j < i:
            runs[j][1] = runs[i][1]
        else:
            runs[j][0] = runs[i][0]
        del runs[i]
        merged = runs[:]
        runs.clear()
        for r in merged:
            push(r)
    return runs


def _quiet_snap(energy, t: float, lo: float, hi: float, hop: float = 0.01) -> float:
    """The quietest moment (smoothed frame energy) in ``[lo, hi]``; ties go to the one nearest ``t``."""
    import numpy as np

    a, b = max(0, int(lo / hop)), min(len(energy), int(hi / hop))
    if b - a < 8:
        return t
    e = np.convolve(energy[a:b], np.ones(7) / 7.0, mode="same")[3:-3]
    if len(e) == 0:
        return t
    cand = np.nonzero(e <= float(e.min()) + 1.0)[0] + 3  # within 1 dB of the quietest frame
    best = min(cand, key=lambda i: abs((a + i) * hop - t))
    return (a + int(best)) * hop


def resegment(
    x,
    regions: List[Tuple[float, float]],
    emb: Path,
    num_speakers: Optional[int],
    min_speakers: Optional[int],
    max_speakers: Optional[int],
    progress: Callable[..., None],
) -> List[Dict[str, Any]]:
    """Windowed-embedding diarization inside ``regions`` (see :func:`diarize_sherpa`)."""
    duration = len(x) / SAMPLE_RATE
    scale = max(1.0, duration / 900.0)  # bounds the window count (the clustering is O(n^2))
    hop = EMBED_HOP * scale
    wins = embedding_windows(regions, EMBED_WINDOW * min(scale, 2.0), hop)
    if not wins:
        return []
    E = _embed(x, wins, emb, progress)
    mass = [hi - lo for lo, hi in window_slots(wins)]
    labels = cluster_embeddings(E, num_speakers, min_speakers, max_speakers, mass)
    raw_labels = labels
    labels = smooth_labels(E, labels)
    runs = merge_runs(window_spans(wins, labels))
    if num_speakers and len({r[2] for r in runs}) < min(num_speakers, len(set(raw_labels.tolist()))):
        # An exact count was asked for: smoothing must not erase a speaker with little speech.
        runs = merge_runs(window_spans(wins, raw_labels), min_run=0.0)
    _, energy = mfcc_frames(x)
    for i in range(1, len(runs)):
        a, b = runs[i - 1], runs[i]
        if abs(a[1] - b[0]) <= 1e-6 and a[2] != b[2]:
            t = _quiet_snap(energy, a[1], max(a[0] + 0.3, a[1] - SNAP_SECONDS), min(b[1] - 0.3, a[1] + SNAP_SECONDS))
            a[1] = b[0] = t
    progress("speakers.diarize", 100.0)
    return relabel([(float(a), float(b), int(lab)) for a, b, lab in runs])


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

MAX_ACTIVE_WIDTH = 960
SFACE_FILENAME = "face_recognition_sface_2021dec.onnx"
SFACE_URL = "https://github.com/opencv/opencv_zoo/raw/main/models/face_recognition_sface/" + SFACE_FILENAME
SFACE_SHA256 = "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79"
#: Faces narrower than this many pixels (analysis frame) get no appearance embedding.
MIN_EMBED_FACE_PX = 24
#: Tracks of one person: cosine similarity of the SFace embeddings (OpenCV's own threshold is 0.363).
SAME_PERSON_COS = 0.40
SAME_TRACK_MIN_COS = 0.25
TRACK_MAX_WIDTH_RATIO = 1.7
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


def _cos(a, b) -> float:
    import numpy as np

    return float(np.dot(a, b) / max(1e-9, float(np.linalg.norm(a) * np.linalg.norm(b))))


def assign_tracks(samples: List[Dict[str, Any]]) -> None:
    """Give every face a ``track`` (and ``id`` = ``track`` until :func:`link_persons` runs) by greedy
    matching to tracks seen in the last 3 s: nearest centre within 0.22 of the frame width, similar
    size, and, when both faces carry an appearance embedding (``_emb``), a similar-looking face.
    Without the size and appearance gates a cut to a close-up glued the close-up to the wide-shot
    face next to it. Tracks are numbered left to right (by mean centre x)."""
    tracks: List[Dict[str, Any]] = []
    for s in samples:
        pairs = []
        for fi, f in enumerate(s["faces"]):
            cx = f["x"] + f["w"] / 2.0
            for ti, tr in enumerate(tracks):
                if s["t"] - tr["last_t"] > TRACK_HOLD_SECONDS:
                    continue
                d = abs(cx - tr["cx"])
                if d > TRACK_MAX_DX:
                    continue
                if max(f["w"], tr["w"]) / max(1e-6, min(f["w"], tr["w"])) > TRACK_MAX_WIDTH_RATIO:
                    continue
                if "_emb" in f and tr["emb"] is not None and _cos(f["_emb"], tr["emb"]) < SAME_TRACK_MIN_COS:
                    continue
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
            tr = tracks[ti]
            tr["cx"] = f["x"] + f["w"] / 2.0
            tr["w"] = f["w"]
            tr["last_t"] = s["t"]
            tr["xs"].append(f["x"] + f["w"] / 2.0)
            if "_emb" in f:
                tr["emb"] = f["_emb"] if tr["emb"] is None else tr["emb"] * 0.8 + f["_emb"] * 0.2
        for fi, f in enumerate(s["faces"]):
            if fi in used_f:
                continue
            cx = f["x"] + f["w"] / 2.0
            f["_track"] = len(tracks)
            tracks.append({"cx": cx, "w": f["w"], "last_t": s["t"], "xs": [cx], "emb": f.get("_emb")})
    order = sorted(range(len(tracks)), key=lambda i: sum(tracks[i]["xs"]) / len(tracks[i]["xs"]))
    rank = {old: new for new, old in enumerate(order)}
    for s in samples:
        for f in s["faces"]:
            f["track"] = rank[f.pop("_track")]
            f["id"] = f["track"]


def link_persons(samples: List[Dict[str, Any]]) -> int:
    """Group tracks into people by appearance, so a person keeps one ``id`` across cuts (wide shot,
    close-up, wide shot again). Tracks that are on screen in the same sample are different people.
    Average-linkage on cosine similarity of the mean face embedding, merged while >= SAME_PERSON_COS.
    Tracks without an embedding (tiny faces) stay separate. ``id`` becomes the person number,
    ordered left to right by mean centre x. Returns the number of people."""
    import numpy as np

    by_track: Dict[int, Dict[str, Any]] = {}
    for si, s in enumerate(samples):
        for f in s["faces"]:
            t = by_track.setdefault(f["track"], {"emb": [], "xs": [], "seen": set()})
            t["xs"].append(f["x"] + f["w"] / 2.0)
            t["seen"].add(si)
            if "_emb" in f:
                t["emb"].append(f["_emb"])
    ids = sorted(by_track)
    mean_emb = {}
    for tid in ids:
        e = by_track[tid]["emb"]
        if len(e) >= 2:
            m = np.mean(np.stack(e), axis=0)
            mean_emb[tid] = m / max(1e-9, float(np.linalg.norm(m)))
    clusters: List[List[int]] = [[tid] for tid in ids]

    def conflict(a: List[int], b: List[int]) -> bool:
        sa = set().union(*(by_track[t]["seen"] for t in a))
        sb = set().union(*(by_track[t]["seen"] for t in b))
        return bool(sa & sb)

    def sim(a: List[int], b: List[int]) -> Optional[float]:
        v = [float(mean_emb[x] @ mean_emb[y]) for x in a if x in mean_emb for y in b if y in mean_emb]
        return sum(v) / len(v) if v else None

    while True:
        best = None
        for i in range(len(clusters)):
            for j in range(i + 1, len(clusters)):
                v = sim(clusters[i], clusters[j])
                if v is None or v < SAME_PERSON_COS or conflict(clusters[i], clusters[j]):
                    continue
                if best is None or v > best[0]:
                    best = (v, i, j)
        if best is None:
            break
        _, i, j = best
        clusters[i].extend(clusters.pop(j))
    mean_x = lambda c: sum(sum(by_track[t]["xs"]) for t in c) / sum(len(by_track[t]["xs"]) for t in c)  # noqa: E731
    clusters.sort(key=mean_x)
    person_of = {t: pi for pi, c in enumerate(clusters) for t in c}
    for s in samples:
        for f in s["faces"]:
            f["id"] = person_of[f["track"]]
    return len(clusters)


def normalize_motion(samples: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Add ``speak`` (0..1) per face, normalized within its own track (a track is one face in one
    shot, so a close-up and a wide shot of the same person are normalized separately); returns
    one stats row per person (``id``)."""
    import numpy as np

    key = lambda f: f.get("track", f["id"])  # noqa: E731
    by_track: Dict[int, List[float]] = {}
    for s in samples:
        for f in s["faces"]:
            by_track.setdefault(key(f), []).append(f["motion"])
    stats: Dict[int, Dict[str, float]] = {}
    for tid, vals in by_track.items():
        v = np.asarray(vals)
        lo, hi = float(np.percentile(v, 10)), float(np.percentile(v, 95))
        stats[tid] = {"lo": lo, "hi": hi, "spread": hi - lo}
    for s in samples:
        for f in s["faces"]:
            st = stats[key(f)]
            if st["spread"] < MIN_MOTION_SPREAD:
                f["speak"] = 0.0
            else:
                f["speak"] = round(min(1.0, max(0.0, (f["motion"] - st["lo"]) / st["spread"])), 4)
    people: Dict[int, List[Dict[str, Any]]] = {}
    for s in samples:
        for f in s["faces"]:
            people.setdefault(f["id"], []).append(f)
    tracks = []
    for pid in sorted(people):
        fs = people[pid]
        tids = {key(f) for f in fs}
        main = max(tids, key=lambda t: len(by_track[t]))
        tracks.append(
            {
                "id": pid,
                "samples": len(fs),
                "mean_x": round(sum(f["x"] + f["w"] / 2.0 for f in fs) / len(fs), 4),
                "mean_area": round(sum(f["w"] * f["h"] for f in fs) / len(fs), 5),
                "motion_p10": round(stats[main]["lo"], 5),
                "motion_p95": round(stats[main]["hi"], 5),
                "static": all(stats[t]["spread"] < MIN_MOTION_SPREAD for t in tids),
                "shots": len(tids),
            }
        )
    return tracks


def ensure_sface(progress: Optional[Callable[..., None]] = None) -> Optional[Path]:
    """The SFace face-recognition model (OpenCV Zoo, Apache-2.0), downloaded on first use and
    verified by sha256. Returns ``None`` when it cannot be had (offline): people are then not linked
    across cuts."""
    p = rustler_home() / "models" / "sface" / SFACE_FILENAME
    try:
        if p.is_file() and _sha256(p) == SFACE_SHA256:
            return p
        p.parent.mkdir(parents=True, exist_ok=True)
        if progress:
            progress("models.fetch", 0.0, f"downloading {SFACE_FILENAME} (SFace, Apache-2.0)")
        tmp = _download(SFACE_URL, p.parent)
        try:
            got = _sha256(tmp)
            if got != SFACE_SHA256:
                raise RpcError("media", f"SFace model has sha256 {got}, expected {SFACE_SHA256}")
            os.replace(tmp, p)
        finally:
            if tmp.exists():
                tmp.unlink()
        return p
    except Exception as e:  # noqa: BLE001 - optional model: degrade, do not fail the analysis
        log.warning("SFace model unavailable, faces are not linked across cuts: %s", e)
        return None


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
    import numpy as np

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
        recognizer = None
        sface = ensure_sface(progress)
        if sface is not None:
            try:
                recognizer = cv2.FaceRecognizerSF.create(str(sface), "")
            except cv2.error as e:
                log.warning("cannot load SFace: %s", e)
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
                    emb = None
                    if recognizer is not None and float(r[2]) >= MIN_EMBED_FACE_PX:
                        try:
                            e = recognizer.feature(recognizer.alignCrop(f0, r)).flatten().astype(np.float32)
                            emb = e / max(1e-9, float(np.linalg.norm(e)))
                        except cv2.error:
                            emb = None
                    entry = {
                            "x": round(box[0], 5),
                            "y": round(box[1], 5),
                            "w": round(box[2], 5),
                            "h": round(box[3], 5),
                            "score": round(float(r[-1]), 4),
                            "motion": round(motion, 5),
                        }
                    if emb is not None:
                        entry["_emb"] = emb
                    fl.append(entry)
            fl.sort(key=lambda f: f["w"] * f["h"], reverse=True)
            samples.append({"t": round(t, 4), "faces": fl})
            if i % 4 == 0 or i == n - 1:
                progress("speakers.active", round((i + 1) / n * 100, 1))
    finally:
        cap.release()
    assign_tracks(samples)
    people = link_persons(samples)
    tracks = normalize_motion(samples)
    for s in samples:
        for f in s["faces"]:
            f["motion"] = round(f["motion"], 5)
            f.pop("_emb", None)
    out = {"width": width, "height": height, "fps": fps, "samples": samples, "tracks": tracks, "people": people}
    if recognizer is None:
        out["warnings"] = ["face recognition model unavailable: faces are not linked across cuts"]
    return out
