"""faces.active tests: mouth motion from landmarks, positional tracks, luma-normalised cut scan and
graphics statistics. The video tests need the YuNet model and a public-domain portrait (Abraham
Lincoln, Alexander Gardner, 1863; Wikimedia Commons, PD-old) and skip when offline."""

import os
import urllib.request
from pathlib import Path

import numpy as np
import pytest

from rustler_engine import active, faces, rpc

cv2 = pytest.importorskip("cv2")

PORTRAIT_URL = (
    "https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/"
    "Abraham_Lincoln_O-77_matte_collodion_print.jpg/960px-Abraham_Lincoln_O-77_matte_collodion_print.jpg"
)
W, H, FPS = 960, 540, 30


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
    home = Path(os.environ.get("RUSTLER_HOME", Path.home() / ".rustler"))
    local = home / "test-media" / "lincoln.jpg"
    path = local
    if not local.is_file():
        path = tmp_path_factory.mktemp("portrait") / "portrait.jpg"
        try:
            req = urllib.request.Request(PORTRAIT_URL, headers={"User-Agent": "rustler-tests/0.1"})
            with urllib.request.urlopen(req, timeout=30) as r:
                path.write_bytes(r.read())
        except Exception as e:
            pytest.skip(f"portrait download failed (offline?): {e}")
    img = cv2.imread(str(path))
    if img is None:
        pytest.skip("portrait could not be decoded")
    return img


def _detect(model_path, frame):
    det = cv2.FaceDetectorYN.create(model_path, "", (frame.shape[1], frame.shape[0]), 0.6, 0.3, 50)
    _, rows = det.detect(frame)
    return [] if rows is None else list(rows)


def two_face_base(img, model_path):
    """Lincoln left and mirrored right on a grey 960x540 frame, plus each face's mouth rectangle."""
    h = 480
    w = round(img.shape[1] * h / img.shape[0])
    small = cv2.resize(img, (w, h), interpolation=cv2.INTER_AREA)
    base = np.full((H, W, 3), 110, dtype=np.uint8)
    base[30 : 30 + h, 20 : 20 + w] = small
    base[30 : 30 + h, W - 20 - w : W - 20] = cv2.flip(small, 1)
    rows = sorted(_detect(model_path, base), key=lambda r: r[0])
    if len(rows) < 2:
        pytest.skip("YuNet did not find both portrait faces")
    mouths = []
    for r in rows[:2]:
        (rmx, rmy), (lmx, lmy) = (r[10], r[11]), (r[12], r[13])
        cx, cy, mw = (rmx + lmx) / 2, (rmy + lmy) / 2, abs(lmx - rmx)
        mouths.append((int(cx - 0.45 * mw), int(cy - 0.1 * mw), int(cx + 0.45 * mw), int(cy + 0.35 * mw)))
    return base, mouths


