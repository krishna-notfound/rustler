//! All Rustler commands. `register_all` is the single entry point front-ends use.

pub mod analyze_cmds;
pub mod clip_cmds;
pub mod guide_cmds;
pub mod edit_cmds;
pub mod project_cmds;
pub mod qc_cmds;
pub mod render_cmds;
pub mod see_cmds;
pub mod setup_cmds;
pub mod source_cmds;
pub mod transcript_cmds;
pub mod understand_cmds;

use rustler_registry::Registry;

/// Registers every command module.
pub fn register_all(reg: &mut Registry) {
    setup_cmds::register(reg);
    project_cmds::register(reg);
    source_cmds::register(reg);
    analyze_cmds::register(reg);
    transcript_cmds::register(reg);
    understand_cmds::register(reg);
    see_cmds::register(reg);
    clip_cmds::register(reg);
    guide_cmds::register(reg);
    edit_cmds::register(reg);
    render_cmds::register(reg);
    qc_cmds::register(reg);
}
