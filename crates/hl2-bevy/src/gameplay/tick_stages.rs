//! Stage helpers extracted from `Gameplay::tick_with`.
use super::{Action, Gameplay, ShotEffect};
use glam::Vec3;
use hl2_simulation::physics::Physics;
use modkit_core::movement::{Player, TICK};

impl Gameplay {
    pub(super) fn tick_actions(
        &mut self,
        physics: &mut Physics,
        player: &Player,
        eye: Vec3,
        direction: Vec3,
        fly: bool,
    ) {
        self.selection
            .tick(&self.inventory, &self.weapons, self.scene.time);
        for action in self.take_actions() {
            self.apply_action(action, physics, player, (eye, direction, fly));
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
    }

    fn apply_action(
        &mut self,
        action: Action,
        physics: &mut Physics,
        player: &Player,
        (eye, direction, fly): (Vec3, Vec3, bool),
    ) {
        let result = match action {
            Action::Loadout => {
                for name in self.weapons.keys() {
                    self.inventory.give(name, &self.weapons, self.scene.time);
                }
                self.inventory.refill_ammo(&self.weapons);
                self.inventory.suit = true;
                self.inventory
                    .give("weapon_crowbar", &self.weapons, self.scene.time);
                return;
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
                return;
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
                    return;
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
                return;
            }
            Action::Impulse => {
                if let Some((id, _)) = physics.ray(eye, direction, 128.) {
                    physics.impulse(id, direction, 6.);
                }
                return;
            }
            Action::ActorPose { .. } | Action::PropPose { .. } | Action::Send { .. } => {
                self.apply_fixture_action(action, physics);
                return;
            }
        };
        self.select(result);
    }

    fn apply_fixture_action(&mut self, action: Action, physics: &mut Physics) {
        match action {
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
            }
            Action::Send {
                target,
                input,
                parameter,
            } => {
                self.scene.send_named(&target, &input, &parameter, 0.);
            }
            _ => {}
        }
    }

    pub(super) fn tick_npc_makers(
        &mut self,
        physics: &mut Physics,
        player: &Player,
        eye: Vec3,
        direction: Vec3,
    ) {
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
    }

    pub(super) fn tick_projectiles_and_aim(
        &mut self,
        physics: &mut Physics,
        player: &Player,
        eye: Vec3,
        direction: Vec3,
    ) {
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
    }

