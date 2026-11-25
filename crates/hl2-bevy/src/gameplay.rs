//! Bevy resource bridge for retained gameplay. Owned file reads occur before App::run.
mod ai_results;
mod empty;
mod npc_death;
mod npc_spawn;
mod player_outcome;
mod shot_effect;
mod tick_stages;
mod weapons_stage;
mod zombies;
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
pub use npc_spawn::{PendingNpcSpawn, spawn_usable_weapons};
use serde::Deserialize;
pub use shot_effect::ShotEffect;
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
    /// A fixture's entity setup (inputs to named entities, seeded actor and prop poses), not
    /// player input: it still applies while the player's controls are ignored.
    pub fn entity_setup(&self) -> bool {
        matches!(
            self,
            Self::Send { .. } | Self::ActorPose { .. } | Self::PropPose { .. }
        )
    }
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
/// The NPC AI switch (`ai_disable`) across map loads: on like Source (ai_disabled 0); `--no-ai`
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
    pub(crate) queued_secondary: bool,
    primary_consumed: bool,
    pub(crate) secondary_consumed: bool,
    pub unplayed_sounds: u64,
    pub sound_cues: Vec<String>,
    pub sound_requests: Vec<hl2_simulation::sounds::SoundRequest>,
    /// Scene speech captions waiting for the sound queue (which resolves actor genders).
    pub captions: Vec<hl2_simulation::captions::CaptionRequest>,
    /// CloseCaption messages for the HUD caption panel.
    pub caption_messages: Vec<hl2_ui::closecaption::Request>,
    weapon_sounds: hl2_simulation::sounds::WeaponAnimationSounds,
    /// Client sound events of entity animations (NPC footsteps).
    entity_sounds: hl2_simulation::sounds::EntityAnimationSounds,
    pub footsteps: hl2_simulation::footsteps::Footsteps,
    /// Local player damage intake at the current skill (see [`Gameplay::set_skill`]).
    pub player_damage: hl2_simulation::player_damage::PlayerDamage,
    /// Client "Damage" messages not yet shown by the HUD.
    pub damage_messages: Vec<hl2_simulation::player_damage::DamageMessage>,
    /// HEV suit voice queue.
    pub suit: hl2_simulation::suit::Suit,
    /// `skill` (1 Easy, 2 Normal, 3 Hard); hosts update it through [`Gameplay::set_skill`].
    skill: u32,
    /// The installed `cfg/skill.cfg`, kept to re-evaluate skill-dependent values.
    skill_config: Option<String>,
    /// Step 14 NPC AI (on unless `--no-ai` or `ai_disable`).
    pub ai: hl2_simulation::ai::runtime::AiRuntime,
    /// Velocity impulses on the player from NPC hits (ApplyAbsVelocityImpulse), applied
    /// before the next player move.
    pub player_impulse: glam::Vec3,
    /// F4 developer NPC spawn overlay and the npc_create* console state.
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
    /// NPCs a broken gas can or propane tank set on fire (report only).
    pub npc_ignitions: Vec<usize>,
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
    /// Diagnostics: the first collision of each gravity gun launch (time, prop, other
    /// side, speed, dot with the away normal, game material, stuck), newest last.
    pub launch_impacts: Vec<serde_json::Value>,
    /// prop_physics_create requests (model, eye, aim) for the host's model loader.
    pub prop_requests: Vec<(String, Vec3, Vec3)>,
    pub(crate) pickup_requests: Vec<crate::runtime_props::Request>,
    /// npc_create / ent_create requests waiting for asynchronous NPC late precaching.
    pub npc_requests: Vec<PendingNpcSpawn>,
    /// Runtime entities whose model draws the host builds next frame (dropped weapons).
    pub model_draws: Vec<usize>,
}

