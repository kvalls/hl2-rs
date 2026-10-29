//! Host simulation loop, player physics integration, command scripting, and console cheat execution.
use crate::source_direction;
use bevy::{app::RunFixedMainLoopSystems, prelude::*};
pub(crate) mod commentary;
pub(crate) mod movie;
pub(crate) mod script;
mod view;
use hl2_simulation::physics::Physics;
use modkit_core::{
    World,
    movement::{Input, Player, TICK},
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct ViewmodelDebug {
    pub enabled: bool,
    pub weapon: String,
    pub profile: Option<hl2_ui::tuning::ViewmodelProfile>,
    pub framing: hl2_ui::tuning::ViewmodelFraming,
    /// `viewmodel_tweak_reset` ran; `controls` zeroes the selected profile once it is selected.
    pub reset_pending: bool,
    /// Edits retained for the current tweak-mode session, keyed by weapon classname and direct profile.
    pub edits:
        BTreeMap<(String, hl2_ui::tuning::ViewmodelProfile), hl2_ui::tuning::ViewmodelFraming>,
}

pub use script::{Command, read_script};
#[derive(Serialize)]
struct Sample {
    label: String,
    host_tick: u64,
    paused: bool,
    fly: bool,
    player: Player,
    gameplay: Option<serde_json::Value>,
    ray_entity: Option<usize>,
    /// Colliders overlapping the player hull (entity, dynamic).
    hull_contacts: Vec<(Option<usize>, bool, f32, glam::Vec3)>,
    /// The player VPhysics shadow's center.
    shadow: Option<glam::Vec3>,
    /// Wall-clock milliseconds of the last physics step.
    physics_ms: f64,
    /// Wall-clock milliseconds of the last whole fixed host step (gameplay, NPCs,
    /// projectiles, physics and player).
    step_ms: f64,
    /// Awake dynamic bodies after the step (props that never sleep at idle).
    awake_bodies: usize,
    console: Option<serde_json::Value>,
    /// Size of the game frame image rendered at the time (`frame_size`).
    frame: Option<[u32; 2]>,
    /// Vertical field of view of the world and viewmodel cameras at the time (`camera_fov`).
    camera: Option<[f32; 2]>,
}
#[derive(Resource)]
pub struct Simulation {
    /// Wall-clock milliseconds of the last whole fixed step (test reports).
    last_step_ms: f64,
    pub player: Player,
    pub physics: Physics,
    eye: glam::Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Scripted horizontal 4:3 field of view in degrees (default 75).
    pub fov: f32,
    pub(crate) input: Input,
    pub fly: bool,
    pub paused: bool,
    /// The loaded map is a main-menu background level (from the console, every frame): like
    /// retail, its world keeps running behind the menu while player input is ignored.
    pub background_level: bool,
    /// The active point_viewcontrol's view (from the latest world tick); the player keeps
    /// its own eye, which still selects the soundscape (SDK `EarPosition`).
    pub view_control: Option<hl2_simulation::view_control::View>,
    /// The env_zoom field of view (from the latest world tick), overriding `fov`.
    pub zoom_fov: Option<f32>,
    pub loading: bool,
    pub focused: bool,
    pub jump_suppressed: bool,
    pub transition: bool,
    pub commands: Vec<Command>,
    next: usize,
    host_tick: u64,
    samples: Vec<Sample>,
    pub finished: bool,
    /// Keep simulating with idle input after the script ends (`--capture-live` bursts).
    pub run_after_script: bool,
    pub dev_overlay: bool,
    /// Runtime-only viewmodel tuning overlay and active direct-profile edit.
    pub viewmodel_debug: ViewmodelDebug,
    /// Runtime background layout visual analysis overlay and FOV tuning.
    pub background_debug: crate::background_debug::BackgroundDebug,
    pub pending_video_config: Option<hl2_ui::config::VideoSettings>,
    /// Window settings last given to the window: startup values (after command-line overrides)
    /// or the last queued apply. `None` applies the next configuration unconditionally.
    pub applied_window: Option<WindowSettings>,
    /// Detail settings last applied: model and texture quality, and whether the frame is HDR.
    pub applied_detail: Option<hl2_ui::config::LoadDetail>,
    /// Size of the game frame image being rendered, kept by `video::record_frame_size` (reports
    /// show whether a resolution change was carried out).
    pub frame_size: Option<[u32; 2]>,
    /// Vertical field of view (degrees) of the world and viewmodel cameras, kept by
    /// `camera::present_player_view` (reports show what the cameras use).
    pub camera_fov: Option<[f32; 2]>,
    /// `video.vertical_fov` (0 = automatic) and `video.viewmodel_fov` of the applied settings.
    pub vertical_fov: f32,
    pub viewmodel_fov: f32,
    /// PlayerDeathThink: every button must be released before one press respawns.
    death_buttons_released: bool,
    /// `impulse 100` presses and commands since the flashlight last read them.
    pub flashlight_toggles: u32,
}
impl Simulation {
    pub fn new(
        world: &World,
        eye: glam::Vec3,
        yaw: f32,
        pitch: f32,
        fly: bool,
        commands: Vec<Command>,
    ) -> Self {
        Self {
            last_step_ms: 0.,
            player: Player::new(eye),
            physics: Physics::new(world),
            eye,
            yaw,
            pitch,
            fov: 75.,
            input: Input::default(),
            fly,
            paused: false,
            background_level: false,
            view_control: None,
            zoom_fov: None,
            loading: false,
            focused: true,
            jump_suppressed: false,
            transition: false,
            commands,
            next: 0,
            host_tick: 0,
            samples: vec![],
            finished: false,
            run_after_script: false,
            dev_overlay: false,
            viewmodel_debug: ViewmodelDebug::default(),
            background_debug: crate::background_debug::BackgroundDebug::default(),
            pending_video_config: None,
            applied_window: None,
            applied_detail: None,
            frame_size: None,
            camera_fov: None,
            vertical_fov: 0.,
            viewmodel_fov: hl2_simulation::viewmodel_effects::VIEWMODEL_FOV,
            death_buttons_released: true,
            flashlight_toggles: 0,
        }
    }
    /// The player's eye.
    pub fn eye(&self) -> Vec3 {
        Vec3::from_array(self.eye.to_array())
    }
    /// Where the view is: the active point_viewcontrol, else the player's eye.
    pub fn view_origin(&self) -> Vec3 {
        match self.view_control {
            Some(view) => Vec3::from_array(view.origin.to_array()),
            None => self.eye(),
        }
    }
    /// View yaw and pitch in radians (pitch positive looks up), like `yaw`/`pitch`.
    pub fn view_angles(&self) -> (f32, f32) {
        match self.view_control {
            Some(view) => (view.yaw.to_radians(), -view.pitch.to_radians()),
            None => (self.yaw, self.pitch),
        }
    }
    /// Horizontal 4:3 field of view of the view: env_zoom's, else `player_fov` (the player's
    /// own, including weapon zoom).
    pub fn view_fov(&self, player_fov: f32) -> f32 {
        self.zoom_fov.unwrap_or(player_fov)
    }
    /// The current viewmodel framing, including the selected direct profile while viewmodel tweak mode is on.
    pub fn viewmodel_framing(&self, weapon: &str, aspect: f32) -> hl2_ui::tuning::ViewmodelFraming {
        let profile = hl2_ui::tuning::viewmodel_profile(aspect);
        if self.viewmodel_debug.enabled
            && let Some(framing) = self
                .viewmodel_debug
                .edits
                .iter()
                .find(|((edited_weapon, edited_profile), _)| {
                    edited_weapon == weapon && *edited_profile == profile
                })
                .map(|(_, framing)| *framing)
                .or_else(|| {
                    (self.viewmodel_debug.weapon == weapon
                        && self.viewmodel_debug.profile == Some(profile))
                    .then_some(self.viewmodel_debug.framing)
                })
        {
            hl2_ui::tuning::viewmodel_framing_with_profile(weapon, aspect, profile, framing)
        } else {
            hl2_ui::tuning::viewmodel_framing(weapon, aspect)
        }
    }
    /// Whether the view is the player's own (no point_viewcontrol camera).
    pub fn player_view(&self) -> bool {
        self.view_control.is_none()
    }
    pub fn flying(&self) -> bool {
        self.fly
    }
    pub fn paused(&self) -> bool {
        self.paused || self.loading
    }
    /// Whether the world (entities, sounds, effects, the player) advances: in gameplay, and
    /// behind the menu of a background level, never while a map loads.
    pub fn world_running(&self) -> bool {
        !self.loading && (!self.paused || self.background_level)
    }
    /// The player takes no input: a menu background level, or a point_viewcontrol with
    /// `SF_CAMERA_PLAYER_TAKECONTROL`.
    fn frozen(&self) -> bool {
        self.background_level || self.view_control.is_some_and(|v| v.freeze_player)
    }
    pub fn loading(&self) -> bool {
        self.loading
    }
    pub fn loading_failed(&mut self) {
        self.loading = false;
        self.transition = true;
    }
    pub fn apply_startup_settings(
        &mut self,
        options: &crate::Options,
        target: &crate::startup::StartupTarget,
    ) {
        self.fov = options.config.video.fov;
        self.vertical_fov = options.config.video.vertical_fov;
        self.viewmodel_fov = options.config.video.viewmodel_fov;
        self.applied_window = Some(
            WindowSettings::new(options.width, options.height, options.display_mode)
                .with_flexible_aspect(options.flexible_aspect),
        );
        self.run_after_script = options.capture_live;
        self.loading = true;
        self.background_level = target.background;
        let is_wide = options.width as f32 / (options.height as f32).max(1.0) > 4. / 3. + 1e-3;
        self.init_background_target(target, is_wide);
    }
    pub fn init_background_target(
        &mut self,
        target: &crate::startup::StartupTarget,
        is_wide: bool,
    ) {
        if target.background {
            let base = hl2_ui::gameui::resources::blur(&target.map);
            self.background_debug.map_name = target.map.clone();
            self.background_debug.blur_texture_name = if is_wide {
                format!("{base}_widescreen")
            } else {
                base.to_owned()
            };
        }
    }
    pub fn change_map(&mut self, world: &World, eye: glam::Vec3, direct: bool) {
        self.physics = Physics::new(world);
        self.eye = eye;
        self.player = Player::new(eye);
        self.view_control = None;
        self.zoom_fov = None;
        self.viewmodel_debug = ViewmodelDebug::default();
        if direct {
            self.fly = false;
            self.yaw = world.spawn().1;
            self.pitch = 0.;
            self.paused = false;
        }
        self.loading = false;
        self.transition = true;
        self.input = Input::default();
        self.jump_suppressed = true;
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"player": self.player, "eye": self.eye.to_array(), "fly": self.fly,
            "paused": self.paused(), "background_level": self.background_level, "world_running": self.world_running(), "dev_overlay":self.dev_overlay,
            "viewmodel_debug": {"visible": self.viewmodel_debug.enabled, "weapon": self.viewmodel_debug.weapon,
                "profile": self.viewmodel_debug.profile.map(|profile| profile.as_str()),
                "fov_offset": self.viewmodel_debug.framing.fov_offset, "offset": self.viewmodel_debug.framing.offset},
            "dead": self.player.dead, "loading":self.loading,"host_tick": self.host_tick, "script_finished": self.finished,
            "samples": self.samples, "colliders": self.physics.colliders.len(),
            "native_convex_shapes": self.physics.native_shape_count, "native_shape_fallbacks": self.physics.native_shape_fallbacks,
            "skipped_colliders": self.physics.skipped, "dynamic_props": self.physics.dynamic.len()})
    }
    fn request_gameplay_map(
        &mut self,
        ui: &mut crate::ui_host::Console,
        map: String,
        game: Option<String>,
    ) {
        ui.map_request = Some(map);
        ui.map_request_game = game;
        ui.background_request = false;
        self.loading = true;
    }
    pub(crate) fn set_fly(&mut self, fly: bool) {
        if self.fly != fly {
            self.fly = fly;
            self.player = Player::new(self.eye);
        }
    }
    /// `viewmodel_tweak_mode`: only while a gameplay map is loaded.
    fn viewmodel_tweak_mode(&mut self, ui: &mut crate::ui_host::Console, on: bool) {
        if !on {
            self.viewmodel_debug = ViewmodelDebug::default();
            ui.source.log("viewmodel_tweak_mode 0");
        } else if self.loading || self.background_level {
            ui.source
                .log("viewmodel_tweak_mode needs a loaded gameplay map; start or load one first.");
        } else {
            self.background_debug.enabled = false;
            ui.source.background_debug_mode = false;
            self.viewmodel_debug.enabled = true;
            ui.source.log("viewmodel_tweak_mode 1");
        }
    }
    /// `viewmodel_tweak_reset`: only while `viewmodel_tweak_mode` is on.
    fn viewmodel_tweak_reset(&mut self, ui: &mut crate::ui_host::Console) {
        if self.viewmodel_debug.enabled {
            self.viewmodel_debug.reset_pending = true;
            ui.source.log("viewmodel_tweak_reset");
        } else {
            ui.source
                .log("viewmodel_tweak_reset needs viewmodel_tweak_mode 1.");
        }
    }
    fn apply_background_debug_effect(
        &mut self,
        ui: &mut crate::ui_host::Console,
        effect: hl2_ui::console::Effect,
    ) {
        use hl2_ui::console::Effect;
        let aspect = self
            .frame_size
            .map_or(4. / 3., |s| s[0] as f32 / s[1].max(1) as f32);
        match effect {
            Effect::BackgroundDebugMode(on) => {
                if !on {
                    self.background_debug.enabled = false;
                    self.background_debug.fov = None;
                    ui.source.background_debug_mode = false;
                    ui.source.log("background_debug_mode 0");
                } else {
                    let can_enable = self.background_level
                        || self.loading
                        || ui.source.can_enable_background_debug();
                    if !can_enable {
                        ui.source.log(
                            "background_debug_mode needs a background map or loading blur; start or return to the main menu.",
                        );
                    } else {
                        self.viewmodel_debug = ViewmodelDebug::default();
                        self.background_debug.enabled = true;
                        ui.source.background_debug_mode = true;
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
                ui.source.background_blur_opacity = opacity;
            }
            Effect::BackgroundMapFov(Some(deg)) => {
                self.background_debug.fov = Some(deg);
                ui.source.log(format!("background_map_fov {deg:.2}"));
            }
            Effect::BackgroundMapFov(None) => {
                let fov = self.background_debug.fov(aspect);
                self.background_debug.fov = None;
                ui.source
                    .log(format!("\"background_map_fov\" = \"{fov:.2}\""));
            }
            _ => {}
        }
    }

    fn apply_pose_effect(
        &mut self,
        ui: &mut crate::ui_host::Console,
        effect: hl2_ui::console::Effect,
    ) {
        use hl2_ui::console::Effect;
        match effect {
            Effect::Getpos => ui.source.log(format!(
                "setpos {:.6} {:.6} {:.6}; setang {:.6} {:.6} 0",
                self.eye.x,
                self.eye.y,
                self.eye.z,
                -self.pitch.to_degrees(),
                self.yaw.to_degrees()
            )),
            Effect::Setpos { x, y, z } => {
                self.player.feet = glam::Vec3::new(x, y, z.unwrap_or(self.player.feet.z));
                self.eye = self.player.eye();
                ui.source.log(format!(
                    "Player origin: {} {} {}",
                    self.player.feet.x, self.player.feet.y, self.player.feet.z
                ));
            }
            Effect::Setang(a) => {
                self.pitch = -a.x.to_radians().clamp(-1.53, 1.53);
                self.yaw = a.y.to_radians();
                ui.source.log(format!(
                    "View angles: {} {} 0",
                    -self.pitch.to_degrees(),
                    self.yaw.to_degrees()
                ));
            }
            _ => {}
        }
    }

    fn apply_npc_command(
        &mut self,
        ui: &mut crate::ui_host::Console,
        game: &mut crate::gameplay::Gameplay,
        line: &str,
    ) {
        match game.spawn_console.parse(line) {
            Ok(Some(hl2_simulation::ai::spawn::Command::Spawn(request))) => {
                let message = self.spawn_npc(game, request);
                ui.source.log(message);
            }
            Ok(Some(hl2_simulation::ai::spawn::Command::SetEquipment(value))) => {
                ui.source.log(format!("npc_create_equipment {value}"));
            }
            Ok(Some(hl2_simulation::ai::spawn::Command::SpawnProp(model))) => {
                let model = crate::runtime_props::model_path(&model);
                ui.source.log(format!("prop_physics_create {model}"));
                game.prop_requests.push((model, self.eye, self.forward()));
            }
            Ok(None) => {}
            Err(e) => ui.source.log(e),
        }
    }

    pub fn console_effects(
        &mut self,
        ui: &mut crate::ui_host::Console,
        game: &mut crate::gameplay::Gameplay,
        effects: Vec<hl2_ui::console::Effect>,
    ) {
        use hl2_ui::console::Effect;
        for effect in effects {
            match effect {
                Effect::Loadout => grant_loadout(ui, game),
                Effect::AiEnable(value) => {
                    let on = set_ai(game, value.unwrap_or(!game.ai.enabled));
                    ui.source.log(format!("ai_enable {}", u8::from(on)));
                }
                // CC_AI_Disable: a toggle; DevMsg "AI Disabled." / "AI Enabled.".
                Effect::AiDisable => {
                    let on = set_ai(game, !game.ai.enabled);
                    ui.source
                        .log(if on { "AI Enabled." } else { "AI Disabled." });
                }
                Effect::Noclip(value) => {
                    self.fly = value.unwrap_or(!self.fly);
                    self.player = Player::new(self.eye);
                    ui.source.log(format!("noclip {}", u8::from(self.fly)));
                }
                Effect::ViewmodelTweakMode(on) => self.viewmodel_tweak_mode(ui, on),
                Effect::Flashlight => self.flashlight_toggles += 1,
                Effect::ViewmodelTweakReset => self.viewmodel_tweak_reset(ui),
                Effect::BackgroundDebugMode(_)
                | Effect::BackgroundBlurOpacity(_)
                | Effect::BackgroundMapFov(_) => self.apply_background_debug_effect(ui, effect),
                Effect::Getpos | Effect::Setpos { .. } | Effect::Setang(_) => {
                    self.apply_pose_effect(ui, effect)
                }
                Effect::ChapterMap(target) => {
                    self.request_gameplay_map(ui, target.map, Some(target.game))
                }
                Effect::Map(map) => self.request_gameplay_map(ui, map, None),
                Effect::MainMenuMap(background) => {
                    if ui.map_request.is_some() && !ui.background_request {
                        ui.source
                            .log("Gameplay map load in progress; ignoring background map request");
                    } else {
                        let is_wide = ui.source.is_widescreen();
                        let blur_name = ui.source.blur_texture_name(&background.map, is_wide);
                        self.background_debug.map_name = background.map.clone();
                        self.background_debug.blur_texture_name = blur_name;
                        ui.map_request = Some(background.map);
                        ui.map_request_game = Some(background.game);
                        ui.background_request = true;
                        self.loading = true;
                    }
                }
                Effect::Fire {
                    target,
                    input,
                    parameter,
                    delay,
                } => {
                    if game.scene.send_named(&target, &input, &parameter, delay) {
                        ui.source
                            .log(format!("Queued {target}.{input} after {delay} seconds."));
                    }
                }
                Effect::NpcCommand(line) => self.apply_npc_command(ui, game, &line),
                Effect::Quit => ui.quit_requested = true,
                Effect::ApplyConfig(cfg) => self.apply_config_effect(ui, game, &cfg),
                Effect::PlaySound(sound) => {
                    game.sound_requests
                        .push(hl2_simulation::sounds::SoundRequest::ui(sound));
                }
                Effect::PlayVideo {
                    filename,
                    exit_command,
                    width,
                    height,
                } => {
                    movie::play_video(self, ui, game, filename, exit_command, width, height);
                }
            }
        }
    }

    fn apply_config_effect(
        &mut self,
        ui: &mut crate::ui_host::Console,
        game: &mut crate::gameplay::Gameplay,
        cfg: &hl2_ui::config::Config,
    ) {
        self.fov = cfg.video.fov;
        self.vertical_fov = cfg.video.vertical_fov;
        self.viewmodel_fov = cfg.video.viewmodel_fov;
        let target = WindowSettings::of(&cfg.video);
        if self.applied_window != Some(target) {
            self.applied_window = Some(target);
            self.pending_video_config = Some(cfg.video.clone());
        }
        let detail = cfg.video.load_detail();
        let detail_changed = self.applied_detail.is_some_and(|prev| prev != detail);
        self.applied_detail = Some(detail);
        ui.source.config = cfg.clone();
        if detail_changed && !self.loading {
            let map = game.world.name.clone();
            if !map.is_empty() {
                ui.map_request = Some(map);
                ui.background_request = ui.source.background_level();
                self.loading = true;
            }
        }
    }
    /// One F4 overlay key; a spawn returns its console message.
    pub fn spawn_overlay_key(
        &mut self,
        game: &mut crate::gameplay::Gameplay,
        key: hl2_simulation::ai::spawn::OverlayKey,
    ) -> Option<String> {
        let command = game.spawn_overlay.key(key);
        self.spawn_overlay_command(game, command)
    }
    /// Carry out what an F4 overlay key or click asked; a spawn returns its console message.
    pub fn spawn_overlay_command(
        &mut self,
        game: &mut crate::gameplay::Gameplay,
        command: Option<hl2_simulation::ai::spawn::OverlayCommand>,
    ) -> Option<String> {
        use hl2_simulation::ai::spawn::OverlayCommand;
        match command? {
            OverlayCommand::Spawn(request) => Some(self.spawn_npc(game, request)),
            OverlayCommand::SpawnProp(model) => {
                let model = crate::runtime_props::model_path(&model);
                let message = format!("prop_physics_create {model}");
                game.prop_requests.push((model, self.eye, self.forward()));
                Some(message)
            }
            OverlayCommand::ToggleNotarget => {
                game.ai.ignore_player = !game.ai.ignore_player;
                None
            }
            OverlayCommand::ToggleFriendlyFire => {
                let on = !game.scene.player_hurts_friendlies;
                game.scene.player_hurts_friendlies = on;
                game.spawn_overlay.friendly_fire = on;
                None
            }
            OverlayCommand::ToggleAi => {
                let on = !game.ai.enabled;
                game.ai.enabled = on;
                crate::gameplay::AI_ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
                None
            }
        }
    }
    /// The view direction (Source axes).
    fn forward(&self) -> glam::Vec3 {
        glam::Vec3::new(
            self.pitch.cos() * self.yaw.cos(),
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
        )
    }
    /// npc_create from the player's eye along the aim (a new runtime entity).
    pub fn spawn_npc(
        &mut self,
        game: &mut crate::gameplay::Gameplay,
        request: hl2_simulation::ai::spawn::SpawnRequest,
    ) -> String {
        let forward = glam::Vec3::new(
            self.pitch.cos() * self.yaw.cos(),
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
        );
        game.request_spawn(request, &mut self.physics, self.eye, forward)
    }
    fn apply_script_command(
        &mut self,
        c: Command,
        mut game: Option<&mut crate::gameplay::Gameplay>,
        ui: Option<&mut crate::ui_host::Console>,
    ) -> Option<String> {
        if let Some(eye) = c.eye {
            self.eye = glam::Vec3::from_array(eye);
            self.player = Player::new(self.eye);
        }
        if let Some(yaw) = c.yaw {
            self.yaw = yaw.to_radians();
        }
        if let Some(pitch) = c.pitch {
            self.pitch = pitch.to_radians();
        }
        if let Some(fov) = c.fov {
            self.fov = fov;
        }
        if let Some(fly) = c.fly {
            self.set_fly(fly);
        }
        if let Some(enabled) = c.dev_overlay {
            self.dev_overlay = enabled;
        }
        if let Some(paused) = c.paused {
            self.paused = paused;
        }
        self.input = Input {
            forward: c.forward,
            side: c.side,
            yaw: self.yaw,
            jump: c.jump,
            crouch: c.crouch,
            sprint: c.sprint,
            slow: c.slow,
            max_speed: None,
            surface: Default::default(),
        };
        if self.background_level || self.frozen() {
            // Retail ignores player input on a menu background level and under a
            // point_viewcontrol that takes control (SDK `EnableControl(FALSE)`).
            self.input = Input {
                yaw: self.yaw,
                ..Input::default()
            };
            if let Some(game) = game.as_deref_mut() {
                if game.scene.commentary.enabled {
                    game.buttons(c.primary, c.secondary);
                    game.actions.extend(c.actions);
                } else {
                    game.actions
                        .extend(c.actions.into_iter().filter(|a| a.entity_setup()));
                }
            }
        } else if let Some(game) = game.as_deref_mut() {
            game.buttons(c.primary, c.secondary);
            game.actions.extend(c.actions);
        }
        if let (Some(ui), Some(game)) = (ui, game) {
            let before = ui.source.mode();
            for action in &c.ui {
                self.ui_action(ui, game, action);
            }
            if ui.source.mode() != before {
                self.paused = ui.source.paused();
                self.transition = true;
                self.jump_suppressed |= c.jump;
                game.consume_attacks();
            }
        }
        if !c.jump {
            self.jump_suppressed = false;
        }
        self.input.jump &= !self.jump_suppressed;
        self.next += 1;
        c.label
    }

    pub(crate) fn step(
        &mut self,
        mut game: Option<&mut crate::gameplay::Gameplay>,
        mut ui: Option<&mut crate::ui_host::Console>,
    ) {
        if (self.finished && !self.run_after_script) || self.loading {
            return;
        }
        if !self.commands.is_empty() {
            self.transition = false;
        }
        if let Some(ui) = ui.as_deref() {
            self.background_level = ui.source.background_level();
        }
        self.host_tick += 1;
        let mut label = None;
        if let Some(c) = self
            .commands
            .get(self.next)
            .filter(|c| c.tick == self.host_tick)
            .cloned()
        {
            label = self.apply_script_command(c, game.as_deref_mut(), ui.as_deref_mut());
        }
        if (self.paused() || self.transition)
            && let Some(game) = game.as_deref_mut()
        {
            game.consume_attacks();
        }
        if self.world_running() && (self.focused || !self.commands.is_empty()) && !self.transition {
            if let Some(game) = game.as_deref_mut() {
                commentary::before_gameplay_tick(self, game, ui.as_deref_mut());
            }
            if let Some(game) = game.as_deref_mut() {
                let direction =
                    glam::Vec3::from_array(source_direction(self.yaw, self.pitch).to_array());
                // GetDefaultFOV for SetFOV ramps (the `fov` command / fixture FOV).
                game.inventory.fov.default_fov = self.fov;
                game.tick(
                    &mut self.physics,
                    &self.player,
                    self.eye,
                    direction,
                    self.fly,
                );
            }
            if self.fly {
                let forward =
                    glam::Vec3::from_array(source_direction(self.yaw, self.pitch).to_array());
                let right = glam::Vec3::new(self.yaw.sin(), -self.yaw.cos(), 0.);
                let vertical = f32::from(self.input.jump) - f32::from(self.input.crouch);
                let direction = (forward * self.input.forward
                    + right * self.input.side
                    + glam::Vec3::Z * vertical)
                    .normalize_or_zero();
                self.eye += direction * if self.input.sprint { 900. } else { 300. } * TICK;
            } else {
                let before = hl2_simulation::footsteps::State {
                    velocity: self.player.velocity,
                    grounded: self.player.grounded,
                    crouched: self.player.crouched,
                    noclip: false,
                };
                let feet = self.player.feet;
                // NPC hits push the player (ApplyAbsVelocityImpulse) before the move.
                if let Some(game) = game.as_deref_mut() {
                    self.player.velocity += std::mem::take(&mut game.player_impulse);
                }
                // SetMaxSpeed from the weapons stage (gravity gun load) for this step.
                self.input.max_speed = game
                    .as_deref()
                    .and_then(|game| game.inventory.player_max_speed());
                self.input.surface = game
                    .as_deref()
                    .filter(|_| self.player.grounded)
                    .map(|game| game.ground_surface(&self.physics, feet))
                    .unwrap_or_default();
                // A scripted camera (point_viewcontrol) freezes the player in place.
                let input = if self.frozen() {
                    Input {
                        yaw: self.yaw,
                        ..Input::default()
                    }
                } else {
                    self.input
                };
                self.player.step(input, &self.physics, TICK);
                self.eye = self.player.eye();
                // The player's VPhysics shadow pushes and weighs down movable props.
                let held = game.as_deref().and_then(|game| {
                    game.inventory.physcannon.held.or(game
                        .inventory
                        .carry
                        .as_ref()
                        .map(|carry| carry.entity))
                });
                self.physics.update_player_shadow(
                    feet,
                    self.player.feet,
                    self.player.crouched,
                    self.player.grounded && !self.player.dead,
                    self.player.step_height,
                    self.player.wish_velocity,
                    held,
                    TICK,
                );
                if let Some(game) = game.as_deref_mut() {
                    commentary::finish_ground_step(self, game, before, feet);
                }
            }
            if let Some(game) = game.as_deref_mut() {
                commentary::after_gameplay_step(self, game);
            }
        }
        if self.player.dead && !self.paused() && !self.transition {
            let any_button = self.input.forward != 0.
                || self.input.side != 0.
                || self.input.jump
                || self.input.crouch
                || game.as_deref().is_some_and(|g| g.primary || g.secondary);
            if !self.death_buttons_released {
                self.death_buttons_released = !any_button;
            } else if any_button
                && let (Some(game), Some(ui)) = (game.as_deref(), ui.as_deref_mut())
            {
                // Singleplayer respawn() issues "reload": the last save, else the map again.
                // Saves are not implemented, so the current map restarts fresh.
                ui.map_request = Some(game.world.name.clone());
                self.loading = true;
            }
        }
        if game.as_ref().is_some_and(|g| g.scene.transition.is_some()) {
            self.loading = true;
        }
        if let Some(label) = label {
            self.samples.push(Sample {
                label,
                host_tick: self.host_tick,
                paused: self.paused,
                fly: self.fly,
                player: self.player.clone(),
                gameplay: game
                    .as_deref()
                    .map(|g| commentary::sample_report(g, &self.physics)),
                console: ui.as_deref().map(|u| u.report()),
                hull_contacts: self.physics.hull_contacts(
                    self.player.feet,
                    if self.player.crouched { 36. } else { 72. },
                ),
                shadow: self.physics.player_shadow_center(),
                physics_ms: self.physics.last_step_ms,
                step_ms: self.last_step_ms,
                awake_bodies: self.physics.awake_bodies(),
                frame: self.frame_size,
                camera: self.camera_fov,
                ray_entity: self
                    .physics
                    .ray(
                        self.eye,
                        glam::Vec3::from_array(source_direction(self.yaw, self.pitch).to_array()),
                        128.,
                    )
                    .map(|(id, _)| id),
            });
        }
        self.finished = !self.commands.is_empty() && self.next == self.commands.len();
    }
}
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MovementControlsSet;

pub struct MovementPlugin;
impl Plugin for MovementPlugin {
    fn build(&self, app: &mut App) {
        let mut virtual_time = Time::<Virtual>::default();
        virtual_time.set_max_delta(std::time::Duration::from_millis(50));
        app.insert_resource(virtual_time);
        app.insert_resource(Time::<Fixed>::from_duration(
            std::time::Duration::from_millis(15),
        ))
        .add_systems(
            RunFixedMainLoop,
            crate::input::controls
                .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
                .in_set(MovementControlsSet),
        )
        .add_systems(FixedUpdate, fixed_step)
        .add_systems(Update, commentary::update_commentary_skins)
        .add_systems(
            RunFixedMainLoop,
            crate::camera::present_player_view.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
        );
    }
}

fn fixed_step(
    mut sim: ResMut<Simulation>,
    mut game: Option<ResMut<crate::gameplay::Gameplay>>,
    mut ui: Option<ResMut<crate::ui_host::Console>>,
    mut exit: MessageWriter<AppExit>,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "fixed_simulation");
    let started = std::time::Instant::now();
    sim.step(game.as_deref_mut(), ui.as_deref_mut());
    sim.last_step_ms = started.elapsed().as_secs_f64() * 1000.;
    if ui.is_some_and(|u| u.quit_requested) {
        exit.write(AppExit::Success);
    }
}
/// `impulse 101`: the implemented weapons, ammunition and the suit.
fn grant_loadout(ui: &mut crate::ui_host::Console, game: &mut crate::gameplay::Gameplay) {
    for name in game.weapons.keys() {
        game.inventory.give(name, &game.weapons, game.scene.time);
    }
    game.inventory.refill_ammo(&game.weapons);
    game.inventory.suit = true;
    game.inventory
        .give("weapon_crowbar", &game.weapons, game.scene.time);
    ui.source.log("Granted the six implemented weapons, ammunition and suit. Remaining HL2 weapons are not implemented.");
}

