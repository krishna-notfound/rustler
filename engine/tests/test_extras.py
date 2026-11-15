"""Optional install extras (lite vs full): features whose extra is missing fail with a clear hint."""

import io
import json
import wave

from rustler_engine import rpc


def run_loop(lines, methods=None):
    inp = io.StringIO("".join(line + "\n" for line in lines))
    out = io.StringIO()
    rpc.serve(inp, out, methods if methods is not None else rpc.default_methods())
    msgs = [json.loads(x) for x in out.getvalue().splitlines()]
    return [m for m in msgs if m.get("method") != "progress"]


def req(id_, method, params=None):
    return json.dumps({"jsonrpc": "2.0", "id": id_, "method": method, "params": params or {}})


def _hide_extra(monkeypatch, extra):
    mods = dict(rpc.EXTRA_MODULES)
    mods[extra] = ("rustler_definitely_not_installed_mod",)
    monkeypatch.setattr(rpc, "EXTRA_MODULES", mods)


def test_require_extra_raises_dependency_missing_with_hint(monkeypatch):
    _hide_extra(monkeypatch, "ocr")
    try:
        rpc.require_extra("ocr", "OCR")
    except rpc.RpcError as e:
        assert e.kind == "dependency_missing" and e.extra == "ocr"
        assert "rustler setup run --with ocr" in e.hint
    else:
        raise AssertionError("expected RpcError")


def test_ocr_run_without_extra_is_dependency_missing(monkeypatch, tmp_path):
    _hide_extra(monkeypatch, "ocr")
    video = tmp_path / "v.mp4"
    video.write_bytes(b"not really a video")
    out = run_loop([req(1, "ocr.run", {"video_path": str(video)})])
    err = out[0]["error"]
    assert err["data"]["kind"] == "dependency_missing"
    assert err["data"]["extra"] == "ocr"
    assert "rustler setup run --with ocr" in err["data"]["hint"]


def test_explicit_yamnet_and_sherpa_without_extra_are_dependency_missing(monkeypatch, tmp_path):
    _hide_extra(monkeypatch, "audio-events")
    _hide_extra(monkeypatch, "speakers")
    wav = tmp_path / "a.wav"
    with wave.open(str(wav), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(b"\x00\x00" * 16000)
    out = run_loop(
        [
            req(1, "audio.events", {"audio_path": str(wav), "model": "yamnet"}),
            req(2, "speakers.diarize", {"audio_path": str(wav), "model": "sherpa"}),
            req(3, "audio.events", {"audio_path": str(wav)}),  # auto: falls back to the heuristic, with a hint
        ]
    )
    assert out[0]["error"]["data"]["kind"] == "dependency_missing"
    assert out[0]["error"]["data"]["extra"] == "audio-events"
    assert out[1]["error"]["data"]["kind"] == "dependency_missing"
    assert "rustler setup run --with speakers" in out[1]["error"]["data"]["hint"]
    assert out[2]["result"]["model"] == "heuristic"
    assert "rustler setup run --with audio-events" in " ".join(out[2]["result"]["warnings"])


def test_stray_import_error_gets_the_enable_hint():
    def boom(params, progress):
        raise ModuleNotFoundError("No module named 'sherpa_onnx'", name="sherpa_onnx")

    out = run_loop([req(1, "x.y")], {"x.y": boom})
    assert out[0]["error"]["data"] == {
        "kind": "dependency_missing",
        "extra": "speakers",
        "hint": rpc.enable_hint("speakers"),
    }


def test_capabilities_report_optional_features(monkeypatch):
    _hide_extra(monkeypatch, "speakers")
    out = run_loop([req(1, "engine.capabilities")])
    feats = out[0]["result"]["features"]
    assert set(feats) == {"ocr", "audio-events", "speakers"}
    assert feats["speakers"]["available"] is False
    assert feats["speakers"]["missing"] == ["rustler_definitely_not_installed_mod"]
