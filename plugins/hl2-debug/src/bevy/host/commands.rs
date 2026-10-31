use super::State;
use crate::viewmodel::ViewmodelDebug;
impl State {
    pub(super) fn viewmodel_debug_mode(
        &mut self,
        sim: &mut hl2_bevy::simulation::Simulation,
        ui: &mut hl2_bevy::ui_host::Console,
        on: bool,
    ) {
        if !on {
            {
                self.viewmodel_debug = ViewmodelDebug::default();
            }
            ui.source.log("viewmodel_debug_mode 0");
        } else if sim.loading || sim.background_level {
            ui.source
                .log("viewmodel_debug_mode needs a loaded gameplay map; start or load one first.");
        } else {
            self.background_debug.enabled = false;
            ui.source.presentation.capture_menu_navigation = false;
            self.viewmodel_debug.enabled = true;
            ui.source.log("viewmodel_debug_mode 1");
        }
    }
    pub(super) fn viewmodel_tweak_reset(
        &mut self,
        _sim: &mut hl2_bevy::simulation::Simulation,
        ui: &mut hl2_bevy::ui_host::Console,
    ) {
        if self.viewmodel_debug.enabled {
            self.viewmodel_debug.reset_pending = true;
            ui.source.log("viewmodel_tweak_reset");
        } else {
            ui.source
                .log("viewmodel_tweak_reset needs viewmodel_debug_mode 1.");
        }
    }
    pub(super) fn apply_background_debug_effect(
        &mut self,
        sim: &mut hl2_bevy::simulation::Simulation,
        ui: &mut hl2_bevy::ui_host::Console,
        effect: crate::commands::Request,
    ) {
        use crate::commands::Request as Effect;
        let aspect = sim
            .frame_size
            .map_or(4. / 3., |s| s[0] as f32 / s[1].max(1) as f32);
        match effect {
            Effect::BackgroundDebugMode(on) => {
                if !on {
                    self.background_debug.enabled = false;
                    self.background_debug.fov = None;
                    ui.source.presentation.capture_menu_navigation = false;
                    ui.source.log("background_debug_mode 0");
                } else {
                    let can_enable =
                        sim.background_level || sim.loading || ui.source.has_background_context();
                    if !can_enable {
                        ui.source.log(
                            "background_debug_mode needs a background map or loading blur; start or return to the main menu.",
                        );
                    } else {
                        {
                            self.viewmodel_debug = ViewmodelDebug::default();
                        }
                        self.background_debug.enabled = true;
                        ui.source.presentation.capture_menu_navigation = true;
                        let is_wide = ui.source.is_widescreen();
                        if let Some((_, map)) = ui.source.current_background() {
                            self.background_debug.map_name = map.to_owned();
                            self.background_debug.blur_texture_name =
                                ui.source.blur_texture_name(map, is_wide);
                        } else if let Some(bg) = ui.source.background() {
                            self.background_debug.map_name = bg.map.clone();
                            self.background_debug.blur_texture_name =
                                ui.source.blur_texture_name(&bg.map, is_wide);
                        }
                        ui.source.log("background_debug_mode 1");
                    }
                }
            }
            Effect::BackgroundBlurOpacity(opacity) => {
                self.background_debug.blur_opacity = opacity;
                self.background_debug.base_blur_opacity = opacity;
                ui.source.presentation.background_opacity = Some(opacity);
            }
            Effect::BackgroundMapFov(Some(deg)) => {
                self.background_debug.fov = Some(deg);
                ui.source.log(format!("background_map_fov {deg:.2}"));
            }
            Effect::BackgroundMapFov(None) => {
                let fov = self.background_debug.fov(
                    hl2_bevy::camera::background_fov(aspect, sim.background_force_default_blur)
                        .to_degrees(),
                );
                self.background_debug.fov = None;
                ui.source
                    .log(format!("\"background_map_fov\" = \"{fov:.2}\""));
            }
            _ => {}
        }
    }
    pub(super) fn spawn_overlay_key(
        &mut self,
        sim: &mut hl2_bevy::simulation::Simulation,
        game: &mut hl2_bevy::gameplay::Gameplay,
        key: crate::spawn::OverlayKey,
    ) -> Option<String> {
        let command = self.overlay.key(key);
        self.spawn_overlay_command(sim, game, command)
    }
    pub(super) fn spawn_overlay_command(
        &mut self,
        sim: &mut hl2_bevy::simulation::Simulation,
        game: &mut hl2_bevy::gameplay::Gameplay,
        command: Option<crate::spawn::OverlayCommand>,
    ) -> Option<String> {
        use crate::spawn::OverlayCommand;
        match command? {
            OverlayCommand::Spawn(request) => Some(sim.spawn_npc(game, request)),
            OverlayCommand::SpawnProp(model) => {
                let model = hl2_bevy::runtime_props::model_path(&model);
                let message = format!("prop_physics_create {model}");
                game.prop_requests.push((
                    model,
                    glam::Vec3::from_array(sim.eye().to_array()),
                    sim.forward(),
                ));
                Some(message)
            }
            OverlayCommand::ToggleNotarget => {
                game.ai.ignore_player = !game.ai.ignore_player;
                None
            }
            OverlayCommand::ToggleFriendlyFire => {
                let on = !game.scene.player_hurts_friendlies;
                game.scene.player_hurts_friendlies = on;
                self.overlay.friendly_fire = on;
                None
            }
            OverlayCommand::ToggleAi => {
                let on = !game.ai.enabled;
                game.ai.enabled = on;
                hl2_bevy::gameplay::AI_ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
                None
            }
        }
    }
    pub(super) fn init_background_target(&mut self, map: &str, background: bool, is_wide: bool) {
        if background {
            let base = hl2_ui::gameui::resources::blur(map);
            self.background_debug.map_name = map.to_owned();
            self.background_debug.blur_texture_name = if is_wide {
                format!("{base}_widescreen")
            } else {
                base.to_owned()
            };
        }
    }
}
