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

/// A muzzle flash or tracer for the effects system (muzzle_effects.rs).
#[derive(Clone, Debug)]
pub enum ShotEffect {
    NpcFlash(hl2_simulation::muzzle_effects::MuzzleKind, glam::Mat4),
    /// A muzzle elight (operator id, origin, MuzzleFlash_Combine_NPC blue).
    MuzzleLight(usize, glam::Vec3, bool),
    /// Start, end, AR2Tracer.
    NpcTracer(Vec3, Vec3, bool),
    /// Start (ComputeTracerStartPosition), end, the view's right axis, AR2Tracer.
    PlayerTracer(Vec3, Vec3, Vec3, bool),
    /// AR2Impact origin and surface normal.
    Ar2Impact(Vec3, Vec3),
}
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
    /// Test only: teleport a map prop_physics body (entity id) to a pose with an
    /// optional velocity, standing in for prop_physics_create until runtime entity
    /// creation exists (DESIGN 14r); never called by ordinary campaign play.
    PropPose {
        entity: usize,
        origin: [f32; 3],
        #[serde(default)]
        angles: [f32; 3],
        #[serde(default)]
        velocity: [f32; 3],
        /// As if just launched by the gravity gun (m_bFirstCollisionAfterLaunch).
        #[serde(default)]
        launched: bool,
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
            Self::PropPose {
                origin,
                angles,
                velocity,
                ..
            } => origin
                .iter()
                .chain(angles)
                .chain(velocity)
                .all(|v| v.is_finite() && v.abs() < 65536.),
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
/// The `ai_enable` switch across map loads: on like Source (ai_disabled 0); `--no-ai`
/// turns it off at startup, the console and the F4 overlay switch it.
pub static AI_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
#[derive(Resource)]
pub struct Gameplay {
    pub world: Arc<World>,
    pub scene: Scene,
    pub inventory: Inventory,
    pub weapons: BTreeMap<String, Weapon>,
    pub selection: Selection,
    pub npcs: Controller,
    pub projectiles: Projectiles,
    /// Gibs of broken props (runtime prop bodies until DESIGN 14r entities).
    pub gibs: hl2_simulation::gibs::Gibs,
    /// NPC death ragdolls (client-side in Source).
    pub ragdolls: hl2_simulation::ragdolls::Ragdolls,
    pub phys_explosions: hl2_simulation::phys_explosion::PhysExplosions,
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
    /// Client sound events of entity animations (NPC footsteps).
    entity_sounds: hl2_simulation::sounds::EntityAnimationSounds,
    pub footsteps: hl2_simulation::footsteps::Footsteps,
    /// Local player damage intake (skill 2, the owner's native setting).
    pub player_damage: hl2_simulation::player_damage::PlayerDamage,
    /// Client "Damage" messages not yet shown by the HUD.
    pub damage_messages: Vec<hl2_simulation::player_damage::DamageMessage>,
    /// HEV suit voice queue.
    pub suit: hl2_simulation::suit::Suit,
    /// Step 14 NPC AI (on unless `--no-ai` / `ai_enable 0`).
    pub ai: hl2_simulation::ai::runtime::AiRuntime,
    /// Velocity impulses on the player from NPC hits (ApplyAbsVelocityImpulse), applied
    /// before the next player move.
    pub player_impulse: glam::Vec3,
    /// F4 developer NPC spawn overlay and the npc_create* console state.
    pub spawn_overlay: hl2_simulation::ai::spawn::Overlay,
    pub spawn_console: hl2_simulation::ai::spawn::Console,
    /// Spawn requests, their CC_NPC_Create placement and the created entity.
    pub spawn_log: Vec<serde_json::Value>,
    /// Muzzle flashes and tracers made by recent ticks, numbered for the effects system
    /// (which keeps the newest number it consumed); entries older than 0.5 s are dropped.
    pub shot_effects: Vec<(u64, f64, ShotEffect)>,
    pub shot_serial: u64,
    /// Active stunsticks' attachment-1 frames this tick (C_WeaponStunStick glow).
    pub stunstick_glows: Vec<glam::Mat4>,
    /// Runtime spawns per overlay entry (they cycle through its templates).
    pub spawn_counts: BTreeMap<String, usize>,
    /// Runtime-spawned entities waiting for their draws: (template, new entity); the
    /// host clones the template's meshes (`rendering::spawn_runtime_draws`).
    pub render_spawns: Vec<(usize, usize)>,
    /// Runtime-spawned entity ids (DESIGN 14r part A), in creation order.
    pub runtime_entities: Vec<usize>,
    /// BecomeTorso replacements (zombie, torso, legs raggib) and splits that failed.
    pub zombie_splits: Vec<(usize, usize, Option<usize>)>,
    /// Lethal-blast crab releases rendered (zombie, chopped).
    pub crab_releases: Vec<(usize, bool)>,
    /// Live headcrabs released by dying zombies (RELEASE_IMMEDIATE).
    pub released_crabs: Vec<usize>,
    /// Black crabs poison zombies threw (AE_ZOMBIE_POISON_THROW_CRAB).
    pub thrown_crabs: Vec<usize>,
    /// Nest crabs dying poison zombies ejected (EvacuateNest).
    pub evacuated_crabs: Vec<usize>,
    pub zombie_split_failures: Vec<String>,
    /// Runtime entities whose Bevy draws were despawned after they were killed.
    pub despawned_draws: std::collections::BTreeSet<usize>,
    /// Spawns that had to copy the World because another Arc held it (should stay 0).
    pub world_copies: usize,
    /// Runtime SetBodygroup (Scene::bodies): the body each entity is drawn with, the draw
    /// rebuilds done and the ones refused (`rendering::swap_runtime_bodies`).
    pub drawn_bodies: std::collections::BTreeMap<usize, i32>,
    pub body_swaps: u32,
    pub body_swap_failures: Vec<String>,
    /// prop_physics_create requests (model, eye, aim) for the host's model loader.
    pub prop_requests: Vec<(String, Vec3, Vec3)>,
    /// Runtime entities whose model draws the host builds next frame (dropped weapons).
    pub model_draws: Vec<usize>,
}
/// The weapons each F4 class can be given: Weapon_CanUse on its pooled spawn's
/// loaded model (every activity of the model, loaded or not).
pub fn spawn_usable_weapons(
    world: &World,
    ai: &hl2_simulation::ai::runtime::AiRuntime,
) -> BTreeMap<String, Vec<String>> {
    use hl2_simulation::ai::spawn;
    spawn::SPAWN_CLASSES
        .iter()
        .filter_map(|class| {
            let id = spawn::free_slot(world, class, &Default::default())?;
            let class_name = spawn::spawn_class(class).0;
            let instance = world
                .model_instances
                .iter()
                .find(|i| i.entity == Some(id))?;
            let rig = world.rigs.get(&instance.asset_key())?;
            Some((
                (*class).to_owned(),
                ai.usable_weapons(class_name, &rig.activities),
            ))
        })
        .collect()
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
        // CBlackHeadcrab::TouchDamage: the bite (DMG_SLASH) leaves at least one health, then
        // DMG_POISON takes all but one; CheckTimeBasedDamage restores the poison later.
        for (crab, bite) in std::mem::take(&mut self.ai.poison_bites) {
            if !invulnerable {
                let inflictor = self.scene.states[crab].origin;
                self.player_damage
                    .poison_bite(&mut self.inventory, bite, inflictor);
            }
        }
        let now = self.scene.time;
        self.player_damage
            .check_time_based(&mut self.inventory, now);
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
    /// CategorizeGroundSurface for the next movement step: the surfaceprop of the
    /// face under the feet (the same lookup as the footsteps), or the defaults when
    /// nothing is found or the surface table is not loaded.
    pub fn ground_surface(
        &self,
        physics: &Physics,
        feet: Vec3,
    ) -> modkit_core::movement::GroundSurface {
        self.impacts
            .ground_property(&self.world, physics, &self.scene, feet, 4.)
            .and_then(|prop| self.world.surface_materials.get(&prop.to_lowercase()))
            .map(|m| modkit_core::movement::GroundSurface {
                friction: m.friction,
                max_speed_factor: m.max_speed_factor,
                jump_factor: m.jump_factor,
            })
            .unwrap_or_default()
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
            gibs: Default::default(),
            ragdolls: Default::default(),
            phys_explosions: Default::default(),
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
            entity_sounds: Default::default(),
            footsteps: Default::default(),
            player_damage: Default::default(),
            damage_messages: Vec::new(),
            suit: Default::default(),
            ai: hl2_simulation::ai::runtime::AiRuntime::new(&World::default(), None),
            player_impulse: glam::Vec3::ZERO,
            spawn_overlay: Default::default(),
            spawn_console: Default::default(),
            spawn_log: Vec::new(),
            shot_effects: Vec::new(),
            shot_serial: 0,
            stunstick_glows: Vec::new(),
            spawn_counts: Default::default(),
            render_spawns: Vec::new(),
            zombie_splits: Vec::new(),
            crab_releases: Vec::new(),
            released_crabs: Vec::new(),
            thrown_crabs: Vec::new(),
            evacuated_crabs: Vec::new(),
            zombie_split_failures: Vec::new(),
            runtime_entities: Vec::new(),
            despawned_draws: Default::default(),
            world_copies: 0,
            drawn_bodies: Default::default(),
            body_swaps: 0,
            body_swap_failures: Vec::new(),
            prop_requests: Vec::new(),
            model_draws: Vec::new(),
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
        if let Some(text) = skill.as_deref() {
            scene.skill = hl2_simulation::skill::Skill::parse(text, 2);
        }
        let mut ai = hl2_simulation::ai::runtime::AiRuntime::new(&world, Some(vfs));
        ai.enabled = AI_ENABLED.load(std::sync::atomic::Ordering::Relaxed);
        let mut spawn_overlay = hl2_simulation::ai::spawn::Overlay::default();
        spawn_overlay.set_usable(spawn_usable_weapons(&world, &ai));
        // The Props tab: every installed (and map pak) model with a .phy.
        spawn_overlay.set_props(vfs.physics_prop_models());
        Ok(Self {
            ai,
            player_impulse: glam::Vec3::ZERO,
            spawn_overlay,
            spawn_console: Default::default(),
            spawn_log: Vec::new(),
            shot_effects: Vec::new(),
            shot_serial: 0,
            stunstick_glows: Vec::new(),
            spawn_counts: Default::default(),
            render_spawns: Vec::new(),
            zombie_splits: Vec::new(),
            crab_releases: Vec::new(),
            released_crabs: Vec::new(),
            thrown_crabs: Vec::new(),
            evacuated_crabs: Vec::new(),
            zombie_split_failures: Vec::new(),
            runtime_entities: Vec::new(),
            despawned_draws: Default::default(),
            world_copies: 0,
            drawn_bodies: Default::default(),
            body_swaps: 0,
            body_swap_failures: Vec::new(),
            prop_requests: Vec::new(),
            model_draws: Vec::new(),
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
            gibs: Default::default(),
            ragdolls: Default::default(),
            phys_explosions: Default::default(),
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
            entity_sounds: Default::default(),
            footsteps: Default::default(),
        })
    }
    /// An npc_create request (F4 Enter or the console): CC_NPC_Create placement from
    /// the eye along the aim, then a new entity copied from the class's hidden template
    /// (DESIGN 14r part A: the template made the map load every asset of the class).
    pub fn request_spawn(
        &mut self,
        request: hl2_simulation::ai::spawn::SpawnRequest,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
    ) -> String {
        use hl2_simulation::ai::spawn;
        let placement = spawn::place(
            &request,
            eye,
            forward,
            &hl2_simulation::ai::host::PhysicsPlacement { physics },
        );
        let mut entity = None;
        let message = match &placement {
            Ok(p) => match self.spawn_from_template(&request, p.origin, p.yaw, physics) {
                Ok(id) => {
                    entity = Some(id);
                    format!(
                        "Created {} ({}) at {:.0} {:.0} {:.0} yaw {:.0} (entity {id})",
                        request.classname,
                        if request.equipment.is_empty() {
                            "no weapon"
                        } else {
                            &request.equipment
                        },
                        p.origin.x,
                        p.origin.y,
                        p.origin.z,
                        p.yaw
                    )
                }
                Err(e) => e,
            },
            Err(e) => e.clone(),
        };
        self.spawn_log.push(serde_json::json!({"request":request,"placement":placement.ok(),"entity":entity,"message":message}));
        message
    }
    /// CreateEntityByName + DispatchSpawn + Activate of a copy of a loaded entity (an F4
    /// class template or a maker's template NPC; DESIGN 14r part A): the template's
    /// keyvalues at `origin` facing `yaw` (with `overrides` replacing keys), its model
    /// instance, colliders, AI actor, movement registration and (next frame) draws.
    /// The map's World grows in place (no copy while nothing else holds it).
    pub fn create_from_template(
        &mut self,
        template: usize,
        origin: Vec3,
        yaw: f32,
        overrides: &[(&str, String)],
        physics: &mut Physics,
    ) -> Result<usize, String> {
        use hl2_simulation::ai::spawn;
        let id = self.world.entities.len();
        let (mut new_entity, instance) = spawn::instantiate(&self.world, template, id, origin, yaw);
        // Keyvalues set before DispatchSpawn (maker spawnflags, npc_create's name,
        // ent_create's targetname/squadname).
        for (key, value) in overrides {
            new_entity
                .properties
                .retain(|(k, _)| !k.eq_ignore_ascii_case(key));
            new_entity.properties.push(((*key).into(), value.clone()));
        }
        let id = self.insert_entity(new_entity, instance, physics)?;
        self.npcs.copy_actor(template, id);
        self.render_spawns.push((template, id));
        self.scene.ai_set_yaw(id, yaw);
        Ok(id)
    }
    /// CreateEntityByName + DispatchSpawn's simulation part for any runtime entity:
    /// append it (and its model instance, whose id must be the next entity id) to the
    /// map's World in place, its spawn state, colliders and AI actor. The host draws it.
    pub fn insert_entity(
        &mut self,
        entity: modkit_core::Entity,
        instance: Option<modkit_core::ModelInstance>,
        physics: &mut Physics,
    ) -> Result<usize, String> {
        let live = self
            .runtime_entities
            .iter()
            .filter(|id| !self.scene.states[**id].killed)
            .count();
        let limit =
            hl2_simulation::ai::spawn::MAX_EDICTS.load(std::sync::atomic::Ordering::Relaxed);
        if limit > 0 && live >= limit {
            return Err(format!(
                "{limit} runtime entities alive (ED_Alloc: no free edicts; --max-edicts)"
            ));
        }
        if Arc::strong_count(&self.world) > 1 {
            self.world_copies += 1;
        }
        let world = Arc::make_mut(&mut self.world);
        let id = self.scene.spawn_entity(world, entity.clone());
        if let Some(instance) = instance {
            debug_assert_eq!(instance.entity, Some(id));
            world.model_instances.push(instance.clone());
            physics.add_spawned_instance(world, &instance);
        }
        self.runtime_entities.push(id);
        self.ai.add_actor(id, &entity);
        Ok(id)
    }
    /// CNPC_BaseZombie::BecomeTorso for the zombies a blast split this tick (DESIGN 14o
    /// session 37). Source keeps the zombie entity and swaps its model and hull; here a
    /// runtime npc_zombie_torso from its pool template replaces it 40 units up with the
    /// zombie's keyvalues (name, squad, outputs, flags), AI state and burning, half its max
    /// health, falling to the ground; the zombie leaves without dying. The legs are a
    /// raggib (CreateRagGib) at the zombie's origin and angles with the legs force. With
    /// a threshold-1 blast the torso dies at once (BecomeRagdollOnClient with the force).
    fn split_zombies(&mut self, physics: &mut Physics) {
        use hl2_simulation::ai::{spawn, zombie};
        let splits = std::mem::take(&mut self.scene.zombie_splits);
        let mut done = std::collections::BTreeSet::new();
        for split in splits {
            let id = split.entity;
            if !done.insert(id) || self.scene.states.get(id).is_none_or(|s| s.killed) {
                continue;
            }
            let Some(&template) = spawn::templates(&self.world, "npc_zombie_torso").first() else {
                self.zombie_split_failures
                    .push(format!("{id}: no npc_zombie_torso template"));
                continue;
            };
            let (origin, rotation) = (self.scene.states[id].origin, self.scene.states[id].rotation);
            let facing = rotation * Vec3::X;
            let yaw = facing.y.atan2(facing.x).to_degrees();
            let keep: Vec<(String, String)> = self.world.entities[id]
                .properties
                .iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.to_ascii_lowercase().as_str(),
                        "classname" | "model" | "origin" | "angles" | "body" | "skin" | "health"
                    )
                })
                .cloned()
                .collect();
            // SF_NPC_FALL_TO_GROUND: the replacement must not take StartNPC's floor drop (the
            // zombie entity natively stays the same; the torso falls from 40 units up).
            let flags = keep
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("spawnflags"))
                .and_then(|(_, v)| v.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let mut overrides: Vec<(&str, String)> = keep
                .iter()
                .filter(|(k, _)| !k.eq_ignore_ascii_case("spawnflags"))
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect();
            overrides.push((
                "spawnflags",
                (flags | hl2_simulation::npc_makers::SF_NPC_FALL_TO_GROUND).to_string(),
            ));
            let torso = match self.create_from_template(
                template,
                origin + Vec3::Z * zombie::TORSO_LIFT,
                yaw,
                &overrides,
                physics,
            ) {
                Ok(torso) => torso,
                Err(error) => {
                    self.zombie_split_failures.push(format!("{id}: {error}"));
                    continue;
                }
            };
            // The zombie leaves (no OnDeath, no ragdoll: the torso is it from now on).
            let state = &mut self.scene.states[id];
            state.killed = true;
            state.visible = false;
            state.enabled = false;
            physics.set_entity(id, origin, rotation, false);
            let now = self.scene.time;
            // No second handle on the World while the legs are created below: make_mut
            // would copy the whole map (a frame hitch).
            self.ai
                .become_torso(&self.world, &mut self.scene, id, torso);
            self.scene.states[torso].value = split.max_health;
            self.scene.npc_max_health.insert(torso, split.max_health);
            // BecomeTorso: a burning zombie's torso burns 30 s; then 50% of blasts ignite
            // it for 5-10 s (Ignite while burning does nothing).
            let flammable = hl2_simulation::fire::Interactions {
                flammable: true,
                ..Default::default()
            };
            let center = origin + Vec3::Z * (zombie::TORSO_LIFT + 12.);
            if self.scene.fires.burning(id) {
                self.scene.fires.extinguish(id, origin);
                self.scene
                    .fires
                    .ignite(torso, flammable, 30., 16., center, now);
            }
            if let Some(time) = split.ignite {
                self.scene
                    .fires
                    .ignite(torso, flammable, time, 16., center, now);
            }
            let legs = spawn::templates(&self.world, &spawn::gib_label(spawn::GIB_MODELS[0]))
                .first()
                .copied()
                .and_then(|gib| {
                    self.create_from_template(gib, origin, yaw, &[], physics)
                        .map_err(|e| self.zombie_split_failures.push(format!("{id} legs: {e}")))
                        .ok()
                });
            if let Some(legs) = legs {
                let world = self.world.clone();
                if !self.ragdolls.spawn_gib(
                    &world,
                    &mut self.scene,
                    physics,
                    legs,
                    split.legs_force,
                ) {
                    self.zombie_split_failures
                        .push(format!("{id} legs: no ragdoll"));
                }
            }
            if let Some(force) = split.dies {
                let state = &mut self.scene.states[torso];
                state.value = 0.;
                state.killed = true;
                state.visible = false;
                physics.set_entity(torso, origin, rotation, false);
                self.scene.fire(torso, "OnDeath", usize::MAX);
                self.scene.npc_deaths.push(torso);
                self.scene.npc_death_forces.insert(torso, (force, None));
            }
            self.zombie_splits.push((id, torso, legs));
        }
    }
    /// CNPC_PoisonZombie AE_ZOMBIE_POISON_THROW_CRAB (DESIGN 14o session 38): a
    /// npc_headcrab_poison from its pool template at the zombie's EyePosition with its
    /// angles, SF_NPC_FALL_TO_GROUND (and SF_NPC_FADE_CORPSE when the zombie has it), set
    /// on fire for 100 s when the zombie burns, then thrown at the enemy's eyes.
    fn throw_crabs(&mut self, physics: &mut Physics) {
        for (zombie, eye, yaw, target) in std::mem::take(&mut self.ai.crab_throws) {
            // GetHeadcrabClassname: npc_headcrab_poison (the same CBlackHeadcrab as
            // npc_headcrab_black, whose pool template stands in when the map has none).
            let Some(&template) = ["npc_headcrab_poison", "npc_headcrab_black"]
                .iter()
                .find_map(|class| {
                    hl2_simulation::ai::spawn::templates(&self.world, class)
                        .first()
                        .copied()
                })
                .as_ref()
            else {
                self.zombie_split_failures
                    .push(format!("{zombie}: no black headcrab template"));
                continue;
            };
            let zombie_flags = self.world.entities[zombie]
                .get("spawnflags")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let flags = hl2_simulation::npc_makers::SF_NPC_FALL_TO_GROUND | (zombie_flags & 512);
            let crab = match self.create_from_template(
                template,
                eye,
                yaw,
                &[("spawnflags", flags.to_string())],
                physics,
            ) {
                Ok(crab) => crab,
                Err(e) => {
                    self.zombie_split_failures
                        .push(format!("{zombie} thrown crab: {e}"));
                    continue;
                }
            };
            if self.scene.fires.burning(zombie) {
                let flammable = hl2_simulation::fire::Interactions {
                    flammable: true,
                    ..Default::default()
                };
                let now = self.scene.time;
                self.scene.fires.ignite(crab, flammable, 100., 8., eye, now);
            }
            self.ai
                .thrown_headcrab(zombie, crab, target, &mut self.scene);
            self.thrown_crabs.push(crab);
        }
    }
    /// CNPC_PoisonZombie::EvacuateNest (DESIGN 14o session 38): the crab in its hand only
    /// loses its bodygroup (the SDK adds it to the count but ejects nest slots only); each
    /// nest crab becomes a npc_headcrab_poison at its headcrab2/3/4 attachment with a random
    /// yaw (HeadcrabFits, or it is dropped), burning 100 s with a burning zombie, ejected at
    /// RandomInt(20, 50) x (U(-1,1), U(-1,1), U(0.5,1)) x 2 (0.1 x the blast damage for a
    /// blast over 10), and its nest bodygroup turns off on the corpse.
    fn evacuate_nest(&mut self, zombie: usize, physics: &mut Physics) {
        use hl2_simulation::ai::zombie;
        let Some((nest, holding, enemy)) = self.ai.take_poison_nest(zombie) else {
            return;
        };
        if holding {
            self.scene
                .set_bodygroup(&self.world, zombie, zombie::BODYGROUP_THROW, 0);
        }
        let scale = match self.scene.npc_death_blasts.get(&zombie) {
            Some(&damage) if damage > 10. => 0.1 * damage,
            _ => 2.,
        };
        let names = ["headcrab2", "headcrab3", "headcrab4"];
        let frames = self.scene.attachment_frames(&self.world, zombie, &names);
        let time = self.scene.time;
        for (i, frame) in frames.into_iter().enumerate() {
            if !nest[i] {
                continue;
            }
            let roll = |salt| hl2_simulation::fire::roll(zombie, time, 60 + i as u64 * 8 + salt);
            let origin = frame.map_or(self.scene.states[zombie].origin + Vec3::Z * 50., |f| {
                f.w_axis.truncate()
            });
            let yaw = 360. * roll(0);
            {
                use hl2_simulation::npc_probe::NpcCollisionWorld;
                let fits = !physics
                    .npc_trace_hull(
                        origin,
                        origin - Vec3::Z,
                        hl2_simulation::ai_hull::HullType::Tiny.bounds(),
                        hl2_simulation::npc_probe::Query {
                            excluded_entities: &[zombie],
                            ..Default::default()
                        },
                    )
                    .trace
                    .start_solid;
                if !fits {
                    self.zombie_split_failures
                        .push(format!("{zombie} nest crab {i} does not fit"));
                    continue;
                }
            }
            let Some(&template) = ["npc_headcrab_poison", "npc_headcrab_black"]
                .iter()
                .find_map(|class| {
                    hl2_simulation::ai::spawn::templates(&self.world, class)
                        .first()
                        .copied()
                })
                .as_ref()
            else {
                continue;
            };
            let flags = hl2_simulation::npc_makers::SF_NPC_FALL_TO_GROUND;
            let crab = match self.create_from_template(
                template,
                origin,
                yaw,
                &[("spawnflags", flags.to_string())],
                physics,
            ) {
                Ok(crab) => crab,
                Err(e) => {
                    self.zombie_split_failures
                        .push(format!("{zombie} nest crab: {e}"));
                    continue;
                }
            };
            if self.scene.fires.burning(zombie) {
                let flammable = hl2_simulation::fire::Interactions {
                    flammable: true,
                    ..Default::default()
                };
                self.scene
                    .fires
                    .ignite(crab, flammable, 100., 8., origin, time);
            }
            let speed = scale * (20. + (31. * roll(1)).floor().min(30.));
            let velocity =
                speed * Vec3::new(2. * roll(2) - 1., 2. * roll(3) - 1., 0.5 + 0.5 * roll(4));
            self.ai
                .ejected_headcrab(zombie, crab, velocity, enemy, &mut self.scene);
            self.scene
                .set_bodygroup(&self.world, zombie, zombie::BODYGROUP_NEST_BASE + i, 0);
            self.evacuated_crabs.push(crab);
        }
    }
    /// A raggib of `model` at `origin` with Source `angles` (degrees), ragdolled with `force`
    /// unless `posed_from` gives the NPC whose death pose it takes. None without a template.
    fn spawn_raggib(
        &mut self,
        model: &str,
        origin: Vec3,
        angles: Vec3,
        force: Vec3,
        posed_from: Option<usize>,
        physics: &mut Physics,
    ) -> Option<usize> {
        use hl2_simulation::ai::spawn;
        let template = *spawn::templates(&self.world, &spawn::gib_label(model)).first()?;
        let id = self
            .create_from_template(
                template,
                origin,
                angles.y,
                &[("angles", format!("{} {} {}", angles.x, angles.y, angles.z))],
                physics,
            )
            .map_err(|e| self.zombie_split_failures.push(format!("{model}: {e}")))
            .ok()?;
        let world = self.world.clone();
        match posed_from {
            Some(npc) => self
                .ragdolls
                .spawn_as(&world, &mut self.scene, physics, npc, id, TICK),
            None => {
                if !self
                    .ragdolls
                    .spawn_gib(&world, &mut self.scene, physics, id, force)
                {
                    self.zombie_split_failures
                        .push(format!("{model}: no ragdoll"));
                }
            }
        }
        Some(id)
    }
    /// ReleaseHeadcrab(RELEASE_RAGDOLL) at a lethal blast (DESIGN 14o session 37): the crab
    /// as a raggib whose root bone sits on the zombie's "headcrab" attachment
    /// (SetHeadcrabSpawnLocation, roll removed; HeadcrabFits is not checked), pushed with
    /// the damage force x 0.25; the body dies headless (the crabless classic.mdl posed
    /// like the zombie), or DieChopped splits it into legs at the origin and a headless
    /// torso 64 up pitched -90.
    fn release_crab(
        &mut self,
        release: hl2_simulation::ai::zombie::CrabRelease,
        physics: &mut Physics,
    ) -> bool {
        let npc = release.entity;
        let (origin, rotation) = (
            self.scene.states[npc].origin,
            self.scene.states[npc].rotation,
        );
        let facing = rotation * Vec3::X;
        let yaw = facing.y.atan2(facing.x).to_degrees();
        // Read the World through the field, never a second Arc handle: an entity created
        // below while another handle lives makes Arc::make_mut copy the whole map (the
        // frame hitch of every crab release before session 38).
        let world = &*self.world;
        // GetHeadcrabModel / GetHeadcrabClassname: the poison zombie's black crab.
        let poison = world.entities[npc].class() == "npc_poisonzombie";
        let (crab_model, crab_class) = if poison {
            ("models/headcrabblack.mdl", "npc_headcrab_poison")
        } else {
            ("models/headcrabclassic.mdl", "npc_headcrab")
        };
        let root = hl2_simulation::ai::spawn::templates(
            world,
            &hl2_simulation::ai::spawn::gib_label(crab_model),
        )
        .first()
        .and_then(|t| world.model_instances.iter().find(|i| i.entity == Some(*t)))
        .and_then(|i| world.rigs.get(&i.asset_key()))
        .and_then(|rig| {
            let first = rig.sequences.iter().min_by_key(|s| s.order)?;
            let skin = rig.matrices(&first.name, 0.);
            Some(*skin.first()? * rig.bones.first()?.inverse_bind.inverse())
        });
        let attachment = self.scene.attachment_frames(world, npc, &["headcrab"])[0];
        let (crab_origin, crab_angles) = match (attachment, root) {
            (Some(frame), Some(root)) => {
                let (_, r, t) = (frame * root.inverse()).to_scale_rotation_translation();
                let (y, p, _) = r.to_euler(glam::EulerRot::ZYX);
                (t, Vec3::new(p.to_degrees(), y.to_degrees(), 0.))
            }
            // EyePosition with the zombie's angles.
            _ => (origin + Vec3::Z * 64., Vec3::new(0., yaw, 0.)),
        };
        // The headless body: the crabless model of the zombie's own kind.
        let corpse_model = if world.entities[npc].class() == "npc_zombie_torso" {
            "models/zombie/classic_torso.mdl"
        } else {
            "models/zombie/classic.mdl"
        };
        if release.live {
            // CNPC_BaseZombie::HeadcrabFits: a tiny hull 1 unit down from the spawn spot
            // must not start solid (ignoring the zombie); otherwise no crab is released
            // and the zombie dies whole.
            use hl2_simulation::npc_probe::NpcCollisionWorld;
            let fits = !physics
                .npc_trace_hull(
                    crab_origin,
                    crab_origin - Vec3::Z,
                    hl2_simulation::ai_hull::HullType::Tiny.bounds(),
                    hl2_simulation::npc_probe::Query {
                        excluded_entities: &[npc],
                        ..Default::default()
                    },
                )
                .trace
                .start_solid;
            if !fits {
                self.zombie_split_failures
                    .push(format!("{npc} crab does not fit"));
                return false;
            }
            // RELEASE_IMMEDIATE: a live npc_headcrab (no pitch), the zombie's squad and
            // inherited flags (gag, long range, fade corpse, always think) plus
            // SF_NPC_FALL_TO_GROUND; the body ragdolls without a force.
            let flags = self.world.entities[npc]
                .get("spawnflags")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let mut overrides = vec![(
                "spawnflags",
                ((flags & (2 | 256 | 512 | 1024))
                    | hl2_simulation::npc_makers::SF_NPC_FALL_TO_GROUND)
                    .to_string(),
            )];
            if let Some(squad) = self.world.entities[npc].get("squadname") {
                overrides.push(("squadname", squad.to_owned()));
            }
            let crab = hl2_simulation::ai::spawn::templates(&self.world, crab_class)
                .first()
                .copied()
                .and_then(|template| {
                    self.create_from_template(
                        template,
                        crab_origin,
                        crab_angles.y,
                        &overrides,
                        physics,
                    )
                    .map_err(|e| self.zombie_split_failures.push(format!("{npc} crab: {e}")))
                    .ok()
                });
            if let Some(crab) = crab {
                let now = self.scene.time;
                self.ai.released_headcrab(npc, crab, now);
                // ShouldIgniteZombieGib: a burning zombie's crab burns 30 s.
                if self.scene.fires.burning(npc) {
                    let flammable = hl2_simulation::fire::Interactions {
                        flammable: true,
                        ..Default::default()
                    };
                    self.scene
                        .fires
                        .ignite(crab, flammable, 30., 8., crab_origin, now);
                }
                self.released_crabs.push(crab);
            }
            self.scene.npc_death_forces.insert(npc, (Vec3::ZERO, None));
            self.crab_releases.push((npc, false));
            // fRemoveHead on the zombie itself (runtime bodygroups); its ragdoll follows.
            if poison {
                self.scene.set_bodygroup(&self.world, npc, 1, 0);
                return false;
            }
            self.spawn_raggib(
                corpse_model,
                origin,
                Vec3::new(0., yaw, 0.),
                Vec3::ZERO,
                Some(npc),
                physics,
            );
            return true;
        }
        self.spawn_raggib(
            crab_model,
            crab_origin,
            crab_angles,
            release.crab_force,
            None,
            physics,
        );
        if poison {
            // The headless poison zombie ragdolls itself with its death force.
            self.scene.set_bodygroup(&self.world, npc, 1, 0);
            self.crab_releases.push((npc, false));
            return false;
        }
        match release.chopped {
            Some((legs, torso)) => {
                self.spawn_raggib(
                    "models/zombie/classic_legs.mdl",
                    origin,
                    Vec3::new(0., yaw, 0.),
                    legs,
                    None,
                    physics,
                );
                self.spawn_raggib(
                    "models/zombie/classic_torso.mdl",
                    origin + Vec3::Z * 64.,
                    Vec3::new(-90., yaw, 0.),
                    torso,
                    None,
                    physics,
                );
            }
            None => {
                self.spawn_raggib(
                    corpse_model,
                    origin,
                    Vec3::new(0., yaw, 0.),
                    Vec3::ZERO,
                    Some(npc),
                    physics,
                );
            }
        }
        self.crab_releases.push((npc, release.chopped.is_some()));
        true
    }
    /// CBaseCombatCharacter::Event_Killed -> Weapon_Drop (DESIGN 14r part B): each NPC
    /// killed this tick drops its active weapon as a new weapon entity with VPhysics
    /// (FallInit; the world model's .phy is preloaded with the NPC weapons), refilled
    /// like an NPC drop (default clip; SMG1: enough player shots to kill two of the
    /// dropper), in front of its shoot position (no hand-bone pose on the simulation
    /// side yet) and thrown 64-128 u/s along the body (the SDK branch without a death
    /// force) with AngularImpulse(200, 200, 200). SF_NPC_NO_WEAPON_DROP removes it.
    fn drop_weapons(&mut self, physics: &mut Physics) {
        let deaths = std::mem::take(&mut self.scene.npc_deaths);
        let releases = std::mem::take(&mut self.scene.crab_releases);
        // Event_Killed -> BecomeRagdollOnClient before the weapon leaves the hand.
        for &npc in &deaths {
            self.evacuate_nest(npc, physics);
            if let Some(release) = releases.iter().find(|r| r.entity == npc)
                && self.release_crab(*release, physics)
            {
                continue;
            }
            let world = self.world.clone();
            self.ragdolls
                .spawn(&world, &mut self.scene, physics, npc, TICK);
        }
        // This tick's client ragdoll effects (shots, explosions, crossbow pins).
        self.ragdolls.impacts(&mut self.scene, physics);
        self.ragdolls.track(&self.world, &self.scene);
        self.scene.npc_death_forces.clear();
        self.scene.npc_death_blasts.clear();
        self.scene.npc_hit_forces.clear();
        for npc in deaths {
            let Some(class) = self.ai.weapon_class(npc) else {
                continue;
            };
            let Some(model) = self.ai.weapon_model(&class) else {
                continue;
            };
            let flags = self.world.entities[npc]
                .get("spawnflags")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(0);
            if flags & (1 << 13) != 0
                || !self.world.model_physics.contains_key(&format!("{model}#0"))
            {
                continue;
            }
            let state = &self.scene.states[npc];
            let body = state.rotation * Vec3::X;
            let flat = Vec3::new(body.x, body.y, 0.).normalize_or(Vec3::X);
            // Weapon_Drop teleports a bone-merged weapon to the owner's Weapon_bone pose
            // (times its inverse root bone); without a rig, in front of the shoot position.
            let (origin, angles) = self
                .scene
                .merged_model_frame(&self.world, npc, &format!("{model}#0"))
                .map(|frame| {
                    let (_, rotation, translation) = frame.to_scale_rotation_translation();
                    let (yaw, pitch, roll) = rotation.to_euler(glam::EulerRot::ZYX);
                    (
                        translation,
                        Vec3::new(pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees()),
                    )
                })
                .unwrap_or_else(|| {
                    let yaw = flat.y.atan2(flat.x).to_degrees();
                    (
                        state.origin + Vec3::Z * 55. + flat * 10.,
                        Vec3::new(0., yaw, 0.),
                    )
                });
            let clip = if class == "weapon_smg1" {
                let health = hl2_simulation::gameplay::npc_spawn_health(
                    &self.scene,
                    &self.world.entities[npc],
                );
                let shot = self.scene.skill.get("sk_plr_dmg_smg1", 4.)
                    * self.scene.skill.player_inflict_scale();
                ((health / shot.max(0.01)) * 2.) as i32
            } else {
                self.weapons.get(&class).map_or(0, |w| w.default_clip)
            };
            let id = self.world.entities.len();
            let entity = modkit_core::Entity {
                properties: [
                    ("classname", class.clone()),
                    ("origin", format!("{} {} {}", origin.x, origin.y, origin.z)),
                    ("angles", format!("{} {} {}", angles.x, angles.y, angles.z)),
                    ("model", model.clone()),
                    ("ammo", clip.to_string()),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            };
            let instance = modkit_core::ModelInstance {
                background: false,
                model: model.clone(),
                origin,
                angles,
                skin: 0,
                body: 0,
                scale: 1.,
                // A VPhysics object like a physics prop (its .phy and propdata).
                kind: "prop_physics".into(),
                solid: true,
                solid_mode: None,
                entity: Some(id),
            };
            let Ok(id) = self.insert_entity(entity, Some(instance), physics) else {
                continue;
            };
            // FallInit: COLLISION_GROUP_WEAPON (the player and NPCs pass over it).
            physics.set_weapon_group(id);
            let speed = 64. + (npc.wrapping_mul(2_654_435_761) % 65) as f32;
            if let Some(handle) = physics.entity_body(id) {
                physics.set_body_velocity(handle, body * speed, Vec3::splat(200.));
            }
            self.model_draws.push(id);
            self.spawn_log
                .push(serde_json::json!({"weapon_drop":class,"npc":npc,"entity":id,"ammo":clip,"origin":origin.to_array(),"angles":angles.to_array(),"npc_origin":self.scene.states[npc].origin.to_array()}));
        }
    }
    /// Merge a late-loaded model's tables into the map's World (keys it lacks only).
    pub fn merge_model(&mut self, loaded: World) {
        if Arc::strong_count(&self.world) > 1 {
            self.world_copies += 1;
        }
        let world = Arc::make_mut(&mut self.world);
        for (k, v) in loaded.model_assets {
            world.model_assets.entry(k).or_insert(v);
        }
        for (k, v) in loaded.model_collision {
            world.model_collision.entry(k).or_insert(v);
        }
        for (k, v) in loaded.rigs {
            world.rigs.entry(k).or_insert(v);
        }
        for (k, v) in loaded.illumination {
            world.illumination.entry(k).or_insert(v);
        }
        for (k, v) in loaded.model_physics {
            world.model_physics.entry(k).or_insert(v);
        }
        for (k, v) in loaded.model_mass {
            world.model_mass.entry(k).or_insert(v);
        }
        for (k, v) in loaded.model_prop_health {
            world.model_prop_health.entry(k).or_insert(v);
        }
        for (k, v) in loaded.model_prop_data {
            world.model_prop_data.entry(k).or_insert(v);
        }
        for (k, v) in loaded.model_breaks {
            world.model_breaks.entry(k).or_insert(v);
        }
        world.warnings.extend(loaded.warnings);
    }
    /// The F4/npc_create part: a class template, then the request's weapon, side and
    /// grenades.
    fn spawn_from_template(
        &mut self,
        request: &hl2_simulation::ai::spawn::SpawnRequest,
        origin: Vec3,
        yaw: f32,
        physics: &mut Physics,
    ) -> Result<usize, String> {
        use hl2_simulation::ai::spawn;
        let label = request.pool_label().to_owned();
        let templates = spawn::templates(&self.world, &label);
        if templates.is_empty() {
            return Err(format!(
                "Can't create {}: the class was not loaded on this map",
                request.classname
            ));
        }
        let template = if label == "npc_citizen" {
            // CNPC_Citizen::SelectModel: the least-used head among living citizens.
            let roll = self.ai.uniform() * 0.5 + 0.5;
            let states = &self.scene.states;
            hl2_simulation::citizens::least_used_template(
                &self.world,
                &templates,
                spawn::POOL_KEY,
                |id| states.get(id).is_some_and(|s| !s.killed),
                roll,
            )
            .unwrap_or(templates[0])
        } else {
            let count = self.spawn_counts.entry(label).or_default();
            let template = templates[*count % templates.len()];
            *count += 1;
            template
        };
        let id = self
            .create_from_template(template, origin, yaw, &request.keyvalues(), physics)
            .map_err(|e| format!("Can't create {}: {e}", request.classname))?;
        self.ai
            .set_equipment(id, &request.classname, &request.equipment);
        // Its walk/run follow the new weapon (preloaded variants).
        self.npcs.set_locomotion(id, &request.equipment);
        self.ai
            .set_player_disposition(id, request.allegiance.disposition());
        if let Some(grenades) = request.grenades {
            self.ai.set_grenades(id, grenades);
        }
        // The new weapon's idle (Weapon_TranslateActivity of ACT_IDLE); the AI picks its
        // own activities from here when it runs this class.
        let idle = hl2_simulation::entities::weapon_idle_activity(&request.equipment);
        let world = self.world.clone();
        self.scene.ai_set_activity(&world, id, idle);
        Ok(id)
    }
    fn push_shot_effect(&mut self, effect: ShotEffect) {
        self.shot_serial += 1;
        self.shot_effects
            .push((self.shot_serial, self.scene.time, effect));
    }
    fn ai_player_state(
        &self,
        player: &Player,
        fly: bool,
        facing: glam::Vec3,
    ) -> hl2_simulation::ai::runtime::PlayerState {
        hl2_simulation::ai::runtime::PlayerState {
            facing,
            feet: player.feet,
            velocity: player.velocity,
            crouched: player.crouched,
            on_ground: player.grounded,
            dead: player.dead,
            suit: self.inventory.suit,
            noclip: fly,
        }
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
                    // CPlayerPickupController first (drop while carrying, pick up a
                    // small prop), then the existing door/button use.
                    if self.inventory.player_use(
                        &self.weapons,
                        &self.world,
                        &mut self.scene,
                        physics,
                        eye,
                        direction,
                        player.feet,
                        player.crouched,
                    ) {
                        continue;
                    }
                    if let Some((id, _)) = physics.ray(eye, direction, 96.) {
                        let state = self.ai_player_state(player, fly, direction);
                        if !self
                            .ai
                            .player_used(id, &self.world, &mut self.scene, physics, state)
                        {
                            self.scene.use_entity_at(&self.world, id, player.feet);
                        }
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
                Action::PropPose {
                    entity,
                    origin,
                    angles,
                    velocity,
                    launched,
                } => {
                    if launched {
                        physics.launched.insert(entity);
                    }
                    if !physics.place_body(
                        entity,
                        Vec3::from_array(origin),
                        Vec3::from_array(angles),
                        Vec3::from_array(velocity),
                    ) {
                        *self
                            .scene
                            .diagnostics
                            .unsupported
                            .entry(format!("fixture prop pose: no dynamic body {entity}"))
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
        // VPhysics objects move in the physics world; scene logic (trigger touches of
        // physics objects and their filters) reads their current pose.
        for &id in physics.dynamic.keys() {
            if let (Some((origin, rotation)), Some(state)) =
                (physics.entity_pose(id), self.scene.states.get_mut(id))
            {
                state.origin = origin;
                state.rotation = rotation;
            }
        }
        self.scene.tick(&self.world, player.feet, TICK);
        // npc_template_maker spawns and CAI_BaseNPC::StartNPC floor drops (DESIGN 14s).
        hl2_simulation::npc_makers::tick(
            &self.world,
            &mut self.scene,
            physics,
            (!player.dead).then_some(hl2_simulation::npc_makers::Viewer {
                eye,
                forward: direction,
                feet: player.feet,
                crouched: player.crouched,
                notarget: self.ai.ignore_player,
            }),
            TICK,
        );
        // Maker spawns after the first: new entities copied from the held template.
        for request in std::mem::take(&mut self.scene.makers.copy_requests) {
            let child = self
                .create_from_template(
                    request.template,
                    request.origin,
                    request.yaw,
                    &[("spawnflags", request.spawnflags.to_string())],
                    physics,
                )
                .ok();
            let world = self.world.clone();
            let mut makers = std::mem::take(&mut self.scene.makers);
            makers.adopt(&world, &mut self.scene, &request, child);
            self.scene.makers = makers;
        }
        self.inventory.advance_projectile_fire(
            &self.world,
            &mut self.scene,
            &self.weapons,
            eye,
            direction,
        );
        // CBasePlayer::PostThink -> CPlayerPickupController::Use(USE_SET): held attack
        // throws, held attack2 drops; the holstered weapon does not fire meanwhile.
        let carrying = self.inventory.carrying();
        if carrying {
            self.inventory.carry_tick(
                &self.weapons,
                &self.world,
                &mut self.scene,
                physics,
                eye,
                direction,
                player.feet,
                player.crouched,
                self.primary,
                self.secondary,
            );
            self.primary_consumed |= self.primary;
            self.secondary_consumed |= self.secondary;
        }
        let allowed = self.selection.pending.is_none() && !carrying;
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
                    | "weapon_physcannon"
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
            let active = self.inventory.active.clone();
            let clip = self.inventory.owned.get(&active).copied();
            self.inventory.attack(
                &self.weapons,
                &self.world,
                &mut self.scene,
                physics,
                eye,
                direction,
            );
            // The weapons' PrimaryAttack: CSoundEnt::InsertSound(SOUND_COMBAT, origin,
            // SOUNDENT_VOLUME_PISTOL/MACHINEGUN/SHOTGUN 1500 (the 357: 600), 0.2).
            let volume = match active.as_str() {
                "weapon_pistol" | "weapon_smg1" | "weapon_ar2" | "weapon_shotgun" => Some(1500.),
                "weapon_357" => Some(600.),
                _ => None,
            };
            if let Some(volume) =
                volume.filter(|_| self.inventory.owned.get(&active).copied() < clip)
            {
                self.ai
                    .insert_combat_sound(player.feet, volume, self.scene.time);
            }
        }
        // weapon_physcannon: ItemPreFrame/ItemPostFrame on the Rapier props (weapons stage).
        self.inventory.physcannon_frame(
            &self.weapons,
            &self.world,
            &mut self.scene,
            physics,
            eye,
            direction,
            player.feet,
            player.crouched,
        );
        self.queued_primary = false;
        self.queued_secondary = false;
        for (id, state) in self.scene.states.iter().enumerate() {
            physics.set_entity(id, state.origin, state.rotation, state.collides());
        }
        // Studio hitboxes follow each NPC's animated skeleton (bullet hitgroups).
        hl2_simulation::gameplay::pose_npc_hitboxes(&self.world, &self.scene, physics);
        // weapon_frag throws from this tick's animation events.
        if !self.inventory.frag.pending.is_empty() {
            let ground = physics
                .ground_ray(player.feet + glam::Vec3::Z * 4., -glam::Vec3::Z, 16.)
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
        // NPC missiles seek their owner's dot while it guides (TASK_CIT_RPG_AUGER).
        for missile in &mut self.projectiles.active {
            if missile.kind != hl2_simulation::projectiles::ProjectileKind::RpgMissile {
                continue;
            }
            if let Some(owner) = missile.npc_owner() {
                missile.npc_dot = self.ai.npc_laser(owner).map(|(position, shoot)| {
                    hl2_simulation::weapon_rpg::LaserDot::npc(position, shoot)
                });
            }
        }
        physics.refresh_entity_queries();
        // NPC stage: the AI thinks (when enabled) before the movement controller tick.
        let ai_player = self.ai_player_state(player, fly, direction);
        self.ai.tick(
            &self.world,
            &mut self.scene,
            &mut self.npcs,
            physics,
            &[],
            ai_player,
        );
        self.scene.npc_knows_player = self.ai.npcs_knowing_player();
        self.throw_crabs(physics);
        // Zombie swats (AE_ZOMBIE_SWATITEM -> IPhysicsObject::AddVelocity).
        for (object, velocity, angular) in std::mem::take(&mut self.ai.prop_swats) {
            physics.add_entity_velocity(object, velocity, angular);
        }
        for hit in std::mem::take(&mut self.ai.player_hits) {
            if hit.damage > 0. {
                self.scene
                    .player_damage
                    .push(hl2_simulation::player_damage::DamageInfo {
                        amount: hit.damage,
                        kind: hl2_simulation::player_damage::DMG_CLUB,
                        inflictor: self.scene.states[hit.attacker].origin,
                    });
            }
            self.player_impulse += hit.impulse;
            if let Some(fade) = hit.fade {
                self.scene.screen_fades.push(fade);
            }
            // ViewPunch needs the punch-angle path of the arsenal branch (not here).
        }
        // Combine grenade tosses: Fraggrenade_Create (NPC damage, 3.5 s fuse; the soldier
        // is the thrower and the blast's attacker).
        for (thrower, position, velocity, spin) in std::mem::take(&mut self.ai.grenade_tosses) {
            let damage = self.scene.skill.get("sk_npc_dmg_fraggrenade", 75.);
            let radius = self.scene.skill.get("sk_fraggrenade_radius", 250.);
            let at = self.scene.time;
            self.projectiles.spawn_npc(
                hl2_simulation::projectiles::ProjectileSpawn {
                    angles: Some(glam::Vec3::ZERO),
                    kind: hl2_simulation::projectiles::ProjectileKind::FragGrenade,
                    position,
                    velocity,
                    angular_velocity: spin,
                    at,
                    damage,
                    radius,
                    mass: 0.,
                    lifetime: 3.5,
                },
                thrower,
                &mut self.scene,
            );
        }
        // NPC melee (stunstick swings on hated NPCs): DMG_CLUB through the shared hit path.
        for melee in std::mem::take(&mut self.ai.melee_hits) {
            let mut blood = Vec::new();
            let origin = self.scene.states[melee.attacker].origin + glam::Vec3::Z * 36.;
            hl2_simulation::gameplay::apply_hit(
                &self.world,
                &mut self.scene,
                physics,
                melee.damage,
                "",
                &hl2_simulation::physics::RayHit {
                    entity: melee.victim,
                    position: melee.position,
                    normal: -melee.direction,
                    hitgroup: 0,
                    bone: None,
                },
                melee.direction,
                true,
                hl2_simulation::gameplay::HitSource {
                    player: false,
                    npc: Some(melee.attacker),
                    origin,
                },
                &mut || self.ai.uniform(),
                &mut blood,
            );
            self.ai.npc_attacked(melee.victim, melee.attacker);
            for (hit, color) in blood {
                self.impacts
                    .add_blood(hit, color, &self.world, physics, &self.scene);
            }
        }
        // Elite combine balls (CreateCombineBall: sk_weapon_ar2_alt_fire_radius/mass/
        // duration; the AR2's own ball damage).
        for (owner, position, velocity) in self.ai.take_ball_launches() {
            let Some(ar2) = self.weapons.get("weapon_ar2") else {
                continue;
            };
            let launch = hl2_simulation::projectiles::ProjectileSpawn {
                angles: None,
                kind: hl2_simulation::projectiles::ProjectileKind::CombineBall,
                position,
                velocity,
                angular_velocity: glam::Vec3::ZERO,
                at: self.scene.time,
                damage: ar2.secondary_damage,
                radius: ar2.secondary_radius,
                mass: ar2.secondary_mass,
                lifetime: ar2.secondary_lifetime,
            };
            self.projectiles
                .spawn_npc_ball(launch, owner, &mut self.scene);
        }
        // NPC RPG launches (CWeaponRPG::Operator_HandleAnimEvent -> CMissile::Create with
        // the NPC as owner, a 0.5 s grace period); the rest are FireBullets.
        let (missiles, shots): (Vec<_>, Vec<_>) = std::mem::take(&mut self.ai.shots)
            .into_iter()
            .partition(|s| s.missile);
        for shot in missiles {
            let forward = shot.directions.first().copied().unwrap_or(glam::Vec3::X);
            let at = self.scene.time;
            self.projectiles.spawn_npc_missile(
                hl2_simulation::projectiles::ProjectileSpawn {
                    angles: None,
                    kind: hl2_simulation::projectiles::ProjectileKind::RpgMissile,
                    position: shot.origin,
                    velocity: forward * 300. + glam::Vec3::Z * 128.,
                    angular_velocity: glam::Vec3::ZERO,
                    at,
                    damage: hl2_simulation::weapon_rpg::MISSILE_DAMAGE,
                    radius: hl2_simulation::weapon_rpg::EXPLOSION_RADIUS,
                    mass: 0.,
                    lifetime: 0.5,
                },
                shot.attacker,
                &mut self.scene,
            );
        }
        // NPC weapons' FireBullets (DESIGN 14w): the shared hit path, the player's hull,
        // impacts and blood like the player's own shots.
        if !shots.is_empty() {
            let target = (!player.dead).then_some(hl2_simulation::gameplay::PlayerTarget {
                feet: player.feet,
                crouched: player.crouched,
                yaw: direction.y.atan2(direction.x).to_degrees(),
            });
            let mut bullets = hl2_simulation::gameplay::NpcBullets::default();
            for shot in &shots {
                hl2_simulation::gameplay::npc_fire_bullets(
                    &self.world,
                    &mut self.scene,
                    physics,
                    shot,
                    target,
                    &mut || self.ai.uniform(),
                    &mut bullets,
                );
            }
            for &(victim, attacker) in &bullets.npc_hits {
                self.ai.npc_attacked(victim, attacker);
            }
            for (inflictor, amount) in bullets.player_hits {
                self.ai.diagnostics.bullet_hits_on_player += 1;
                self.scene
                    .player_damage
                    .push(hl2_simulation::player_damage::DamageInfo {
                        amount,
                        kind: hl2_simulation::player_damage::DMG_BULLET,
                        inflictor,
                    });
            }
            for (hit, direction) in bullets.impacts {
                self.impacts
                    .add_shot(hit, direction, &self.world, physics, &mut self.scene);
            }
            for (hit, color) in bullets.blood {
                self.impacts
                    .add_blood(hit, color, &self.world, physics, &self.scene);
            }
            // ComputeTracerStartPosition: an NPC's tracer starts at its weapon's first
            // attachment (the muzzle), else at the shoot position.
            for (origin, normal) in std::mem::take(&mut bullets.ar2_impacts) {
                self.push_shot_effect(ShotEffect::Ar2Impact(origin, normal));
            }
            for (attacker, start, end, ar2) in bullets.tracers {
                let start = self
                    .ai
                    .weapon_world_model(attacker)
                    .and_then(|model| {
                        self.scene.merged_attachment_frame(
                            &self.world,
                            attacker,
                            &format!("{model}#0"),
                            None,
                            1,
                        )
                    })
                    .map_or(start, |frame| frame.w_axis.truncate());
                self.push_shot_effect(ShotEffect::NpcTracer(start, end, ar2));
            }
        }
        // AE_NPC_MUZZLEFLASH: the weapon world model's named attachment on its owner.
        for (npc, options) in std::mem::take(&mut self.ai.muzzle_flashes) {
            let Some((kind, attachment)) = hl2_simulation::muzzle_effects::parse(&options) else {
                continue;
            };
            if let Some(frame) = self.ai.weapon_world_model(npc).and_then(|model| {
                self.scene.merged_attachment_frame(
                    &self.world,
                    npc,
                    &format!("{model}#0"),
                    Some(&attachment),
                    1,
                )
            }) {
                self.push_shot_effect(ShotEffect::NpcFlash(kind, frame));
                if kind == hl2_simulation::muzzle_effects::MuzzleKind::Combine {
                    self.push_shot_effect(ShotEffect::MuzzleLight(
                        npc,
                        frame.w_axis.truncate(),
                        true,
                    ));
                }
            }
        }
        // DoMuzzleFlash -> the weapon world model's ProcessMuzzleFlashEvent at attachment 1.
        for npc in std::mem::take(&mut self.ai.weapon_muzzle_flashes) {
            if let Some(frame) = self.ai.weapon_world_model(npc).and_then(|model| {
                self.scene
                    .merged_attachment_frame(&self.world, npc, &format!("{model}#0"), None, 1)
            }) {
                self.push_shot_effect(ShotEffect::MuzzleLight(npc, frame.w_axis.truncate(), false));
            }
        }
        for (start, end, right, ar2) in std::mem::take(&mut self.inventory.tracer_requests) {
            self.push_shot_effect(ShotEffect::PlayerTracer(start, end, right, ar2));
        }
        for (origin, normal) in std::mem::take(&mut self.inventory.ar2_impacts) {
            self.push_shot_effect(ShotEffect::Ar2Impact(origin, normal));
        }
        self.stunstick_glows = self
            .ai
            .active_stunsticks()
            .into_iter()
            .filter(|id| {
                self.scene
                    .states
                    .get(*id)
                    .is_some_and(|s| s.visible && !s.killed)
            })
            .filter_map(|id| {
                let model = self.ai.weapon_world_model(id)?;
                self.scene
                    .merged_attachment_frame(&self.world, id, &format!("{model}#0"), None, 1)
            })
            .collect();
        let horizon = self.scene.time - 0.5;
        self.shot_effects.retain(|(_, at, _)| *at >= horizon);
        actors::tick_npcs(
            &mut self.npcs,
            &mut self.scene,
            &self.world,
            physics,
            player,
            fly,
        );
        // CAI_BaseActor::MaintainLookTargets random glances (0.1 s NPC think).
        hl2_simulation::attention::think(
            &mut self.scene,
            &self.world,
            physics,
            player.velocity.length() > 1.,
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
        for owner in std::mem::take(&mut self.projectiles.npc_missiles_died) {
            self.ai.npc_missile_died(owner);
        }
        for event in std::mem::take(&mut self.projectiles.missile_events) {
            self.inventory
                .rpg_missile_event(event, &self.weapons, &self.world, &mut self.scene);
        }
        // env_physexplosion pushes (and blast damage) from this tick's inputs.
        self.phys_explosions
            .apply(&self.world, &mut self.scene, physics);
        self.phys_explosions
            .apply_impacts(&self.world, &mut self.scene, physics);
        // CEntityFlame::FlameThink: burning props hurt themselves and what is near them.
        if !self.scene.fires.flames.is_empty() {
            let states = &self.scene.states;
            let center = |id: usize| -> Option<glam::Vec3> {
                let state = states.get(id).filter(|s| !s.killed)?;
                Some(
                    physics
                        .entity_body(id)
                        .and_then(|b| physics.center_of_mass(b))
                        .unwrap_or(state.origin + glam::Vec3::Z * 36.),
                )
            };
            let others: Vec<(usize, glam::Vec3)> = self
                .world
                .entities
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.class().starts_with("prop_physics") || e.class().starts_with("npc_")
                })
                .filter_map(|(id, _)| center(id).map(|c| (id, c)))
                .collect();
            let player_center = (!player.dead).then(|| player.feet + glam::Vec3::Z * 36.);
            let burns = self
                .scene
                .fires
                .think(self.scene.time, center, &others, player_center);
            damage.extend(burns);
        }
        // trigger_hurt touchers from this tick's scene logic share the entity path.
        damage.append(&mut self.scene.entity_damage);
        self.inventory
            .apply_projectile_damage(damage, &self.world, &mut self.scene, physics);
        // Explosive props broken by that damage explode before their gibs spawn; chains
        // resolve within the tick (bounded).
        for _ in 0..32 {
            if self.scene.prop_explosions.is_empty() {
                break;
            }
            let blast = self.projectiles.explode_props(
                &self.world,
                &mut self.scene,
                physics,
                player.feet,
                player.crouched,
            );
            self.inventory
                .apply_projectile_damage(blast, &self.world, &mut self.scene, physics);
        }
        self.split_zombies(physics);
        let fire_sounds = std::mem::take(&mut self.scene.fires.sounds);
        self.scene.sounds.extend(fire_sounds);
        // PropBreakableCreateAll for props broken by this tick's damage.
        self.gibs.spawn(
            std::mem::take(&mut self.scene.prop_breaks),
            &self.world,
            physics,
            self.scene.time,
        );
        self.ragdolls
            .spawn_props(&self.world, &mut self.scene, physics);
        physics.tick(TICK);
        self.ragdolls.update(&mut self.scene, physics);
        // VPhysicsShadowCollision impact damage to NPCs (queued for the next tick).
        let held = |id: usize| {
            self.inventory.physcannon.held == Some(id)
                || self
                    .inventory
                    .carry
                    .as_ref()
                    .is_some_and(|c| c.entity == id)
        };
        hl2_simulation::impact_damage::queue_npc_damage(
            &physics.resolved_collisions,
            &self.world,
            &mut self.scene,
            &held,
        );
        hl2_simulation::impact_damage::queue_prop_damage(
            &physics.resolved_collisions,
            &self.world,
            &mut self.scene,
            &mut physics.launched,
            &mut physics.thrown_by_player,
            &held,
        );
        // VPhysicsUpdate: a sleeping prop forgets that the player threw it.
        let sleeping: Vec<usize> = physics
            .thrown_by_player
            .iter()
            .copied()
            .filter(|id| {
                physics
                    .dynamic
                    .get(id)
                    .and_then(|h| physics.bodies.get(*h))
                    .is_none_or(|b| b.is_sleeping())
            })
            .collect();
        for id in sleeping {
            physics.thrown_by_player.remove(&id);
        }
        for (prop, npc) in std::mem::take(&mut self.scene.thrown_hits) {
            let world = self.world.clone();
            self.ai
                .hit_by_player_thrown_object(npc, prop, &world, &mut self.scene);
        }
        // Gib poses and SUB_FadeOut (not while seen or held).
        self.gibs
            .tick(physics, self.scene.time, TICK, eye, direction, held);
        // The ragdoll LRU retires the oldest beyond g_ragdoll_maxcount by fading them.
        self.ragdolls
            .retire(&mut self.scene, physics, eye, direction, TICK);
        // NPCs killed this tick drop their weapons.
        self.drop_weapons(physics);
        // VPhysics impact sounds of props and thrown frags (PhysCollisionSound).
        self.impacts
            .physics_collision_sounds(physics, &self.world, &mut self.scene);
        for event in
            self.weapon_sounds
                .events(&self.world, &self.inventory, &self.weapons, self.scene.time)
        {
            self.scene.sounds.push(event.options.into());
        }
        for sound in self.entity_sounds.events(&self.world, &self.scene, TICK) {
            match sound {
                hl2_simulation::sounds::AnimationSound::Named(request) => {
                    self.scene.sounds.push(request)
                }
                hl2_simulation::sounds::AnimationSound::Material {
                    entity,
                    origin,
                    left,
                    volume,
                } => {
                    // MaterialFootstepSound: the surface 48 units under the foot (the
                    // entity origin stands in for the LeftFoot/RightFoot attachment).
                    let step = self
                        .impacts
                        .ground_property(
                            &self.world,
                            physics,
                            &self.scene,
                            origin + Vec3::Z * 2.,
                            48.,
                        )
                        .map(|prop| self.impacts.step_surface(&prop));
                    let name = step.and_then(|s| if left { s.step_left } else { s.step_right });
                    if let Some(name) = name {
                        self.scene
                            .sounds
                            .push(hl2_simulation::sounds::SoundRequest {
                                origin: Some(origin),
                                volume: Some(volume),
                                emitter: Some(entity as u64),
                                ..name.as_str().into()
                            });
                    }
                }
            }
        }
        for (hit, _melee, direction) in self.inventory.impacts.drain(..) {
            self.impacts
                .add_shot(hit, direction, &self.world, physics, &mut self.scene);
        }
        // TraceBleed: blood on the world behind shot NPCs.
        for (hit, color) in self.inventory.blood.drain(..) {
            self.impacts
                .add_blood(hit, color, &self.world, physics, &self.scene);
        }
        // Projectile UTIL_ImpactTrace (crossbow bolts in the world).
        for hit in std::mem::take(&mut self.projectiles.impacts) {
            self.impacts.add(hit, &self.world, physics, &mut self.scene);
        }
        // Projectile UTIL_DecalTrace (the bug bait's BeerSplash).
        for (hit, material) in std::mem::take(&mut self.projectiles.decals) {
            self.impacts
                .add_decal(hit, material, &self.world, physics, &self.scene);
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
        // BecomeTorso replacements: (zombie, torso, legs), each torso's origin, death, health.
        let zombies = serde_json::json!({"splits":self.zombie_splits,"torsos":self.zombie_splits.iter().map(|(_, t, _)| (t.to_string(), (self.scene.states[*t].origin.to_array(), self.scene.states[*t].killed, self.scene.states[*t].value))).collect::<std::collections::BTreeMap<_,_>>(),"failures":self.zombie_split_failures,"crab_releases":self.scene.zombie_crab_releases,"released":self.crab_releases,"live_crabs":self.released_crabs,"thrown_crabs":self.thrown_crabs,"evacuated_crabs":self.evacuated_crabs,"chops":self.scene.zombie_chops});
        let ragdolls = serde_json::json!({"active":self.ragdolls.active.len(),"spawned":self.ragdolls.spawned,"skipped":self.ragdolls.skipped,"removed":self.ragdolls.removed,"shot_pushes":self.ragdolls.shot_pushes,"blast_pushes":self.ragdolls.blast_pushes,"pins":self.ragdolls.pins,"death_poses":self.ragdolls.death_poses,"bolt_misses":self.ragdolls.bolt_misses,"last_missed_bolt":self.ragdolls.last_missed_bolt.map(|(a,b)| [a.to_array(),b.to_array()]),"missed_bolt_bodies":self.ragdolls.missed_bolt_bodies.iter().map(|v| v.to_array()).collect::<Vec<_>>(),"fading":self.ragdolls.fading(),"shots_seen":self.ragdolls.shots_seen,"shots_busy":self.ragdolls.shots_busy,"simulating":self.ragdolls.simulating(physics),"forced_sleeps":self.ragdolls.forced_sleeps,"props":self.ragdolls.props.len(),"prop_origins":self.ragdolls.props.keys().map(|id| (id.to_string(), (self.scene.states[*id].origin.to_array(), self.scene.ragdoll_matrices.contains_key(id)))).collect::<std::collections::BTreeMap<_,_>>(),"props_skipped":self.ragdolls.props_skipped,"last_force_body":self.ragdolls.last_force_body,"activity":self.ragdolls.activity(physics),"last_missed_shot":self.ragdolls.last_missed_shot.map(|(a,b)| [a.to_array(),b.to_array()]),"origins":self.ragdolls.active.keys().map(|id| (id.to_string(), self.scene.states[*id].origin.to_array())).collect::<std::collections::BTreeMap<_,_>>()});
        serde_json::json!({"time":self.scene.time,"inventory":self.inventory,"carrying":self.inventory.carrying(),"player_damage":self.player_damage,"viewmodel":{"animation":self.inventory.animation,"at":self.inventory.animation_at,"frag":self.inventory.frag},"suit":self.suit,"globals":self.scene.globals,"pending":self.selection.pending,"gesture_compose_errors":compose_errors,
            "entities":entities,"io":self.scene.diagnostics,"choreography":self.scene.choreography_states(&self.world),
            "animations":self.scene.animation_states(&self.world),"look_targets":self.scene.look_targets.report(),"random_look_targets":self.scene.look_targets.random_report(),"heads":self.scene.heads.iter().map(|(id,h)| (id.to_string(), h.goal.to_array())).collect::<std::collections::BTreeMap<_,_>>(),"gesture_layers":self.scene.gestures.report(),"monitors":self.scene.monitors,"npc_goals":self.npcs.snapshots(),"npc_paths":self.npcs.path_stats(),
            "ai":serde_json::to_value(&self.ai).unwrap_or_else(|e| serde_json::json!({"report_error":e.to_string()})),"npc_spawns":self.spawn_log,"zombies":zombies,"runtime_entities":{"despawned_draws":self.despawned_draws.len(),"created":self.runtime_entities.len(),"live":self.runtime_entities.iter().filter(|id| !self.scene.states[**id].killed).count(),"world_copies":self.world_copies},"bodies":{"current":self.scene.bodies.iter().map(|(id, b)| (id.to_string(), *b)).collect::<std::collections::BTreeMap<_,_>>(),"swaps":self.body_swaps,"failures":self.body_swap_failures},"spawn_overlay":self.spawn_overlay,"npc_makers":{"diagnostics":self.scene.makers.diagnostics,"live":self.scene.makers.makers.values().map(|m| (m.id.to_string(), m.live.clone())).collect::<std::collections::BTreeMap<_,_>>()},"last_npc_hit":self.scene.last_npc_hit,"npc_health":self.world.entities.iter().enumerate().filter(|(id,e)| e.class().starts_with("npc_") && self.scene.states[*id].value != 0.).map(|(id,_)| (id.to_string(), self.scene.states[id].value)).collect::<std::collections::BTreeMap<_,_>>(),"phys_explosions":self.phys_explosions,"ragdolls":ragdolls,"gibs":{"active":self.gibs.active,"spawned":self.gibs.spawned,"removed":self.gibs.removed,"skipped":self.gibs.skipped},"projectiles":{"active":self.projectiles.active,"effects":self.projectiles.effects,"diagnostics":self.projectiles.diagnostics},"impacts":{"studio_created":self.impacts.studio_created,"created":self.impacts.created,"unclippable":self.impacts.unclippable,"active":self.impacts.marks.len(),"errors":self.impacts.errors},"transition":self.scene.transition,
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
