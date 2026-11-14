import json
import os
import re
import urllib.request
from pathlib import Path
from types import SimpleNamespace

import numpy as np
import pytest

from rustler_engine import rpc, transcribe

SR = 16000


class FakeModel:
    """Mimics faster_whisper.WhisperModel.transcribe: one word at 1.0-2.0s of every chunk."""

    def __init__(self, language="en", prob=0.9):
        self.calls = []
        self.language = language
        self.prob = prob

    def transcribe(self, audio, **kwargs):
        self.calls.append((len(audio), kwargs))
        n = len(self.calls)
        word = SimpleNamespace(word=f" w{n}", start=1.0, end=2.0, probability=0.75)
        blank = SimpleNamespace(word="  ", start=2.0, end=2.1, probability=0.1)
        seg = SimpleNamespace(words=[word, blank])
        nowords = SimpleNamespace(words=None)
        info = SimpleNamespace(language=self.language, language_probability=self.prob)
        return iter([seg, nowords]), info


def make_params(tmp_path, **over):
    audio = tmp_path / "a.wav"
    audio.write_bytes(b"x")
    p = {
        "audio_path": str(audio),
        "model": "tiny",
        "language": "auto",
        "device": "cpu",
        "parts_dir": str(tmp_path / "parts"),
        "chunk_seconds": 10,
        "max_seconds": None,
    }
    p.update(over)
    return p


def noprogress(*a, **k):
    pass


def fake_decode(seconds):
    def decode(path):
        return np.zeros(int(seconds * SR), dtype=np.float32)

    return decode


def test_chunking_offsets_and_parts(tmp_path):
    model = FakeModel()
    events = []
    out = transcribe.run(
        make_params(tmp_path),
        lambda stage, pct=None, message=None: events.append((stage, pct)),
        model_factory=lambda *a: model,
        decode=fake_decode(25),
    )
    assert out["complete"] is True
    assert out["language"] == "en"
    starts = [w["start"] for w in out["words"]]
    assert starts == [1.0, 11.0, 21.0]  # offsets applied
    assert [w["text"] for w in out["words"]] == ["w1", "w2", "w3"]  # stripped, blanks dropped
    assert all(w["end"] >= w["start"] and w["conf"] == 0.75 for w in out["words"])
    parts = sorted(p.name for p in (tmp_path / "parts").glob("*.json"))
    assert parts == ["0000.json", "0001.json", "0002.json"]
    assert [round(e[1]) for e in events] == [33, 67, 100]
    assert all(e[0] == "transcribe" for e in events)
    assert model.calls[0][1]["word_timestamps"] is True
    assert model.calls[0][1]["vad_filter"] is True
    assert model.calls[0][0] == 10 * SR and model.calls[2][0] == 5 * SR


def test_tiny_tail_merged_into_previous_chunk(tmp_path):
    model = FakeModel()
    out = transcribe.run(
        make_params(tmp_path),
        noprogress,
        model_factory=lambda *a: model,
        decode=fake_decode(20.5),
    )
    assert len(model.calls) == 2
    assert model.calls[1][0] == int(10.5 * SR)
    assert out["complete"]


def test_resume_reuses_parts_without_decoding_or_model(tmp_path):
    params = make_params(tmp_path)
    first = transcribe.run(params, noprogress, model_factory=lambda *a: FakeModel(), decode=fake_decode(25))
    part_files = sorted((tmp_path / "parts").glob("*.json"))
    mtimes = {p.name: p.stat().st_mtime_ns for p in part_files}

    def no_decode(path):
        raise AssertionError("decode must not be called when all parts exist")

    def no_model(*a):
        raise AssertionError("model must not be loaded when all parts exist")

    second = transcribe.run(params, noprogress, model_factory=no_model, decode=no_decode)
    assert second == first
    assert {p.name: p.stat().st_mtime_ns for p in part_files} == mtimes


