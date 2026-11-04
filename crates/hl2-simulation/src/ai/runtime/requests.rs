//! Applying what thinking decided, and world events that reach the AI: melee hits, thrown
//! objects, +USE, missiles, lasers, combat sounds and dispositions.

use super::{
    center_z, classify, eye_z, senses, yaw_of, zombie, AiClass, AiRuntime, Class, Context,
    CopRequest, Disposition, MeleeHit, PhysicsSight, PlayerHit, PlayerState, SightWorld, PLAYER,
};
use crate::entities::Scene;
use crate::npc::Controller;
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::World;

/// One melee swing: the swept box, its damage, view punch and velocity push.
struct Swing {
    start: Vec3,
    end: Vec3,
    mins: Vec3,
    maxs: Vec3,
    forward: Vec3,
    damage: f32,
    punch: Vec3,
    impulse: Vec3,
}

/// `CopRequest::Claw`'s swing.
struct Claw {
    start: Vec3,
    forward: Vec3,
    punch: Vec3,
    impulse: Vec3,
    reach: f32,
    bite: bool,
    damage: Option<f32>,
}

impl AiRuntime {
    /// Would a stunstick hit knock the player out (the cop's policing goal)?
    pub(super) fn knock_out_player(
        &self,
        id: usize,
        world: &World,
        scene: &Scene,
        physics: &Physics,
        player: PlayerState,
    ) -> bool {
        let Some(policing) = self.actors[&id]
            .class
            .cop()
            .and_then(|c| c.policing.as_ref())
        else {
            return false;
        };
        let visible = PhysicsSight { physics, world }.visible(
            scene.states[id].origin + Vec3::Z * eye_z(world, id),
            player.center(),
            &[id],
        );
        policing.goal.should_knock_out(player.center(), visible)
    }

