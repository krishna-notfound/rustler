"""audio.events tests on synthetic audio (always run: heuristic) and on YAMNet (skipped offline)."""

import shutil
import subprocess
import urllib.request
import wave
from pathlib import Path

import numpy as np
import pytest

from rustler_engine import audio_events as ae
from rustler_engine import rpc

SR = 16000
APPLAUSE_URL = "https://upload.wikimedia.org/wikipedia/commons/0/09/Applause_ii.ogg"  # public domain


def noop(*a, **k):
    pass


def write_wav(path, x):
    pcm = (np.clip(x, -1, 1) * 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def synthetic():
    """3 s silence, 3 s 440 Hz tone, 4 s dense noise bursts (clap-like), 2 s steady quiet hiss."""
    rng = np.random.default_rng(7)
    sil = np.zeros(3 * SR, dtype=np.float32)
    tone = (0.3 * np.sin(2 * np.pi * 440 * np.arange(3 * SR) / SR)).astype(np.float32)
    bursts = np.zeros(4 * SR, dtype=np.float32)
    for _ in range(4 * 7 * 8):
        p = int(rng.integers(0, 4 * SR - 2000))
        n = int(SR * rng.uniform(0.008, 0.03))
        burst = rng.standard_normal(n) * np.exp(-np.arange(n) / (n / 4))
        bursts[p : p + n] += (burst * rng.uniform(0.1, 0.4)).astype(np.float32)
    bursts = bursts / np.abs(bursts).max() * 0.5
    return np.concatenate([sil, tone, bursts])


def label_at(events, t):
    for e in events:
        if e["start"] <= t < e["end"]:
            return e["label"]
    return None


def check_events(events, duration):
    prev_end = 0.0
    for e in events:
        assert e["label"] in ae.LABELS
        assert 0.0 <= e["score"] <= 1.0
        assert e["start"] >= prev_end - 1e-6 and e["end"] > e["start"]
        assert e["end"] <= duration + 1e-6
        prev_end = e["end"]


def test_heuristic_on_synthetic_audio(tmp_path):
    p = tmp_path / "syn.wav"
    x = synthetic()
    write_wav(p, x)
    out = ae.run({"audio_path": str(p), "model": "heuristic", "hop": 0.5}, noop)
    assert out["model"] == "heuristic"
    assert out["duration"] == pytest.approx(10.0, abs=0.01)
    check_events(out["events"], 10.0)
    assert label_at(out["events"], 1.0) == "silence"
    assert label_at(out["events"], 4.0) == "music"  # steady tone
    assert label_at(out["events"], 8.0) == "applause"  # dense clap-like noise bursts
    # adjacent hops with the same label are merged into one event
    silence = [e for e in out["events"] if e["label"] == "silence"]
    assert len(silence) == 1 and silence[0]["start"] == 0.0 and silence[0]["end"] == pytest.approx(3.0, abs=0.01)


def test_decide_rules():
    # order: laughter, applause, cheer, music, speech, silence
    assert ae.decide([0.6, 0.1, 0.0, 0.2, 0.7, 0.0], -30.0)[0] == "laughter"  # reaction beats speech
    assert ae.decide([0.0, 0.0, 0.0, 0.1, 0.8, 0.0], -30.0)[0] == "speech"
    assert ae.decide([0.0, 0.0, 0.0, 0.8, 0.4, 0.0], -30.0)[0] == "music"
    assert ae.decide([0.0, 0.0, 0.0, 0.0, 0.0, 0.0], -30.0)[0] is None
    assert ae.decide([0.0, 0.9, 0.0, 0.0, 0.0, 0.0], -70.0) == ("silence", 1.0)  # digital quiet wins


def test_merge_bridges_one_hop_gaps():
    labels = [("speech", 0.8), ("speech", 0.8), (None, 0.0), ("speech", 0.6), ("music", 0.9)]
    ev = ae.merge_bins(labels, 0.5, 2.5)
    assert [(e["label"], e["start"], e["end"]) for e in ev] == [("speech", 0.0, 2.0), ("music", 2.0, 2.5)]


def test_class_groups_use_names():
    names = ["Speech", "Laughter", "Giggle", "Applause", "Clapping", "Cheering", "Crowd", "Music", "Silence", "Dog", "Hubbub, speech noise, speech babble"]
    g = ae.group_indices(names)
    assert [i for i, _ in g["laughter"]] == [1, 2]
    assert [i for i, _ in g["applause"]] == [3, 4]
    assert [i for i, _ in g["cheer"]] == [5, 6]
    assert [i for i, _ in g["speech"]] == [0]
    assert [i for i, _ in g["music"]] == [7]


def test_validation_errors(tmp_path):
    with pytest.raises(rpc.RpcError) as e:
        ae.run({}, noop)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        ae.run({"audio_path": str(tmp_path / "nope.wav")}, noop)
    assert e.value.kind == "validation"
    bad = tmp_path / "bad.wav"
    bad.write_bytes(b"not a wav")
    with pytest.raises(rpc.RpcError) as e:
        ae.run({"audio_path": str(bad)}, noop)
    assert e.value.kind == "media"
    good = tmp_path / "g.wav"
    write_wav(good, np.zeros(SR, dtype=np.float32))
    with pytest.raises(rpc.RpcError) as e:
        ae.run({"audio_path": str(good), "hop": 0.01}, noop)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        ae.run({"audio_path": str(good), "model": "bogus"}, noop)
    assert e.value.kind == "validation"


def test_resamples_other_rates(tmp_path):
    p = tmp_path / "r8k.wav"
    t = np.arange(8000 * 2) / 8000
    pcm = (0.3 * np.sin(2 * np.pi * 440 * t) * 32767).astype("<i2")
    with wave.open(str(p), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(8000)
        w.writeframes(pcm.tobytes())
    x = ae.load_wav(str(p))
    assert len(x) == 2 * SR


def test_method_registered():
    m = rpc.default_methods()
    assert "audio.events" in m and "ocr.run" in m


# --- YAMNet (needs the model download; skipped offline) --------------------------------------


@pytest.fixture(scope="module")
def yamnet_ready():
    pytest.importorskip("onnxruntime")
    try:
        ae.ensure_model(noop)
    except rpc.RpcError as e:
        pytest.skip(f"YAMNet model unavailable (offline?): {e.message}")


def test_yamnet_on_synthetic_silence_and_tone(tmp_path, yamnet_ready):
    p = tmp_path / "syn.wav"
    write_wav(p, synthetic())
    out = ae.run({"audio_path": str(p), "model": "yamnet"}, noop)
    assert out["model"] == "yamnet"
    check_events(out["events"], 10.0)
    assert label_at(out["events"], 1.0) == "silence"
    assert label_at(out["events"], 4.0) not in ("speech", "laughter", "applause")


def test_yamnet_on_real_applause(tmp_path, yamnet_ready):
    ffmpeg = shutil.which("ffmpeg") or str(ae.rustler_home() / "bin" / "ffmpeg.exe")
    if not Path(ffmpeg).is_file() and not shutil.which("ffmpeg"):
        pytest.skip("ffmpeg not available to decode the applause sample")
    ogg = tmp_path / "applause.ogg"
    try:
        req = urllib.request.Request(APPLAUSE_URL, headers={"User-Agent": "rustler-tests/0.1 (support@digitalheroes.co.in)"})
        with urllib.request.urlopen(req, timeout=30) as r:
            ogg.write_bytes(r.read())
    except Exception as e:
        pytest.skip(f"applause sample download failed (offline?): {e}")
    wav = tmp_path / "applause.wav"
    subprocess.run([ffmpeg, "-y", "-loglevel", "error", "-i", str(ogg), "-ac", "1", "-ar", "16000", str(wav)], check=True)
    out = ae.run({"audio_path": str(wav)}, noop)
    assert out["model"] == "yamnet"
    applause = [e for e in out["events"] if e["label"] == "applause"]
    assert applause, out["events"]
    assert sum(e["end"] - e["start"] for e in applause) >= 4.0
    assert max(e["score"] for e in applause) > 0.5


def test_models_fetch_yamnet(yamnet_ready):
    out = rpc.default_methods()["models.fetch"]({"yamnet": True}, noop)
    assert out["fetched"] == ["yamnet"] and out["yamnet_path"].endswith("yamnet.onnx")
