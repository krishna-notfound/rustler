"""Rustler Python worker: transcription, scene detection and transliteration over stdio JSON-RPC."""

from importlib import metadata as _metadata

try:
    __version__ = _metadata.version("rustler-engine")
except _metadata.PackageNotFoundError:  # running from a source tree that is not installed
    __version__ = "0.1.0"
