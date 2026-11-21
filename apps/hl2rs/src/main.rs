//! Selects application extensions and injects them into the reusable host.
#[cfg(feature = "trace-alloc")]
mod alloc_count;
mod plugins;
fn main() -> anyhow::Result<()> {
    #[cfg_attr(
        not(any(
            feature = "advanced-options",
            feature = "debug-tools",
            feature = "trace-chrome"
        )),
        allow(unused_mut)
    )]
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
    // Bevy writes the trace to `TRACE_CHROME` when it is set (otherwise to a timestamped
    // file in the working directory, whose name the application does not know).
    #[cfg(feature = "trace-chrome")]
    {
        application.trace = Some(hl2_bevy::performance::status::TraceRecording {
            file: std::env::var_os("TRACE_CHROME").map(std::path::PathBuf::from),
            #[cfg(feature = "trace-alloc")]
            allocations: Some(alloc_count::totals),
            #[cfg(not(feature = "trace-alloc"))]
            allocations: None,
        });
    }
    hl2_bevy::run(application)
}
