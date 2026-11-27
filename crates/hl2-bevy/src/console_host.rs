//! Host side of the developer console: what its arguments complete from and the engine log
//! mirrored into it. The command history lasts one run, as in retail.
//!
//! * Completion: the installed maps (listed once at startup) and the loaded map's entity names
//!   and the inputs its own connections fire ([`MapEntityNames`], replaced on each map load)
//!   reach the console whenever either changes ([`sync_sources`]).
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

/// Receiver for background installed-map discovery; cleared once received.
#[derive(Resource, Default)]
pub struct PendingInstalledMaps(Option<Mutex<std::sync::mpsc::Receiver<Vec<String>>>>);

/// Discovers installed maps on a background thread without delaying startup.
pub fn spawn_installed_maps(root: std::path::PathBuf) -> PendingInstalledMaps {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let _ = std::thread::Builder::new()
        .name("hl2-installed-maps".into())
        .spawn(move || {
            crate::priority::set_current_thread(crate::priority::Priority::Background);
            let _ = tx.send(source_assets::install::installed_maps(&root));
        });
    PendingInstalledMaps(Some(Mutex::new(rx)))
}

/// Applies the background installed-map scan once ready, then stops polling.
pub fn poll_installed_maps(
    mut pending: ResMut<PendingInstalledMaps>,
    mut installed: ResMut<InstalledMaps>,
) {
    let Some(recv) = pending.0.as_ref().map(|rx| {
        rx.lock()
            .map_or(Err(std::sync::mpsc::TryRecvError::Disconnected), |guard| {
                guard.try_recv()
            })
    }) else {
        return;
    };
    match recv {
        Ok(maps) => {
            installed.0 = maps;
            pending.0 = None;
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            pending.0 = None;
        }
    }
}

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

/// `LogPlugin::custom_layer`: queues log lines for the console; trace builds add the Chrome trace
/// layer, which starts writing with gameplay.
pub fn log_layer(app: &mut App) -> Option<BoxedLayer> {
    #[cfg(feature = "trace")]
    {
        let layers: Vec<BoxedLayer> = vec![
            Box::new(ToConsole),
            crate::performance::trace_writer::layer(app),
        ];
        Some(Box::new(layers))
    }
    #[cfg(not(feature = "trace"))]
    {
        let _ = app;
        Some(Box::new(ToConsole))
    }
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

    #[test]
    fn poll_installed_maps_applies_once_and_clears_receiver() {
        use bevy::ecs::system::RunSystemOnce;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        tx.send(vec![
            "d1_trainstation_01".into(),
            "d1_trainstation_02".into(),
        ])
        .unwrap();
        let mut world = World::default();
        world.insert_resource(InstalledMaps::default());
        world.insert_resource(PendingInstalledMaps(Some(Mutex::new(rx))));
        world.run_system_once(poll_installed_maps).unwrap();
        assert_eq!(
            world.resource::<InstalledMaps>().0,
            ["d1_trainstation_01", "d1_trainstation_02"]
        );
        assert!(world.resource::<PendingInstalledMaps>().0.is_none());
        world.run_system_once(poll_installed_maps).unwrap();
        assert_eq!(
            world.resource::<InstalledMaps>().0,
            ["d1_trainstation_01", "d1_trainstation_02"]
        );
    }
}