impl Gameplay {
    /// Applies the `skill` setting. Like CGameRules::Think, which reads `skill` every frame,
    /// a change takes effect immediately; values outside 1..=3 fall back to Normal.
    pub fn set_skill(&mut self, skill: u8) {
        let skill = if (1..=3).contains(&skill) {
            u32::from(skill)
        } else {
            2
        };
        if skill != self.skill {
            self.skill = skill;
            self.scene.skill.level = skill;
            self.player_damage.take_scale =
                hl2_simulation::player_damage::PlayerDamage::take_scale_for(
                    skill,
                    self.skill_config.as_deref(),
                );
        }
    }
    /// CBasePlayer::Event_Killed and DeathSound, the tick the health runs out.
    fn player_dies(&mut self, player: &mut modkit_core::movement::Player, suit: bool) {
        use hl2_simulation::player_damage::DMG_FALL;
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
    /// CheckSuitUpdate's speech for this tick.
    fn suit_speech(&mut self, suit: bool, now: f64) {
        // CheckSuitUpdate (PostThink), then UpdateClientData after the damage.
        self.suit.think(suit, now);
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
        // The Props tab: every installed (and map pak) model with a .phy.
        Ok(Self::from_prepared(
            world,
            empty::PreparedGameplay {
                ai,
                player_damage,
                skill_config: skill,
                impacts,
                scene,
                weapons,
                npcs,
            },
        ))
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
    /// CNPC_PoisonZombie AE_ZOMBIE_POISON_THROW_CRAB (DESIGN 14o session 38): a
    /// npc_headcrab_poison from its pool template at the zombie's EyePosition with its
    /// angles, SF_NPC_FALL_TO_GROUND (and SF_NPC_FADE_CORPSE when the zombie has it), set
    /// on fire for 100 s when the zombie burns, then thrown at the enemy's eyes.
    /// GetHeadcrabClassname's npc_headcrab_poison pool template (the same CBlackHeadcrab
    /// as npc_headcrab_black, whose template stands in when the map has none).
    fn black_crab_template(&self) -> Option<usize> {
        ["npc_headcrab_poison", "npc_headcrab_black"]
            .iter()
            .find_map(|class| {
                hl2_simulation::ai::spawn::templates(&self.world, class)
                    .first()
                    .copied()
            })
    }
    fn throw_crabs(&mut self, physics: &mut Physics) {
        for (zombie, eye, yaw, target) in std::mem::take(&mut self.ai.crab_throws) {
            // GetHeadcrabClassname: npc_headcrab_poison (the same CBlackHeadcrab as
            // npc_headcrab_black, whose pool template stands in when the map has none).
            let Some(template) = self.black_crab_template() else {
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
    /// DieChopped's gibs: the legs at the origin and the torso 64 up pitched -90, each a
    /// raggib pushed with its force.
    fn spawn_chopped(
        &mut self,
        origin: Vec3,
        yaw: f32,
        legs: Vec3,
        torso: Vec3,
        physics: &mut Physics,
    ) {
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
            // OnTakeDamage_Alive releases the head crab before Event_Killed evacuates
            // the nest, so a live head crab can block nest spots.
            let replaced = releases
                .iter()
                .find(|r| r.entity == npc)
                .is_some_and(|release| self.release_crab(*release, physics));
            self.evacuate_nest(npc, physics);
            let stays = self.scene.states[npc].corpse;
            if replaced || stays || self.ai.take_eaten(npc) {
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
            self.drop_weapon(npc, physics);
        }
    }
    /// Merge a late-loaded model's tables into the map's World (keys it lacks only).
    pub fn merge_model(&mut self, loaded: World) {
        if Arc::strong_count(&self.world) > 1 {
            self.world_copies += 1;
        }
        Arc::make_mut(&mut self.world).merge_model_tables(loaded);
    }
    fn push_shot_effect(&mut self, effect: ShotEffect) {
        self.shot_serial += 1;
        self.shot_effects
            .push((self.shot_serial, self.scene.time, effect));
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
        self.tick_with(
            physics,
            player,
            (eye, direction, fly),
            crate::isolation::Isolation::default(),
            None,
        );
    }

    pub(crate) fn tick_with(
        &mut self,
        physics: &mut Physics,
        player: &Player,
        (eye, direction, fly): (Vec3, Vec3, bool),
        isolation: crate::isolation::Isolation,
        performance: Option<&crate::performance::Performance>,
    ) {
        self.tick_actions(physics, player, eye, direction, fly);
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
        timed(performance, "sim_scene", || {
            self.scene.tick(&self.world, player.feet, TICK);
        });
        self.tick_npc_makers(physics, player, eye, direction);
        self.weapons_stage(physics, player, eye, direction);
        for (id, state) in self.scene.states.iter().enumerate() {
            physics.set_entity(id, state.origin, state.rotation, state.collides());
        }
        // Studio hitboxes follow each NPC's animated skeleton (bullet hitgroups).
        timed(performance, "sim_hitboxes", || {
            hl2_simulation::gameplay::pose_npc_hitboxes(&self.world, &self.scene, physics);
        });
        self.tick_projectiles_and_aim(physics, player, eye, direction);
        timed(performance, "sim_query_refresh", || {
            physics.refresh_entity_queries()
        });
        // NPC stage: the AI thinks (when enabled) before the movement controller tick.
        let ai_player = self.ai_player_state(player, fly, direction);
        timed(performance, "sim_ai", || {
            self.ai.tick(
                &self.world,
                &mut self.scene,
                &mut self.npcs,
                physics,
                &[],
                ai_player,
            );
        });
        self.scene.npc_knows_player = self.ai.npcs_knowing_player();
        self.throw_crabs(physics);
        // Zombie swats (AE_ZOMBIE_SWATITEM -> IPhysicsObject::AddVelocity).
        for (object, velocity, angular) in std::mem::take(&mut self.ai.prop_swats) {
            physics.add_entity_velocity(object, velocity, angular);
        }
        self.apply_ai_results(physics);
        self.npc_shots_and_flashes(physics, player, direction);
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
        self.apply_explosions_and_ragdolls(physics, player);
        if isolation.no_physics_step {
            physics.last_step_ms = 0.;
            if let Some(p) = performance {
                p.record_ms("sim_physics", 0.0);
            }
        } else {
            timed(performance, "sim_physics", || physics.tick(TICK));
        }
        self.post_physics_impacts(physics);
        self.tick_sounds_and_decals(physics, eye, direction);
    }
    pub fn report(&self, physics: &Physics) -> serde_json::Value {
        let entities: Vec<_> = self.world.entities.iter().enumerate()
            .filter(|(_, e)| e.class().contains("door")
                || e.class().starts_with("prop_physics")
                || source_assets::models::physical_pickup(e.class()))
            .map(|(id, e)| {
                let state = &self.scene.states[id];
                let (origin, rotation) = physics.entity_pose(id).unwrap_or((state.origin, state.rotation));
                serde_json::json!({"entity":id,"name":e.get("targetname"),"class":e.class(),
                    "origin":origin.to_array(),"rotation":rotation.to_array(),"visible":state.visible,"killed":state.killed,
                    "body": physics.entity_rest_state(id),"body_contact":physics.entity_in_contact(id)})
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
        let zombies = serde_json::json!({"splits":self.zombie_splits,"torsos":self.zombie_splits.iter().map(|(_, t, _)| (t.to_string(), (self.scene.states[*t].origin.to_array(), self.scene.states[*t].killed, self.scene.states[*t].value))).collect::<std::collections::BTreeMap<_,_>>(),"failures":self.zombie_split_failures,"crab_releases":self.scene.zombie_crab_releases,"released":self.crab_releases,"live_crabs":self.released_crabs,"thrown_crabs":self.thrown_crabs,"evacuated_crabs":self.evacuated_crabs,"npc_ignitions":self.npc_ignitions,"chops":self.scene.zombie_chops});
        let ragdolls = serde_json::json!({"active":self.ragdolls.active.len(),"spawned":self.ragdolls.spawned,"skipped":self.ragdolls.skipped,"removed":self.ragdolls.removed,"shot_pushes":self.ragdolls.shot_pushes,"blast_pushes":self.ragdolls.blast_pushes,"pins":self.ragdolls.pins,"death_poses":self.ragdolls.death_poses,"bolt_misses":self.ragdolls.bolt_misses,"last_missed_bolt":self.ragdolls.last_missed_bolt.map(|(a,b)| [a.to_array(),b.to_array()]),"missed_bolt_bodies":self.ragdolls.missed_bolt_bodies.iter().map(|v| v.to_array()).collect::<Vec<_>>(),"fading":self.ragdolls.fading(),"shots_seen":self.ragdolls.shots_seen,"shots_busy":self.ragdolls.shots_busy,"simulating":self.ragdolls.simulating(physics),"forced_sleeps":self.ragdolls.forced_sleeps,"props":self.ragdolls.props.len(),"prop_origins":self.ragdolls.props.keys().map(|id| (id.to_string(), (self.scene.states[*id].origin.to_array(), self.scene.ragdoll_matrices.contains_key(id)))).collect::<std::collections::BTreeMap<_,_>>(),"props_skipped":self.ragdolls.props_skipped,"last_force_body":self.ragdolls.last_force_body,"activity":self.ragdolls.activity(physics),"last_missed_shot":self.ragdolls.last_missed_shot.map(|(a,b)| [a.to_array(),b.to_array()]),"origins":self.ragdolls.active.keys().map(|id| (id.to_string(), self.scene.states[*id].origin.to_array())).collect::<std::collections::BTreeMap<_,_>>()});
        serde_json::json!({"time":self.scene.time,"inventory":self.inventory,"flashlight":self.scene.flashlight.state(),"carrying":self.inventory.carrying(),"player_damage":self.player_damage,"viewmodel":{"animation":self.inventory.animation,"at":self.inventory.animation_at,"frag":self.inventory.frag},"suit":self.suit,"globals":self.scene.globals,"pending":self.selection.pending,"gesture_compose_errors":compose_errors,
            "entities":entities,"io":self.scene.diagnostics,"choreography":self.scene.choreography_states(&self.world),
            "animations":self.scene.animation_states(&self.world),"look_targets":self.scene.look_targets.report(),"random_look_targets":self.scene.look_targets.random_report(),"heads":self.scene.heads.iter().map(|(id,h)| (id.to_string(), h.goal.to_array())).collect::<std::collections::BTreeMap<_,_>>(),"gesture_layers":self.scene.gestures.report(),"sequence_fades":self.scene.sequence_transitions.iter().filter(|(_,t)| !t.fades.is_empty()).map(|(id,t)| (id.to_string(), &t.fades)).collect::<std::collections::BTreeMap<_,_>>(),"monitors":self.scene.monitors,"npc_goals":self.npcs.snapshots(),"npc_paths":self.npcs.path_stats(),
            "ai":serde_json::to_value(&self.ai).unwrap_or_else(|e| serde_json::json!({"report_error":e.to_string()})),"npc_spawns":self.spawn_log,"zombies":zombies,"runtime_entities":{"despawned_draws":self.despawned_draws.len(),"created":self.runtime_entities.len(),"live":self.runtime_entities.iter().filter(|id| !self.scene.states[**id].killed).count(),"world_copies":self.world_copies},"bodies":{"current":self.scene.bodies.iter().map(|(id, b)| (id.to_string(), *b)).collect::<std::collections::BTreeMap<_,_>>(),"swaps":self.body_swaps,"failures":self.body_swap_failures},"npc_makers":{"diagnostics":self.scene.makers.diagnostics,"live":self.scene.makers.makers.values().map(|m| (m.id.to_string(), m.live.clone())).collect::<std::collections::BTreeMap<_,_>>()},"last_npc_hit":self.scene.last_npc_hit,"npc_health":self.world.entities.iter().enumerate().filter(|(id,e)| e.class().starts_with("npc_") && self.scene.states[*id].value != 0.).map(|(id,_)| (id.to_string(), self.scene.states[id].value)).collect::<std::collections::BTreeMap<_,_>>(),"phys_explosions":self.phys_explosions,"ragdolls":ragdolls,"gibs":{"active":self.gibs.active,"spawned":self.gibs.spawned,"removed":self.gibs.removed,"skipped":self.gibs.skipped},"projectiles":{"active":self.projectiles.active,"effects":self.projectiles.effects,"diagnostics":self.projectiles.diagnostics},"launch_impacts":self.launch_impacts,"impacts":{"studio_created":self.impacts.studio_created,"created":self.impacts.created,"unclippable":self.impacts.unclippable,"active":self.impacts.marks.len(),"errors":self.impacts.errors},"transition":self.scene.transition,
            "unplayed_sounds":self.unplayed_sounds,"queued_sounds":self.sound_requests.len(),"recent_sound_cues":self.sound_cues})
    }
}

fn timed<R>(
    performance: Option<&crate::performance::Performance>,
    name: &'static str,
    f: impl FnOnce() -> R,
) -> R {
    let started = performance.map(|_| std::time::Instant::now());
    let result = f();
    if let (Some(p), Some(t)) = (performance, started) {
        p.record_ms(name, t.elapsed().as_secs_f64() * 1000.);
    }
    result
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
    #[test]
    fn skill_setting_rescales_damage_taken_and_inflicted_immediately() {
        let mut game = Gameplay::synthetic(World::default());
        game.skill_config = Some("sk_dmg_take_scale3 \"2.0\"\n".into());
        assert_eq!((game.skill, game.player_damage.take_scale), (2, 1.));
        assert_eq!(
            (
                game.scene.skill.level,
                game.scene.skill.player_inflict_scale()
            ),
            (2, 1.0)
        );
        game.set_skill(1);
        assert_eq!((game.skill, game.player_damage.take_scale), (1, 0.5));
        assert_eq!(
            (
                game.scene.skill.level,
                game.scene.skill.player_inflict_scale()
            ),
            (1, 1.5)
        );
        game.set_skill(3);
        assert_eq!(game.player_damage.take_scale, 2., "skill.cfg override");
        assert_eq!(
            (
                game.scene.skill.level,
                game.scene.skill.player_inflict_scale()
            ),
            (3, 0.75)
        );
        game.set_skill(9);
        assert_eq!((game.skill, game.player_damage.take_scale), (2, 1.));
        assert_eq!(
            (
                game.scene.skill.level,
                game.scene.skill.player_inflict_scale()
            ),
            (2, 1.0)
        );
    }
}
