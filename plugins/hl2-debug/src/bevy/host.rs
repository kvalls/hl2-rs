//! HL2 host adapter. All developer policy and mutable state belongs to this plugin.
mod background;
mod bootstrap;
mod commands;
mod draw;
mod input;
mod loading;
mod session;
mod subtitles;
#[cfg(test)]
mod tests;
mod ui;
use crate::{spawn::Overlay, viewmodel::ViewmodelDebug};
use hl2_bevy::extensions::{Context, Input, RuntimeExtension, UiInput};
#[derive(Default)]
pub struct State {
    dev_overlay: bool,
    subtitles: subtitles::RemoteSubtitles,
    agent_session: Option<session::Session>,
    agent_bootstrap: Option<bootstrap::Bootstrap>,
    viewmodel_debug: ViewmodelDebug,
    background_debug: crate::bevy::background::BackgroundDebug,
    overlay: Overlay,
    repeats: crate::bevy::input::DebugRepeats,
    repeat: crate::bevy::repeat::KeyRepeat,
    font: Option<crate::spawn_panel::PanelFont>,
    hits: Vec<([f32; 4], crate::spawn::PanelTarget)>,
    rows: usize,
    lines: Vec<String>,
    /// F1 header rows for the profilers this run has.
    profiling_lines: Vec<String>,
    /// The artwork choice last seen in a config update; only a change resets temporary FOV.
    force_default_blur: bool,
    /// The AI and notarget switches last drawn, for the HUD report.
    panel_switches: Option<(bool, bool)>,
    /// The Props listing being read off the frame thread for the installed map.
    props: Option<PropListing>,
    /// Usable weapons per F4 spawn class being resolved off the frame thread for the installed map.
    usable_weapons: Option<PropListing>,
    /// Async late-precache status shown in the F4 footer until `spawn_log` advances.
    loading_status: Option<(usize, String)>,
}
type PropListing =
    std::sync::Arc<std::sync::OnceLock<std::collections::BTreeMap<String, Vec<String>>>>;
