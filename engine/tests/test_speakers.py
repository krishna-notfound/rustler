"""speakers.diarize / speakers.active tests.

* pure helpers and the numpy heuristic diarizer on synthetic two-"voice" audio always run;
* the sherpa-onnx path runs on two public-domain recordings (JFK sample from the test media, and
  FDR's Pearl Harbor address, Wikimedia Commons, US federal work) and skips when the models or the
  recording cannot be downloaded (offline);
* ``speakers.active`` runs on a synthetic two-face video built from a public-domain portrait
  (Abraham Lincoln, Alexander Gardner, 1863) whose "mouth" flickers for the speaking face only.
"""

import urllib.request
import wave
from pathlib import Path

import numpy as np
import pytest

from rustler_engine import rpc, speakers

SR = 16000
PORTRAIT_URL = (
    "https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/"
    "Abraham_Lincoln_O-77_matte_collodion_print.jpg/960px-Abraham_Lincoln_O-77_matte_collodion_print.jpg"
)
FDR_URL = "https://commons.wikimedia.org/wiki/Special:FilePath/1941_Roosevelt_speech_pearlharbor_p1.ogg"
UA = {"User-Agent": "rustler-tests/0.1"}


def noop(*a, **k):
    pass


def write_wav(path, x):
    pcm = (np.clip(x, -1, 1) * 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def read_wav(path):
    with wave.open(str(path), "rb") as w:
        return np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float32) / 32768.0


def pseudo_voice(seconds, f0, formants, seed):
    """Syllable-modulated glottal pulse train through resonators: two calls with different
    pitch/formants sound like two different speakers to an MFCC-statistics clusterer."""
    rng = np.random.default_rng(seed)
    n = int(seconds * SR)
    t = np.arange(n) / SR
    pulse = np.zeros(n)
    pulse[:: int(SR / f0)] = 1.0
    x = pulse + 0.02 * rng.standard_normal(n)
    for fc, bw in formants:  # two-pole resonator
        r = np.exp(-np.pi * bw / SR)
        a1, a2 = -2 * r * np.cos(2 * np.pi * fc / SR), r * r
        y = np.zeros(n)
        for i in range(2, n):
            y[i] = x[i] - a1 * y[i - 1] - a2 * y[i - 2]
        x = y
    env = 0.5 * (1 + np.sin(2 * np.pi * 3.5 * t + rng.uniform(0, 6)))
    x = x * (0.15 + 0.85 * env)
    return (x / np.abs(x).max() * 0.5).astype(np.float32)


# --- pure helpers ---------------------------------------------------------------------------------


def test_relabel_orders_by_first_appearance():
    out = speakers.relabel([(5.0, 6.0, 7), (0.0, 1.0, 3), (2.0, 3.0, 7), (3.0, 3.0, 9)])
    assert out == [
        {"start": 0.0, "end": 1.0, "speaker": "S1"},
        {"start": 2.0, "end": 3.0, "speaker": "S2"},
        {"start": 5.0, "end": 6.0, "speaker": "S2"},
    ]


def test_mouth_region_is_lower_face_and_inside_frame():
    x0, y0, x1, y1 = speakers.mouth_region((0.1, 0.2, 0.2, 0.4), 640, 360)
    assert 0 <= x0 < x1 <= 640 and 0 <= y0 < y1 <= 360
    assert y0 >= (0.2 + 0.4 * 0.5) * 360  # starts below the middle of the face
    assert y1 == round(0.6 * 360)
    # a box touching the frame edge is clamped, never empty
    x0, y0, x1, y1 = speakers.mouth_region((0.99, 0.99, 0.2, 0.2), 100, 100)
    assert x1 > x0 and y1 > y0


def face(x, motion, w=0.2, h=0.3):
    return {"x": x, "y": 0.2, "w": w, "h": h, "score": 0.9, "motion": motion}