    pub(super) fn npc_shots_and_flashes(
        &mut self,
        physics: &mut Physics,
        player: &Player,
        direction: Vec3,
    ) {
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
            self.npc_fire_shots(physics, player, direction, &shots);
        }
        self.npc_muzzle_flashes_and_tracers();
    }

    fn npc_fire_shots(
        &mut self,
        physics: &mut Physics,
        player: &Player,
        direction: Vec3,
        shots: &[hl2_simulation::npc_weapons::NpcShot],
    ) {
        let target = (!player.dead).then_some(hl2_simulation::gameplay::PlayerTarget {
            feet: player.feet,
            crouched: player.crouched,
            yaw: direction.y.atan2(direction.x).to_degrees(),
        });
        let mut bullets = hl2_simulation::gameplay::NpcBullets::default();
        for shot in shots {
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

    fn npc_muzzle_flashes_and_tracers(&mut self) {
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
    }

    pub(super) fn apply_explosions_and_ragdolls(&mut self, physics: &mut Physics, player: &Player) {
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
        damage.extend(self.burning_props_damage(physics, player));
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
        // Gas cans and propane tanks set the NPCs around them on fire after their blast
        // (a blast-split zombie's torso is the same entity in Source, so it burns too).
        for (origin, radius, attacker) in std::mem::take(&mut self.scene.prop_ignitions) {
            let ignited = self
                .scene
                .ignite_near(&self.world, origin, radius, attacker);
            self.npc_ignitions.extend(ignited);
        }
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
    }

    fn burning_props_damage(
        &mut self,
        physics: &Physics,
        player: &Player,
    ) -> Vec<hl2_simulation::projectiles::Damage> {
        if self.scene.fires.flames.is_empty() {
            return Vec::new();
        }
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
            .filter(|(_, e)| e.class().starts_with("prop_physics") || e.class().starts_with("npc_"))
            .filter_map(|(id, _)| center(id).map(|c| (id, c)))
            .collect();
        let player_center = (!player.dead).then(|| player.feet + glam::Vec3::Z * 36.);
        self.scene
            .fires
            .think(self.scene.time, center, &others, player_center)
    }

    pub(super) fn post_physics_impacts(&mut self, physics: &mut Physics) {
        self.ragdolls.update(&mut self.scene, physics);
        // VPhysicsShadowCollision impact damage to NPCs (queued for the next tick).
        let physcannon_held = self.inventory.physcannon.held;
        let carry_held = self.inventory.carry.as_ref().map(|c| c.entity);
        let held = |id: usize| physcannon_held == Some(id) || carry_held == Some(id);
        hl2_simulation::impact_damage::queue_npc_damage(
            &physics.resolved_collisions,
            &self.world,
            &mut self.scene,
            &held,
        );
        // A launched sawblade embeds in the wall it hits (before the launch flag goes).
        let sticks = hl2_simulation::impact_damage::world_sticks(
            &physics.resolved_collisions,
            &self.world,
            &physics.launched,
            |event, side| {
                self.impacts
                    .collision_game_material(event, side, &self.world, physics, &self.scene)
            },
        );
        self.record_launch_impacts(physics, &sticks);
        for (id, position, save) in sticks {
            physics.stick(id, position, save);
            self.scene
                .sounds
                .push(hl2_simulation::sounds::SoundRequest {
                    origin: Some(position),
                    .."Metal.SawbladeStick".into()
                });
        }
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
    }

    fn record_launch_impacts(&mut self, physics: &Physics, sticks: &[(usize, Vec3, Vec3)]) {
        let mut seen = std::collections::HashSet::new();
        for event in &physics.resolved_collisions {
            for side in 0..2 {
                let Some(id) = event.entities[side].filter(|id| physics.launched.contains(id))
                else {
                    continue;
                };
                if !seen.insert(id) {
                    continue;
                }
                let velocity = event.pre_velocities[side];
                let away = if side == 0 {
                    event.normal
                } else {
                    -event.normal
                };
                let material = self.impacts.collision_game_material(
                    event,
                    1 - side,
                    &self.world,
                    physics,
                    &self.scene,
                );
                self.launch_impacts.push(serde_json::json!({
                    "time": self.scene.time, "entity": id, "other": event.entities[1 - side],
                    "other_mass": event.masses[1 - side], "speed": velocity.length(),
                    "dot": velocity.normalize_or_zero().dot(away), "material": material.to_string(),
                    "origin": event.origins[side].to_array(), "contact": event.position.to_array(),
                    "normal": event.normal.to_array(),
                    "stuck": sticks.iter().any(|(s, _, _)| *s == id),
                }));
            }
        }
    }

    pub(super) fn tick_sounds_and_decals(
        &mut self,
        physics: &mut Physics,
        eye: Vec3,
        direction: Vec3,
    ) {
        let physcannon_held = self.inventory.physcannon.held;
        let carry_held = self.inventory.carry.as_ref().map(|c| c.entity);
        let held = |id: usize| physcannon_held == Some(id) || carry_held == Some(id);
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
        // Sequence changes made by this tick's AI and scripts start their client blends.
        self.scene.update_sequence_transitions(&self.world);
        for sound in self.scene.sounds.drain(..) {
            self.sound_cues.push(sound.name.clone());
            if self.sound_requests.len() < 1024 {
                self.sound_requests.push(sound);
            } else {
                self.unplayed_sounds += 1;
            }
        }
        self.captions.append(&mut self.scene.captions);
        let excess = self.sound_cues.len().saturating_sub(64);
        self.sound_cues.drain(..excess);
    }
}
