import shutil
import subprocess

import numpy as np
import pytest

from rustler_engine import rpc, scenes

cv2 = pytest.importorskip("cv2")

W, H, FPS = 64, 48, 30
COLORS = [(0, 0, 0), (255, 255, 255), (0, 0, 255)]  # BGR black, white, red (large HSV deltas)


def write_video(path, colors, seconds_each=1.5):
    fourcc = cv2.VideoWriter_fourcc(*"MJPG")
    vw = cv2.VideoWriter(str(path), fourcc, FPS, (W, H))
    if not vw.isOpened():
        pytest.skip("OpenCV cannot write MJPG video here")
    for c in colors:
        frame = np.zeros((H, W, 3), dtype=np.uint8)
        frame[:] = c
        for _ in range(int(FPS * seconds_each)):
            vw.write(frame)
    vw.release()


def test_detects_hard_cuts(tmp_path):
    video = tmp_path / "cuts.avi"
    write_video(video, COLORS)
    out = scenes.detect({"video_path": str(video), "threshold": 27.0}, lambda *a, **k: None)
    sc = out["scenes"]
    assert len(sc) == 3
    assert sc[0]["start"] == pytest.approx(0.0, abs=0.01)
    assert sc[0]["end"] == pytest.approx(1.5, abs=0.1)
    assert sc[1]["start"] == pytest.approx(sc[0]["end"], abs=0.001)
    assert sc[2]["end"] == pytest.approx(4.5, abs=0.1)
    assert all(s["end"] > s["start"] for s in sc)


def test_no_cuts_is_one_scene(tmp_path):
    video = tmp_path / "flat.avi"
    write_video(video, [COLORS[0]], seconds_each=2.0)
    out = scenes.detect({"video_path": str(video)}, lambda *a, **k: None)
    assert len(out["scenes"]) == 1
    assert out["scenes"][0]["start"] == 0.0
    assert out["scenes"][0]["end"] == pytest.approx(2.0, abs=0.1)


def test_missing_video_is_validation_error(tmp_path):
    with pytest.raises(rpc.RpcError) as e:
        scenes.detect({"video_path": str(tmp_path / "nope.mp4")}, lambda *a, **k: None)
    assert e.value.kind == "validation"


def test_garbage_file_is_media_error(tmp_path):
    bad = tmp_path / "bad.mp4"
    bad.write_bytes(b"not a video at all" * 100)
    with pytest.raises(rpc.RpcError) as e:
        scenes.detect({"video_path": str(bad)}, lambda *a, **k: None)
    assert e.value.kind == "media"


def test_bad_threshold_is_validation_error(tmp_path):
    with pytest.raises(rpc.RpcError) as e:
        scenes.detect({"video_path": str(tmp_path), "threshold": "high"}, lambda *a, **k: None)
    assert e.value.kind == "validation"


@pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not on PATH")
def test_detects_cuts_in_ffmpeg_mp4(tmp_path):
    video = tmp_path / "cuts.mp4"
    inputs = []
    for color in ("black", "white"):
        inputs += ["-f", "lavfi", "-i", f"color=c={color}:s=128x96:d=1.5:r=30"]
    subprocess.run(
        ["ffmpeg", "-y", "-v", "error", *inputs, "-filter_complex", "concat=n=2:v=1:a=0",
         "-pix_fmt", "yuv420p", str(video)],
        check=True,
    )
    out = scenes.detect({"video_path": str(video)}, lambda *a, **k: None)
    assert len(out["scenes"]) == 2
    assert out["scenes"][0]["end"] == pytest.approx(1.5, abs=0.1)
