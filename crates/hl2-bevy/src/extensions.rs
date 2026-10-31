//! Application-injected runtime extensions; the host has no concrete extension dependencies.
use crate::{gameplay::Gameplay, simulation::Simulation, ui_host::Console};
use bevy::prelude::{App, ButtonInput, KeyCode, MouseButton, Window};
use bevy::window::CursorOptions;

/// Input claims are cleared before extension input, then consumed by the normal host routing.
#[derive(Default, Clone, Copy)]
pub struct InputClaims {
    pub attacks: bool,
    pub keyboard: bool,
    pub pointer: bool,
    pub escape: bool,
}

pub struct Input<'a> {
    pub keys: &'a ButtonInput<KeyCode>,
    pub buttons: &'a ButtonInput<MouseButton>,
    pub focused: bool,
    pub cursor_grabbed: bool,
    pub aspect: f32,
    pub time: Option<f64>,
}

pub struct MapLoad<'a> {
    pub map: &'a str,
    pub background: bool,
    pub result: Option<bool>,
}

pub struct UiInput<'a> {
    pub keys: &'a ButtonInput<KeyCode>,
    pub buttons: &'a ButtonInput<MouseButton>,
    pub text: &'a str,
    pub wheel: f32,
    pub delta: f32,
    pub window: &'a Window,
    pub cursor: &'a CursorOptions,
    pub selection_cancelled: bool,
}

/// Implementations own their state. The application supplies instances before startup.
/// Empty defaults let a contributor implement only the lifecycle hooks it needs.
pub trait RuntimeExtension: Send + Sync + 'static {
    fn input(&mut self, _context: &mut Context<'_>, _input: &Input<'_>) {}
    fn effect(&mut self, _context: &mut Context<'_>, _event: &serde_json::Value) -> bool {
        false
    }
    fn ui_before(&mut self, _context: &mut Context<'_>, _input: &UiInput<'_>) {}
    fn ui_after(&mut self, _context: &mut Context<'_>, _input: &UiInput<'_>) {}
    fn present(
        &mut self,
        _context: &mut Context<'_>,
        _hud: &mut crate::canvas_renderer::Hud,
        _status: &crate::Status,
    ) {
    }
    fn loading(&mut self, _context: &mut Context<'_>, _event: &MapLoad<'_>) {}
    fn map_changed(&mut self, _simulation: &mut Simulation) {}
    fn map_installed(
        &mut self,
        _game: &mut Gameplay,
        _vfs: &std::sync::Arc<source_assets::vpk::Vfs>,
    ) {
    }
    /// The stored settings, before the first `level_target`.
    fn startup(&mut self, _config: &hl2_ui::config::Config) {}
    fn level_target(&mut self, _map: &str, _background: bool, _widescreen: bool) {}
    fn config_changed(&mut self, _context: &mut Context<'_>, _config: &hl2_ui::config::Config) {}
    fn action(&mut self, _context: &mut Context<'_>, _action: &serde_json::Value) -> bool {
        false
    }
    /// Handles a replay event; an event no contributor handles fails the run.
    fn script(&mut self, _simulation: &mut Simulation, _event: &serde_json::Value) -> bool {
        false
    }
    fn framing(
        &self,
        _weapon: &str,
        _aspect: f32,
    ) -> Option<modkit_core::viewmodel::ViewmodelFraming> {
        None
    }
    fn projection(&self, _simulation: &Simulation, _aspect: f32) -> Option<f32> {
        None
    }
    fn hud_report(&self, _hud: &crate::canvas_renderer::Hud) -> Option<serde_json::Value> {
        None
    }
    fn gameplay_report(&self) -> Option<(&'static str, serde_json::Value)> {
        None
    }
    fn report(&self) -> Option<serde_json::Value> {
        None
    }
}

pub struct Context<'a> {
    pub simulation: &'a mut Simulation,
    pub gameplay: Option<&'a mut Gameplay>,
    pub console: Option<&'a mut Console>,
}

/// The executable supplies selected implementations and Bevy plugin registration.
#[derive(Default)]
pub struct Application {
    pub ui: hl2_ui::extensions::Extensions,
    pub runtime: Vec<Box<dyn RuntimeExtension>>,
    pub install: Vec<fn(&mut App)>,
}

#[derive(bevy::prelude::Resource)]
pub(crate) struct UiContributions(pub hl2_ui::extensions::Extensions);

