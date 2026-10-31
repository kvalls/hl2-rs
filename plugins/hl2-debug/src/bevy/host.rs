//! HL2 host adapter. All developer policy and mutable state belongs to this plugin.
mod background;
mod commands;
mod draw;
mod input;
mod loading;
#[cfg(test)]
mod tests;
mod ui;
use crate::{spawn::Overlay, viewmodel::ViewmodelDebug};
use hl2_bevy::extensions::{Context, Input, RuntimeExtension, UiInput};
#[derive(Default)]
pub struct State {
    dev_overlay: bool,
    viewmodel_debug: ViewmodelDebug,
    background_debug: crate::bevy::background::BackgroundDebug,
    overlay: Overlay,
    repeats: crate::bevy::input::DebugRepeats,
    repeat: crate::bevy::repeat::KeyRepeat,
    font: Option<crate::spawn_panel::PanelFont>,
    hits: Vec<([f32; 4], crate::spawn::PanelTarget)>,
    rows: usize,
    lines: Vec<String>,
    /// The artwork choice last seen in a config update; only a change resets temporary FOV.
    force_default_blur: bool,
    /// The AI and notarget switches last drawn, for the HUD report.
    panel_switches: Option<(bool, bool)>,
    /// The Props listing being read off the frame thread for the installed map.
    props: Option<PropListing>,
}
type PropListing =
    std::sync::Arc<std::sync::OnceLock<std::collections::BTreeMap<String, Vec<String>>>>;
