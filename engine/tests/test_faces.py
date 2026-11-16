"""faces.track tests. The detector test needs the YuNet model and a public-domain portrait
(Abraham Lincoln, Alexander Gardner, 1863; Wikimedia Commons, PD-old) and skips when offline."""

import urllib.request

import numpy as np
import pytest

from rustler_engine import faces, rpc

cv2 = pytest.importorskip("cv2")

PORTRAIT_URL = (
    "https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/"
    "Abraham_Lincoln_O-77_matte_collodion_print.jpg/960px-Abraham_Lincoln_O-77_matte_collodion_print.jpg"
)
CW, CH, VFPS = 640, 360, 10


def noop(*a, **k):
    pass


@pytest.fixture(scope="module")
def model_path():
    try:
        return str(faces.ensure_model(None, noop))
    except rpc.RpcError as e:
        pytest.skip(f"YuNet model unavailable (offline?): {e.message}")


@pytest.fixture(scope="module")
def portrait(tmp_path_factory):
    path = tmp_path_factory.mktemp("portrait") / "portrait.jpg"
    try:
        req = urllib.request.Request(PORTRAIT_URL, headers={"User-Agent": "rustler-tests/0.1 (support@digitalheroes.co.in)"})
        with urllib.request.urlopen(req, timeout=30) as r:
            path.write_bytes(r.read())
    except Exception as e:
        pytest.skip(f"portrait download failed (offline?): {e}")
    img = cv2.imread(str(path))
    if img is None:
        pytest.skip("portrait could not be decoded")
    return img


def write_moving_video(path, img, seconds=3.0):
    h = 320
    w = round(img.shape[1] * h / img.shape[0])
    small = cv2.resize(img, (w, h), interpolation=cv2.INTER_AREA)
    vw = cv2.VideoWriter(str(path), cv2.VideoWriter_fourcc(*"MJPG"), VFPS, (CW, CH))
    if not vw.isOpened():
        pytest.skip("OpenCV cannot write MJPG video here")
    n = int(seconds * VFPS)
    for i in range(n):
        frame = np.full((CH, CW, 3), 128, dtype=np.uint8)
        x = round(i / (n - 1) * (CW - w))
        frame[20 : 20 + h, x : x + w] = small
        vw.write(frame)
    vw.release()


def test_tracks_face_moving_right(tmp_path, portrait, model_path):
    video = tmp_path / "moving.avi"
    write_moving_video(video, portrait)
    events = []
    out = faces.track(
        {"video_path": str(video), "start": 0.0, "end": 3.0, "fps": 4.0, "model_path": model_path},
        lambda *a, **k: events.append(a),
    )
    assert (out["width"], out["height"]) == (CW, CH)
    samples = out["samples"]
    assert len(samples) == 12
    assert [s["t"] for s in samples][:3] == [0.0, 0.25, 0.5]
    assert events and events[-1][0] == "faces.track" and events[-1][1] == 100.0
    centres = []
    for s in samples:
        assert s["faces"], f"no face at t={s['t']}"
        f = s["faces"][0]
        assert 0 <= f["x"] <= 1 and 0 <= f["y"] <= 1 and f["x"] + f["w"] <= 1.0001
        assert f["score"] >= 0.6
        lm = f["lm"]
        assert len(lm) == 5 and all(len(p) == 2 and 0 <= p[0] <= 1 and 0 <= p[1] <= 1 for p in lm)
        # eyes above the nose, nose above the mouth corners, all near the box
        assert max(lm[0][1], lm[1][1]) < lm[2][1] < min(lm[3][1], lm[4][1])
        assert all(f["x"] - 0.05 <= p[0] <= f["x"] + f["w"] + 0.05 for p in lm)
        centres.append(f["x"] + f["w"] / 2)
    assert centres[-1] > centres[0] + 0.3
    assert all(b >= a - 0.01 for a, b in zip(centres, centres[1:])), centres


def test_landmarks_are_normalized_and_clamped():
    row = [10, 20, 40, 50, 20, 30, 40, 30, 30, 40, 22, 55, -5, 55, 0.9]
    lm = faces.landmarks(row, 100, 100)
    assert lm == [[0.2, 0.3], [0.4, 0.3], [0.3, 0.4], [0.22, 0.55], [0.0, 0.55]]


def test_blank_video_has_no_faces(tmp_path, model_path):
    video = tmp_path / "blank.avi"
    vw = cv2.VideoWriter(str(video), cv2.VideoWriter_fourcc(*"MJPG"), VFPS, (CW, CH))
    if not vw.isOpened():
        pytest.skip("OpenCV cannot write MJPG video here")
    for _ in range(20):
        vw.write(np.full((CH, CW, 3), 90, dtype=np.uint8))
    vw.release()
    out = faces.track({"video_path": str(video), "start": 0.5, "end": 1.5, "fps": 2.0, "model_path": model_path}, noop)
    assert [s["t"] for s in out["samples"]] == [0.5, 1.0]
    assert all(s["faces"] == [] for s in out["samples"])


def test_validation_errors(tmp_path):
    with pytest.raises(rpc.RpcError) as e:
        faces.track({}, noop)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        faces.track({"video_path": str(tmp_path / "nope.mp4"), "start": 0, "end": 1}, noop)
    assert e.value.kind == "validation"
    f = tmp_path / "x.mp4"
    f.write_bytes(b"x")
    with pytest.raises(rpc.RpcError) as e:
        faces.track({"video_path": str(f), "start": 2, "end": 1}, noop)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        faces.track({"video_path": str(f), "start": 0, "end": 1, "fps": 0}, noop)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        faces.track({"video_path": str(f), "start": 0, "end": 1, "model_path": str(tmp_path / "m.onnx")}, noop)
    assert e.value.kind == "validation"


def test_methods_registered():
    methods = rpc.default_methods()
    assert "faces.track" in methods and "models.fetch" in methods


def test_models_fetch_yunet(model_path, monkeypatch):
    out = rpc.default_methods()["models.fetch"]({"yunet": True}, noop)
    assert out["fetched"] == ["yunet"]
    assert out["yunet_path"].endswith(faces.YUNET_FILENAME)
