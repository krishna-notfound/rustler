import io
import json
import subprocess
import sys
from pathlib import Path

from rustler_engine import rpc

ENGINE_DIR = Path(__file__).resolve().parent.parent


def run_loop(lines, methods=None):
    inp = io.StringIO("".join(line + "\n" for line in lines))
    out = io.StringIO()
    rpc.serve(inp, out, methods if methods is not None else rpc.default_methods())
    return [json.loads(x) for x in out.getvalue().splitlines()]


def req(id_, method, params=None):
    return json.dumps({"jsonrpc": "2.0", "id": id_, "method": method, "params": params or {}})


def test_ping_and_unknown_method():
    out = run_loop([req(1, "engine.ping"), req(2, "nope.nothing")])
    assert out[0]["id"] == 1 and out[0]["jsonrpc"] == "2.0"
    assert out[0]["result"]["version"]
    assert out[0]["result"]["python"].startswith("3.")
    assert out[1]["id"] == 2
    assert out[1]["error"]["code"] == -32601


def test_parse_error_does_not_kill_loop():
    out = run_loop(["{not json", req(5, "engine.ping")])
    assert out[0]["error"]["code"] == -32700 and out[0]["id"] is None
    assert out[1]["id"] == 5 and "result" in out[1]


def test_invalid_request():
    out = run_loop([json.dumps({"jsonrpc": "2.0", "id": 3}), json.dumps([1, 2])])
    assert out[0]["error"]["code"] == -32600 and out[0]["id"] == 3
    assert out[1]["error"]["code"] == -32600


def test_blank_lines_ignored():
    out = run_loop(["", req(1, "engine.ping"), "   "])
    assert len(out) == 1


def test_error_kinds_and_progress_before_result():
    def boom_missing(params, progress):
        raise ModuleNotFoundError("faster_whisper")

    def boom_validation(params, progress):
        raise rpc.RpcError("validation", "bad param")

    def boom_other(params, progress):
        raise RuntimeError("kaboom")

    def with_progress(params, progress):
        progress("stage1", 50.0)
        progress("stage1", 100.0, message="done")
        return {"ok": True}

    methods = {
        "t.missing": boom_missing,
        "t.validation": boom_validation,
        "t.other": boom_other,
        "t.progress": with_progress,
    }
    out = run_loop(
        [req(1, "t.missing"), req(2, "t.validation"), req(3, "t.other"), req(4, "t.progress")],
        methods,
    )
    assert out[0]["error"]["code"] == -32000
    assert out[0]["error"]["data"] == {"kind": "dependency_missing"}
    assert out[1]["error"]["data"]["kind"] == "validation" and out[1]["error"]["message"] == "bad param"
    assert out[2]["error"]["data"]["kind"] == "internal" and "kaboom" in out[2]["error"]["message"]
    assert out[3]["method"] == "progress"
    assert out[3]["params"] == {"id": 4, "stage": "stage1", "pct": 50.0}
    assert out[4]["params"]["message"] == "done"
    assert out[5] == {"jsonrpc": "2.0", "id": 4, "result": {"ok": True}}
    assert "id" not in out[3]


def test_missing_params_defaults_to_empty():
    out = run_loop([json.dumps({"jsonrpc": "2.0", "id": 1, "method": "engine.ping"})])
    assert "result" in out[0]


def test_capabilities_shape():
    out = run_loop([req(1, "engine.capabilities")])
    res = out[0]["result"]
    assert isinstance(res["cuda"], bool)
    assert "faster_whisper" in res["packages"]
    assert "numpy" in res["packages"]


def test_translit_via_rpc():
    out = run_loop([req(1, "translit.to_latin", {"texts": ["bro क्या scene है", "hello"]})])
    assert out[0]["result"] == {"texts": ["bro kya scene hai", "hello"]}


def test_translit_validation_error():
    out = run_loop([req(1, "translit.to_latin", {"texts": "oops"})])
    assert out[0]["error"]["data"]["kind"] == "validation"


def test_transcribe_missing_file_is_validation():
    out = run_loop(
        [req(1, "transcribe.run", {"audio_path": "/no/such.wav", "model": "tiny", "parts_dir": "/tmp/x"})]
    )
    assert out[0]["error"]["data"]["kind"] == "validation"


def test_scenes_missing_file_is_validation():
    out = run_loop([req(1, "scenes.detect", {"video_path": "/no/such.mp4"})])
    assert out[0]["error"]["data"]["kind"] == "validation"


def test_stdout_is_protocol_only_even_if_handlers_print():
    """Library prints / native fd-1 writes must not reach the protocol stream."""
    script = (
        "import os, sys\n"
        "from rustler_engine import __main__ as m\n"
        "def noisy(params, progress):\n"
        "    print('LIBRARY NOISE')\n"
        "    os.write(1, b'NATIVE NOISE\\n')\n"
        "    sys.stdout.write('more noise\\n')\n"
        "    return {'ok': True}\n"
        "m.main(extra_methods={'t.noisy': noisy})\n"
    )
    stdin = req(1, "t.noisy") + "\n" + req(2, "engine.ping") + "\n"
    p = subprocess.run(
        [sys.executable, "-c", script],
        input=stdin.encode(),
        capture_output=True,
        cwd=ENGINE_DIR,
        timeout=120,
    )
    assert p.returncode == 0, p.stderr.decode(errors="replace")
    parsed = [json.loads(x) for x in p.stdout.decode().splitlines()]  # every line must be JSON
    assert [x["id"] for x in parsed] == [1, 2]
    err = p.stderr.decode()
    assert "LIBRARY NOISE" in err and "NATIVE NOISE" in err


def test_module_entrypoint_roundtrip():
    stdin = req(1, "engine.ping") + "\n" + req(2, "bogus") + "\n"
    p = subprocess.run(
        [sys.executable, "-m", "rustler_engine"],
        input=stdin.encode(),
        capture_output=True,
        cwd=ENGINE_DIR,
        timeout=120,
    )
    assert p.returncode == 0
    parsed = [json.loads(x) for x in p.stdout.decode().splitlines()]
    assert parsed[0]["result"]["version"]
    assert parsed[1]["error"]["code"] == -32601
