"""ocr.run tests: text rendered with cv2.putText into a short MJPG video must be read back."""

import difflib
import re

import numpy as np
import pytest

from rustler_engine import ocr, rpc

cv2 = pytest.importorskip("cv2")
pytest.importorskip("rapidocr")

W, H, FPS = 960, 540, 10


def noop(*a, **k):
    pass


def fuzzy(a: str, b: str) -> float:
    n = lambda s: re.sub(r"[^a-z0-9]", "", s.lower())
    return difflib.SequenceMatcher(None, n(a), n(b)).ratio()


def frame_with(text: str, y: int, scale: float = 1.6):
    img = np.full((H, W, 3), 40, dtype=np.uint8)
    cv2.putText(img, text, (60, y), cv2.FONT_HERSHEY_SIMPLEX, scale, (255, 255, 255), 3, cv2.LINE_AA)
    return img


def write_video(path, parts):
    """parts = [(seconds, frame_image)]."""
    vw = cv2.VideoWriter(str(path), cv2.VideoWriter_fourcc(*"MJPG"), FPS, (W, H))
    if not vw.isOpened():
        pytest.skip("OpenCV cannot write MJPG video here")
    for secs, img in parts:
        for _ in range(int(secs * FPS)):
            vw.write(img)
    vw.release()


@pytest.fixture(scope="module")
def video(tmp_path_factory):
    path = tmp_path_factory.mktemp("ocr") / "text.avi"
    write_video(
        path,
        [
            (3.0, frame_with("BREAKING NEWS TODAY", 100)),
            (3.0, frame_with("Subscribe for more", 470)),
            (2.0, np.full((H, W, 3), 40, dtype=np.uint8)),
        ],
    )
    return path


def all_text(out):
    return [l["text"] for f in out["frames"] for l in f["lines"]]


def test_reads_known_text_and_dedupes(video):
    events = []
    out = ocr.run({"video_path": str(video), "every": 1.0}, lambda *a, **k: events.append(a))
    assert out["samples"] == 8
    assert (out["width"], out["height"]) == (W, H)
    texts = all_text(out)
    assert any(fuzzy(t, "BREAKING NEWS TODAY") > 0.85 for t in texts), texts
    assert any(fuzzy(t, "Subscribe for more") > 0.85 for t in texts), texts
    # identical consecutive text collapses into one entry per caption: 2 entries, not 6 frames
    assert len(out["frames"]) == 2, out["frames"]
    first, second = out["frames"]
    assert first["t"] == 0.0 and first["t_end"] >= 2.0
    assert second["t"] >= 3.0 and second["t_end"] >= 5.0
    # the blank tail has no text and adds nothing
    assert all(f["t"] < 6.0 for f in out["frames"])
    assert events and events[-1][0] == "ocr.run" and events[-1][1] == 100.0


def test_boxes_are_normalized_and_located(video):
    out = ocr.run({"video_path": str(video), "times": [1.0, 4.0]}, noop)
    top, bottom = out["frames"]
    t = top["lines"][0]
    b = bottom["lines"][0]
    for line in (t, b):
        bx = line["box"]
        assert 0 <= bx["x"] <= 1 and 0 <= bx["y"] <= 1
        assert bx["x"] + bx["w"] <= 1.0001 and bx["y"] + bx["h"] <= 1.0001
        assert 0.5 <= line["conf"] <= 1.0
    assert t["box"]["y"] < 0.3  # text drawn near the top
    assert b["box"]["y"] > 0.7  # text drawn near the bottom


def test_start_end_window(video):
    out = ocr.run({"video_path": str(video), "every": 1.0, "start": 3.0, "end": 6.0}, noop)
    assert out["samples"] == 3
    assert all(fuzzy(t, "Subscribe for more") > 0.85 for t in all_text(out))


def test_blank_video_has_no_text(tmp_path):
    path = tmp_path / "blank.avi"
    write_video(path, [(2.0, np.full((H, W, 3), 90, dtype=np.uint8))])
    out = ocr.run({"video_path": str(path), "every": 1.0}, noop)
    assert out["frames"] == [] and out["samples"] == 2


def test_validation_errors(tmp_path):
    with pytest.raises(rpc.RpcError) as e:
        ocr.run({}, noop)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        ocr.run({"video_path": str(tmp_path / "nope.mp4")}, noop)
    assert e.value.kind == "validation"
    f = tmp_path / "x.mp4"
    f.write_bytes(b"x")
    with pytest.raises(rpc.RpcError) as e:
        ocr.run({"video_path": str(f)}, noop)
    assert e.value.kind == "media"


def test_sample_time_rules():
    assert ocr.sample_times({"every": 2.0, "start": 1.0, "end": 8.0}, None)[0] == [1.0, 3.0, 5.0, 7.0]
    assert ocr.sample_times({"times": [3, 1, 1]}, 10.0)[0] == [1.0, 3.0]
    with pytest.raises(rpc.RpcError):
        ocr.sample_times({"every": 0}, 10.0)
    with pytest.raises(rpc.RpcError):
        ocr.sample_times({"every": 0.01}, 10000.0)  # too many samples
    with pytest.raises(rpc.RpcError):
        ocr.sample_times({"times": []}, 10.0)


def test_method_registered():
    assert "ocr.run" in rpc.default_methods()


def test_similar_caption_swap_in_the_same_place_is_not_mistaken_for_a_still_frame(tmp_path):
    path = tmp_path / "swap.avi"
    write_video(
        path,
        [
            (2.0, frame_with("not because it is easy", 470, 1.2)),
            (2.0, frame_with("but because it is hard", 470, 1.2)),
        ],
    )
    out = ocr.run({"video_path": str(path), "every": 1.0}, noop)
    texts = all_text(out)
    assert any(fuzzy(t, "not because it is easy") > 0.85 for t in texts), texts
    assert any(fuzzy(t, "but because it is hard") > 0.85 for t in texts), texts
    assert len(out["frames"]) == 2
