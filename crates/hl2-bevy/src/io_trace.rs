//! `--io-trace PATH`: the scene's entity I/O log, one line per fired output as Source prints it
//! at `developer 2`, for comparisons with native console logs. Without the option the system
//! below finds no resource and does nothing.

use bevy::prelude::{ResMut, Resource};
use std::io::Write;

#[derive(Resource)]
pub struct IoTraceFile(std::io::BufWriter<std::fs::File>);

impl IoTraceFile {
    pub fn create(path: &std::path::Path) -> anyhow::Result<Self> {
        Ok(Self(std::io::BufWriter::new(std::fs::File::create(path)?)))
    }
}

/// Before each fixed step: the current map's scene keeps the log (a new map's from its first
/// tick), and what the last steps fired goes to the file.
pub fn flush(file: Option<ResMut<IoTraceFile>>, game: Option<ResMut<crate::gameplay::Gameplay>>) {
    let (Some(mut file), Some(mut game)) = (file, game) else {
        return;
    };
    let game = &mut *game;
    let trace = game.scene.io_trace.get_or_insert_with(Default::default);
    let lines = trace.take_lines(&game.world);
    if lines.is_empty() {
        return;
    }
    // Tracing must never stop the game; a failed write only loses diagnostics.
    let _ = lines
        .iter()
        .try_for_each(|line| writeln!(file.0, "{line}"))
        .and_then(|()| file.0.flush());
}