def test_tracks_are_numbered_left_to_right_and_normalized_per_face():
    # the right face enters first, the left one later; ids must still be left=0, right=1
    samples = []
    for i in range(20):
        faces = [face(0.65, 0.05 if i % 2 else 0.01)]
        if i >= 4:
            faces.append(face(0.10, 0.002))
        samples.append({"t": i * 0.2, "faces": faces})
    speakers.assign_tracks(samples)
    ids = {round(f["x"], 2): f["id"] for s in samples for f in s["faces"]}
    assert ids == {0.1: 0, 0.65: 1}
    tracks = speakers.normalize_motion(samples)
    by_id = {t["id"]: t for t in tracks}
    assert by_id[0]["static"] is True and by_id[1]["static"] is False
    assert all(f["speak"] == 0.0 for s in samples for f in s["faces"] if f["id"] == 0)
    hi = [f["speak"] for s in samples for f in s["faces"] if f["id"] == 1 and f["motion"] == 0.05]
    lo = [f["speak"] for s in samples for f in s["faces"] if f["id"] == 1 and f["motion"] == 0.01]
    assert min(hi) > 0.9 and max(lo) < 0.1


def test_validation_errors(tmp_path):
    for params in (
        {},
        {"audio_path": str(tmp_path / "missing.wav")},
        {"audio_path": "x.wav", "num_speakers": 0},
        {"audio_path": "x.wav", "num_speakers": "2"},
        {"audio_path": "x.wav", "min_speakers": 4, "max_speakers": 2},
        {"audio_path": "x.wav", "model": "magic"},
    ):
        with pytest.raises(rpc.RpcError) as e:
            speakers.diarize(params, noop)
        assert e.value.kind == "validation", params
    for params in ({}, {"video_path": str(tmp_path / "no.mp4")}, {"video_path": "x", "fps": 0}):
        with pytest.raises(rpc.RpcError) as e:
            speakers.active(params, noop)
        assert e.value.kind == "validation", params
    f = tmp_path / "v.mp4"
    f.write_bytes(b"x")
    with pytest.raises(rpc.RpcError) as e:
        speakers.active({"video_path": str(f), "start": 3, "end": 1}, noop)
    assert e.value.kind == "validation"


def test_methods_registered():
    m = rpc.default_methods()
    assert "speakers.diarize" in m and "speakers.active" in m


# --- heuristic diarizer ---------------------------------------------------------------------------


def two_voice_audio(tmp_path):
    a = pseudo_voice(5.0, 110.0, [(700, 80), (1200, 100), (2600, 150)], 1)
    b = pseudo_voice(5.0, 230.0, [(350, 60), (2300, 120), (3400, 200)], 2)
    gap = np.zeros(int(0.8 * SR), dtype=np.float32)
    x = np.concatenate([a, gap, b, gap, a[: 3 * SR]])
    p = tmp_path / "two_voices.wav"
    write_wav(p, x)
    return p, [(0.0, 5.0, "S1"), (5.8, 10.8, "S2"), (11.6, 14.6, "S1")]


def speaker_at(segments, t):
    for s in segments:
        if s["start"] <= t < s["end"]:
            return s["speaker"]
    return None


def dominant(segments, lo, hi):
    """Speaker with the most speech time inside [lo, hi]."""
    tot = {}
    for s in segments:
        ov = min(s["end"], hi) - max(s["start"], lo)
        if ov > 0:
            tot[s["speaker"]] = tot.get(s["speaker"], 0.0) + ov
    return max(tot, key=tot.get) if tot else None


def test_heuristic_separates_two_synthetic_voices(tmp_path):
    path, truth = two_voice_audio(tmp_path)
    out = speakers.diarize({"audio_path": str(path), "model": "heuristic", "num_speakers": 2}, noop)
    assert out["model"] == "heuristic"
    assert out["warnings"], "the heuristic result must say it is a heuristic"
    assert out["num_speakers"] == 2
    segs = out["segments"]
    assert all(s["end"] > s["start"] for s in segs)
    # the voice at the middle of each true region must match the region's identity
    first, second, third = [dominant(segs, a, b) for a, b, _ in truth]
    assert first and second and third
    assert first == third and first != second


def test_heuristic_silence_has_no_segments(tmp_path):
    p = tmp_path / "silence.wav"
    write_wav(p, np.zeros(SR * 3, dtype=np.float32))
    out = speakers.diarize({"audio_path": str(p), "model": "heuristic"}, noop)
    assert out["segments"] == [] and out["num_speakers"] == 0


# --- sherpa-onnx diarizer (real models) -------------------------------------------------------------


@pytest.fixture(scope="module")
def sherpa_models():
    pytest.importorskip("sherpa_onnx")
    try:
        return speakers.ensure_models(noop)
    except rpc.RpcError as e:
        pytest.skip(f"diarization models unavailable (offline?): {e.message}")


