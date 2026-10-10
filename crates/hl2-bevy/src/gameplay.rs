//! Bevy resource bridge for retained gameplay. Owned file reads occur before App::run.
use anyhow::Result;
use bevy::prelude::Resource;
use glam::Vec3;
use hl2_simulation::{
    actors,
    entities::Scene,
    gameplay::{Inventory, Weapon},
    npc::Controller,
    physics::Physics,
    projectiles::Projectiles,
    selection::{Selection, SelectionResult},
};
use modkit_core::{
    World,
    movement::{Player, TICK},
};
use serde::Deserialize;
use source_assets::vpk::Vfs;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Loadout,
    Slot {
        slot: usize,
    },
    Wheel {
        delta: i32,
    },
    Confirm,
    Cancel,
    Previous,
    Reload,
    Use,
    Impulse,
    /// Seed an isolated test actor; never called by ordinary campaign play.
    ActorPose {
        target: String,
        origin: [f32; 3],
        yaw: f32,
    },
    Send {
        target: String,
        input: String,
        #[serde(default)]
        parameter: String,
    },
}
impl Action {
    pub fn valid(&self) -> bool {
        match self {
            Self::ActorPose {
                target,
                origin,
                yaw,
            } => {
                !target.is_empty()
                    && target.len() <= 128
                    && origin.iter().all(|v| v.is_finite())
                    && yaw.is_finite()
            }
            Self::Slot { slot } => *slot < 6,
            Self::Wheel { delta } => (-100..=100).contains(delta),
            Self::Send {
                target,
                input,
                parameter,
            } => {
                !target.is_empty()
                    && target.len() <= 128
                    && !input.is_empty()
                    && input.len() <= 128
                    && parameter.len() <= 1024
            }
            _ => true,
        }
    }
}
#[derive(Resource)]
pub struct Gameplay {
    pub world: Arc<World>,
    pub scene: Scene,
    pub inventory: Inventory,
    pub weapons: BTreeMap<String, Weapon>,
    pub selection: Selection,
    pub npcs: Controller,
    pub projectiles: Projectiles,
    pub impacts: hl2_simulation::impacts::Impacts,
    pub primary: bool,
    pub secondary: bool,
    pub actions: Vec<Action>,
    queued_primary: bool,
    queued_secondary: bool,
    primary_consumed: bool,
    secondary_consumed: bool,
    pub unplayed_sounds: u64,
    pub sound_cues: Vec<String>,
    pub sound_requests: Vec<hl2_simulation::sounds::SoundRequest>,
    weapon_sounds: hl2_simulation::sounds::WeaponAnimationSounds,
    pub footsteps: hl2_simulation::footsteps::Footsteps,
    /// Local player damage intake (skill 2, the owner's native setting).
    pub player_damage: hl2_simulation::player_damage::PlayerDamage,
    /// Client "Damage" messages not yet shown by the HUD.
    pub damage_messages: Vec<hl2_simulation::player_damage::DamageMessage>,
    /// HEV suit voice queue.
    pub suit: hl2_simulation::suit::Suit,
}
impl Gameplay {
    /// After the player move: this tick's damage (trigger_hurt, blasts, then the
    /// landing's fall damage) through the SDK player path, then death
    /// (Event_Killed/DeathSound). Returns true when the player died this tick.
    pub fn player_outcome(&mut self, player: &mut modkit_core::movement::Player) -> bool {
        use hl2_simulation::player_damage::{DMG_FALL, DamageInfo};
        // CHL2_Player::OnTakeDamage refuses everything while gordon_invulnerable is on
        // (d1_trainstation_01-03 and parts of 04); TakeHealth is separate.
        let invulnerable = self.scene.globals.is_on("gordon_invulnerable");
        let mut infos = std::mem::take(&mut self.scene.player_damage);
        if let Some(speed) = player.landed {
            let damage = hl2_simulation::gameplay::fall_damage(speed);
            if damage > 0. {
                // CMoveHelperServer::PlayerFallingDamage: the world inflicts DMG_FALL.
                infos.push(DamageInfo {
                    amount: damage,
                    kind: DMG_FALL,
                    inflictor: Vec3::ZERO,
                });
            }
        }
        for info in infos {
            if info.amount < 0. {
                if self.inventory.health > 0. {
                    self.inventory.health = (self.inventory.health - info.amount).min(100.);
                }
            } else if !invulnerable
                && self.player_damage.take(&mut self.inventory, info) > 0.
                && info.kind & DMG_FALL != 0
                && player.landed.is_some()
            {
                self.scene.sounds.push("Player.FallDamage".into());
            }
        }
        // PlayerFallingDamage: a landing that leaves the player dead cuts to black
        // (FlPlayerFallDeathDoesScreenFade, duration 0, hold 9999, OUT | STAYOUT).
        if player
            .landed
            .is_some_and(|speed| hl2_simulation::gameplay::fall_damage(speed) > 0.)
            && self.inventory.health <= 0.
            && !player.dead
        {
            use hl2_simulation::player_damage::{FFADE_OUT, FFADE_STAYOUT, ScreenFade};
            self.player_damage.fades.push(ScreenFade::new(
                [0, 0, 0, 255],
                0.,
                9999.,
                FFADE_OUT | FFADE_STAYOUT,
            ));
        }
        let suit = self.inventory.suit;
        for sound in self.player_damage.sounds.drain(..) {
            self.scene.sounds.push(sound.into());
        }
        let now = self.scene.time;
        for update in std::mem::take(&mut self.player_damage.suit) {
            self.suit
                .update(suit, update.sentence, update.no_repeat, now);
        }
        // Weapon SetSuitUpdate requests (crossbow/RPG out of ammo).
        for (sentence, no_repeat) in std::mem::take(&mut self.inventory.suit_updates) {
            self.suit.update(suit, &sentence, no_repeat, now);
        }
        let died = !player.dead && self.inventory.health <= 0.;
        if died {
            player.dead = true;
            player.crouched = false;
            self.scene.player_dead = true;
            // DeathSound: the accumulated damage bits decide the fall splat.
            self.scene.sounds.push(
                if self.player_damage.bits() & DMG_FALL != 0 {
                    "Player.FallGib"
                } else {
                    "Player.Death"
                }
                .into(),
            );
            // DeathSound: UTIL_EmitGroupnameSuit(HEV_DEAD) at once; Event_Killed then
            // clears the queued suit sentences.
            self.suit.emit(suit, "#HEV_DEAD");
            self.suit.clear();
            // Event_Killed holsters the active weapon.
            self.inventory.previous = std::mem::take(&mut self.inventory.active);
        }
        // CheckSuitUpdate (PostThink), then UpdateClientData after the damage.
        self.suit.think(suit && !player.dead, now);
        // UTIL_EmitSoundSuit: suitvolume 0.25 (SDK default, the owner's config.cfg).
        for sentence in std::mem::take(&mut self.suit.play) {
            let name = if let Some(group) = sentence.strip_prefix('#') {
                format!("#{group}")
            } else {
                format!("!{sentence}")
            };
            self.scene
                .sounds
                .push(hl2_simulation::sounds::SoundRequest {
                    volume: Some(0.25),
                    ..name.into()
                });
        }
        if let Some(message) = self.player_damage.client_message() {
            self.damage_messages.push(message);
        }
        died
    }
    /// SDK step sounds for one movement tick: UpdateStepSound with the state before the move,
    /// then the jump or landing step the move produced.
    pub fn step_sounds(
        &mut self,
        physics: &Physics,
        before: hl2_simulation::footsteps::State,
        feet: Vec3,
        player: &modkit_core::movement::Player,
    ) {
        let world = self.world.clone();
        let surface_at = |game: &Self, at: Vec3| {
            game.impacts
                .ground_property(&world, physics, &game.scene, at, 4.)
                .map(|prop| game.impacts.step_surface(&prop))
        };
        let mut steps = Vec::new();
        let mut footsteps = std::mem::take(&mut self.footsteps);
        steps.extend(footsteps.update(before, TICK, || surface_at(self, feet)));
        if player.jumped {
            steps.extend(footsteps.jump(surface_at(self, feet)));
        }
        if let Some(fall) = player.landed {
            steps.extend(footsteps.land(fall, || surface_at(self, player.feet)));
        }
        self.footsteps = footsteps;
        for step in steps {
            self.scene
                .sounds
                .push(hl2_simulation::sounds::SoundRequest {
                    volume: Some(step.volume),
                    ..step.sound.into()
                });
        }
    }
    #[cfg(test)]
    pub(crate) fn synthetic(world: World) -> Self {
        Self {
            scene: Scene::new(&world),
            world: Arc::new(world),
            inventory: Inventory::default(),
            weapons: BTreeMap::new(),
            selection: Selection::new(),
            npcs: Controller::new(None, Default::default()),
            projectiles: Projectiles::default(),
            impacts: Default::default(),
            primary: false,
            secondary: false,
            actions: vec![],
            queued_primary: false,
            queued_secondary: false,
            primary_consumed: false,
            secondary_consumed: false,
            unplayed_sounds: 0,
            sound_cues: vec![],
            sound_requests: vec![],
            weapon_sounds: Default::default(),
            footsteps: Default::default(),
            player_damage: Default::default(),
            damage_messages: Vec::new(),
            suit: Default::default(),
        }
    }
    pub fn load_with_campaign(
        mut world: World,
        vfs: &Vfs,
        revision: u32,
        new_game: bool,
    ) -> Result<Self> {
        let mut scene = Scene::with_campaign(&world, new_game);
        scene.load_choreography(&world, vfs)?;
        actors::prepare_choreography_animations(&mut world, vfs, &scene);
        let map = world.name.clone();
        let npcs = actors::prepare_npcs(&mut world, vfs, &map, revision);
        let weapons = hl2_simulation::gameplay::definitions(vfs)?;
        actors::prepare_weapons(&mut world, vfs, &weapons)?;
        let impacts = hl2_simulation::impacts::Impacts::new(vfs, &world);
        let skill = vfs
            .read("cfg/skill.cfg")?
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let player_damage =
            hl2_simulation::player_damage::PlayerDamage::with_skill(2, skill.as_deref());
        Ok(Self {
            player_damage,
            damage_messages: Vec::new(),
            suit: Default::default(),
            impacts,
            world: Arc::new(world),
            scene,
            inventory: Inventory::default(),
            weapons,
            selection: Selection::new(),
            npcs,
            projectiles: Projectiles::default(),
            primary: false,
            secondary: false,
            actions: vec![],
            queued_primary: false,
            queued_secondary: false,
            primary_consumed: false,
            secondary_consumed: false,
            unplayed_sounds: 0,
            sound_cues: vec![],
            sound_requests: vec![],
            weapon_sounds: Default::default(),
            footsteps: Default::default(),
        })
    }
    pub fn consume_attacks(&mut self) {
        self.primary_consumed = true;
        self.secondary_consumed = true;
        self.primary = false;
        self.secondary = false;
        self.queued_primary = false;
        self.queued_secondary = false;
        self.actions.clear();
    }
    pub fn buttons(&mut self, primary: bool, secondary: bool) {
        self.queued_primary |=
            primary && !(self.primary || self.queued_primary) && !self.primary_consumed;
        self.queued_secondary |=
            secondary && !(self.secondary || self.queued_secondary) && !self.secondary_consumed;
        self.primary_consumed &= primary;
        self.secondary_consumed &= secondary;
        self.primary = primary;
        self.secondary = secondary;
    }
    fn select(&mut self, result: SelectionResult) {
        if let Some(name) = result.weapon {
            self.inventory.give(&name, &self.weapons, self.scene.time);
        }
        self.scene
            .sounds
            .extend(result.sounds.into_iter().map(Into::into));
    }
    pub fn tick(
        &mut self,
        physics: &mut Physics,
        player: &Player,
        eye: Vec3,
        direction: Vec3,
        fly: bool,
    ) {
        self.selection
            .tick(&self.inventory, &self.weapons, self.scene.time);
        for action in std::mem::take(&mut self.actions) {
            let result = match action {
                Action::Loadout => {
                    for name in self.weapons.keys() {
                        self.inventory.give(name, &self.weapons, self.scene.time);
                    }
                    self.inventory.refill_ammo(&self.weapons);
                    self.inventory.suit = true;
                    self.inventory
                        .give("weapon_crowbar", &self.weapons, self.scene.time);
                    continue;
                }
                Action::Slot { slot } => {
                    self.selection
                        .slot(&self.inventory, &self.weapons, slot, self.scene.time)
                }
                Action::Wheel { delta } => {
                    self.selection
                        .wheel(&self.inventory, &self.weapons, delta, self.scene.time)
                }
                Action::Confirm => {
                    let result =
                        self.selection
                            .confirm(&self.inventory, &self.weapons, self.scene.time);
                    self.primary_consumed = true;
                    self.secondary_consumed = true;
                    result
                }
                Action::Cancel => self.selection.cancel(),
                Action::Previous => self.selection.last(&self.inventory, &self.weapons),
                Action::Reload => {
                    self.inventory
                        .reload(&self.weapons, &mut self.scene, &self.world);
                    continue;
                }
                Action::Use => {
                    if let Some((id, _)) = physics.ray(eye, direction, 96.) {
                        self.scene.use_entity_at(&self.world, id, player.feet);
                    }
                    continue;
                }
                Action::Impulse => {
                    if let Some((id, _)) = physics.ray(eye, direction, 128.) {
                        physics.impulse(id, direction, 6.);
                    }
                    continue;
                }
                Action::ActorPose {
                    target,
                    origin,
                    yaw,
                } => {
                    if !self.scene.fixture_actor_pose(
                        &self.world,
                        &target,
                        Vec3::from_array(origin),
                        yaw,
                    ) {
                        *self
                            .scene
                            .diagnostics
                            .unsupported
                            .entry(format!("fixture actor pose target missing: {target}"))
                            .or_default() += 1;
                    }
                    continue;
                }
                Action::Send {
                    target,
                    input,
                    parameter,
                } => {
                    self.scene.send_named(&target, &input, &parameter, 0.);
                    continue;
                }
            };
            self.select(result);
        }
        if self.selection.pending.is_some()
            && ((self.primary || self.queued_primary) && !self.primary_consumed
                || (self.secondary || self.queued_secondary) && !self.secondary_consumed)
        {
            let result = self
                .selection
                .confirm(&self.inventory, &self.weapons, self.scene.time);
            if result.weapon.is_some() {
                self.primary_consumed = true;
                self.secondary_consumed = true;
            }
            self.select(result);
        }
        self.scene.tick(&self.world, player.feet, TICK);
        self.inventory.advance_projectile_fire(
            &self.world,
            &mut self.scene,
            &self.weapons,
            eye,
            direction,
        );
        let allowed = self.selection.pending.is_none();
        let primary = allowed && (self.primary || self.queued_primary) && !self.primary_consumed;
        let secondary = allowed
            && (self.secondary || self.queued_secondary)
            && !self.secondary_consumed
            && matches!(
                self.inventory.active.as_str(),
                "weapon_shotgun"
                    | "weapon_smg1"
                    | "weapon_ar2"
                    | "weapon_frag"
                    | "weapon_crossbow"
                    | "weapon_bugbait"
            );
        self.inventory.set_attack_input(primary, secondary);
        self.inventory.ducking = player.crouched;
        let attacking = primary
            || secondary
            || allowed
                && (!self.primary_consumed && self.inventory.delayed_attack
                    || !self.secondary_consumed && self.inventory.delayed_secondary_attack);
        self.inventory.tick(
            &self.world,
            &mut self.scene,
            &self.weapons,
            player.feet,
            attacking,
            TICK,
        );
        if allowed
            && !self.secondary_consumed
            && (secondary || self.inventory.delayed_secondary_attack)
        {
            self.inventory.secondary_attack(
                &self.weapons,
                &self.world,
                &mut self.scene,
                physics,
                eye,
                direction,
            );
        } else if allowed && !self.primary_consumed && (primary || self.inventory.delayed_attack) {
            self.inventory.attack(
                &self.weapons,
                &self.world,
                &mut self.scene,
                physics,
                eye,
                direction,
            );
        }
        self.queued_primary = false;
        self.queued_secondary = false;
        for (id, state) in self.scene.states.iter().enumerate() {
            physics.set_entity(id, state.origin, state.rotation, state.collides());
        }
        // weapon_frag throws from this tick's animation events.
        if !self.inventory.frag.pending.is_empty() {
            let ground = physics
                .impact_ray(player.feet + glam::Vec3::Z * 4., -glam::Vec3::Z, 16.)
                .map(|hit| hit.normal);
            self.inventory.launch_frags(
                &self.weapons,
                physics,
                eye,
                direction,
                player.velocity,
                player.feet,
                ground,
                self.scene.time,
            );
        }
        // weapon_bugbait ThrowGrenade from this tick's EVENT_WEAPON_THROW.
        self.inventory
            .launch_bugbait(physics, eye, direction, player.velocity, self.scene.time);
        for launch in self.inventory.projectile_spawns.drain(..) {
            self.projectiles.spawn(launch, &mut self.scene);
        }
        // CWeaponRPG::UpdateLaserPosition after the weapon frame; missiles seek it.
        self.inventory.rpg_aim(
            physics,
            &self.world,
            eye,
            direction,
            player.feet + glam::Vec3::Z * if player.crouched { 18. } else { 36. },
            player.feet,
            self.scene.time,
        );
        self.projectiles.laser_dot = self.inventory.rpg.dot;
        physics.refresh_entity_queries();
        actors::tick_npcs(
            &mut self.npcs,
            &mut self.scene,
            &self.world,
            physics,
            player,
            fly,
        );
        let mut damage = self.projectiles.tick(
            &self.world,
            &mut self.scene,
            physics,
            player.feet,
            player.crouched,
            TICK,
        );
        // Bug bait splats/squeezes are for step 14 (antlion reactions); not consumed yet.
        self.inventory.bugbait_events.clear();
        self.projectiles.bugbait_events.clear();
        // CMissile::Explode -> CWeaponRPG::NotifyRocketDied.
        for event in std::mem::take(&mut self.projectiles.missile_events) {
            self.inventory
                .rpg_missile_event(event, &self.weapons, &self.world, &mut self.scene);
        }
        // trigger_hurt touchers from this tick's scene logic share the entity path.
        damage.append(&mut self.scene.entity_damage);
        self.inventory
            .apply_projectile_damage(damage, &self.world, &mut self.scene, physics);
        physics.tick(TICK);
        for event in
            self.weapon_sounds
                .events(&self.world, &self.inventory, &self.weapons, self.scene.time)
        {
            self.scene.sounds.push(event.options.into());
        }
        for (hit, _melee) in self.inventory.impacts.drain(..) {
            self.impacts.add(hit, &self.world, physics, &mut self.scene);
        }
        // Projectile UTIL_ImpactTrace (crossbow bolts in the world).
        for hit in std::mem::take(&mut self.projectiles.impacts) {
            self.impacts.add(hit, &self.world, physics, &mut self.scene);
        }
        for sound in self.scene.sounds.drain(..) {
            self.sound_cues.push(sound.name.clone());
            if self.sound_requests.len() < 1024 {
                self.sound_requests.push(sound);
            } else {
                self.unplayed_sounds += 1;
            }
        }
        let excess = self.sound_cues.len().saturating_sub(64);
        self.sound_cues.drain(..excess);
    }
    pub fn report(&self, physics: &Physics) -> serde_json::Value {
        let entities: Vec<_> = self.world.entities.iter().enumerate()
            .filter(|(_, e)| e.class().contains("door") || e.class().starts_with("prop_physics"))
            .map(|(id, e)| {
                let state = &self.scene.states[id];
                let (origin, rotation) = physics.entity_pose(id).unwrap_or((state.origin, state.rotation));
                serde_json::json!({"entity":id,"name":e.get("targetname"),"class":e.class(),
                    "origin":origin.to_array(),"rotation":rotation.to_array(),"visible":state.visible,"killed":state.killed})
            }).collect();
        // Composition failures drop a gesture layer silently in presentation; surface them.
        let compose_errors: BTreeMap<usize, Vec<String>> = self
            .scene
            .gestures
            .report()
            .keys()
            .filter_map(|&id| {
                let instance = self
                    .world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(id))?;
                let rig = self.world.rigs.get(&instance.asset_key())?;
                let params = self.scene.actor_pose_values(rig, id);
                let (_, errors) = self.scene.gestures.compose(
                    rig,
                    id,
                    &self.scene.states[id].animation,
                    self.scene.animation_time(id),
                    &params,
                );
                (!errors.is_empty())
                    .then(|| (id, errors.iter().map(|e| format!("{e:?}")).collect()))
            })
            .collect();
        serde_json::json!({"time":self.scene.time,"inventory":self.inventory,"player_damage":self.player_damage,"viewmodel":{"animation":self.inventory.animation,"at":self.inventory.animation_at,"frag":self.inventory.frag},"suit":self.suit,"globals":self.scene.globals,"pending":self.selection.pending,"gesture_compose_errors":compose_errors,
            "entities":entities,"io":self.scene.diagnostics,"choreography":self.scene.choreography_states(&self.world),
            "animations":self.scene.animation_states(&self.world),"look_targets":self.scene.look_targets.report(),"gesture_layers":self.scene.gestures.report(),"monitors":self.scene.monitors,"npc_goals":self.npcs.snapshots(),
            "projectiles":{"active":self.projectiles.active,"effects":self.projectiles.effects,"diagnostics":self.projectiles.diagnostics},"impacts":{"created":self.impacts.created,"unclippable":self.impacts.unclippable,"active":self.impacts.marks.len(),"errors":self.impacts.errors},"transition":self.scene.transition,
            "unplayed_sounds":self.unplayed_sounds,"queued_sounds":self.sound_requests.len(),"recent_sound_cues":self.sound_cues})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn armed() -> (Gameplay, Physics, Player) {
        let mut game = Gameplay::synthetic(World::default());
        game.weapons.insert(
            "weapon_pistol".into(),
            Weapon {
                magazine: 18,
                default_clip: 18,
                ammo_type: "Pistol".into(),
                damage: 5.,
                slot: 1,
                ..Default::default()
            },
        );
        game.inventory.suit = true;
        game.inventory.give("weapon_pistol", &game.weapons, 0.);
        game.scene.time = 1.;
        (
            game,
            Physics::new(&World::default()),
            Player::new(Vec3::Z * 64.),
        )
    }
    fn step(game: &mut Gameplay, physics: &mut Physics, player: &Player) {
        game.tick(physics, player, player.eye(), Vec3::X, false);
    }
    #[test]
    fn fast_press_and_release_between_fixed_steps_fires_once() {
        let (mut game, mut physics, player) = armed();
        game.buttons(true, false);
        game.buttons(false, false);
        step(&mut game, &mut physics, &player);
        assert_eq!(game.inventory.shots, 1);
        assert_eq!(game.inventory.owned["weapon_pistol"], 17);
        step(&mut game, &mut physics, &player);
        assert_eq!(game.inventory.shots, 1);
    }
    #[test]
    fn confirming_selection_consumes_held_attack_until_release() {
        let (mut game, mut physics, player) = armed();
        game.actions.push(Action::Slot { slot: 1 });
        game.buttons(true, false);
        step(&mut game, &mut physics, &player);
        assert!(game.selection.pending.is_none());
        assert_eq!(game.inventory.shots, 0);
        for _ in 0..30 {
            game.buttons(true, false);
            step(&mut game, &mut physics, &player);
        }
        assert_eq!(game.inventory.shots, 0);
        game.buttons(false, false);
        game.buttons(true, false);
        step(&mut game, &mut physics, &player);
        assert_eq!(game.inventory.shots, 1);
    }
    #[test]
    fn pause_capture_consumes_click_and_discards_queued_actions() {
        let (mut game, mut physics, player) = armed();
        game.buttons(true, true);
        game.actions.push(Action::Loadout);
        game.consume_attacks();
        game.buttons(true, true);
        step(&mut game, &mut physics, &player);
        assert_eq!(game.inventory.shots, 0);
        assert!(game.actions.is_empty());
        game.buttons(false, true);
        game.buttons(true, true);
        step(&mut game, &mut physics, &player);
        assert_eq!(game.inventory.shots, 1);
    }
}