    /// Carry out the cop's requests (sentences, look targets, outputs, sounds, hits).
    /// SCHED_ZOMBIE_ATTACKITEM: a claw that missed strikes m_hPhysicsEnt (TakeDamage with a
    /// 120 push toward it, then m_flNextSwat = now + U(2, 4)). True when it hit.
    fn strike_attack_item(
        &mut self,
        id: usize,
        origin: Vec3,
        damage: f32,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
    ) -> bool {
        let attack_item =
            self.actors[&id].npc.schedule.as_deref() == Some("SCHED_ZOMBIE_ATTACKITEM");
        let roll = self.random();
        let AiClass::Zombie(z) = &mut self.actors.get_mut(&id).expect("actor").class else {
            return false;
        };
        let Some((object, center)) = attack_item.then(|| z.physics_ent.take()).flatten() else {
            return false;
        };
        let me = origin + Vec3::Z * center_z(world, id);
        let direction = (center - me).normalize_or_zero();
        scene.entity_damage.push(crate::projectiles::Damage {
            target: crate::projectiles::DamageTarget::Entity(object),
            amount: damage,
            kind: crate::player_damage::DMG_SLASH,
            dissolve: false,
            direction,
            origin: center,
            attacker: crate::projectiles::Attacker::Entity(id),
        });
        if let Some((mass, _, _)) = physics.entity_rest_state(object) {
            self.prop_swats.push((
                object,
                direction * zombie::ATTACKITEM_FORCE / mass.max(1.),
                Vec3::ZERO,
            ));
        }
        z.next_swat = scene.time + 2. + 2. * f64::from(roll);
        self.diagnostics.obstructor_swats += 1;
        true
    }
    /// CheckTraceHullAttack for a claw or kick: the box swept from `start` to `end` meets
    /// the hated player (damage, view punch and push) or the first NPC it may hurt.
    fn swing(
        &mut self,
        id: usize,
        swing: &Swing,
        world: &World,
        scene: &Scene,
        player: PlayerState,
    ) -> bool {
        let (start, end, mins, maxs) = (swing.start, swing.end, swing.mins, swing.maxs);
        let player_hit = self.actors[&id].npc.enemy == Some(PLAYER)
            && crate::ai::metropolice::segment_hits_box(
                start,
                end,
                player.feet + Vec3::new(-16., -16., 0.) - maxs,
                player.feet + Vec3::new(16., 16., 72.) - mins,
            );
        if player_hit {
            self.player_hits.push(PlayerHit {
                attacker: id,
                damage: swing.damage,
                punch: swing.punch,
                impulse: swing.impulse,
                fade: None,
            });
            self.diagnostics.player_hits += 1;
            return true;
        }
        let Some(victim) = self.melee_victim(id, world, scene, start, end, mins, maxs) else {
            return false;
        };
        self.diagnostics.npc_melee_hits += 1;
        self.melee_hits.push(MeleeHit {
            attacker: id,
            victim,
            damage: swing.damage,
            position: scene.states[victim].origin + Vec3::Z * center_z(world, victim),
            direction: swing.forward,
        });
        true
    }
    /// A zombie's ClawAttack: the swing, then its hit or miss sound.
    fn claw(
        &mut self,
        id: usize,
        claw: Claw,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) {
        let Claw {
            start,
            forward,
            punch,
            impulse,
            reach,
            bite,
            damage,
        } = claw;
        let origin = scene.states[id].origin;
        let enemy = self.actors[&id].npc.enemy;
        let enemy_center = enemy.map(|e| {
            if e == PLAYER {
                player.center()
            } else {
                scene.states[e].origin + Vec3::Z * center_z(world, e)
            }
        });
        // ClawAttack: no swing through a wall between the zombie and its
        // enemy (an 8-unit hull to the enemy's center, MASK_SOLID_BRUSHONLY:
        // world brushes and static props, not physics props).
        if enemy_center.is_some_and(|c| {
            physics
                .filtered_ray(start, c, &|entity| entity == usize::MAX)
                .is_some()
        }) {
            return;
        }
        let end = start + forward * reach;
        // The hull made a cube (the x extents).
        let cube = world
            .entities
            .get(id)
            .and_then(|e| crate::ai_hull::class_hull(e.class()))
            .map_or(13., |h| h.bounds().maxs.x);
        let (mins, maxs) = (-Vec3::splat(cube), Vec3::splat(cube));
        let damage = damage.unwrap_or(if bite {
            self.zombie_poison_spit_damage
        } else {
            self.zombie_slash_damage
        });
        let swing = Swing {
            start,
            end,
            mins,
            maxs,
            forward,
            damage,
            punch,
            impulse,
        };
        let hit = self.swing(id, &swing, world, scene, player)
            || self.strike_attack_item(id, origin, damage, world, scene, physics);
        let cue = if hit {
            "Zombie.AttackHit"
        } else {
            "Zombie.AttackMiss"
        };
        let cue = match &self.actors[&id].class {
            AiClass::Zombie(z) => z.kind.cue(cue),
            _ => cue,
        };
        scene.sounds.push(crate::sounds::SoundRequest {
            origin: Some(origin + Vec3::Z * 36.),
            ..String::from(cue).into()
        });
    }
    pub(super) fn apply_requests(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) {
        let origin = scene.states[id].origin;
        let requests = {
            let actor = self.actors.get_mut(&id).expect("actor");
            std::mem::take(actor.class.requests_mut())
        };
        for request in requests {
            match request {
                CopRequest::Sentence(group) => {
                    self.say(id, group, world, scene);
                }
                CopRequest::Concept(concept) => {
                    self.ally_say(id, concept, world, scene, player);
                }
                CopRequest::LookAtPlayer {
                    importance,
                    duration,
                } => scene.look_targets.add(
                    id,
                    crate::attention::Target::Player,
                    importance,
                    scene.time,
                    f64::from(duration),
                ),
                CopRequest::Output(name) => scene.fire(id, name, usize::MAX),
                CopRequest::Swat {
                    entity,
                    velocity,
                    angular,
                } => {
                    self.prop_swats.push((entity, velocity, angular));
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 36.),
                        ..String::from("NPC_BaseZombie.Swat").into()
                    });
                }
                CopRequest::Patch { cue, op } => {
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 64.),
                        ..crate::sounds::SoundRequest::patch(
                            cue,
                            crate::sounds::entity_emitter(id),
                            op,
                        )
                    });
                }
                CopRequest::Sound(name) => {
                    let cue = match name {
                        "melee_hit" => self.stunstick_sounds.0.clone(),
                        "melee_miss" => self.stunstick_sounds.1.clone(),
                        other => Some(other.to_owned()),
                    };
                    if let Some(cue) = cue {
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(origin + Vec3::Z * 36.),
                            ..cue.into()
                        });
                    }
                }
                CopRequest::PlayerHit {
                    damage,
                    punch,
                    impulse,
                    fade,
                    knock_out,
                } => {
                    self.diagnostics.player_hits += 1;
                    if knock_out {
                        let cop = self.actors[&id].class.cop();
                        if let Some(goal) =
                            cop.and_then(|c| c.policing.as_ref()).map(|p| p.goal.entity)
                        {
                            scene.fire(goal, "OnKnockOut", usize::MAX);
                        }
                    }
                    self.player_hits.push(PlayerHit {
                        attacker: id,
                        damage,
                        punch,
                        impulse,
                        fade: fade.map(|(color, duration, hold, flags)| {
                            crate::player_damage::ScreenFade::new(color, duration, hold, flags)
                        }),
                    });
                }
                CopRequest::CallForJustice => self.call_for_justice(id, world, scene, physics),
                CopRequest::DieInstantly(kind) => {
                    let health = match scene.states[id].value {
                        v if v > 0. => v,
                        _ => crate::gameplay::npc_spawn_health(scene, &world.entities[id]),
                    };
                    scene.entity_damage.push(crate::projectiles::Damage {
                        target: crate::projectiles::DamageTarget::Entity(id),
                        amount: health,
                        kind,
                        dissolve: false,
                        direction: Vec3::splat(0.1).normalize(),
                        origin,
                        attacker: crate::projectiles::Attacker::Entity(id),
                    });
                }
                CopRequest::AltFire(target) => {
                    // FireNPCSecondaryAttack: WeaponSound(WPN_DOUBLE) and a ball from
                    // Weapon_ShootPosition toward the elite's m_vecAltFireTarget.
                    let start = origin + Vec3::Z * 55.;
                    let aim = (target - start).normalize_or_zero();
                    self.ball_launches.push((id, start, aim * 1000.));
                    let cue = self.actors[&id]
                        .weapon
                        .as_ref()
                        .and_then(|w| self.weapon_def(&w.class))
                        .and_then(|d| d.sounds.get("double_shot").cloned());
                    if let Some(cue) = cue {
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(start),
                            ..cue.into()
                        });
                    }
                }
                CopRequest::GrenadeToss => {
                    let position = scene
                        .attachment_frames(world, id, &["lefthand"])
                        .first()
                        .copied()
                        .flatten()
                        .map_or(origin + Vec3::Z * 48., |m| m.w_axis.truncate());
                    let spin = Vec3::new(
                        self.random() * 2000. - 1000.,
                        self.random() * 2000. - 1000.,
                        self.random() * 2000. - 1000.,
                    );
                    let now = scene.time;
                    if let Some(AiClass::Humanoid(h)) =
                        self.actors.get_mut(&id).map(|a| &mut a.class)
                    {
                        self.grenade_tosses
                            .push((id, position, h.toss_velocity, spin));
                        h.grenades -= 1;
                        // Wait six seconds before even looking again.
                        h.next_grenade_check = now + 6.;
                    }
                }
                CopRequest::Claw {
                    start,
                    forward,
                    punch,
                    impulse,
                    reach,
                    bite,
                    damage,
                } => {
                    let claw = Claw {
                        start,
                        forward,
                        punch,
                        impulse,
                        reach,
                        bite,
                        damage,
                    };
                    self.claw(id, claw, world, scene, physics, player);
                }
                CopRequest::SetBodygroup { group, value } => {
                    scene.set_bodygroup(world, id, group, value);
                }
                CopRequest::ThrowCrab { target } => {
                    let eye = origin + Vec3::Z * eye_z(world, id);
                    self.crab_throws.push((id, eye, yaw_of(scene, id), target));
                }
                CopRequest::Leap(velocity) => {
                    // CBaseHeadcrab::Leap: off the ground (MoveOrigin(0, 0, 1) with headroom)
                    // with the velocity; step_falls carries it and its LeapTouch.
                    scene.states[id].origin.z += 1.;
                    if let Some(actor) = self.actors.get_mut(&id) {
                        actor.fall = Some((velocity, scene.time));
                    }
                }
                CopRequest::Kick { start, forward } => {
                    let end = start + forward * 70.;
                    let (mins, maxs) = (-Vec3::new(16., 16., 18.), Vec3::new(16., 16., 18.));
                    let elite = self.actors[&id].class.humanoid().is_some_and(|h| h.elite);
                    let damage = if elite {
                        self.combine_guard_kick_damage
                    } else {
                        self.combine_kick_damage
                    };
                    // ViewPunch(-12, -7, 0) and forward * 100 + up * 50.
                    let swing = Swing {
                        start,
                        end,
                        mins,
                        maxs,
                        forward,
                        damage,
                        punch: Vec3::new(-12., -7., 0.),
                        impulse: forward * 100. + Vec3::Z * 50.,
                    };
                    let hit = self.swing(id, &swing, world, scene, player);
                    if hit {
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(origin + Vec3::Z * 36.),
                            ..String::from("NPC_Combine.WeaponBash").into()
                        });
                    }
                    self.diagnostics.sentences += 1;
                    scene.sounds.push(crate::sounds::SoundRequest {
                        origin: Some(origin + Vec3::Z * 64.),
                        actor: Some(crate::sounds::SoundActor {
                            name: scene
                                .entity(world, id)
                                .and_then(|e| e.get("targetname"))
                                .unwrap_or("")
                                .into(),
                            model: String::new(),
                            entity: Some(id),
                        }),
                        ..String::from("#COMBINE_KICK").into()
                    });
                }
                CopRequest::Swing {
                    start,
                    end,
                    mins,
                    maxs,
                    damage,
                } => {
                    let victim = self.melee_victim(id, world, scene, start, end, mins, maxs);
                    let cue = if let Some(victim) = victim {
                        self.diagnostics.npc_melee_hits += 1;
                        let center =
                            scene.states[victim].origin + Vec3::Z * center_z(world, victim);
                        self.melee_hits.push(MeleeHit {
                            attacker: id,
                            victim,
                            damage,
                            position: center,
                            direction: (center - (origin + Vec3::Z * 36.)).normalize_or_zero(),
                        });
                        // CTraceFilterMelee: InsertSound(SOUND_COMBAT, damage position, 200, 0.2).
                        self.sounds.insert(
                            senses::sound::COMBAT,
                            center,
                            200.,
                            0.2,
                            Some(id),
                            scene.time,
                        );
                        self.stunstick_sounds.0.clone()
                    } else {
                        self.stunstick_sounds.1.clone()
                    };
                    if let Some(cue) = cue {
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(origin + Vec3::Z * 36.),
                            ..cue.into()
                        });
                    }
                }
            }
        }
    }

    /// CheckTraceHullAttack's NPC part: the nearest NPC along the swept box (`mins`..`maxs`
    /// around the segment) whose hull it touches and that the attacker hates
    /// (CTraceFilterMelee only damages hated combat characters). World blocking is not
    /// traced (the player swing does not trace it either).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn melee_victim(
        &self,
        id: usize,
        world: &World,
        scene: &Scene,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
    ) -> Option<usize> {
        let precriminal = scene.globals.is_on("gordon_precriminal");
        world
            .entities
            .iter()
            .enumerate()
            .filter(|(other, e)| *other != id && e.class().starts_with("npc_"))
            .filter_map(|(other, e)| {
                let state = scene.states.get(other)?;
                if state.killed || !state.visible {
                    return None;
                }
                let (hull_min, hull_max) = crate::physics::npc_hull(e.class())?;
                let box_min = state.origin + hull_min - maxs;
                let box_max = state.origin + hull_max - mins;
                (crate::ai::metropolice::segment_hits_box(start, end, box_min, box_max)
                    && self.npc_disposition(world, id, other, precriminal) == Disposition::HT)
                    .then(|| {
                        (
                            other,
                            (state.origin + Vec3::Z * 36.).distance_squared(start),
                        )
                    })
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(other, _)| other)
    }

    /// AdministerJustice's search: the first other allowed-to-respond cop within 512
    /// units that sees this cop and the player.
    fn call_for_justice(&mut self, id: usize, world: &World, scene: &Scene, physics: &Physics) {
        let origin = scene.states[id].origin + Vec3::Z * 36.;
        let sight = PhysicsSight { physics, world };
        let helper = self.actors.iter().find_map(|(other, actor)| {
            let cop = actor.class.cop()?;
            let center = scene.states[*other].origin + Vec3::Z * 36.;
            (*other != id
                && !scene.states[*other].killed
                && cop.spawnflags & crate::ai::metropolice::SF_METROPOLICE_ALLOWED_TO_RESPOND != 0
                && center.distance(origin) < 512.
                && sight.visible(origin, center, &[id, *other])
                && cop.player.is_some_and(|p| p.visible))
            .then_some(*other)
        });
        if let Some(helper) = helper {
            let now = scene.time;
            let ctx = Context {
                now,
                origin: scene.states[helper].origin,
                yaw: yaw_of(scene, helper),
                random: self.random(),
                ..Default::default()
            };
            let actor = self.actors.get_mut(&helper).expect("actor");
            if let AiClass::Metropolice(cop) = &mut actor.class {
                cop.administer_justice(&mut actor.npc, &ctx);
            }
        }
    }

    /// g_interactionHitByPlayerThrownPhysObj for an NPC struck by a prop the player
    /// launched or threw (the host's physics-collision hook).
    /// g_interactionHitByPlayerThrownPhysObj from `prop`. CNPC_MetroPolice also fires
    /// OnCupCopped when the prop is named cupcop_can.
    pub fn hit_by_player_thrown_object(
        &mut self,
        id: usize,
        prop: usize,
        world: &World,
        scene: &mut Scene,
    ) {
        let cop = self
            .actors
            .get(&id)
            .is_some_and(|a| a.class.cop().is_some());
        let can = world
            .entities
            .get(prop)
            .and_then(|e| e.get("targetname"))
            .is_some_and(|n| n.eq_ignore_ascii_case("cupcop_can"));
        if cop && can {
            scene.fire(id, "OnCupCopped", usize::MAX);
        }
        let now = scene.time;
        let random = self.random();
        let Some(actor) = self.actors.get_mut(&id) else {
            return;
        };
        let ctx = Context {
            now,
            origin: scene.states[id].origin,
            yaw: yaw_of(scene, id),
            random,
            ..Default::default()
        };
        // npc_metropolice's interaction; the other classes do not handle it.
        if let AiClass::Metropolice(cop) = &mut actor.class {
            cop.hit_by_player_thrown_object(&mut actor.npc, &ctx);
        }
        self.diagnostics.thrown_object_hits += 1;
    }

    /// +USE on an AI NPC (npc_metropolice's SetUse(PrecriminalUse)). False when the AI
    /// is off or `id` is not an AI cop, so the host falls back to the ordinary use (the
    /// companions' CommanderUse/speech is not reconstructed).
    pub fn player_used(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) -> bool {
        if !self.enabled || self.actors.get(&id).is_none_or(|a| a.class.cop().is_none()) {
            return false;
        }
        if !scene.ai_scripted(id) {
            let ctx = self.context(id, world, scene, player, scene.time, 0.);
            let actor = self.actors.get_mut(&id).expect("actor");
            if let AiClass::Metropolice(cop) = &mut actor.class {
                cop.precriminal_use(&mut actor.npc, &ctx);
            }
            self.apply_requests(id, world, scene, physics, player);
        }
        true
    }

    /// An NPC's bullet struck AI actor `target` (the victim responds at its next think).
    /// CMissile::Explode / removal -> the owner's CWeaponRPG::m_hMissile clears.
    pub fn npc_missile_died(&mut self, owner: usize) {
        if let Some(weapon) = self.actors.get_mut(&owner).and_then(|a| a.weapon.as_mut()) {
            weapon.missile_out = false;
        }
    }

    /// An NPC's guiding laser dot (TASK_CIT_RPG_AUGER): (position, shoot position).
    pub fn npc_laser(&self, owner: usize) -> Option<(Vec3, Vec3)> {
        let h = self.actors.get(&owner)?.class.humanoid()?;
        Some((h.rpg_laser?, h.shoot_position))
    }

    /// CWeaponAR2::FireNPCSecondaryAttack launches this tick: (shooter, origin, velocity).
    pub fn take_ball_launches(&mut self) -> Vec<(usize, Vec3, Vec3)> {
        std::mem::take(&mut self.ball_launches)
    }

    pub fn npc_attacked(&mut self, target: usize, attacker: usize) {
        if target == attacker {
            return;
        }
        if let Some(actor) = self.actors.get_mut(&target) {
            actor.damaged_by = Some(attacker);
        }
    }

    /// CSoundEnt::InsertSound for host-side sounds (the player's gunfire).
    pub fn insert_combat_sound(&mut self, origin: Vec3, volume: f32, now: f64) {
        self.sounds
            .insert(senses::sound::COMBAT, origin, volume, 0.2, None, now);
    }

    /// A spawn's disposition toward the player (F4 allegiance).
    pub fn set_player_disposition(&mut self, id: usize, disposition: Option<Disposition>) {
        if let Some(actor) = self.actors.get_mut(&id) {
            actor.player_disposition = disposition;
        }
        match disposition {
            Some(d) => self.allegiances.insert(id, d),
            None => self.allegiances.remove(&id),
        };
    }

    /// IRelationType of NPC `subject` toward NPC `target`. F4 allegiances make sides: two
    /// NPCs with allegiances hate each other when they differ and like each other when
    /// they match; one with an allegiance hates an NPC whose class (SDK table) hates the
    /// player when its allegiance is friendly, and one whose class likes the player when
    /// it is hostile (our extension: the SDK has only per-entity ai_relationship). Without
    /// allegiances the SDK class table decides.
    pub fn npc_disposition(
        &self,
        world: &World,
        subject: usize,
        target: usize,
        precriminal: bool,
    ) -> Disposition {
        let class = |id: usize| {
            classify(
                world.entities.get(id).map_or("", |e| e.class()),
                precriminal,
            )
        };
        let side = |id: usize| -> Option<Disposition> {
            self.allegiances.get(&id).copied().or_else(|| {
                match self.relationships.get(class(id), Class::Player).0 {
                    Disposition::HT => Some(Disposition::HT),
                    Disposition::LI => Some(Disposition::LI),
                    _ => None,
                }
            })
        };
        if let Some(&d) = self.relationship_overrides.get(&(subject, target)) {
            return d;
        }
        match (
            self.allegiances.get(&subject),
            self.allegiances.get(&target),
        ) {
            (Some(a), Some(b)) => {
                if a == b {
                    Disposition::LI
                } else {
                    Disposition::HT
                }
            }
            (Some(a), None) => match side(target) {
                Some(b) if b != *a => Disposition::HT,
                _ => Disposition::NU,
            },
            (None, Some(b)) => match side(subject) {
                Some(a) if a != *b => Disposition::HT,
                _ => Disposition::NU,
            },
            (None, None) => self.relationships.get(class(subject), class(target)).0,
        }
    }

    /// Switching the AI off (debug): every AI NPC drops its schedule, enemy and
    /// movement goal and returns to its idle activity.
    pub(super) fn stop_all(
        &mut self,
        world: &World,
        scene: &mut Scene,
        controller: &mut Controller,
    ) {
        for (&id, actor) in self.actors.iter_mut() {
            if let Some(key) = actor.goal.take() {
                controller.cancel(key);
                controller.forget(key);
                scene.ai_forget_move(key);
            }
            actor.npc.schedule = None;
            actor.npc.enemy = None;
            if !scene.ai_scripted(id) {
                scene.ai_set_activity(world, id, "ACT_IDLE");
            }
        }
    }

    /// CSoundEnt::InsertSound from gameplay (gunfire, impacts, explosions, footsteps).
    pub fn insert_sound(&mut self, kind: u32, origin: Vec3, volume: f32, duration: f64, now: f64) {
        self.sounds
            .insert(kind, origin, volume, duration, None, now);
    }
}
