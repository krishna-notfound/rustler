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


# --- Devanagari (lang param) ------------------------------------------------------------------

HINDI = "नमस्ते दोस्तों"


def _ffmpeg():
    import shutil as _sh

    home = ocr.rustler_home()
    for cand in (home / "bin" / "ffmpeg.exe", home / "bin" / "ffmpeg"):
        if cand.is_file():
            return str(cand)
    return _sh.which("ffmpeg")


def _deva_font():
    d = ocr.rustler_home() / "fonts" / "NotoSansDevanagari"
    fonts = sorted(d.glob("*.ttf")) if d.is_dir() else []
    return fonts[0] if fonts else None


def deva_frame(tmp_path, text=HINDI, size=96):
    """A 960x540 frame with `text` rendered by ffmpeg drawtext (proper conjunct shaping)."""
    import subprocess
    import shutil as _sh

    ff, font = _ffmpeg(), _deva_font()
    if not ff or not font:
        pytest.skip("ffmpeg or the Noto Sans Devanagari font is not installed")
    local = tmp_path / "noto.ttf"
    _sh.copy(font, local)
    png = tmp_path / "deva.png"
    vf = f"drawtext=fontfile=noto.ttf:text='{text}':fontsize={size}:fontcolor=white:x=(w-text_w)/2:y=(h-text_h)/2"
    r = subprocess.run(
        [ff, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", f"color=c=0x203040:s={W}x{H}:d=1", "-vf", vf, "-frames:v", "1", "deva.png"],
        cwd=str(tmp_path),
        capture_output=True,
    )
    if r.returncode != 0 or not png.is_file():
        pytest.skip("ffmpeg drawtext failed here")
    img = cv2.imread(str(png))
    assert img is not None
    return img


def deva_ratio(got: str, want: str = HINDI) -> float:
    n = lambda s: re.sub(r"\s+", "", s)
    return difflib.SequenceMatcher(None, n(got), n(want)).ratio()


@pytest.fixture(scope="module")
def deva_model():
    try:
        return ocr.ensure_devanagari(noop)
    except rpc.RpcError as e:
        pytest.skip(f"Devanagari OCR model unavailable (offline?): {e.message}")


def test_devanagari_model_is_hash_verified(deva_model):
    assert ocr._sha256(deva_model) == ocr.DEVANAGARI_SHA256


def test_latin_default_cannot_read_devanagari_but_devanagari_lang_can(tmp_path, deva_model):
    vid = tmp_path / "deva.avi"
    write_video(vid, [(2.0, deva_frame(tmp_path))])
    latin = ocr.run({"video_path": str(vid), "every": 1.0}, noop)
    assert latin["lang"] == "latin"
    assert deva_ratio(" ".join(all_text(latin))) < 0.6, all_text(latin)
    out = ocr.run({"video_path": str(vid), "every": 1.0, "lang": "devanagari"}, noop)
    assert out["lang"] == "devanagari"
    text = " ".join(all_text(out))
    assert deva_ratio(text) >= 0.85, text
    assert all(l["lang"] == "devanagari" for f in out["frames"] for l in f["lines"])
    assert len(out["frames"]) == 1 and out["frames"][0]["t_end"] >= 1.0


def test_auto_picks_devanagari_on_hindi_and_keeps_latin_on_english(tmp_path, deva_model):
    hindi = ocr.recognize(deva_frame(tmp_path), "auto")
    text = " ".join(l["text"] for l in hindi)
    assert deva_ratio(text) >= 0.85, text
    assert any(l["lang"] == "devanagari" for l in hindi)
    eng = ocr.recognize(frame_with("BREAKING NEWS TODAY", 100), "auto")
    assert any(fuzzy(l["text"], "BREAKING NEWS TODAY") > 0.85 for l in eng), eng
    assert all(l["lang"] == "latin" for l in eng)


def test_auto_merges_a_mixed_frame(tmp_path, deva_model):
    img = deva_frame(tmp_path)
    cv2.putText(img, "Subscribe for more", (60, 500), cv2.FONT_HERSHEY_SIMPLEX, 1.6, (255, 255, 255), 3, cv2.LINE_AA)
    lines = ocr.recognize(img, "auto")
    langs = {l["lang"] for l in lines}
    assert deva_ratio(" ".join(l["text"] for l in lines if l["lang"] == "devanagari")) >= 0.85, lines
    assert any(l["lang"] == "latin" and fuzzy(l["text"], "Subscribe for more") > 0.8 for l in lines), (langs, lines)


def test_merge_readings_rules():
    box = {"x": 0.1, "y": 0.1, "w": 0.3, "h": 0.1}
    far = {"x": 0.1, "y": 0.7, "w": 0.3, "h": 0.1}
    latin = [{"text": "naaaa", "box": box, "conf": 0.55}]
    deva = [
        {"text": "नमस्ते", "box": box, "conf": 0.95},
        {"text": "hello", "box": far, "conf": 0.9},
    ]
    out = ocr.merge_readings(latin, deva)
    assert [(l["text"], l["lang"]) for l in out] == [("नमस्ते", "devanagari"), ("hello", "devanagari")]
    # a Devanagari-model reading of Latin letters never beats the Latin model on the same line
    out = ocr.merge_readings([{"text": "HELLO", "box": box, "conf": 0.7}], [{"text": "HELL0", "box": box, "conf": 0.99}])
    assert [(l["text"], l["lang"]) for l in out] == [("HELLO", "latin")]


def test_lang_validation(tmp_path):
    f = tmp_path / "x.avi"
    write_video(f, [(1.0, np.full((H, W, 3), 40, dtype=np.uint8))])
    with pytest.raises(rpc.RpcError) as e:
        ocr.run({"video_path": str(f), "lang": "klingon"}, noop)
    assert e.value.kind == "validation"


def test_models_fetch_accepts_ocr_devanagari(deva_model):
    out = rpc.default_methods()["models.fetch"]({"ocr_devanagari": True}, noop)
    assert out["fetched"] == ["ocr_devanagari"]
    assert out["ocr_devanagari_path"].endswith(ocr.DEVANAGARI_FILE)