impl State {
    /// Developer shortcuts follow gameplay input: not during scripts, pause, transitions or
    /// typing, and only with a captured cursor unless the F4 panel freed it.
    fn shortcuts_allowed(&self, sim: &hl2_bevy::simulation::Simulation, input: &Input<'_>) -> bool {
        sim.commands.is_empty()
            && !sim.paused()
            && !sim.transition
            && !self.overlay.typing
            && (input.cursor_grabbed || self.overlay.open)
    }
    fn adopt_finished_props(&mut self) {
        if let Some(folders) = self.props.as_ref().and_then(|props| props.get()) {
            self.overlay.set_props(folders.clone());
            self.props = None;
        }
    }
}
impl RuntimeExtension for State {
    fn input(&mut self, context: &mut Context<'_>, input: &Input<'_>) {
        self.adopt_finished_props();
        let sim = &mut *context.simulation;
        if input.focused && input.keys.just_pressed(crate::bevy::input::OVERLAY_KEY) {
            self.dev_overlay = !self.dev_overlay;
        }
        let mut repeats = std::mem::take(&mut self.repeats);
        input::process_debug_tuning_keys(
            self,
            sim,
            context.gameplay.as_deref(),
            context.console.as_deref_mut(),
            input::DebugTuningInput {
                keys: input.keys,
                time: input.time,
                aspect: input.aspect,
            },
            &mut repeats,
        );
        self.repeats = repeats;
        sim.input_claims.attacks |= self.overlay.open;
        sim.input_claims.keyboard |= self.overlay.typing;
        sim.input_claims.pointer |= self.overlay.open;
        if !self.shortcuts_allowed(sim, input) {
            return;
        }
        if input.keys.just_pressed(crate::bevy::input::FLIGHT_KEY) {
            let fly = !sim.flying();
            sim.set_fly(fly);
        }
        if let Some(game) = context.gameplay.as_deref_mut() {
            let config = context.console.as_ref().map(|ui| &ui.source.config);
            if hl2_bevy::input::action_just_pressed("loadout", input.keys, input.buttons, config) {
                game.actions.push(hl2_bevy::gameplay::Action::Loadout);
            }
        }
    }
    fn effect(&mut self, context: &mut Context<'_>, event: &serde_json::Value) -> bool {
        let Ok(request) = serde_json::from_value::<crate::commands::Request>(event.clone()) else {
            return false;
        };
        let Some(ui) = context.console.as_deref_mut() else {
            return false;
        };
        match request {
            crate::commands::Request::ViewmodelDebugMode(on) => {
                self.viewmodel_debug_mode(context.simulation, ui, on)
            }
            crate::commands::Request::ViewmodelTweakReset => {
                self.viewmodel_tweak_reset(context.simulation, ui)
            }
            other => self.apply_background_debug_effect(context.simulation, ui, other),
        }
        true
    }
    fn ui_before(&mut self, context: &mut Context<'_>, input: &UiInput<'_>) {
        self.before_ui(context, input);
    }
    fn ui_after(&mut self, context: &mut Context<'_>, input: &UiInput<'_>) {
        self.after_ui(context, input);
    }
    fn present(
        &mut self,
        context: &mut Context<'_>,
        hud: &mut hl2_bevy::canvas_renderer::Hud,
        status: &hl2_bevy::Status,
    ) {
        self.draw(context, hud, status);
    }
    fn loading(&mut self, context: &mut Context<'_>, event: &hl2_bevy::extensions::MapLoad<'_>) {
        if let Some(ui) = context.console.as_deref_mut() {
            match event.result {
                None => loading::log_debug_load_start(event, self, ui),
                Some(success) => loading::log_debug_load_finish(event, success, self, ui),
            }
        }
    }
    fn map_changed(&mut self, _sim: &mut hl2_bevy::simulation::Simulation) {
        self.viewmodel_debug = ViewmodelDebug::default();
    }
    fn map_installed(
        &mut self,
        game: &mut hl2_bevy::gameplay::Gameplay,
        vfs: &std::sync::Arc<source_assets::vpk::Vfs>,
    ) {
        self.overlay = Overlay::default();
        self.overlay
            .set_usable(hl2_bevy::gameplay::spawn_usable_weapons(
                &game.world,
                &game.ai,
            ));
        // Listing every archive is too slow for the frame that adopts the map.
        let listing = PropListing::default();
        self.props = Some(listing.clone());
        let vfs = vfs.clone();
        std::thread::spawn(move || {
            let _ = listing.set(vfs.physics_prop_models());
        });
    }
    fn startup(&mut self, config: &hl2_ui::config::Config) {
        self.force_default_blur = config.video.background_force_default_blur;
    }
    fn level_target(&mut self, map: &str, background: bool, widescreen: bool) {
        self.init_background_target(map, background, widescreen);
    }
    fn config_changed(&mut self, context: &mut Context<'_>, config: &hl2_ui::config::Config) {
        let force = config.video.background_force_default_blur;
        if std::mem::replace(&mut self.force_default_blur, force) == force {
            return;
        }
        self.background_debug.fov = None;
        if self.background_debug.enabled
            && let Some(ui) = context.console.as_deref()
        {
            self.background_debug.blur_texture_name = ui
                .source
                .blur_texture_name(&self.background_debug.map_name, ui.source.is_widescreen());
        }
    }
    fn script(
        &mut self,
        _sim: &mut hl2_bevy::simulation::Simulation,
        event: &serde_json::Value,
    ) -> bool {
        let enabled = event
            .get("developer_overlay")
            .and_then(serde_json::Value::as_bool);
        if let Some(enabled) = enabled {
            self.dev_overlay = enabled;
        }
        enabled.is_some()
    }
    fn action(&mut self, context: &mut Context<'_>, action: &serde_json::Value) -> bool {
        self.replay_action(context, action)
    }
    fn framing(
        &self,
        weapon: &str,
        aspect: f32,
    ) -> Option<modkit_core::viewmodel::ViewmodelFraming> {
        let debug = &self.viewmodel_debug;
        if !debug.enabled {
            return None;
        }
        let profile = hl2_ui::tuning::viewmodel_profile(aspect);
        let value = debug
            .edits
            .iter()
            .find(|((name, edited), _)| name == weapon && *edited == profile)
            .map(|(_, framing)| *framing)
            .or_else(|| {
                (debug.weapon == weapon && debug.profile == Some(profile)).then_some(debug.framing)
            })?;
        Some(hl2_ui::tuning::viewmodel_framing_with_profile(
            weapon, aspect, profile, value,
        ))
    }
    fn projection(&self, sim: &hl2_bevy::simulation::Simulation, aspect: f32) -> Option<f32> {
        self.background_debug.enabled.then(|| {
            self.background_debug
                .fov(
                    hl2_bevy::camera::background_fov(aspect, sim.background_force_default_blur)
                        .to_degrees(),
                )
                .to_radians()
        })
    }
    fn report(&self) -> Option<serde_json::Value> {
        Some(
            serde_json::json!({"dev_overlay":self.dev_overlay,"viewmodel_debug":self.viewmodel_debug.enabled}),
        )
    }
    fn hud_report(&self, hud: &hl2_bevy::canvas_renderer::Hud) -> Option<serde_json::Value> {
        Some(
            serde_json::json!({"spawn_overlay":self.overlay.open.then(||{let (ai,notarget)=self.panel_switches.unwrap_or((true,false));self.overlay.panel(ai,notarget)}),"developer_overlay":{"visible":!self.lines.is_empty(),"fps":hud.frame_statistics().0,"frame_ms":hud.frame_statistics().1,"benchmark":hud.frame_statistics().2,"lines":self.lines}}),
        )
    }
    fn gameplay_report(&self) -> Option<(&'static str, serde_json::Value)> {
        Some((
            "spawn_overlay",
            serde_json::to_value(&self.overlay).expect("overlay report"),
        ))
    }
}