fn resolve_spawn_usable_weapons(
    vfs: &source_assets::vpk::Vfs,
    map_name: &str,
) -> std::collections::BTreeMap<String, Vec<String>> {
    use std::collections::{BTreeMap, BTreeSet};
    let Ok(defs) = hl2_simulation::npc_weapons::definitions(vfs) else {
        return BTreeMap::new();
    };
    let mut model_activities: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for label in hl2_simulation::ai::spawn::SPAWN_CLASSES {
        let (class_name, _) = hl2_simulation::ai::spawn::spawn_class(label);
        if !hl2_simulation::npc_weapons::uses_weapons(class_name) {
            continue;
        }
        let Some(template) = hl2_simulation::ai::spawn::template_entity(label, 0) else {
            continue;
        };
        let mut mini = modkit_core::World {
            name: map_name.to_owned(),
            entities: vec![template],
            ..Default::default()
        };
        hl2_simulation::citizens::select_models(&mut mini, hl2_simulation::ai::spawn::POOL_KEY);
        let Some(model_path) = mini.entities[0]
            .get("model")
            .map(str::to_owned)
            .or_else(|| hl2_simulation::ai::spawn::default_model(class_name).map(str::to_owned))
        else {
            continue;
        };
        let key = model_path.to_lowercase();
        let activities = model_activities.entry(key).or_insert_with(|| {
            source_assets::animation::activity_names(vfs, &model_path).unwrap_or_default()
        });
        let weapons = hl2_simulation::npc_weapons::usable_weapons(class_name, activities, &defs);
        out.insert((*label).to_owned(), weapons);
    }
    out
}
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
    /// What the F4 panel takes from gameplay input while a game is loaded.
    fn panel_claims(&self, has_game: bool) -> hl2_bevy::extensions::InputClaims {
        hl2_bevy::extensions::InputClaims {
            attacks: has_game && self.overlay.open,
            keyboard: has_game && self.overlay.typing,
            pointer: has_game && self.overlay.open,
            escape: false,
        }
    }
    pub(super) fn adopt_finished_props(&mut self) {
        if let Some(folders) = self.props.as_ref().and_then(|props| props.get()) {
            self.overlay.set_props(folders.clone());
            self.props = None;
        }
    }
    pub(super) fn adopt_finished_usable(&mut self) {
        if let Some(usable) = self.usable_weapons.as_ref().and_then(|u| u.get()) {
            self.overlay.set_usable(usable.clone());
            self.usable_weapons = None;
        }
    }
}
impl RuntimeExtension for State {
    fn shutdown(&mut self) {
        self.agent_bootstrap = None;
        self.agent_session = None;
    }
    fn remote_console_chat(&mut self, line: &str) {
        self.subtitles.push(line);
    }
    fn gameplay_captions(&mut self, captions: &hl2_ui::closecaption::CaptionHud) {
        self.subtitles.submit(captions);
    }
    fn input(&mut self, context: &mut Context<'_>, input: &Input<'_>) {
        if !context.simulation.loading
            && let Some(ui) = context.console.as_deref_mut()
        {
            self.process_agent_bootstrap(ui);
        }
        self.adopt_finished_usable();
        self.adopt_finished_props();
        let sim = &mut *context.simulation;
        if input.focused && input.keys.just_pressed(crate::bevy::input::OVERLAY_KEY) {
            self.dev_overlay = !self.dev_overlay;
            if let Some(ui) = context.console.as_deref_mut() {
                crate::overlay::save_shown(&mut ui.source, self.dev_overlay);
            }
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
        let claims = self.panel_claims(context.gameplay.is_some());
        sim.input_claims.attacks |= claims.attacks;
        sim.input_claims.keyboard |= claims.keyboard;
        sim.input_claims.pointer |= claims.pointer;
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
            crate::commands::Request::UiDump => super::agent::ui_dump(context.simulation, ui),
            crate::commands::Request::MapDump(query) => {
                super::map_dump::dump(context.simulation, context.gameplay.as_deref(), ui, query)
            }
            crate::commands::Request::NpcDump(query) => {
                super::agent::npc_dump(context.simulation, context.gameplay.as_deref(), ui, query)
            }
            crate::commands::Request::AgentResume => {
                super::agent::agent_resume(context.simulation, ui)
            }
            crate::commands::Request::AgentRcon => self.agent_rcon(ui),
            crate::commands::Request::AgentCaption(line) => self.subtitles.push(&line),
            crate::commands::Request::AiEnable(value) => {
                if let Some(game) = context.gameplay.as_deref_mut() {
                    let on = hl2_bevy::simulation::set_ai_enabled(
                        game,
                        value.unwrap_or(!game.ai.enabled),
                    );
                    ui.source.log(format!("ai_enable {}", u8::from(on)));
                }
            }
            crate::commands::Request::SpawnAt(request) => {
                let message = if !ui.source.cheats {
                    "Can't use spawn_at without sv_cheats 1".to_owned()
                } else if let Some(game) = context.gameplay.as_deref_mut()
                    && !context.simulation.loading
                    && !context.simulation.background_level
                {
                    commands::spawn_at(context.simulation, game, request)
                } else {
                    "spawn_at needs a loaded gameplay map".to_owned()
                };
                ui.source.log(message);
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
        self.subtitles = subtitles::RemoteSubtitles::default();
    }
    fn map_installed(
        &mut self,
        game: &mut hl2_bevy::gameplay::Gameplay,
        vfs: &std::sync::Arc<source_assets::vpk::Vfs>,
    ) {
        self.overlay = Overlay::default();
        self.loading_status = None;
        self.overlay
            .set_usable(hl2_bevy::gameplay::spawn_usable_weapons(
                &game.world,
                &game.ai,
            ));
        // Resolving every SPAWN_CLASS model off-thread avoids blocking the map-adopt frame.
        let usable_listing = PropListing::default();
        self.usable_weapons = Some(usable_listing.clone());
        let vfs_usable = vfs.clone();
        let map_name = game.world.name.clone();
        std::thread::spawn(move || {
            let _ = usable_listing.set(resolve_spawn_usable_weapons(&vfs_usable, &map_name));
        });
        // Listing every archive is too slow for the frame that adopts the map.
        let listing = PropListing::default();
        self.props = Some(listing.clone());
        let vfs = vfs.clone();
        std::thread::spawn(move || {
            let _ = listing.set(vfs.physics_prop_models());
        });
    }
    fn startup(&mut self, config: &hl2_ui::config::Config) {
        if let Err(error) = self.start_agent_bootstrap() {
            bevy::log::warn!("Could not start automatic agent attachment: {error}");
        }
        self.dev_overlay = crate::overlay::shown(config);
        self.force_default_blur = crate::standard_background::forced(&config.video);
    }
    fn level_target(&mut self, map: &str, background: bool, widescreen: bool) {
        self.init_background_target(map, background, widescreen);
    }
    fn config_changed(&mut self, context: &mut Context<'_>, config: &hl2_ui::config::Config) {
        let force = crate::standard_background::forced(&config.video);
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
        profiles: &dyn hl2_ui::extensions::FramingProfiles,
        weapon: &str,
        aspect: f32,
    ) -> Option<modkit_core::viewmodel::ViewmodelFraming> {
        let debug = &self.viewmodel_debug;
        if !debug.enabled {
            return None;
        }
        let profile = profiles.profile_at(aspect);
        let value = debug
            .edits
            .get(&(weapon.to_owned(), profile))
            .copied()
            .or_else(|| {
                (debug.weapon == weapon && debug.profile == Some(profile)).then_some(debug.framing)
            })?;
        Some(profiles.blended(weapon, aspect, Some((profile, value))))
    }
    fn projection(&self, sim: &hl2_bevy::simulation::Simulation, aspect: f32) -> Option<f32> {
        self.background_debug.enabled.then(|| {
            self.background_debug
                .fov(hl2_bevy::camera::background_fov(aspect, sim.standard_background).to_degrees())
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
            serde_json::json!({"spawn_overlay":self.overlay.open.then(||{let (ai,notarget)=self.panel_switches.unwrap_or((true,false));self.overlay.panel(ai,notarget)}),"developer_overlay":{"visible":!self.lines.is_empty(),"fps":hud.frame_statistics().0,"frame_ms":hud.frame_statistics().1,"benchmark":hud.frame_statistics().2,"lines":self.lines,"profiling":self.profiling_lines}}),
        )
    }
    fn gameplay_report(&self) -> Option<(&'static str, serde_json::Value)> {
        Some((
            "spawn_overlay",
            serde_json::to_value(&self.overlay).expect("overlay report"),
        ))
    }
}
