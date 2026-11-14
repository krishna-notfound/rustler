"""Entry point: ``python -m rustler_engine`` serves JSON-RPC on stdin/stdout.

Protocol lines are the only thing written to the real stdout. Before anything else is
imported, fd 1 is duplicated for the protocol and then pointed at stderr, so library
prints, tqdm and native code can never corrupt the stream.
"""

from __future__ import annotations

import io
import logging
import os
import sys
from typing import Callable, Dict, Optional


def main(extra_methods: Optional[Dict[str, Callable]] = None) -> int:
    # Keep the real stdout for the protocol; send everything else to stderr (python level and fd level).
    proto_fd = os.dup(1)
    os.dup2(2, 1)
    protocol_out = io.TextIOWrapper(
        os.fdopen(proto_fd, "wb", buffering=0), encoding="utf-8", newline="\n", write_through=True
    )
    sys.stdout = sys.stderr
    protocol_in = io.TextIOWrapper(sys.stdin.buffer, encoding="utf-8", newline=None)

    logging.basicConfig(
        stream=sys.stderr,
        level=os.environ.get("RUSTLER_ENGINE_LOG", "INFO").upper(),
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )
    os.environ.setdefault("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")

    from . import rpc

    methods = rpc.default_methods()
    if extra_methods:
        methods.update(extra_methods)
    try:
        rpc.serve(protocol_in, protocol_out, methods)
    except BrokenPipeError:
        return 0
    finally:
        try:
            protocol_out.flush()
        except Exception:
            pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