/// The part of the video settings that the window carries: resolution and display mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
    pub display_mode: hl2_ui::config::DisplayMode,
    /// The game frame follows the window's aspect ratio (Video > Aspect Ratio "Fit Display").
    pub flexible_aspect: bool,
}

impl WindowSettings {
    pub fn new(width: u32, height: u32, display_mode: hl2_ui::config::DisplayMode) -> Self {
        Self {
            width,
            height,
            display_mode: display_mode.effective(),
            flexible_aspect: false,
        }
    }

    /// These settings with a flexible-aspect frame or not.
    pub fn with_flexible_aspect(self, flexible_aspect: bool) -> Self {
        Self {
            flexible_aspect,
            ..self
        }
    }

    pub fn of(video: &hl2_ui::config::VideoSettings) -> Self {
        Self::new(video.width, video.height, video.display_mode)
            .with_flexible_aspect(video.flexible_aspect)
    }
}

/// Turns the AI on or off; returns the new state.
fn set_ai(game: &mut crate::gameplay::Gameplay, on: bool) -> bool {
    game.ai.enabled = on;
    crate::gameplay::AI_ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
    on
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        ecs::system::RunSystemOnce,
        input::mouse::AccumulatedMouseMotion,
        window::{CursorGrabMode, CursorOptions, PrimaryWindow},
    };

    fn step_input(app: &mut App) {
        app.world_mut()
            .run_system_once(crate::ui_system::update_ui)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::input::controls)
            .unwrap();
    }

    #[test]
    fn capturing_discards_pointer_motion_and_consumes_held_jump_until_release() {
        let mut app = App::new();
        app.insert_resource(Simulation::new(
            &World::default(),
            glam::Vec3::new(0., 0., 128.),
            0.,
            0.,
            false,
            vec![],
        ))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .insert_resource(AccumulatedMouseMotion {
            delta: Vec2::new(100., 100.),
        })
        .add_message::<AppExit>()
        .add_message::<bevy::input::keyboard::KeyboardInput>()
        .add_message::<bevy::input::mouse::MouseWheel>();
        app.world_mut().spawn((
            Window {
                focused: true,
                ..default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        step_input(&mut app);
        let sim = app.world().resource::<Simulation>();
        assert!(sim.transition && sim.jump_suppressed);
        assert_eq!((sim.yaw, sim.pitch), (0., 0.));
        assert!(!sim.input.jump);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        step_input(&mut app);
        assert!(!app.world().resource::<Simulation>().input.jump);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Space);
        step_input(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        step_input(&mut app);
        assert!(app.world().resource::<Simulation>().input.jump);
    }
    #[test]
    fn ui_open_close_and_resume_consume_pointer_attack_and_held_jump() {
        use hl2_ui::console::Mode;
        let mut app = App::new();
        app.insert_resource(Simulation::new(
            &World::default(),
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            vec![],
        ))
        .insert_resource(crate::gameplay::Gameplay::synthetic(World::default()))
        .insert_resource(crate::ui_host::Console::new(Default::default()))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .insert_resource(AccumulatedMouseMotion {
            delta: Vec2::splat(100.),
        })
        .add_message::<AppExit>()
        .add_message::<bevy::input::keyboard::KeyboardInput>()
        .add_message::<bevy::input::mouse::MouseWheel>();
        app.world_mut().spawn((
            Window {
                focused: true,
                ..default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Backquote);
        step_input(&mut app);
        assert_eq!(
            app.world()
                .resource::<crate::ui_host::Console>()
                .source
                .mode(),
            Mode::Console
        );
        assert!(app.world().resource::<Simulation>().paused);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        step_input(&mut app);
        assert_eq!(
            app.world()
                .resource::<crate::ui_host::Console>()
                .source
                .mode(),
            Mode::Console,
            "the console closes when Escape is released"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Escape);
        step_input(&mut app);
        assert_eq!(
            app.world()
                .resource::<crate::ui_host::Console>()
                .source
                .mode(),
            Mode::Pause,
            "closing the console leaves the pause menu active"
        );
        assert!(app.world().resource::<Simulation>().paused);
        // Resuming from the pause menu via Escape returns to active gameplay
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        step_input(&mut app);
        let sim = app.world().resource::<Simulation>();
        assert!(!sim.paused && sim.transition && sim.jump_suppressed && !sim.input.jump);
        assert_eq!((sim.yaw, sim.pitch), (0., 0.));
        assert!(!app.world().resource::<crate::gameplay::Gameplay>().primary);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .resource_mut::<crate::ui_host::Console>()
            .source
            .set_mode(Mode::Pause);
        step_input(&mut app);
        assert!(app.world().resource::<Simulation>().paused);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        step_input(&mut app);
        assert!(
            app.world().resource::<Simulation>().paused,
            "click outside Resume cannot close the menu"
        );
        // No entry is armed until the keyboard or pointer selects one; Down arms Resume.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowDown);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        step_input(&mut app);
        assert!(app.world().resource::<Simulation>().transition);
        assert!(!app.world().resource::<Simulation>().paused);
    }
    #[test]
    fn console_setpos_preserves_velocity_and_getpos_uses_eye() {
        let mut sim = Simulation::new(
            &World::default(),
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            vec![],
        );
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());
        sim.player.velocity = glam::Vec3::new(1., 2., 3.);
        sim.ui_action(
            &mut ui,
            &mut game,
            &crate::ui_host::Action::Command {
                command:
                    "sv_cheats 1; setpos 10 20 30; setang 15 90; getpos; ent_fire missing Open"
                        .into(),
            },
        );
        assert_eq!(sim.player.feet, glam::Vec3::new(10., 20., 30.));
        assert_eq!(sim.player.velocity, glam::Vec3::new(1., 2., 3.));
        assert_eq!(sim.eye, glam::Vec3::new(10., 20., 94.));
        let position = ui
            .source
            .shell
            .panel
            .output
            .iter()
            .find(|s| s.starts_with("setpos 10.000000 20.000000 94.000000; setang "))
            .expect("camera origin log");
        let angles: Vec<_> = position
            .split("setang ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .collect();
        assert!((angles[0].parse::<f32>().unwrap() - 15.).abs() < 1e-4);
        assert!((angles[1].parse::<f32>().unwrap() - 90.).abs() < 1e-4);
    }
    #[test]
    fn pause_does_not_advance_player_and_resume_retains_fixed_tick() {
        let mut sim = Simulation::new(
            &World::default(),
            glam::Vec3::new(0., 0., 128.),
            0.,
            0.,
            false,
            vec![],
        );
        sim.step(None, None);
        let before = sim.player.clone();
        sim.paused = true;
        for _ in 0..10 {
            sim.step(None, None);
        }
        assert_eq!(sim.player.ticks, before.ticks);
        assert_eq!(sim.player.feet, before.feet);
        assert_eq!(sim.player.velocity, before.velocity);
        sim.paused = false;
        sim.step(None, None);
        assert_eq!(sim.player.ticks, before.ticks + 1);
    }
    #[test]
    fn script_ui_entries_keep_running_while_a_menu_dialog_pauses_the_world() {
        let script: Vec<Command> = serde_json::from_value(serde_json::json!([
            {"tick": 2, "label": "options-open"},
            {"tick": 5, "ui": [{"action": "command", "command": "sv_cheats 1"}], "label": "dialog-input"}
        ]))
        .unwrap();
        let mut ui = crate::ui_host::Console::new(Default::default());
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut sim = Simulation::new(
            &World::default(),
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            script,
        );
        ui.source.set_mode(hl2_ui::console::Mode::Options);
        sim.paused = ui.source.paused();
        for _ in 0..5 {
            sim.step(Some(&mut game), Some(&mut ui));
        }
        assert!(!sim.world_running());
        assert_eq!(sim.host_tick, 5);
        assert!(sim.finished);
        assert!(ui.source.cheats);
        assert_eq!(ui.source.mode(), hl2_ui::console::Mode::Options);
        let labels: Vec<_> = sim
            .samples
            .iter()
            .map(|s| (s.label.as_str(), s.host_tick))
            .collect();
        assert_eq!(labels, [("options-open", 2), ("dialog-input", 5)]);
    }
    #[test]
    fn background_level_world_runs_behind_the_menu_without_player_input() {
        let forward: Vec<Command> = serde_json::from_str(
            &serde_json::to_string(
                &(1..=20)
                    .map(|tick| serde_json::json!({"tick": tick, "forward": 1.0, "primary": true}))
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        )
        .unwrap();
        let run = |background: bool| {
            let mut ui = crate::ui_host::Console::new(Default::default());
            ui.source.level_loaded(background);
            if !background {
                ui.source.set_mode(hl2_ui::console::Mode::Pause);
            }
            let mut game = crate::gameplay::Gameplay::synthetic(World::default());
            let start = glam::Vec3::new(0., 0., 128.);
            let mut sim = Simulation::new(&World::default(), start, 0., 0., false, forward.clone());
            sim.paused = ui.source.paused();
            for _ in 0..20 {
                sim.step(Some(&mut game), Some(&mut ui));
            }
            (sim, game.scene.time)
        };

        // Main menu over a background level: the world advances, the player ignores input.
        let (sim, time) = run(true);
        assert!(sim.paused() && sim.world_running());
        assert!(time > 0., "scene time {time}");
        assert_eq!(sim.player.ticks, 20);
        assert_eq!(sim.player.feet.truncate(), glam::Vec2::ZERO);

        // Pause menu over gameplay: nothing advances.
        let (sim, time) = run(false);
        assert!(sim.paused() && !sim.world_running());
        assert_eq!(time, 0.);
        assert_eq!(sim.player.ticks, 0);
    }
    #[test]
    fn a_frozen_view_ignores_player_actions_but_applies_entity_setup() {
        // d1_trainstation_01 starts under the intro's point_viewcontrol: a fixture's inputs to
        // map entities still arrive, its weapon actions do not.
        let script: Vec<Command> = serde_json::from_value(serde_json::json!([{
            "tick": 1,
            "actions": [
                {"action": "loadout"},
                {"action": "send", "target": "relay", "input": "Trigger"}
            ]
        }]))
        .unwrap();
        let world = World {
            entities: vec![modkit_core::Entity {
                properties: vec![
                    ("classname".into(), "logic_relay".into()),
                    ("targetname".into(), "relay".into()),
                ],
            }],
            ..Default::default()
        };
        let mut game = crate::gameplay::Gameplay::synthetic(world.clone());
        let mut sim = Simulation::new(&world, glam::Vec3::Z * 128., 0., 0., false, script);
        sim.view_control = Some(hl2_simulation::view_control::View {
            origin: glam::Vec3::ZERO,
            pitch: 0.,
            yaw: 0.,
            freeze_player: true,
        });
        sim.step(Some(&mut game), None);
        assert_eq!(game.scene.diagnostics.inputs_delivered, 1);
        assert!(game.inventory.owned.is_empty());
    }
    #[test]
    fn movement_schedule_uses_fifteen_milliseconds() {
        let mut app = App::new();
        app.add_plugins(MovementPlugin);
        assert_eq!(
            app.world().resource::<Time<Fixed>>().timestep(),
            std::time::Duration::from_millis(15)
        );
    }
    #[test]
    fn config_key_bindings_and_mouse_sensitivity_are_respected() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        let mut sim = Simulation::new(
            &World::default(),
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            vec![],
        );
        sim.focused = true;
        let mut custom_config = hl2_ui::config::Config::default();
        custom_config.mouse.sensitivity = 6.0;
        custom_config.mouse.invert = true;
        custom_config.keys.insert("+forward".into(), "KeyI".into());
        custom_config.keys.insert("+jump".into(), "KeyJ".into());

        let mut ui = crate::ui_host::Console::new(Default::default());
        ui.source.config = custom_config;

        app.insert_resource(sim)
            .insert_resource(ui)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .insert_resource(AccumulatedMouseMotion {
                delta: Vec2::new(50., 50.),
            })
            .add_message::<AppExit>()
            .add_message::<bevy::input::keyboard::KeyboardInput>()
            .add_message::<bevy::input::mouse::MouseWheel>();

        app.world_mut().spawn((
            Window {
                focused: true,
                ..default()
            },
            CursorOptions {
                grab_mode: CursorGrabMode::Locked,
                visible: false,
                ..default()
            },
            PrimaryWindow,
        ));

        // Press remapped keys KeyI and KeyJ
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyI);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);

        app.world_mut()
            .run_system_once(crate::input::controls)
            .unwrap();

        let sim = app.world().resource::<Simulation>();
        assert_eq!(sim.input.forward, 1.0);
        assert!(sim.input.jump);
        // With 6.0 sensitivity (scale 2.0) and inverted pitch:
        // yaw: -50 * 0.001152 * 2.0 = -0.1152
        // pitch: 0 - 50 * 0.001152 * 2.0 * (-1.0) = +0.1152
        assert!((sim.yaw - (-0.1152)).abs() < 1e-4);
        assert!((sim.pitch - 0.1152).abs() < 1e-4);
    }
    #[test]
    fn only_window_setting_changes_queue_a_window_apply() {
        let mut sim = Simulation::new(
            &World::default(),
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            vec![],
        );
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());
        sim.applied_window = Some(WindowSettings::of(&ui.source.config.video));
        let mut apply = |sim: &mut Simulation,
                         ui: &mut crate::ui_host::Console,
                         cfg: hl2_ui::config::Config| {
            // As in the real path: the console stores the new configuration before the
            // effect reaches the simulation.
            ui.source.config = cfg.clone();
            sim.console_effects(
                ui,
                &mut game,
                vec![hl2_ui::console::Effect::ApplyConfig(Box::new(cfg))],
            );
            sim.pending_video_config.take().is_some()
        };
        let mut cfg = ui.source.config.clone();
        cfg.mouse.sensitivity += 1.;
        cfg.audio.volume = 0.3;
        cfg.video.fov = 80.;
        cfg.video.gamma = 2.0;
        assert!(
            !apply(&mut sim, &mut ui, cfg.clone()),
            "non-window settings"
        );
        cfg.video.width += 16;
        assert!(apply(&mut sim, &mut ui, cfg.clone()), "resolution");
        cfg.video.display_mode = match cfg.video.display_mode {
            hl2_ui::config::DisplayMode::Windowed => hl2_ui::config::DisplayMode::Fullscreen,
            _ => hl2_ui::config::DisplayMode::Windowed,
        };
        assert!(apply(&mut sim, &mut ui, cfg.clone()), "display mode");
        assert!(!apply(&mut sim, &mut ui, cfg), "same settings again");
    }

    #[test]
    fn config_effect_updates_simulation_fov_and_console_config() {
        let mut sim = Simulation::new(
            &World::default(),
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            vec![],
        );
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());
        assert_eq!(sim.fov, 75.0);

        let mut new_config = ui.source.config.clone();
        new_config.video.fov = 90.0;
        new_config.audio.volume = 0.42;

        sim.console_effects(
            &mut ui,
            &mut game,
            vec![hl2_ui::console::Effect::ApplyConfig(Box::new(
                new_config.clone(),
            ))],
        );

        assert_eq!(sim.fov, 90.0);
        assert_eq!(ui.source.config.video.fov, 90.0);
        assert_eq!(ui.source.config.audio.volume, 0.42);
    }

    #[test]
    fn config_effect_detail_change_reloads_active_map() {
        let mut sim = Simulation::new(&World::default(), glam::Vec3::ZERO, 0., 0., false, vec![]);
        let world = World {
            name: "d1_trainstation_01".into(),
            ..Default::default()
        };
        let mut game = crate::gameplay::Gameplay::synthetic(world);
        let mut ui = crate::ui_host::Console::new(Default::default());

        // First apply initializes applied_detail without reload
        let mut cfg = ui.source.config.clone();
        sim.console_effects(
            &mut ui,
            &mut game,
            vec![hl2_ui::console::Effect::ApplyConfig(Box::new(cfg.clone()))],
        );
        assert_eq!(ui.map_request, None);
        assert_eq!(sim.applied_detail, Some(cfg.video.load_detail()));

        // Change model detail -> triggers map reload
        cfg.video.advanced.model_detail = hl2_ui::config::ModelDetail::Low;
        sim.console_effects(
            &mut ui,
            &mut game,
            vec![hl2_ui::console::Effect::ApplyConfig(Box::new(cfg.clone()))],
        );
        assert_eq!(ui.map_request.as_deref(), Some("d1_trainstation_01"));
        assert!(sim.loading);
        ui.map_request = None;
        sim.loading = false;

        // Change texture detail -> triggers map reload
        cfg.video.advanced.texture_detail = hl2_ui::config::TextureDetail::Low;
        sim.console_effects(
            &mut ui,
            &mut game,
            vec![hl2_ui::console::Effect::ApplyConfig(Box::new(cfg.clone()))],
        );
        assert_eq!(ui.map_request.as_deref(), Some("d1_trainstation_01"));
        assert!(sim.loading);
    }

    #[test]
    fn config_effect_hdr_change_reloads_only_when_the_frame_kind_changes() {
        use hl2_ui::config::HdrMode;
        let mut sim = Simulation::new(&World::default(), glam::Vec3::ZERO, 0., 0., false, vec![]);
        let world = World {
            name: "d1_trainstation_01".into(),
            ..Default::default()
        };
        let mut game = crate::gameplay::Gameplay::synthetic(world);
        let mut ui = crate::ui_host::Console::new(Default::default());
        let mut cfg = ui.source.config.clone();
        let mut apply = |sim: &mut Simulation, cfg: &hl2_ui::config::Config| {
            sim.console_effects(
                &mut ui,
                &mut game,
                vec![hl2_ui::console::Effect::ApplyConfig(Box::new(cfg.clone()))],
            );
            let reloaded = ui.map_request.take().is_some();
            sim.loading = false;
            reloaded
        };
        assert!(!apply(&mut sim, &cfg), "the first apply only records");
        cfg.video.hdr = HdrMode::Bloom;
        assert!(apply(&mut sim, &cfg), "Full to Bloom leaves HDR");
        cfg.video.hdr = HdrMode::None;
        assert!(!apply(&mut sim, &cfg), "Bloom and None are both LDR");
        cfg.video.hdr = HdrMode::Full;
        assert!(apply(&mut sim, &cfg), "back to HDR");
    }

    #[test]
    fn console_effects_routes_play_sound_to_ui_sound_requests() {
        let mut sim = Simulation::new(&World::default(), glam::Vec3::ZERO, 0., 0., false, vec![]);
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());

        sim.console_effects(
            &mut ui,
            &mut game,
            vec![
                hl2_ui::console::Effect::PlaySound("ui/buttonrollover.wav".into()),
                hl2_ui::console::Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            ],
        );

        assert_eq!(game.sound_requests.len(), 2);
        assert_eq!(
            game.sound_requests[0],
            hl2_simulation::sounds::SoundRequest::ui("ui/buttonrollover.wav")
        );
        assert!(game.sound_requests[0].ui);
        assert_eq!(
            game.sound_requests[1],
            hl2_simulation::sounds::SoundRequest::ui("ui/buttonclickrelease.wav")
        );
        assert!(game.sound_requests[1].ui);
    }

    #[test]
    fn viewmodel_tweak_commands_follow_the_gameplay_map_state() {
        use hl2_ui::console::Effect::{ViewmodelTweakMode, ViewmodelTweakReset};
        let mut sim = Simulation::new(&World::default(), glam::Vec3::ZERO, 0., 0., false, vec![]);
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());
        let mut run =
            |sim: &mut Simulation, effect| sim.console_effects(&mut ui, &mut game, vec![effect]);

        run(&mut sim, ViewmodelTweakReset);
        assert!(!sim.viewmodel_debug.reset_pending);
        sim.background_level = true;
        run(&mut sim, ViewmodelTweakMode(true));
        assert!(!sim.viewmodel_debug.enabled);
        sim.background_level = false;
        sim.loading = true;
        run(&mut sim, ViewmodelTweakMode(true));
        assert!(!sim.viewmodel_debug.enabled);
        sim.loading = false;
        run(&mut sim, ViewmodelTweakMode(true));
        assert!(sim.viewmodel_debug.enabled);
        run(&mut sim, ViewmodelTweakReset);
        assert!(sim.viewmodel_debug.reset_pending);
        sim.change_map(&World::default(), glam::Vec3::ZERO, true);
        assert!(!sim.viewmodel_debug.enabled && !sim.viewmodel_debug.reset_pending);
        run(&mut sim, ViewmodelTweakMode(true));
        run(&mut sim, ViewmodelTweakMode(false));
        assert!(!sim.viewmodel_debug.enabled);
    }

    #[test]
    fn background_and_viewmodel_debug_modes_mutually_exclusive() {
        use hl2_ui::console::Effect::{BackgroundDebugMode, ViewmodelTweakMode};
        let mut sim = Simulation::new(&World::default(), glam::Vec3::ZERO, 0., 0., false, vec![]);
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());
        let mut run =
            |sim: &mut Simulation, effect| sim.console_effects(&mut ui, &mut game, vec![effect]);

        sim.background_level = true;
        run(&mut sim, BackgroundDebugMode(true));
        assert!(sim.background_debug.enabled);
        assert!(!sim.viewmodel_debug.enabled);

        sim.background_level = false;
        run(&mut sim, ViewmodelTweakMode(true));
        assert!(sim.viewmodel_debug.enabled);
        assert!(!sim.background_debug.enabled);

        sim.background_level = true;
        run(&mut sim, BackgroundDebugMode(true));
        assert!(sim.background_debug.enabled);
        assert!(!sim.viewmodel_debug.enabled);
    }

    #[test]
    fn background_map_setting_does_not_interrupt_gameplay_map_load() {
        let mut sim = Simulation::new(
            &World::default(),
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            vec![],
        );
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());

        // Simulate gameplay map load requested
        sim.console_effects(
            &mut ui,
            &mut game,
            vec![hl2_ui::console::Effect::Map("ep1_citadel_00".into())],
        );
        assert!(sim.loading);
        assert_eq!(ui.map_request.as_deref(), Some("ep1_citadel_00"));
        assert!(!ui.background_request);

        // Attempting to set a background map while gameplay map is loading must NOT overwrite the gameplay load
        sim.console_effects(
            &mut ui,
            &mut game,
            vec![hl2_ui::console::Effect::MainMenuMap(
                hl2_ui::console::GameMap::new("hl2", "background02"),
            )],
        );
        assert!(sim.loading);
        assert_eq!(ui.map_request.as_deref(), Some("ep1_citadel_00"));
        assert!(!ui.background_request);
        assert!(
            ui.source
                .shell
                .panel
                .output
                .iter()
                .any(|s| s.contains("Gameplay map load in progress"))
        );
    }
    #[test]
    fn running_background_worker_does_not_drop_new_background_request() {
        let mut sim = Simulation::new(
            &World::default(),
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            vec![],
        );
        let mut game = crate::gameplay::Gameplay::synthetic(World::default());
        let mut ui = crate::ui_host::Console::new(Default::default());
        sim.loading = true;
        ui.source.start_background_loading("hl2", "background01");
        // The campaign has consumed the first request while its worker remains active.
        assert!(!ui.background_request);
        sim.console_effects(
            &mut ui,
            &mut game,
            vec![hl2_ui::console::Effect::MainMenuMap(
                hl2_ui::console::GameMap::new("ep2", "ep2_background01"),
            )],
        );
        assert_eq!(ui.map_request.as_deref(), Some("ep2_background01"));
        assert_eq!(ui.map_request_game.as_deref(), Some("ep2"));
        assert!(ui.background_request);
    }
}