def talking(frame, rect, i):
    """An opening/closing mouth: a dark ellipse whose height follows a 5 Hz syllable rhythm."""
    x0, y0, x1, y1 = rect
    open_ = 0.5 + 0.5 * np.sin(2 * np.pi * 5.0 * i / FPS)
    cx, cy = (x0 + x1) // 2, (y0 + y1) // 2
    ax, ay = max(2, (x1 - x0) // 2), max(1, int((y1 - y0) / 2 * open_))
    cv2.ellipse(frame, (cx, cy), (ax, ay), 0, 0, 360, (25, 20, 20), -1)


def write(path, frames):
    vw = cv2.VideoWriter(str(path), cv2.VideoWriter_fourcc(*"MJPG"), FPS, (W, H))
    if not vw.isOpened():
        pytest.skip("OpenCV cannot write MJPG video here")
    for f in frames:
        vw.write(f)
    vw.release()


# --- pure helpers -------------------------------------------------------------------------------


def test_mouth_energy_ignores_head_motion_and_sees_mouth_motion():
    rng = np.random.default_rng(1)
    tex = rng.integers(60, 200, size=(200, 200)).astype(np.uint8)
    lm = [[0.35, 0.35], [0.65, 0.35], [0.5, 0.5], [0.4, 0.7], [0.6, 0.7]]
    rects = active.mouth_rects(lm, 200, 200)
    assert rects is not None
    # whole-face shift (head motion): both bands change alike -> ~0
    shifted = [tex, np.roll(tex, 2, axis=1), np.roll(tex, 4, axis=1)]
    assert active.mouth_energy(shifted, rects) < 0.05
    # only the mouth changes -> clearly positive
    (mx0, my0, mx1, my1), _ = rects
    a, b = tex.copy(), tex.copy()
    b[my0:my1, mx0:mx1] = 255 - b[my0:my1, mx0:mx1]
    assert active.mouth_energy([a, b, a], rects) > 0.5


def test_dark_cut_is_as_visible_as_a_bright_one():
    left = np.zeros((360, 640), np.uint8)
    left[:, :320] = 255
    right = np.zeros((360, 640), np.uint8)
    right[:, 320:] = 255
    bright = active.thumb_diff(active.thumb(left), active.thumb(right))
    dark = active.thumb_diff(active.thumb((left * 0.08).astype(np.uint8)), active.thumb((right * 0.08).astype(np.uint8)))
    same = active.thumb_diff(active.thumb(left), active.thumb(left))
    assert bright >= 0.45 and dark >= 0.45, (bright, dark)
    assert abs(bright - dark) < 0.15
    assert same < 0.01


def test_tracks_follow_position_and_split_at_jumps():
    samples = [
        {"t": 0.0, "faces": [{"x": 0.1, "y": 0.2, "w": 0.1, "h": 0.2, "mouth": 0.1}, {"x": 0.7, "y": 0.2, "w": 0.1, "h": 0.2, "mouth": 0.0}]},
        {"t": 0.2, "faces": [{"x": 0.72, "y": 0.2, "w": 0.1, "h": 0.2, "mouth": 0.0}, {"x": 0.12, "y": 0.2, "w": 0.1, "h": 0.2, "mouth": 0.2}]},
        {"t": 0.4, "faces": [{"x": 0.4, "y": 0.1, "w": 0.3, "h": 0.5, "mouth": 0.3}]},  # close-up: new track
    ]
    rows = active.assign_tracks(samples)
    left0 = samples[0]["faces"][0]["track"]
    right0 = samples[0]["faces"][1]["track"]
    assert left0 != right0
    by_x = {round(f["x"], 2): f["track"] for f in samples[1]["faces"]}
    assert by_x[0.12] == left0 and by_x[0.72] == right0
    assert samples[2]["faces"][0]["track"] not in (left0, right0)
    assert len(rows) == 3 and rows[0]["samples"] == 2


def test_validation_errors():
    with pytest.raises(rpc.RpcError) as e:
        active.active({"video_path": ""}, noop)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError):
        active.active({"video_path": __file__, "fps": 30}, noop)
    with pytest.raises(rpc.RpcError):
        active.active({"video_path": __file__, "scan": "yes"}, noop)


def test_method_is_registered():
    assert "faces.active" in rpc.default_methods()


# --- video tests ----------------------------------------------------------------------------------