def test_partial_resume_only_redoes_missing_chunk(tmp_path):
    params = make_params(tmp_path)
    transcribe.run(params, noprogress, model_factory=lambda *a: FakeModel(), decode=fake_decode(25))
    parts = tmp_path / "parts"
    keep = {p.name: p.stat().st_mtime_ns for p in parts.glob("*.json") if p.name != "0001.json"}
    (parts / "0001.json").unlink()
    model = FakeModel()
    out = transcribe.run(params, noprogress, model_factory=lambda *a: model, decode=fake_decode(25))
    assert len(model.calls) == 1
    assert len(out["words"]) == 3 and out["complete"]
    assert [w["start"] for w in out["words"]] == [1.0, 11.0, 21.0]
    assert {p.name: p.stat().st_mtime_ns for p in parts.glob("*.json") if p.name in keep} == keep


def test_changed_settings_invalidate_parts(tmp_path):
    transcribe.run(make_params(tmp_path), noprogress, model_factory=lambda *a: FakeModel(), decode=fake_decode(25))
    model = FakeModel()
    transcribe.run(
        make_params(tmp_path, chunk_seconds=20),
        noprogress,
        model_factory=lambda *a: model,
        decode=fake_decode(25),
    )
    assert len(model.calls) >= 1  # not silently reused


def test_corrupt_part_is_redone(tmp_path):
    params = make_params(tmp_path)
    transcribe.run(params, noprogress, model_factory=lambda *a: FakeModel(), decode=fake_decode(5))
    (tmp_path / "parts" / "0000.json").write_text("{trunc")
    model = FakeModel()
    out = transcribe.run(params, noprogress, model_factory=lambda *a: model, decode=fake_decode(5))
    assert len(model.calls) == 1 and out["complete"]


def test_max_seconds_stops_early_then_resumes(tmp_path):
    ticks = iter(range(0, 1000, 5))  # every clock() call advances 5s
    params = make_params(tmp_path, max_seconds=1)
    model = FakeModel()
    out = transcribe.run(
        params,
        noprogress,
        model_factory=lambda *a: model,
        decode=fake_decode(25),
        clock=lambda: float(next(ticks)),
    )
    assert out["complete"] is False
    assert len(model.calls) == 1  # always at least one chunk, then budget exhausted
    assert [w["start"] for w in out["words"]] == [1.0]

    params["max_seconds"] = None
    model2 = FakeModel()
    out2 = transcribe.run(params, noprogress, model_factory=lambda *a: model2, decode=fake_decode(25))
    assert out2["complete"] is True
    assert len(model2.calls) == 2
    assert [w["start"] for w in out2["words"]] == [1.0, 11.0, 21.0]


def test_language_auto_picks_most_confident_chunk(tmp_path):
    langs = iter([("hi", 0.3), ("en", 0.95), ("en", 0.9)])

    class Model(FakeModel):
        def transcribe(self, audio, **kw):
            segs, info = super().transcribe(audio, **kw)
            info.language, info.language_probability = next(langs)
            return segs, info

    out = transcribe.run(make_params(tmp_path), noprogress, model_factory=lambda *a: Model(), decode=fake_decode(25))
    assert out["language"] == "en"


def test_explicit_language_is_passed_and_returned(tmp_path):
    model = FakeModel(language="hi")
    out = transcribe.run(
        make_params(tmp_path, language="hi"),
        noprogress,
        model_factory=lambda *a: model,
        decode=fake_decode(5),
    )
    assert model.calls[0][1]["language"] == "hi"
    assert out["language"] == "hi"


def test_auto_device_resolution(monkeypatch):
    monkeypatch.setattr(transcribe, "cuda_available", lambda: True)
    assert transcribe.resolve_device("auto") == ("cuda", "float16")
    monkeypatch.setattr(transcribe, "cuda_available", lambda: False)
    assert transcribe.resolve_device("auto") == ("cpu", "int8")
    assert transcribe.resolve_device("cpu") == ("cpu", "int8")
    assert transcribe.resolve_device("cuda") == ("cuda", "float16")


