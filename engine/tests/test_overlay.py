"""overlay.find_box tests: synthetic scenes (no network) and the stream fixture (Lincoln portrait,
public domain, overlaid on testsrc2 at a known rectangle; needs ffmpeg, YuNet and the portrait)."""

import shutil
import subprocess
import urllib.request
from pathlib import Path

import numpy as np
import pytest

from rustler_engine import faces, overlay, rpc

cv2 = pytest.importorskip("cv2")

TOL = 0.015  # snapped edges must be within 1.5 % of the frame of the true overlay edges
PORTRAIT_URL = (
    "https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/"
    "Abraham_Lincoln_O-77_matte_collodion_print.jpg/960px-Abraham_Lincoln_O-77_matte_collodion_print.jpg"
)


def noop(*a, **k):
    pass


def assert_close(rect, truth, W, H, tol=TOL):
    got = (rect["x"], rect["y"], rect["x"] + rect["w"], rect["y"] + rect["h"])
    want = (truth[0] / W, truth[1] / H, truth[2] / W, truth[3] / H)
    for name, g, w in zip(("left", "top", "right", "bottom"), got, want):
        assert abs(g - w) <= tol, f"{name}: got {g:.4f} want {w:.4f} (rect {rect})"


# --- synthetic scenes ------------------------------------------------------------------------


