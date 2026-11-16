"""reference.measure on synthetic videos with known cut times, a caption band, hook text and a zoom.

Videos are written with OpenCV (no ffmpeg needed); tolerances are generous because the measures are
heuristics on a compressed proxy, but wrong in a way a human would notice fails the test.
"""

import numpy as np
import pytest

from rustler_engine import reference, rpc

cv2 = pytest.importorskip("cv2")

W, H, FPS = 540, 960, 15


def noop(*a, **k):
    pass


def textured(seed, w=W, h=H, lo=40, hi=200):
    """Low-frequency colourful texture with some corners (blobs + rectangles)."""
    rng = np.random.RandomState(seed)
    base = rng.randint(lo, hi, (h // 8, w // 8, 3)).astype(np.uint8)
    img = cv2.resize(base, (w, h), interpolation=cv2.INTER_CUBIC)
    for _ in range(60):
        c = tuple(int(x) for x in rng.randint(0, 255, 3))
        x, y = int(rng.randint(0, w)), int(rng.randint(0, h))
        if rng.rand() < 0.5:
            cv2.rectangle(img, (x, y), (x + int(rng.randint(10, 60)), y + int(rng.randint(10, 60))), c, -1)
        else:
            cv2.circle(img, (x, y), int(rng.randint(6, 30)), c, -1)
    return img


def put_caption(img, text, y_frac, color=(0, 255, 255), scale=1.9):
    (tw, th), _ = cv2.getTextSize(text, cv2.FONT_HERSHEY_DUPLEX, scale, 8)
    x = (img.shape[1] - tw) // 2
    y = int(img.shape[0] * y_frac) + th // 2
    cv2.putText(img, text, (x, y), cv2.FONT_HERSHEY_DUPLEX, scale, (0, 0, 0), 14, cv2.LINE_AA)
    cv2.putText(img, text, (x, y), cv2.FONT_HERSHEY_DUPLEX, scale, color, 5, cv2.LINE_AA)


def write_video(path, frames):
    vw = cv2.VideoWriter(str(path), cv2.VideoWriter_fourcc(*"mp4v"), FPS, (W, H))
    assert vw.isOpened()
    for f in frames:
        vw.write(f)
    vw.release()


def measure(path, **extra):
    return reference.measure({"video_path": str(path), "faces": False, **extra}, noop)


@pytest.fixture(scope="module")
def captioned(tmp_path_factory):
    """12 s, hard cuts at 3/6/9 s, a caption (changing every 0.6 s) centred at y=0.75, hook text for 2.5 s."""
    path = tmp_path_factory.mktemp("ref") / "captioned.mp4"
    shots = [textured(s) for s in (1, 2, 3, 4)]
    words = ["ALPHA BETA", "GAMMA DELTA", "EPSILON ZETA", "ETA THETA", "IOTA KAPPA", "LAMBDA MU", "NU XI", "OMICRON PI"]
    frames = []
    for i in range(12 * FPS):
        t = i / FPS
        img = shots[int(t // 3)].copy()
        # slow drift so shots are not frozen
        img = np.roll(img, int(t * 4), axis=1)
        put_caption(img, words[int(t / 0.6) % len(words)], 0.75)
        if t < 2.5:
            put_caption(img, "WAIT FOR IT", 0.12, color=(255, 255, 255), scale=1.4)
        frames.append(img)
    write_video(path, frames)
    return path


def test_cuts_are_found_at_known_times(captioned):
    r = measure(captioned)
    times = r["cuts"]["times"]
    assert len(times) == 3, times
    for got, want in zip(times, (3.0, 6.0, 9.0)):
        assert abs(got - want) <= 0.25, (got, want)
    assert abs(r["cuts"]["per_minute"] - 15.0) <= 1.0
    assert r["cuts"]["shot_length"]["count"] == 4
    assert abs(r["cuts"]["shot_length"]["median"] - 3.0) <= 0.3


def test_caption_band_position_words_and_colour(captioned):
    r = measure(captioned)
    cap = r["text"]["caption"]
    assert cap["present"], r["text"]
    assert 0.72 <= cap["y_center"] <= 0.80, cap
    assert 0.02 <= cap["height"] <= 0.10, cap
    # two words per caption line
    assert 1.0 <= cap["words_per_line"]["median"] <= 3.0, cap["words_per_line"]
    # yellow fill (BGR 0,255,255 -> #ffff00)
    top = cap["fill_colors"][0]["hex"]
    rr, gg, bb = int(top[1:3], 16), int(top[3:5], 16), int(top[5:7], 16)
    assert rr > 200 and gg > 200 and bb < 90, cap["fill_colors"]


def test_hook_text_in_first_three_seconds(captioned):
    r = measure(captioned)
    hook = r["text"]["hook"]
    assert hook["present"], r["text"]
    assert hook["y0"] < 0.2 and hook["y1"] < 0.3, hook
    assert 1.5 <= hook["visible_to"] <= 3.5, hook
    assert r["text"]["any_text_first_3s"]


def test_zoom_in_is_detected_and_a_still_video_has_none(tmp_path):
    big = textured(7, 1080, 1920)
    frames = []
    for i in range(5 * FPS):
        z = 1.0 + 0.5 * i / (5 * FPS)  # 1.0 -> 1.5 over 5 s
        cw, ch = int(1080 / z), int(1920 / z)
        x0, y0 = (1080 - cw) // 2, (1920 - ch) // 2
        frames.append(cv2.resize(big[y0 : y0 + ch, x0 : x0 + cw], (W, H), interpolation=cv2.INTER_AREA))
    p = tmp_path / "zoom.mp4"
    write_video(p, frames)
    r = measure(p)
    ev = [e for e in r["zoom"]["events"] if e["kind"] == "zoom_in"]
    assert ev, r["zoom"]
    assert r["zoom"]["coverage"] > 0.5
    assert max(e["factor"] for e in ev) > 1.04
    assert r["cuts"]["count"] == 0

    still = [cv2.resize(big[0:1920, 0:1080], (W, H), interpolation=cv2.INTER_AREA)] * (4 * FPS)
    p2 = tmp_path / "still.mp4"
    write_video(p2, still)
    assert measure(p2)["zoom"]["events"] == []


def test_cut_in_dark_low_contrast_footage(tmp_path):
    frames = []
    a, b = textured(11, lo=5, hi=22), textured(12, lo=5, hi=22)
    for i in range(8 * FPS):
        f = (a if i < 4 * FPS else b).copy()
        f = np.roll(f, i % 6, axis=0)
        frames.append(f)
    p = tmp_path / "dark.mp4"
    write_video(p, frames)
    r = measure(p)
    assert len(r["cuts"]["times"]) == 1 and abs(r["cuts"]["times"][0] - 4.0) <= 0.25, r["cuts"]


def test_cut_between_flat_colours_and_fade_is_not_a_cut(tmp_path):
    frames = []
    for i in range(6 * FPS):
        frames.append(np.full((H, W, 3), (200, 60, 20) if i < 3 * FPS else (30, 160, 40), np.uint8))
    p = tmp_path / "flat.mp4"
    write_video(p, frames)
    assert len(measure(p)["cuts"]["times"]) == 1

    # a slow brightness fade on one textured shot: zero cuts
    base = textured(3)
    fade = [cv2.convertScaleAbs(base, alpha=1.0 - 0.8 * i / (6 * FPS)) for i in range(6 * FPS)]
    p2 = tmp_path / "fade.mp4"
    write_video(p2, fade)
    assert measure(p2)["cuts"]["times"] == []


def test_no_caption_in_a_plain_video(tmp_path):
    p = tmp_path / "plain.mp4"
    write_video(p, [textured(5)] * (5 * FPS))
    r = measure(p)
    assert r["text"]["caption"]["present"] is False
    assert r["text"]["hook"]["present"] is False


def test_music_heuristic_separates_speech_from_speech_over_music(tmp_path):
    import wave

    sr = 16000
    rng = np.random.RandomState(0)
    t = np.arange(sr * 12) / sr
    # "speech": irregular bursts with real pauses over a quiet room-noise floor
    env = np.zeros(len(t))
    pos = 0
    while pos < len(t):
        on, off = int(rng.uniform(0.08, 0.45) * sr), int(rng.uniform(0.05, 0.6) * sr)
        env[pos : pos + on] = 1.0
        pos += on + off
    speech = (rng.randn(len(t)) * 0.08 + 0.05 * np.sin(2 * np.pi * 180 * t)) * env + rng.randn(len(t)) * 0.003
    # "music": steady chord with a 2 Hz pulse underneath
    music = 0.06 * (np.sin(2 * np.pi * 220 * t) + np.sin(2 * np.pi * 277 * t) + np.sin(2 * np.pi * 330 * t)) * (0.55 + 0.45 * (np.sin(2 * np.pi * 2.0 * t) > 0))

    def save(name, x):
        p = tmp_path / name
        with wave.open(str(p), "wb") as wf:
            wf.setnchannels(1)
            wf.setsampwidth(2)
            wf.setframerate(sr)
            wf.writeframes((np.clip(x, -1, 1) * 32767).astype(np.int16).tobytes())
        return str(p)

    only = reference.music_heuristic(save("speech.wav", speech))
    mixed = reference.music_heuristic(save("mixed.wav", speech + music))
    assert only["available"] and mixed["available"]
    assert mixed["likelihood"] > only["likelihood"] + 0.25, (only, mixed)
    assert mixed["music_present"] and not only["music_present"], (only, mixed)


def test_bad_params():
    with pytest.raises(rpc.RpcError) as e:
        reference.measure({}, noop)
    assert e.value.kind == "validation"
    with pytest.raises(rpc.RpcError) as e:
        reference.measure({"video_path": "does-not-exist.mp4"}, noop)
    assert e.value.kind == "validation"


def test_method_is_registered():
    assert "reference.measure" in rpc.default_methods()
