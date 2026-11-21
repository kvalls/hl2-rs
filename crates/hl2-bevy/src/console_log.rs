//! `con_logfile` / `-condebug`: what the console prints is appended to a file next to the
//! settings file. Off, the system compares the cvar with the open name and returns; on, it
//! writes and flushes once in frames that printed something.

use bevy::prelude::{App, Last, ResMut, Resource, Update, warn};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// The open log file and the `con_logfile` value it was opened for.
#[derive(Resource, Default)]
pub struct LogFile {
    name: String,
    writer: Option<BufWriter<File>>,
}

/// Adds the log writer, the `screenshot` saver and, with `-usercon`, the remote console.
pub fn install(app: &mut App, usercon: bool) {
    app.init_resource::<LogFile>()
        .add_systems(Last, write_log)
        .add_systems(Update, crate::capture::console_screenshot);
    if usercon {
        crate::rcon::install(app);
    }
}

/// Opens `relative` (as `con_logfile` validated it) next to the settings file, appending.
fn open(folder: &Path, relative: &Path) -> std::io::Result<BufWriter<File>> {
    let path = folder.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::options().create(true).append(true).open(path)?;
    Ok(BufWriter::new(file))
}

pub(crate) fn settings_folder() -> PathBuf {
    let path = hl2_ui::config::Config::config_path();
    path.parent().map_or_else(PathBuf::new, Path::to_owned)
}

/// Follows `con_logfile` and appends the lines printed since the last frame.
pub fn write_log(ui: Option<ResMut<crate::ui_host::Console>>, mut log: ResMut<LogFile>) {
    let Some(mut ui) = ui else {
        return;
    };
    if log.name != ui.source.remote.con_logfile {
        let name = ui.source.remote.con_logfile.clone();
        log.writer = match hl2_ui::console::remote::log_file(&name) {
            Ok(Some(relative)) => open(&settings_folder(), &relative)
                .inspect_err(|error| warn!("Could not open the console log {name}: {error}"))
                .ok(),
            _ => None,
        };
        ui.source.shell.panel.keep_printed(log.writer.is_some());
        log.name = name;
    }
    let Some(writer) = log.writer.as_mut() else {
        return;
    };
    let panel = &mut ui.source.shell.panel;
    let lines = panel.take_printed();
    if lines.is_empty() {
        return;
    }
    let mut written = Ok(());
    for line in &lines {
        written = written.and_then(|()| writeln!(writer, "{}", panel.shown_text(line)));
    }
    if let Err(error) = written.and_then(|()| writer.flush()) {
        warn!("Could not write the console log: {error}");
        log.writer = None;
        ui.source.shell.panel.keep_printed(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_append_in_the_folder_creating_subfolders() {
        let folder = std::env::temp_dir().join(format!("hl2rs-console-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for text in ["one", "two"] {
            let mut writer = open(&folder, Path::new("runs/console.log")).expect("open");
            writeln!(writer, "{text}").expect("write");
        }
        let written = std::fs::read_to_string(folder.join("runs/console.log")).expect("read");
        assert_eq!(written, "one\ntwo\n");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
