//! Headcrabs and zombie torsos off the ground: MOVETYPE_STEP falls and leap touches, and
//! the hand-offs between a zombie and its headcrab (BecomeTorso, released, thrown and
//! ejected crabs, the poison zombie's nest).

use super::{
    center_z, headcrab, overlaps, push_out_of_box, zombie, AiClass, AiRuntime, MeleeHit, NpcState,
    PlayerHit, PlayerState, PLAYER,
};
use crate::entities::Scene;
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::World;

impl AiRuntime {
    /// MOVETYPE_STEP falling (sv_gravity 600) for NPCs off the ground (a zombie torso
    /// after BecomeTorso): a hull trace down each tick until it lands (FL_ONGROUND).
    pub(super) fn step_falls(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) {
        use crate::npc_probe::NpcCollisionWorld;
        let now = scene.time;
        // Airborne crab steps that did not bite the player: (crab, swept box start, end,
        // mins, maxs, hit a solid after the world-ignore window, landed).
        let mut npc_touches = Vec::new();
        let mut panics = Vec::new();
        // PhysicsTryMove traces with the NPC's solid mask, which the player's box blocks
        // (NPCs have colliders; the player has none, so it is a transient box here). A
        // dead player is not solid.
        let player_box = (!player.noclip && !player.dead).then(|| crate::npc_probe::ActorHull {
            entity: None,
            feet: player.feet,
            hull: crate::npc_probe::Hull {
                mins: Vec3::new(-16., -16., 0.),
                maxs: Vec3::new(16., 16., if player.crouched { 36. } else { 72. }),
            },
        });
        let transients: Vec<_> = player_box.into_iter().collect();
        for (&id, actor) in self.actors.iter_mut() {
            let Some((velocity, last)) = actor.fall else {
                continue;
            };
            if scene.states[id].killed {
                actor.fall = None;
                continue;
            }
            let dt = (now - last).clamp(0., 0.1) as f32;
            let mut velocity = velocity - Vec3::Z * 600. * dt;
            let hull = world
                .entities
                .get(id)
                .and_then(|e| crate::ai_hull::class_hull(e.class()))
                .unwrap_or(crate::ai_hull::HullType::Human)
                .bounds();
            let mut origin = scene.states[id].origin;
            // Our player moves after the NPCs against their colliders from before this
            // tick's NPC moves, so a leaping crab can start a tick a few units inside the
            // player's box (Source moves both on current positions and never overlaps):
            // push it out sideways first; if that fails the player box is left out.
            let mut transients = transients.as_slice();
            if let Some(player_box) = player_box {
                let (lo, hi) = (
                    player_box.feet + player_box.hull.mins,
                    player_box.feet + player_box.hull.maxs,
                );
                if let Some(out) = push_out_of_box(origin, hull, lo, hi, |p| {
                    !physics
                        .npc_trace_hull(
                            p,
                            p,
                            hull,
                            crate::npc_probe::Query {
                                excluded_entities: &[id],
                                ..Default::default()
                            },
                        )
                        .trace
                        .start_solid
                }) {
                    origin = out;
                    self.diagnostics.player_box_pushouts += 1;
                } else if overlaps(origin + hull.mins, origin + hull.maxs, lo, hi) {
                    transients = &[];
                }
            }
            let end = origin + velocity * dt;
            let trace = |excluded: &[usize]| {
                physics.npc_trace_hull(
                    origin,
                    end,
                    hull,
                    crate::npc_probe::Query {
                        excluded_entities: excluded,
                        transients,
                        ..Default::default()
                    },
                )
            };
            // A dead entity's colliders stay until the next collider pose stage (the zombie
            // a torso replaces this tick, whose mesh collider the torso starts inside): they
            // do not hold the faller up.
            let mut excluded = vec![id];
            excluded.extend(actor.fall_owner);
            let mut hit = trace(&excluded);
            // CHalfLife2::ShouldCollide: HL2COLLISION_GROUP_HEADCRAB never collides with
            // itself (a poison zombie's nest crabs eject from overlapping spots).
            let crab = matches!(actor.class, AiClass::Headcrab(_));
            let passable = |e: usize| {
                scene.states.get(e).is_some_and(|s| s.killed)
                    || crab
                        && world
                            .entities
                            .get(e)
                            .is_some_and(|x| headcrab::is_headcrab_class(x.class()))
            };
            while let Some(dead) = hit.entity.filter(|e| excluded.len() < 8 && passable(*e)) {
                excluded.push(dead);
                hit = trace(&excluded);
            }
            let fraction = if hit.trace.start_solid {
                0.
            } else {
                hit.trace.fraction.clamp(0., 1.)
            };
            let position = origin + (end - origin) * fraction;
            scene.states[id].origin = position;
            // The player's box was the first solid in this step's path.
            let player_hit = hit.transient && hit.entity.is_none() && fraction < 1.;
            // A floor (or a stuck start) lands; a wall or ceiling stops that part of the
            // motion and the fall goes on. The player's box never lands it (below).
            let landed = fraction < 1.
                && !player_hit
                && (hit.trace.start_solid
                    || hit.trace.normal.z > 0.7
                    || velocity.z <= 0. && velocity.truncate().length() < 1.);
            // PhysicsTryMove's ClipVelocity (overbounce 1): a crab that strikes the player
            // keeps only the motion along his box and slides down beside him.
            if fraction < 1. && !landed {
                let n = hit.trace.normal;
                velocity -= n * velocity.dot(n).min(0.);
            }
            // Onto his head (JumpAttack aims at the eyes inside his box, so a crab can land
            // on top, as the SDK notes): Source stands it on him and its AI walks it off;
            // ours cannot stand on the player, so it drops off his nearest side at once.
            if let (true, Some(player_box)) = (player_hit && hit.trace.normal.z > 0.7, player_box) {
                let lo = player_box.feet + player_box.hull.mins;
                let mut hi = player_box.feet + player_box.hull.maxs;
                hi.z = hi.z.max(position.z + hull.maxs.z + 1.);
                let clear = |p: Vec3| {
                    !physics
                        .npc_trace_hull(
                            p,
                            p,
                            hull,
                            crate::npc_probe::Query {
                                excluded_entities: &[id],
                                ..Default::default()
                            },
                        )
                        .trace
                        .start_solid
                };
                if let Some(off) = push_out_of_box(position, hull, lo, hi, clear) {
                    scene.states[id].origin = off;
                    velocity.x = 0.;
                    velocity.y = 0.;
                    self.diagnostics.player_box_pushouts += 1;
                }
            }
            // CBaseHeadcrab::LeapTouch while airborne: the hated player's hull bites (once);
            // another solid after the world-ignore window fails the attack; landing ends it.
            if let AiClass::Headcrab(crab) = &mut actor.class {
                let touching = player_hit;
                // LeapTouch starts with m_bMidJump = false: any touch or landing ends the
                // jump (a thrown crab never runs the leap task that also clears it).
                if touching || landed {
                    crab.mid_jump = false;
                }
                if crab.leap_touch {
                    if touching && actor.npc.enemy == Some(PLAYER) && !player.dead {
                        self.diagnostics.player_hits += 1;
                        crab.leap_touch = false;
                        crab.attack_failed = false;
                        crab.bites += 1;
                        if crab.black {
                            self.poison_bites.push((id, self.headcrab_melee_damage));
                        } else {
                            self.player_hits.push(PlayerHit {
                                attacker: id,
                                damage: self.headcrab_melee_damage,
                                punch: Vec3::ZERO,
                                impulse: Vec3::ZERO,
                                fade: None,
                            });
                        }
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(position + Vec3::Z * 12.),
                            ..crab.sound("Bite").into()
                        });
                    } else {
                        let failed = fraction < 1. && now >= crab.ignore_world_until && !landed;
                        npc_touches.push((id, origin, end, hull.mins, hull.maxs, failed, landed));
                    }
                }
            }
            if fraction < 1. || landed {
                let center = scene.states[id].origin + Vec3::Z * center_z(world, id);
                super::zombies::leap_touch(&mut actor.class, &mut velocity, center, scene, id);
            }
            actor.fall = (!landed).then_some((velocity, now));
            // CBlackHeadcrab::EjectTouch: on the ground, Panic(U(2, 8)).
            if let AiClass::Headcrab(crab) = &mut actor.class {
                if landed && std::mem::take(&mut crab.eject_touch) {
                    panics.push(id);
                }
            }
            if actor.fall.is_none() {
                actor.fall_owner = None;
                let landing = (
                    id,
                    scene.states[id].origin.z,
                    hit.entity,
                    hit.trace.start_solid,
                );
                super::zombies::record_landing(&mut self.diagnostics, landing, fraction);
            }
        }
        // LeapTouch on an NPC: a hated one (that takes damage) in the step bites
        // (TouchDamage: sk_headcrab_melee_dmg, through the shared melee hit path); else a
        // solid after the world-ignore window fails the attack and a landing ends it.
        for id in panics {
            let duration = 2. + 6. * f64::from(self.random());
            if let Some(AiClass::Headcrab(crab)) = self.actors.get_mut(&id).map(|a| &mut a.class) {
                crab.start_panic(now, duration);
            }
        }
        for (id, start, end, mins, maxs, failed, landed) in npc_touches {
            let victim = self.melee_victim(id, world, scene, start, end, mins, maxs);
            let damage = self.headcrab_melee_damage;
            let Some(actor) = self.actors.get_mut(&id) else {
                continue;
            };
            let AiClass::Headcrab(crab) = &mut actor.class else {
                continue;
            };
            // CBlackHeadcrab::TouchDamage leaves the victim one health.
            let health = victim.map_or(0., |v| scene.states[v].value);
            let damage = if crab.black {
                headcrab::poison_bite(damage, health).map(|(bite, poison)| bite + poison)
            } else {
                Some(damage)
            };
            if let (Some(victim), Some(damage)) = (victim, damage) {
                // The victim's box stops the leap like the player's (NPC traces do not
                // include NPC boxes).
                if let Some((velocity, _)) = &mut actor.fall {
                    velocity.x = 0.;
                    velocity.y = 0.;
                }
                crab.leap_touch = false;
                crab.attack_failed = false;
                crab.bites += 1;
                self.diagnostics.npc_melee_hits += 1;
                self.melee_hits.push(MeleeHit {
                    attacker: id,
                    victim,
                    damage,
                    position: scene.states[victim].origin + Vec3::Z * center_z(world, victim),
                    direction: (end - start).normalize_or_zero(),
                });
                scene.sounds.push(crate::sounds::SoundRequest {
                    origin: Some(end + Vec3::Z * 12.),
                    ..crab.sound("Bite").into()
                });
            } else if failed {
                crab.attack_failed = true;
                crab.leap_touch = false;
            } else if landed {
                crab.leap_touch = false;
            }
        }
    }
    /// CNPC_BaseZombie::BecomeTorso's AI part on the torso entity that replaces the
    /// zombie (DESIGN 14o session 37; Source keeps one entity): the zombie's state, enemy,
    /// memories and squad carry over; ClearSchedule, SetSchedule(SCHED_FALL_TO_GROUND)
    /// from 40 units up with no torso impulse; SetZombieModel's SetActivity(ACT_IDLE).
    pub fn become_torso(
        &mut self,
        world: &World,
        scene: &mut Scene,
        zombie: usize,
        torso: usize,
    ) -> bool {
        let now = scene.time;
        let Some(old) = self.actors.get(&zombie) else {
            return false;
        };
        let npc = old.npc.clone();
        let (squad, squad_name, disposition) = (
            old.squad.clone(),
            old.squad_name.clone(),
            old.player_disposition,
        );
        let Some(actor) = self.actors.get_mut(&torso) else {
            return false;
        };
        actor.npc = npc;
        actor.npc.entity = torso;
        actor.npc.force_schedule("SCHED_FALL_TO_GROUND");
        actor.npc.activity = "ACT_IDLE".into();
        scene.ai_set_activity(world, torso, "ACT_IDLE");
        actor.squad = squad;
        actor.squad_name = squad_name;
        actor.player_disposition = disposition;
        actor.fall = Some((Vec3::ZERO, now));
        actor.next_think = now;
        self.npc_ids = None;
        true
    }
    /// ReleaseHeadcrab's live crab (RELEASE_IMMEDIATE): it takes the zombie's enemy
    /// (SetEnemy) and memory, idles (SetActivity(ACT_IDLE)), leaps no sooner than 1 s
    /// (m_flNextAttack) and falls from the head (SF_NPC_FALL_TO_GROUND).
    pub fn released_headcrab(&mut self, zombie: usize, crab: usize, now: f64) -> bool {
        let Some(old) = self.actors.get(&zombie) else {
            return false;
        };
        let (enemy, enemies) = (old.npc.enemy, old.npc.enemies.clone());
        let Some(actor) = self.actors.get_mut(&crab) else {
            return false;
        };
        actor.npc.enemy = enemy;
        actor.npc.enemies = enemies;
        if enemy.is_some() {
            actor.npc.state = NpcState::Combat;
        }
        actor.npc.activity = "ACT_IDLE".into();
        actor.fall = Some((Vec3::ZERO, now));
        actor.next_think = now;
        if let AiClass::Headcrab(h) = &mut actor.class {
            h.next_attack = now + 1.;
        }
        self.npc_ids = None;
        true
    }
    /// AE_ZOMBIE_POISON_THROW_CRAB's crab (CreateNoSpawn at the zombie's EyePosition, owner
    /// the zombie, SF_NPC_FALL_TO_GROUND): ACT_RANGE_ATTACK1, the zombie's enemy, and
    /// ThrowAt(the enemy's eyes) = JumpAttack thrown (LeapTouch armed); no target: it falls.
    pub fn thrown_headcrab(
        &mut self,
        zombie: usize,
        crab: usize,
        target: Option<Vec3>,
        scene: &mut Scene,
    ) -> bool {
        let now = scene.time;
        let Some(old) = self.actors.get(&zombie) else {
            return false;
        };
        let (enemy, enemies) = (old.npc.enemy, old.npc.enemies.clone());
        let origin = scene.states[crab].origin;
        let Some(actor) = self.actors.get_mut(&crab) else {
            return false;
        };
        actor.npc.enemy = enemy;
        actor.npc.enemies = enemies;
        if enemy.is_some() {
            actor.npc.state = NpcState::Combat;
        }
        actor.npc.activity = "ACT_RANGE_ATTACK1".into();
        actor.next_think = now;
        actor.fall_owner = Some(zombie);
        let velocity = target.map_or(Vec3::ZERO, |t| {
            headcrab::jump_velocity(origin, t, 600., true)
        });
        if let AiClass::Headcrab(h) = &mut actor.class {
            if target.is_some() {
                // Leap: MoveOrigin(0, 0, 1), LeapTouch, the world-ignore window.
                scene.states[crab].origin.z += 1.;
                h.mid_jump = true;
                h.leap_touch = true;
                h.ignore_world_until = now + headcrab::IGNORE_WORLD_COLLISION_TIME;
            }
        }
        actor.fall = Some((velocity, now));
        self.npc_ids = None;
        true
    }
    /// CNPC_PoisonZombie::Event_Killed (not a torso): the nest crabs to evacuate (and
    /// whether one was in its hand, which only turns its bodygroup off), with the zombie's
    /// enemy for them; the nest empties. None for other NPCs.
    pub fn take_poison_nest(&mut self, zombie: usize) -> Option<([bool; 3], bool, Option<usize>)> {
        let actor = self.actors.get_mut(&zombie)?;
        let enemy = actor.npc.enemy;
        match &mut actor.class {
            AiClass::Zombie(z) if z.kind == zombie::Kind::Poison && !z.torso => {
                let nest = z.nest.take().unwrap_or_default();
                Some((nest, std::mem::take(&mut z.holding_crab), enemy))
            }
            _ => None,
        }
    }
    /// CBlackHeadcrab::Eject (EvacuateNest): off the ground with `velocity`, the zombie's
    /// attacker as enemy (here its enemy), alert, ACT_RANGE_ATTACK1, EjectTouch = LeapTouch
    /// then Panic(U(2, 8)) on landing, the zombie left out of its fall trace.
    pub fn ejected_headcrab(
        &mut self,
        zombie: usize,
        crab: usize,
        velocity: Vec3,
        enemy: Option<usize>,
        scene: &mut Scene,
    ) -> bool {
        let now = scene.time;
        let enemies = self.actors.get(&zombie).map(|a| a.npc.enemies.clone());
        let Some(actor) = self.actors.get_mut(&crab) else {
            return false;
        };
        actor.npc.enemy = enemy;
        if let Some(enemies) = enemies {
            actor.npc.enemies = enemies;
        }
        actor.npc.state = NpcState::Alert;
        actor.npc.activity = "ACT_RANGE_ATTACK1".into();
        actor.next_think = now;
        actor.fall_owner = Some(zombie);
        actor.fall = Some((velocity, now));
        if let AiClass::Headcrab(h) = &mut actor.class {
            h.leap_touch = true;
            h.ignore_world_until = now;
            h.eject_touch = h.black;
        }
        self.npc_ids = None;
        true
    }
}
