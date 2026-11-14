//! Media operations over ffmpeg/ffprobe subprocesses: probe, proxy/audio extraction,
//! frames/tiles/previews (`see.*`), audio analysis, encoder choice, the render
//! filtergraph compiler, rendering and QC.

pub mod audio_analysis;
pub mod compile;
pub mod encoders;
pub mod frames;
pub mod probe;
pub mod proxy;
pub mod qc;
pub mod render;
pub mod tiles;

pub use audio_analysis::analyze_audio;
pub use compile::{CompiledRender, SourceMedia, compile};
pub use encoders::{EncoderChoice, pick_encoder};
pub use frames::{frame, preview};
pub use probe::probe;
pub use proxy::{extract_audio_wav, make_proxy};
pub use qc::run_qc;
pub use render::{preview_frame, render};
pub use tiles::{TileLabel, TileSpec, tiles};