@pytest.fixture(scope="module")
def two_real_voices(tmp_path_factory):
    """JFK (test media) + FDR (Wikimedia): jfk, FDR, first 5 s of jfk. Returns (wav, boundaries)."""
    jfk = Path.home() / ".rustler" / "test-media" / "jfk.wav"
    if not jfk.is_file():
        pytest.skip("jfk.wav missing")
    d = tmp_path_factory.mktemp("real")
    ogg = d / "fdr.ogg"
    try:
        req = urllib.request.Request(FDR_URL, headers=UA)
        with urllib.request.urlopen(req, timeout=60) as r:
            ogg.write_bytes(r.read())
    except Exception as e:
        pytest.skip(f"FDR recording download failed (offline?): {e}")
    import shutil
    import subprocess

    ffmpeg = Path.home() / ".rustler" / "bin" / "ffmpeg.exe"
    exe = str(ffmpeg) if ffmpeg.is_file() else shutil.which("ffmpeg")
    if not exe:
        pytest.skip("ffmpeg not available")
    fdr = d / "fdr.wav"
    j16 = d / "jfk16.wav"
    for args in (
        ["-ss", "5", "-t", "12", "-i", str(ogg), "-ac", "1", "-ar", "16000", str(fdr)],
        ["-i", str(jfk), "-ac", "1", "-ar", "16000", str(j16)],
    ):
        subprocess.run([exe, "-hide_banner", "-loglevel", "error", "-y", *args], check=True)
    a, b = read_wav(j16), read_wav(fdr)
    gap = np.zeros(SR // 2, dtype=np.float32)
    x = np.concatenate([a, gap, b, gap, a[: 5 * SR]])
    out = d / "two_real.wav"
    write_wav(out, x)
    t1 = len(a) / SR
    t2 = t1 + 0.5 + len(b) / SR
    return out, (t1, t2 + 0.5)


@pytest.mark.parametrize("n", [None, 2])
def test_sherpa_separates_two_real_speakers(sherpa_models, two_real_voices, n):
    wav, (t1, t2) = two_real_voices
    params = {"audio_path": str(wav)}
    if n:
        params["num_speakers"] = n
    out = speakers.diarize(params, noop)
    assert out["model"] == speakers.MODEL_NAME
    assert out["num_speakers"] == 2, out["segments"]
    segs = out["segments"]
    assert [s for s in segs if s["end"] > s["start"]] == segs
    first = dominant(segs, 0.0, t1)
    mid = dominant(segs, t1, t2)
    last = dominant(segs, t2, t2 + 5.0)
    assert first == "S1"  # relabelled in order of first appearance
    assert mid and mid != first
    assert last == first


def test_sherpa_single_speaker_and_min_max(sherpa_models):
    jfk = Path.home() / ".rustler" / "test-media" / "jfk.wav"
    if not jfk.is_file():
        pytest.skip("jfk.wav missing")
    out = speakers.diarize({"audio_path": str(jfk)}, noop)
    assert out["num_speakers"] == 1
    forced = speakers.diarize({"audio_path": str(jfk), "min_speakers": 2}, noop)
    assert forced["num_speakers"] == 2  # min_speakers forces a re-cluster to at least 2


def test_sherpa_models_are_verified_and_cached(sherpa_models):
    seg, emb = sherpa_models
    assert speakers._sha256(seg) == speakers.SEG_MODEL_SHA256
    assert speakers._sha256(emb) == speakers.EMB_SHA256
    assert (seg.parent / "pyannote-segmentation-3-0.LICENSE").read_text().startswith("MIT License")
    again = speakers.ensure_models(noop)
    assert again == sherpa_models


def test_auto_falls_back_to_heuristic_when_models_unavailable(tmp_path, monkeypatch):
    path, _ = two_voice_audio(tmp_path)

    def boom(progress=None):
        raise rpc.RpcError("media", "offline")

    monkeypatch.setattr(speakers, "ensure_models", boom)
    out = speakers.diarize({"audio_path": str(path), "num_speakers": 2}, noop)
    assert out["model"] == "heuristic"
    assert any("fell back" in w for w in out["warnings"])
    with pytest.raises(rpc.RpcError):  # an explicit sherpa request must not silently degrade
        speakers.diarize({"audio_path": str(path), "model": "sherpa"}, noop)


# --- speakers.active ----------------------------------------------------------------------------------

cv2 = pytest.importorskip("cv2")
CW, CH, VFPS = 640, 360, 10


@pytest.fixture(scope="module")
def portrait(tmp_path_factory):
    local = Path.home() / ".rustler" / "test-media" / "lincoln.jpg"
    if local.is_file():
        img = cv2.imread(str(local))
        if img is not None:
            return img
    path = tmp_path_factory.mktemp("portrait") / "portrait.jpg"
    try:
        req = urllib.request.Request(PORTRAIT_URL, headers=UA)
        with urllib.request.urlopen(req, timeout=30) as r:
            path.write_bytes(r.read())
    except Exception as e:
        pytest.skip(f"portrait download failed (offline?): {e}")
    img = cv2.imread(str(path))
    if img is None:
        pytest.skip("portrait could not be decoded")
    return img


@pytest.fixture(scope="module")
def yunet():
    from rustler_engine import faces

    try:
        return faces.ensure_model(None, noop)
    except rpc.RpcError as e:
        pytest.skip(f"YuNet model unavailable (offline?): {e.message}")


def write_two_face_video(path, img, windows, seconds):
    """Two portraits (left and the mirrored one on the right). ``windows`` = [(t0, t1, 'L'|'R')]:
    during a window the named face's mouth area flickers."""
    h = 320
    w = round(img.shape[1] * h / img.shape[0])
    left = cv2.resize(img, (w, h), interpolation=cv2.INTER_AREA)
    right = left[:, ::-1].copy()
    vw = cv2.VideoWriter(str(path), cv2.VideoWriter_fourcc(*"MJPG"), VFPS, (CW, CH))
    if not vw.isOpened():
        pytest.skip("OpenCV cannot write MJPG video here")
    xl, xr, y = 10, CW - 10 - w, 20
    for i in range(int(seconds * VFPS)):
        t = i / VFPS
        frame = np.full((CH, CW, 3), 128, dtype=np.uint8)
        frame[y : y + h, xl : xl + w] = left
        frame[y : y + h, xr : xr + w] = right
        for t0, t1, side in windows:
            if t0 <= t < t1:
                x0 = (xl if side == "L" else xr) + int(0.33 * w)
                x1 = x0 + int(0.30 * w)
                open_h = 6 + 22 * (i % 2)  # mouth opens and closes every frame
                yy = y + int(0.52 * h)
                cv2.ellipse(frame, ((x0 + x1) // 2, yy + open_h // 2), ((x1 - x0) // 2, open_h // 2), 0, 0, 360, (20, 20, 20), -1)
        vw.write(frame)
    vw.release()


def test_active_mouth_motion_follows_the_speaking_face(tmp_path, portrait, yunet):
    video = tmp_path / "two_faces.avi"
    windows = [(0.0, 3.0, "L"), (3.0, 6.0, "R"), (6.0, 9.0, "L")]
    write_two_face_video(video, portrait, windows, 9.0)
    events = []
    out = speakers.active({"video_path": str(video), "start": 0.0, "end": 9.0, "fps": 5}, lambda *a, **k: events.append(a))
    assert (out["width"], out["height"]) == (CW, CH)
    assert len(out["samples"]) == 45
    assert events and events[-1][0] == "speakers.active" and events[-1][1] == 100.0
    assert [t["id"] for t in out["tracks"]] == [0, 1]
    assert out["tracks"][0]["mean_x"] < 0.5 < out["tracks"][1]["mean_x"]

    def mean_speak(face_id, t0, t1):
        v = [f["speak"] for s in out["samples"] if t0 <= s["t"] < t1 for f in s["faces"] if f["id"] == face_id]
        assert v, f"face {face_id} not seen in {t0}-{t1}"
        return sum(v) / len(v)

    for t0, t1, side in windows:
        on, off = (0, 1) if side == "L" else (1, 0)
        assert mean_speak(on, t0 + 0.4, t1 - 0.2) > 0.6, (t0, t1)
        assert mean_speak(off, t0 + 0.4, t1 - 0.2) < 0.25, (t0, t1)
    for s in out["samples"]:
        for f in s["faces"]:
            assert 0.0 <= f["speak"] <= 1.0 and f["motion"] >= 0.0 and f["score"] >= 0.6
