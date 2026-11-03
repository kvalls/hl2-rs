//! Optional Bevy adapter; no dependency on the application host.
pub mod background;
pub mod host;
pub mod input;
pub mod repeat;
/// The developer runtime; the application injects it before startup settings are applied.
pub fn runtime() -> Box<dyn hl2_bevy::extensions::RuntimeExtension> {
    Box::<host::State>::default()
}
