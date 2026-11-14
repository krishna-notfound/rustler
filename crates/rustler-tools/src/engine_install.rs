//! Extract the embedded Python engine into `home/engine`, then `uv python install 3.11`
//! and `uv sync --project home/engine`.
//!
//! TODO(Task 4): embed the engine with
//! `include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../engine")` (excluding `tests/`
//! and `.venv/`). Not added in the Wave 0 skeleton because `engine/` was still being
//! written by Task 5 when the skeleton was frozen.
