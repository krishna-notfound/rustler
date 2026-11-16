//! External tools: install (`setup`), health check (`doctor`), tool paths and the
//! subprocess helper. Task 4 implements the bodies.

pub(crate) mod archive;
pub mod assets;
pub mod deno_install;
pub mod doctor;
pub mod download;
pub mod engine_install;
pub mod ffmpeg_install;
pub mod fonts;
pub mod paths;
pub mod run;
pub mod setup;
pub mod uv_install;
pub mod ytdlp_install;

pub use doctor::{
    DoctorReport, EncoderStatus, NetworkCheck, OptionalFeature, ToolInfo, doctor, network_probe, optional_features,
};
pub use paths::{Tool, ToolPaths};
pub use run::{run, run_streaming};
pub use setup::{
    Component, GpuMode, InstallPlan, InstallState, Profile, SetupOptions, SetupReport, installed_state,
    profile_name, setup,
};