def test_alternating_mouth_motion_marks_the_talking_face(tmp_path, model_path, portrait):
    base, mouths = two_face_base(portrait, model_path)
    frames = []
    for i in range(6 * FPS):
        f = base.copy()
        t = i / FPS
        talking(f, mouths[0] if (t < 3.0) else mouths[1], i)
        frames.append(f)
    path = tmp_path / "two.avi"
    write(path, frames)
    out = active.active({"video_path": str(path), "start": 0.0, "end": 6.0, "fps": 5.0}, noop)
    assert len(out["samples"]) == 30
    with_two = [s for s in out["samples"] if len(s["faces"]) == 2]
    assert len(with_two) >= 25
    for s in with_two:
        assert all("lm" in f and "mouth" in f and "track" in f for f in s["faces"])

    def mean_mouth(lo, hi, left):
        vals = []
        for s in with_two:
            if lo <= s["t"] < hi:
                f = sorted(s["faces"], key=lambda f: f["x"])[0 if left else 1]
                vals.append(f["mouth"])
        return sum(vals) / max(1, len(vals))

    assert mean_mouth(0.2, 2.8, True) > 3 * mean_mouth(0.2, 2.8, False) + 0.01
    assert mean_mouth(3.2, 5.8, False) > 3 * mean_mouth(3.2, 5.8, True) + 0.01
    # two stable positional tracks
    big = [t for t in out["tracks"] if t["samples"] >= 10]
    assert len(big) == 2


def test_scan_finds_the_cut_and_flags_the_graphics_shot(tmp_path, model_path, portrait):
    base, _ = two_face_base(portrait, model_path)
    card = np.zeros((H, W, 3), np.uint8)
    card[:] = (90, 30, 10)
    cv2.putText(card, "THREE RULES", (250, 250), cv2.FONT_HERSHEY_SIMPLEX, 2.4, (255, 255, 255), 6)
    cv2.putText(card, "for speaking", (300, 340), cv2.FONT_HERSHEY_SIMPLEX, 1.6, (255, 255, 255), 4)
    frames = [base.copy() for _ in range(2 * FPS)] + [card.copy() for _ in range(2 * FPS)]
    path = tmp_path / "cut.avi"
    write(path, frames)
    out = active.active({"video_path": str(path), "start": 0.0, "end": 4.0, "fps": 4.0, "motion": False, "scan": True}, noop)
    cuts = [p for p in out["peaks"] if p["diff"] >= 0.3]
    assert len(cuts) == 1, out["peaks"]
    assert abs(cuts[0]["t"] - 2.0) <= 1.5 / FPS, cuts
    face_g = [s["g"] for s in out["samples"] if "g" in s and s["t"] < 1.9]
    card_g = [s for s in out["samples"] if "g" in s and s["t"] > 2.1]
    assert card_g and all(not s["faces"] for s in card_g)
    assert all(s["g"]["flat"] > 0.6 and s["g"]["edges"] > 0.005 for s in card_g), card_g
    assert all(0.2 < s["g"]["x_lo"] and s["g"]["x_hi"] < 0.8 for s in card_g), card_g  # centred text
    assert face_g and all(g["flat"] < card_g[0]["g"]["flat"] for g in face_g)
    assert all("mouth" not in f for s in out["samples"] for f in s["faces"])


def test_low_frame_rate_video_keeps_one_sample_per_period(tmp_path, model_path, portrait):
    """At 10 fps the burst frames are also the next sample's frame; samples must not drift."""
    base, mouths = two_face_base(portrait, model_path)
    path = tmp_path / "low.avi"
    vw = cv2.VideoWriter(str(path), cv2.VideoWriter_fourcc(*"MJPG"), 10, (W, H))
    if not vw.isOpened():
        pytest.skip("OpenCV cannot write MJPG video here")
    for i in range(40):
        f = base.copy()
        if i % 2 == 0:
            talking(f, mouths[0], 7)  # the left mouth opens and closes every frame
        vw.write(f)
    vw.release()
    out = active.active({"video_path": str(path), "start": 0.0, "end": 4.0, "fps": 5.0}, noop)
    assert len(out["samples"]) == 20
    assert out["samples"][-1]["t"] == pytest.approx(3.8)
    two = [s for s in out["samples"] if len(s["faces"]) == 2]
    left = [min(s["faces"], key=lambda f: f["x"])["mouth"] for s in two]
    right = [max(s["faces"], key=lambda f: f["x"])["mouth"] for s in two]
    assert len(two) >= 15
    assert sum(left) > 3 * sum(right) + 0.05
