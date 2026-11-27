//! Selects application extensions and injects them into the reusable host.
#[cfg(feature = "trace-alloc")]
mod alloc_count;
mod plugins;
fn main() -> anyhow::Result<()> {
    // The main thread runs the frame; Bevy's pool threads take theirs as they spawn.
    hl2_bevy::priority::set_process();
    hl2_bevy::priority::set_current_thread(hl2_bevy::priority::Priority::Frame);
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
            max_bytes: hl2_bevy::performance::status::TraceRecording::max_bytes_from_mb(
                std::env::var("TRACE_CHROME_MAX_MB").ok().as_deref(),
            ),
            #[cfg(feature = "trace-alloc")]
            allocations: Some(alloc_count::totals),
            #[cfg(not(feature = "trace-alloc"))]
            allocations: None,
        });
    }
    hl2_bevy::run(application)
}
