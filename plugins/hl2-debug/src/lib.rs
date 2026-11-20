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
    ]);
    extensions.commands(&commands::specs::<hl2_ui::console::Console>())?;
    extensions.select_standard_background(standard_background::forced)?;
    extensions.bindings.extend_from_slice(DEFAULT_BINDINGS);
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
