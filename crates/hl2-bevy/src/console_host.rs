//! Host side of the developer console: what its arguments complete from, its command history
//! on disk, and the engine log mirrored into it.
//!
//! * Completion: the installed maps (listed once at startup) and the loaded map's entity names
//!   and the inputs its own connections fire ([`MapEntityNames`], replaced on each map load)
//!   reach the console whenever either changes ([`sync_sources`]).
//! * History: `hl2-console-history.txt` next to the settings file, never in the game install;
//!   loaded at startup and written whenever the console's history changed ([`save_history`]).
//! * Log: warnings and errors from every target, and info from HL2-RS's own crates, are queued
//!   by a `tracing` layer ([`log_layer`]) and printed in the console each frame ([`drain_log`]).

use bevy::log::{
    BoxedLayer,
    tracing::{self, field::Visit},
    tracing_subscriber::{Layer, layer::Context},
};
use bevy::prelude::*;
use hl2_ui::console::complete::Sources;
use source_ui::console::panel::Level;
use std::sync::{Mutex, OnceLock};

/// Every map name the installed games ship, sorted.
#[derive(Resource, Default)]
pub struct InstalledMaps(pub Vec<String>);

/// The loaded map's entity targetnames and the inputs its connections fire, sorted, each once.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct MapEntityNames {
    pub targets: Vec<String>,
    pub inputs: Vec<String>,
}

impl MapEntityNames {
    /// Source connections are output keys (`OnTrigger`, ...) whose value is
    /// `target,input,parameter,delay,times` (`,` or ESC separated).
    pub fn of(entities: &[modkit_core::Entity]) -> Self {
        let mut targets = std::collections::BTreeSet::new();
        let mut inputs = std::collections::BTreeSet::new();
        for entity in entities {
            if let Some(name) = entity.get("targetname").filter(|n| !n.is_empty()) {
                targets.insert(name.to_owned());
            }
            for (key, value) in &entity.properties {
                if !key.starts_with("On") && !key.starts_with("on") {
                    continue;
                }
                let mut fields = value.split([',', '\u{1b}']);
                if let (Some(_), Some(input)) = (fields.next(), fields.next())
                    && !input.is_empty()
                    && fields.next().is_some()
                {
                    inputs.insert(input.to_owned());
                }
            }
        }
        Self {
            targets: targets.into_iter().collect(),
            inputs: inputs.into_iter().collect(),
        }
    }
}

/// Passes the completion sources to the console when they change.
pub fn sync_sources(
    maps: Res<InstalledMaps>,
    names: Res<MapEntityNames>,
    ui: Option<ResMut<crate::ui_host::Console>>,
) {
    let Some(mut ui) = ui else {
        return;
    };
    if maps.is_changed() || names.is_changed() || ui.is_added() {
        ui.source.set_completion_sources(Sources {
            maps: maps.0.clone(),
            entity_targets: names.targets.clone(),
            entity_inputs: names.inputs.clone(),
        });
    }
}

fn history_path() -> std::path::PathBuf {
    hl2_ui::config::Config::config_path().with_file_name("hl2-console-history.txt")
}

/// The saved command history, oldest first.
pub fn load_history() -> Vec<String> {
    std::fs::read_to_string(history_path())
        .map(|text| {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Writes the command history when it changed.
pub fn save_history(ui: Option<ResMut<crate::ui_host::Console>>) {
    let Some(history) = ui.and_then(|mut ui| ui.source.take_changed_history()) else {
        return;
    };
    if let Err(error) = std::fs::write(history_path(), history.join("\n")) {
        warn!("Could not save the console history: {error}");
    }
}

static LOG: OnceLock<Mutex<Vec<(Level, String)>>> = OnceLock::new();

/// HL2-RS's own crates, whose info lines the console shows.
const OWN_TARGETS: &[&str] = &["hl2_", "source_assets", "modkit_core"];

struct ToConsole;

impl<S: tracing::Subscriber> Layer<S> for ToConsole {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let metadata = event.metadata();
        let level = match *metadata.level() {
            tracing::Level::ERROR => Level::Error,
            tracing::Level::WARN => Level::Warning,
            tracing::Level::INFO
                if OWN_TARGETS.iter().any(|t| metadata.target().starts_with(t)) =>
            {
                Level::Developer
            }
            _ => return,
        };
        struct Message(String);
        impl Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        if let Ok(mut queue) = LOG.get_or_init(Default::default).lock() {
            queue.push((level, message.0));
        }
    }
}

/// `LogPlugin::custom_layer`: queues log lines for the console.
pub fn log_layer(_: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(ToConsole))
}

/// The window cursor for a placement hint: resize arrows over the console's corner grips.
pub fn cursor_icon(hint: source_ui::placement::CursorHint) -> bevy::window::CursorIcon {
    use bevy::window::SystemCursorIcon;
    use source_ui::placement::CursorHint;
    bevy::window::CursorIcon::System(match hint {
        CursorHint::Arrow => SystemCursorIcon::Default,
        CursorHint::ResizeNwSe => SystemCursorIcon::NwseResize,
        CursorHint::ResizeNeSw => SystemCursorIcon::NeswResize,
    })
}

/// Shows the console's cursor hint on the window, written only when it changes; the arrow
/// whenever no dialog asks for another shape.
pub fn apply_cursor_hint(
    ui: Option<Res<crate::ui_host::Console>>,
    windows: Query<(Entity, Option<&bevy::window::CursorIcon>), With<bevy::window::PrimaryWindow>>,
    mut commands: Commands,
) {
    let hint = ui.map_or_else(Default::default, |ui| ui.source.cursor_hint());
    let icon = cursor_icon(hint);
    if let Ok((window, current)) = windows.single()
        && current != Some(&icon)
    {
        commands.entity(window).insert(icon);
    }
}

/// Prints the queued log lines in the console.
pub fn drain_log(ui: Option<ResMut<crate::ui_host::Console>>) {
    let (Some(mut ui), Some(queue)) = (ui, LOG.get()) else {
        return;
    };
    let lines = queue
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default();
    for (level, text) in lines {
        ui.source.print(level, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(properties: &[(&str, &str)]) -> modkit_core::Entity {
        modkit_core::Entity {
            properties: properties
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn entity_names_come_from_targetnames_and_connections() {
        let names = MapEntityNames::of(&[
            entity(&[("classname", "logic_relay"), ("targetname", "relay")]),
            entity(&[
                ("targetname", "lab_door"),
                ("OnTrigger", "relay,Trigger,,0,-1"),
                ("OnOpen", "lamp\u{1b}TurnOn\u{1b}\u{1b}0.5\u{1b}1"),
                ("origin", "0 0 0"),
            ]),
            entity(&[("targetname", ""), ("OnBroken", "bad")]),
        ]);
        assert_eq!(names.targets, ["lab_door", "relay"]);
        assert_eq!(names.inputs, ["Trigger", "TurnOn"]);
    }
}