impl Simulation {
    pub fn gameplay_report(&self, game: &Gameplay) -> serde_json::Value {
        let mut report = game.report(&self.physics);
        for extension in &self.extensions {
            if let Some((key, value)) = extension.gameplay_report() {
                report[key] = value;
            }
        }
        report
    }
    pub fn hud_report(&self, hud: &crate::canvas_renderer::Hud) -> serde_json::Value {
        let mut report = hud.report();
        for extension in &self.extensions {
            if let Some(serde_json::Value::Object(fields)) = extension.hud_report(hud) {
                report.as_object_mut().expect("HUD report").extend(fields);
            }
        }
        report
    }

    pub fn notify_loading(
        &mut self,
        console: &mut Console,
        map: &str,
        background: bool,
        result: Option<bool>,
    ) {
        let event = MapLoad {
            map,
            background,
            result,
        };
        self.with_extensions(|extension, simulation| {
            extension.loading(
                &mut Context {
                    simulation,
                    gameplay: None,
                    console: Some(console),
                },
                &event,
            )
        });
    }

    pub(crate) fn script_extension(&mut self, event: &serde_json::Value) {
        let mut handled = false;
        self.with_extensions(|extension, simulation| {
            handled |= extension.script(simulation, event)
        });
        if !handled {
            self.reject_extension_event("script event", event);
        }
    }

    pub(crate) fn extensions_input(
        &mut self,
        mut gameplay: Option<&mut Gameplay>,
        mut console: Option<&mut Console>,
        input: &Input<'_>,
    ) {
        self.with_extensions(|extension, simulation| {
            extension.input(
                &mut Context {
                    simulation,
                    gameplay: gameplay.as_deref_mut(),
                    console: console.as_deref_mut(),
                },
                input,
            );
        });
    }

    /// Records an opaque contribution no selected extension handled; a script run then fails.
    pub(crate) fn reject_extension_event(&mut self, kind: &str, event: &serde_json::Value) {
        bevy::log::error!("no extension handled the {kind} {event}");
        self.extension_rejected.push(format!("{kind}: {event}"));
    }

    /// Moves the existing vector while callbacks run; no per-frame allocation or locking.
    pub fn with_extensions(&mut self, mut visit: impl FnMut(&mut dyn RuntimeExtension, &mut Self)) {
        let mut extensions = std::mem::take(&mut self.extensions);
        for extension in &mut extensions {
            visit(extension.as_mut(), self);
        }
        self.extensions = extensions;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An independent contributor: nothing in core names it.
    #[derive(Default)]
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl RuntimeExtension for Recorder {
        fn startup(&mut self, _config: &hl2_ui::config::Config) {
            self.0.lock().unwrap().push("startup".into());
        }
        fn map_changed(&mut self, _simulation: &mut Simulation) {
            self.0.lock().unwrap().push("map_changed".into());
        }
        fn script(&mut self, _simulation: &mut Simulation, event: &serde_json::Value) -> bool {
            event.get("recorder").is_some()
        }
    }

    fn simulation() -> Simulation {
        Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        )
    }

    #[test]
    fn an_injected_contributor_receives_lifecycle_callbacks_without_core_edits() {
        let log = std::sync::Arc::default();
        let mut sim = simulation();
        sim.extensions
            .push(Box::new(Recorder(std::sync::Arc::clone(&log))));
        sim.change_map(&modkit_core::World::default(), glam::Vec3::ZERO, true);
        assert_eq!(*log.lock().unwrap(), ["map_changed"]);
    }

    #[test]
    fn unhandled_script_events_are_rejected_rather_than_ignored() {
        let mut sim = simulation();
        sim.extensions.push(Box::<Recorder>::default());
        sim.script_extension(&serde_json::json!({"recorder": 1}));
        assert!(sim.extension_rejected.is_empty());
        sim.script_extension(&serde_json::json!({"typo": 1}));
        assert_eq!(sim.extension_rejected.len(), 1);
        assert!(
            sim.report()["extension_rejected"]
                .as_array()
                .is_some_and(|a| a.len() == 1)
        );
    }

    #[test]
    fn without_a_contributor_every_event_is_rejected() {
        let mut sim = simulation();
        sim.script_extension(&serde_json::json!({"developer_overlay": true}));
        assert_eq!(sim.extension_rejected.len(), 1);
    }
}