def scene(frame_idx, W=1280, H=720, box=(936, 24, 1256, 264), border=0, flush=False):
    """Random coloured blocks as gameplay (a few static, the rest moving) and a textured 'webcam'."""
    rng = np.random.default_rng(7)
    img = np.zeros((H, W, 3), np.uint8)
    img[:] = (60, 90, 50)
    for _ in range(40):  # static blocks
        x, y = int(rng.integers(0, W - 80)), int(rng.integers(0, H - 80))
        w, h = int(rng.integers(30, 200)), int(rng.integers(30, 160))
        cv2.rectangle(img, (x, y), (x + w, y + h), tuple(int(v) for v in rng.integers(0, 255, 3)), -1)
    mrng = np.random.default_rng(100 + frame_idx)
    for _ in range(25):  # blocks that differ in every frame
        x, y = int(mrng.integers(0, W - 80)), int(mrng.integers(0, H - 80))
        w, h = int(mrng.integers(20, 160)), int(mrng.integers(20, 120))
        cv2.rectangle(img, (x, y), (x + w, y + h), tuple(int(v) for v in mrng.integers(0, 255, 3)), -1)
    x0, y0, x1, y1 = box
    cam = np.full((y1 - y0, x1 - x0, 3), (170, 160, 150), np.uint8)
    ch, cw = cam.shape[:2]
    cv2.ellipse(cam, (cw // 2 + (frame_idx % 3) * 3, ch // 2), (cw // 7, ch // 4), 0, 0, 360, (90, 120, 200), -1)  # "face"
    cv2.putText(cam, "cam", (8 + frame_idx * 4, ch - 12), cv2.FONT_HERSHEY_SIMPLEX, 0.8, (30, 30, 30), 2)
    img[y0:y1, x0:x1] = cam
    if border:
        cv2.rectangle(img, (x0, y0), (x1 - 1, y1 - 1), (255, 255, 255), border * 2 - 1)
    return img


def face_of(box, W=1280, H=720):
    x0, y0, x1, y1 = box
    cw, ch = x1 - x0, y1 - y0
    fx0, fy0, fw, fh = x0 + cw / 2 - cw / 7, y0 + ch / 4, 2 * cw / 7, ch / 2
    return {"x": fx0 / W, "y": fy0 / H, "w": fw / W, "h": fh / H}


def test_snaps_a_synthetic_overlay_across_frames():
    box = (936, 24, 1256, 264)
    frames = [scene(i, box=box) for i in range(5)]
    r = overlay.find_rect(frames, face_of(box))
    assert r["rect"], r
    assert_close(r["rect"], box, 1280, 720, 0.006)
    assert r["confidence"] >= 0.8


def test_single_image_and_white_border():
    box = (900, 40, 1240, 300)
    r = overlay.find_rect([scene(0, box=box, border=4)], face_of(box))
    assert r["rect"], r
    assert_close(r["rect"], box, 1280, 720, 0.01)
    assert r["confidence"] >= 0.6


def test_overlay_flush_with_the_frame_corner_uses_frame_edges():
    box = (960, 0, 1280, 240)
    frames = [scene(i, box=box) for i in range(4)]
    r = overlay.find_rect(frames, face_of(box))
    assert r["rect"], r
    assert_close(r["rect"], box, 1280, 720, 0.006)
    assert r["sides"]["top"]["frame_edge"] and r["sides"]["right"]["frame_edge"]
    assert not r["sides"]["left"]["frame_edge"] and not r["sides"]["bottom"]["frame_edge"]


def test_no_overlay_gives_no_confident_rectangle():
    # the "face" is just a patch of plain gameplay: nothing to snap to
    plain = np.full((720, 1280, 3), 80, np.uint8)
    r = overlay.find_rect([plain, plain.copy()], {"x": 0.7, "y": 0.1, "w": 0.08, "h": 0.15})
    assert r["rect"] is None and r["confidence"] == 0.0


def test_find_box_rpc_shape_and_validation(tmp_path):
    img = scene(0)
    p = tmp_path / "f.png"
    cv2.imwrite(str(p), img)
    out = overlay.find_box({"image_path": str(p), "near": face_of((936, 24, 1256, 264))}, noop)
    assert out["source"] == "edges" and out["rect"] and out["frames"] == 1
    assert (out["width"], out["height"]) == (1280, 720)
    for bad in (
        {"image_path": str(p)},
        {"image_path": str(p), "near": {"x": 0.5, "y": 0.5, "w": 0, "h": 0.1}},
        {"image_path": str(tmp_path / "missing.png"), "near": {"x": 0.1, "y": 0.1, "w": 0.1, "h": 0.1}},
        {"near": {"x": 0.1, "y": 0.1, "w": 0.1, "h": 0.1}},
        {"video_path": str(p), "times": [], "near": {"x": 0.1, "y": 0.1, "w": 0.1, "h": 0.1}},
    ):
        with pytest.raises(rpc.RpcError) as e:
            overlay.find_box(bad, noop)
        assert e.value.kind == "validation", bad
    assert "overlay.find_box" in rpc.default_methods()


# --- the stream fixture: portrait over testsrc2 ------------------------------------------------


def _ffmpeg():
    home = faces.rustler_home()
    for cand in (home / "bin" / "ffmpeg.exe", home / "bin" / "ffmpeg"):
        if cand.is_file():
            return str(cand)
    return shutil.which("ffmpeg")


@pytest.fixture(scope="module")
def portrait(tmp_path_factory):
    cache = faces.rustler_home() / "test-media" / "lincoln.jpg"
    if cache.is_file() and cache.stat().st_size > 10_000:
        return cache
    path = tmp_path_factory.mktemp("portrait") / "lincoln.jpg"
    try:
        req = urllib.request.Request(PORTRAIT_URL, headers={"User-Agent": "rustler-tests/0.1 (support@digitalheroes.co.in)"})
        with urllib.request.urlopen(req, timeout=30) as r:
            path.write_bytes(r.read())
    except Exception as e:
        pytest.skip(f"portrait download failed (offline?): {e}")
    return path


@pytest.fixture(scope="module")
def yunet():
    try:
        return str(faces.ensure_model(None, noop))
    except rpc.RpcError as e:
        pytest.skip(f"YuNet model unavailable (offline?): {e.message}")


def make_stream(tmp_path, portrait, border):
    ff = _ffmpeg()
    if not ff:
        pytest.skip("ffmpeg not available")
    img = cv2.imread(str(portrait))
    pw = 320
    ph = round(img.shape[0] * pw / img.shape[1])
    bw = pw + 2 * border
    bh = ph + 2 * border
    chain = f"[1:v]scale={pw}:-2" + (f",pad={bw}:{bh}:{border}:{border}:white" if border else "") + "[p];[0:v][p]overlay=W-w-24:24"
    out = tmp_path / f"stream_b{border}.mp4"
    r = subprocess.run(
        [ff, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=6", "-i", str(portrait),
         "-filter_complex", chain, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "veryfast", str(out)],
        capture_output=True,
    )
    if r.returncode != 0:
        pytest.skip(f"fixture video failed: {r.stderr[-200:]!r}")
    # true outer overlay rectangle in pixels (overlay=W-w-24:24 -> right edge at W-24)
    x1 = 1280 - 24
    return out, (x1 - bw, 24, x1, 24 + bh)


@pytest.mark.parametrize("border", [0, 4])
def test_snaps_the_lincoln_overlay_on_testsrc2(tmp_path, portrait, yunet, border):
    video, truth = make_stream(tmp_path, portrait, border)
    times = [0.4, 1.4, 2.4, 3.4, 4.4, 5.4]
    tr = faces.track({"video_path": str(video), "start": 0.0, "end": 6.0, "fps": 1.0, "model_path": yunet}, noop)
    found = [s["faces"][0] for s in tr["samples"] if s["faces"]]
    assert found, "YuNet found no face in the fixture"
    near = {k: float(np.median([f[k] for f in found])) for k in ("x", "y", "w", "h")}
    out = overlay.find_box({"video_path": str(video), "times": times, "near": near}, noop)
    print("TRUTH", truth, "GOT", out)
    assert out["source"] == "edges" and out["confidence"] >= 0.6, out
    assert_close(out["rect"], truth, 1280, 720)


# --- the real stream demo (optional) ------------------------------------------------------------


def test_stream_demo_if_present(yunet):
    demo = faces.rustler_home() / "test-media" / "stream_demo.mp4"
    if not demo.is_file():
        pytest.skip("stream_demo.mp4 not present")
    tr = faces.track({"video_path": str(demo), "start": 3.0, "end": 9.0, "fps": 1.0, "model_path": yunet}, noop)
    found = [s["faces"][0] for s in tr["samples"] if s["faces"]]
    if not found:
        pytest.skip("no face in the demo")
    near = {k: float(np.median([f[k] for f in found])) for k in ("x", "y", "w", "h")}
    out = overlay.find_box({"video_path": str(demo), "times": [3.3, 10, 16.7, 23.3, 30, 36.7], "near": near}, noop)
    assert out["source"] == "edges", out
    # overlay: 400 px wide at x 1490..1890, y 30..~550 of 1920x1080
    assert_close(out["rect"], (1490, 30, 1890, 550), 1920, 1080)
