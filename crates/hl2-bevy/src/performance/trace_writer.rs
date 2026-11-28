//! Profiling builds only (`trace` feature): the Chrome trace layer, writing nothing until
//! gameplay starts (`status::TraceCommand::Start`) and finished before the process exits.
//! Bevy's own `trace_chrome` writes from the launch and keeps its writer private, so trace builds
//! install the same layer here instead.
use super::status::{TraceCommand, TraceSwitch};
use bevy::log::{
    BoxedLayer,
    tracing_subscriber::{
        Registry,
        fmt::{FormattedFields, format::DefaultFields},
    },
};
use bevy::prelude::App;
use std::{
    io::{BufWriter, Write},
    sync::Mutex,
};
use tracing_chrome::{ChromeLayerBuilder, EventOrSpan, FlushGuard};

/// The trace layer, writing to a sink until gameplay starts it; its switch goes into `app`.
pub fn layer(app: &mut App) -> BoxedLayer {
    let (layer, guard) = ChromeLayerBuilder::<Registry>::new()
        .writer(std::io::sink())
        // Span names carry their fields (`npc: id=31`), as Bevy's trace layer writes them.
        .name_fn(Box::new(|event_or_span| match event_or_span {
            EventOrSpan::Event(event) => event.metadata().name().into(),
            EventOrSpan::Span(span) => match span
                .extensions()
                .get::<FormattedFields<DefaultFields>>()
            {
                Some(fields) => format!("{}: {}", span.metadata().name(), fields.fields.as_str()),
                None => span.metadata().name().into(),
            },
        }))
        .build();
    let guard = Mutex::new(Some(guard));
    app.insert_resource(TraceSwitch(Box::new(move |command| {
        let mut guard = guard.lock().expect("trace writer");
        match command {
            TraceCommand::Start(file) => start(guard.as_ref(), file),
            // Dropping the guard writes the closing bracket and joins the writer thread.
            TraceCommand::Finish => drop(guard.take()),
        }
    })));
    Box::new(layer)
}

fn start(guard: Option<&FlushGuard>, file: Option<&std::path::Path>) {
    let Some(guard) = guard else { return };
    let writer = file.and_then(|path| match std::fs::File::create(path) {
        Ok(file) => Some(Box::new(BufWriter::new(file)) as Box<dyn Write + Send>),
        Err(error) => {
            bevy::log::warn!("trace file {}: {error}", path.display());
            None
        }
    });
    guard.start_new(writer);
}
