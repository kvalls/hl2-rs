//! Optional hl2-debug extension; portable policy and separately enabled Bevy integration.
pub mod agent;
#[cfg(feature = "bevy")]
pub mod bevy;
pub mod commands;
pub mod overlay;
pub mod spawn;
pub mod spawn_panel;
pub mod standard_background;
pub mod viewmodel;
/// Additional default bindings installed only by the debug feature.
pub const DEFAULT_BINDINGS: &[(&str, &str)] = &[("loadout", "F3")];

pub fn register(extensions: &mut hl2_ui::extensions::Extensions) -> Result<(), String> {
    extensions.localization.extend([
        ("HL2RS_AgentThinking", "Thinking..."),
        ("HL2RS_AgentWaiting", "Still waiting for a reply..."),
        (
            "HL2RS_AgentRconLocal",
            "Run agent_rcon in the local developer console.",
        ),
        ("HL2RS_AgentRconReady", "Agent session ready on"),
        ("HL2RS_AgentRconFailed", "Could not enable agent session"),
    ]);
    extensions.commands(&commands::specs::<hl2_ui::console::Console>())?;
    extensions.select_standard_background(standard_background::forced)?;
    extensions.bindings.extend_from_slice(DEFAULT_BINDINGS);
    // The session watcher sets the agent status on every message: not worth a log line each.
    extensions.quiet_remote.push("agent_status");
    Ok(())
}
pub fn painter(console: &hl2_ui::console::Console) -> overlay::OverlayPainter<'_> {
    overlay::OverlayPainter {
        canvas: &console.canvas,
        font: console.overlay_font(),
    }
}

#[cfg(test)]
mod ui_tests;
