"""Scene detection via PySceneDetect (ContentDetector)."""

from __future__ import annotations

import logging
import os
from typing import Any, Callable, Dict

from .rpc import RpcError

log = logging.getLogger("rustler_engine.scenes")


def detect(params: Dict[str, Any], progress: Callable[..., None]) -> Dict[str, Any]:
    video_path = params.get("video_path")
    if not isinstance(video_path, str) or not video_path:
        raise RpcError("validation", "video_path is required")
    threshold = params.get("threshold", 27.0)
    if isinstance(threshold, bool) or not isinstance(threshold, (int, float)) or threshold <= 0:
        raise RpcError("validation", "threshold must be a positive number")
    if not os.path.isfile(video_path):
        raise RpcError("validation", f"video not found: {video_path}")

    from scenedetect import ContentDetector, detect as sd_detect

    try:
        scene_list = sd_detect(
            video_path,
            ContentDetector(threshold=float(threshold)),
            show_progress=False,
            start_in_scene=True,  # a video without cuts is one scene, not zero
        )
    except Exception as e:
        raise RpcError("media", f"scene detection failed for {video_path}: {type(e).__name__}: {e}") from e

    scenes = [
        {"start": round(s.seconds, 3), "end": round(e.seconds, 3)}
        for s, e in scene_list
        if e.seconds > s.seconds
    ]
    return {"scenes": scenes}
