//! Selects application extensions and injects them into the reusable host.
mod plugins;
fn main() -> anyhow::Result<()> {
    let mut application = hl2_bevy::extensions::Application {
        install: vec![plugins::install],
        ..Default::default()
    };
    #[cfg(feature = "advanced-options")]
    hl2_advanced_options::register(&mut application.ui).map_err(anyhow::Error::msg)?;
    #[cfg(feature = "debug-tools")]
    {
        hl2_debug::register(&mut application.ui).map_err(anyhow::Error::msg)?;
        application.runtime.push(hl2_debug::bevy::runtime());
    }
    hl2_bevy::run(application)
}