def test_cuda_failure_in_auto_falls_back_to_cpu(tmp_path, monkeypatch):
    monkeypatch.setattr(transcribe, "cuda_available", lambda: True)
    made = []

    def factory(name, device, compute):
        made.append((device, compute))
        if device == "cuda":
            raise RuntimeError("Library cublas64_12.dll is not found")
        return FakeModel()

    out = transcribe.run(
        make_params(tmp_path, device="auto"), noprogress, model_factory=factory, decode=fake_decode(5)
    )
    assert made == [("cuda", "float16"), ("cpu", "int8")]
    assert out["complete"]


def test_validation_errors(tmp_path):
    with pytest.raises(rpc.RpcError) as e:
        transcribe.run(make_params(tmp_path, audio_path=str(tmp_path / "missing.wav")), noprogress)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        transcribe.run({"model": "tiny"}, noprogress)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        transcribe.run(make_params(tmp_path, chunk_seconds=0), noprogress)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        transcribe.run(make_params(tmp_path, device="tpu"), noprogress)
    assert e.value.kind == "validation"


def test_undecodable_audio_is_media_error(tmp_path):
    params = make_params(tmp_path)  # a.wav contains the byte "x"
    with pytest.raises(rpc.RpcError) as e:
        transcribe.run(params, noprogress, model_factory=lambda *a: FakeModel())
    assert e.value.kind == "media"


def test_empty_audio_gives_no_words(tmp_path):
    out = transcribe.run(
        make_params(tmp_path), noprogress, model_factory=lambda *a: FakeModel(), decode=fake_decode(0)
    )
    assert out == {"language": "und", "words": [], "complete": True}


JFK_URL = "https://github.com/ggml-org/whisper.cpp/raw/master/samples/jfk.wav"


@pytest.fixture(scope="session")
def jfk_wav(tmp_path_factory):
    cache = os.environ.get("RUSTLER_TEST_JFK")
    if cache and Path(cache).exists():
        return Path(cache)
    dest = tmp_path_factory.mktemp("jfk") / "jfk.wav"
    urllib.request.urlretrieve(JFK_URL, dest)
    return dest


@pytest.mark.slow
def test_jfk_words(jfk_wav, tmp_path):
    params = {
        "audio_path": str(jfk_wav),
        "model": "tiny",
        "language": "en",
        "device": "auto",
        "parts_dir": str(tmp_path / "parts"),
        "chunk_seconds": 600,
        "max_seconds": None,
    }
    events = []
    out = transcribe.run(params, lambda stage, pct=None, message=None: events.append((stage, pct)))
    assert out["complete"] is True
    assert out["language"] == "en"
    texts = [re.sub(r"\W", "", w["text"]).lower() for w in out["words"]]
    assert "americans" in texts
    assert out["words"][0]["start"] < 1.0
    assert all(w["end"] >= w["start"] for w in out["words"])
    assert all(set(w) == {"text", "start", "end", "conf"} for w in out["words"])
    assert events and events[-1] == ("transcribe", 100.0)

    part = tmp_path / "parts" / "0000.json"
    assert part.exists()
    mtime = part.stat().st_mtime_ns
    again = transcribe.run(params, lambda *a, **k: None)
    assert part.stat().st_mtime_ns == mtime  # reused, not re-decoded
    assert again == out
    json.dumps(out)  # JSON-serialisable


def test_explicit_cuda_with_missing_libs_is_dependency_missing(tmp_path):
    class Broken(FakeModel):
        def transcribe(self, audio, **kw):
            raise RuntimeError("Library cublas64_12.dll is not found or cannot be loaded")

    with pytest.raises(rpc.RpcError) as e:
        transcribe.run(
            make_params(tmp_path, device="cuda"),
            noprogress,
            model_factory=lambda *a: Broken(),
            decode=fake_decode(5),
        )
    assert e.value.kind == "dependency_missing"
