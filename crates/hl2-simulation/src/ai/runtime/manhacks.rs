//! Manhacks in the world (DESIGN 14o session 47): `ai::manhack` decides; this stage steers once per
//! think and carries the body every tick. It flies straight at what it is after: there is no route
//! round what stands in the way yet, so a wall stops it until its enemy moves.

use super::super::manhack::{self, Manhack};
use super::super::metropolice::{CopRequest, PLAYER};
use super::birds::Air;
use super::crabs::fall_hull;
use super::scanners::{break_body, strikes_player};
use super::{AiActor, AiClass, AiRuntime, MeleeHit, PlayerHit, PlayerState, THINK_INTERVAL};
use crate::entities::Scene;
use crate::physics::{angles, Physics};
use crate::sounds::{entity_emitter, PatchOp, SoundRequest};
use crate::view_control::angles_toward;
use glam::Vec3;
use modkit_core::World;

/// sk_manhack_melee_dmg where skill.cfg leaves it out.
const CUT_DAMAGE: f32 = 20.;
/// The bodygroup that shows its blade.
const BLADE_GROUP: usize = 1;
/// How close to an NPC's middle counts as touching it.
const CUT_REACH: f32 = 32.;
/// How fast the body turns toward what it flies at, degrees a second.
const TURN_RATE: f32 = 360.;

/// What a think cut, for the stage to pay out once the actors are free again.
enum Cut {
    Player,
    Npc(usize, Vec3),
}

/// Its enemy's place across the ground and at height, for a burst's checks.
struct EnemyView {
    across: f32,
    in_height: bool,
    to_eyes: Vec3,
}

fn enemy_view(origin: Vec3, feet: Vec3, eyes: Vec3) -> EnemyView {
    EnemyView {
        across: (feet - origin).truncate().length(),
        in_height: origin.z > feet.z && origin.z < eyes.z + manhack::EYE_HEIGHT,
        to_eyes: eyes - origin,
    }
}

/// A think's collisions: a cut if it touches what it hunts, else a bump off what is in its way.
fn collide(m: &mut Manhack, air: &Air, origin: Vec3, hit: Option<Cut>, now: f64) -> Option<Cut> {
    if let Some(cut) = hit {
        return Some(cut);
    }
    let ahead = air.trace(origin, origin + m.hover.velocity * THINK_INTERVAL as f32);
    if ahead.trace.fraction < 1. {
        m.hover.bounce(ahead.trace.normal);
        m.stop_burst(now);
        m.requests.push(CopRequest::Sound(manhack::GRIND));
    }
    None
}

/// The enemy's feet and eyes, and whether it is the player.
struct Enemy {
    id: usize,
    feet: Vec3,
    eyes: Vec3,
}

fn enemy_of(actor: &AiActor, world: &World, scene: &Scene, player: &PlayerState) -> Option<Enemy> {
    let id = actor.npc.enemy?;
    if id == PLAYER {
        return Some(Enemy {
            id,
            feet: player.feet,
            eyes: player.eye(),
        });
    }
    let feet = scene.states.get(id)?.origin;
    let height = crate::ai_hull::eye_height(world.entities.get(id)?.class());
    Some(Enemy {
        id,
        feet,
        eyes: feet + Vec3::Z * height,
    })
}

/// One think of flight: thrust, noise, its limits, a burst if one is due, and what it touched.
fn steer(
    m: &mut Manhack,
    air: &Air,
    scene: &mut Scene,
    enemy: Option<&Enemy>,
    player: PlayerState,
) -> Option<Cut> {
    let (id, now) = (air.id, scene.time);
    let origin = scene.states[id].origin;
    let interval = THINK_INTERVAL as f32;
    if !m.blade_shut && !std::mem::replace(&mut m.blade_out, true) {
        scene.set_bodygroup(air.world, id, BLADE_GROUP, 1);
    }
    m.target = enemy.map(|e| e.eyes);
    m.fly(origin, interval, now);
    let view = enemy.map(|e| enemy_view(origin, e.feet, e.eyes));
    m.hover
        .add_noise(m.bob, m.noise(view.as_ref().map(|v| v.across)), now as f32);
    m.hover.velocity.z = m
        .hover
        .velocity
        .z
        .clamp(-manhack::MAX_CLIMB, manhack::MAX_CLIMB);
    m.hover.limit(manhack::MAX_SPEED);
    if let Some(view) = &view {
        if m.burst_ready(view.across, view.in_height, view.to_eyes, now) {
            m.start_burst(view.to_eyes, now);
        }
    }
    m.engine_sound(enemy.map(|e| origin.distance(e.feet)));
    let touching = enemy.and_then(|e| {
        if e.id == PLAYER {
            strikes_player(air, origin, player).then_some(Cut::Player)
        } else {
            let middle = (e.feet + e.eyes) * 0.5;
            (origin.distance(middle) < CUT_REACH).then_some(Cut::Npc(e.id, middle))
        }
    });
    collide(m, air, origin, touching, now)
}

/// A killed manhack's end: the engines stop, it says so and its pieces fly.
fn end(m: &mut Manhack, air: &Air, scene: &mut Scene) {
    let id = air.id;
    let origin = scene.states[id].origin;
    if std::mem::take(&mut m.engine_on) {
        for cue in Manhack::engine_cues() {
            scene.sounds.push(SoundRequest {
                origin: Some(origin),
                ..SoundRequest::patch(cue, entity_emitter(id), PatchOp::Destroy)
            });
        }
    }
    scene.sounds.push(SoundRequest {
        origin: Some(origin),
        ..manhack::DIE.into()
    });
    break_body(air, scene, m.hover.velocity);
}

/// SF_NPC_WAIT_FOR_SCRIPT.
const SF_NPC_WAIT_FOR_SCRIPT: u32 = 128;

impl AiRuntime {
    /// InputUnpack.
    pub(super) fn unpack_manhack(&mut self, id: usize) {
        if let Some(AiClass::Manhack(m)) = self.actors.get_mut(&id).map(|actor| &mut actor.class) {
            m.wake();
        }
    }
    /// This tick's manhacks: a steer once per think and each one's step.
    pub(super) fn fly_manhacks(
        &mut self,
        (world, physics): (&World, &Physics),
        scene: &mut Scene,
        player: PlayerState,
    ) {
        let now = scene.time;
        let per_second = scene.skill.get("sk_manhack_melee_dmg", CUT_DAMAGE);
        let held = &self.held_npcs;
        let mut cuts = Vec::new();
        for (&id, actor) in self.actors.iter_mut() {
            let enemy = enemy_of(actor, world, scene, &player);
            let AiClass::Manhack(m) = &mut actor.class else {
                continue;
            };
            actor.fall = None;
            if scene.states[id].killed || held.contains(&id) {
                continue;
            }
            let air = Air {
                world,
                physics,
                hull: fall_hull(world, id),
                id,
            };
            if scene.states[id].corpse {
                end(m, &air, scene);
                continue;
            }
            // OverrideMove: a packed manhack executes no move code.
            if m.packed {
                continue;
            }
            if now >= m.next_steer {
                m.next_steer = now + THINK_INTERVAL;
                if let Some(cut) = steer(m, &air, scene, enemy.as_ref(), player) {
                    let origin = scene.states[id].origin;
                    let from = match &cut {
                        Cut::Player => player.feet + Vec3::Z * 36.,
                        Cut::Npc(_, middle) => *middle,
                    };
                    m.rebound(origin - from, now);
                    cuts.push((id, cut, origin));
                }
            }
            let dt = air.step(&mut m.hover.velocity, &mut m.last_step, scene);
            if let Some(look) = m.target {
                let yaw = angles_toward(look - scene.states[id].origin).1;
                let current = super::super::host::yaw_of(scene, id);
                let turned = super::super::host::clamp_yaw(TURN_RATE, current, yaw, dt);
                scene.states[id].rotation = angles(Vec3::new(0., turned, 0.));
            }
        }
        for (id, cut, origin) in cuts {
            self.pay_cut(scene, id, cut, origin, per_second);
        }
    }
    /// What a cut does: the damage and its sound.
    fn pay_cut(&mut self, scene: &mut Scene, id: usize, cut: Cut, origin: Vec3, per_second: f32) {
        let interval = THINK_INTERVAL as f32;
        scene.sounds.push(SoundRequest {
            origin: Some(origin),
            ..manhack::SLICE.into()
        });
        match cut {
            Cut::Player => self.player_hits.push(PlayerHit {
                attacker: id,
                damage: manhack::cut_damage(per_second, interval, true),
                punch: Vec3::ZERO,
                impulse: Vec3::ZERO,
                fade: None,
            }),
            Cut::Npc(victim, middle) => self.melee_hits.push(MeleeHit {
                attacker: id,
                victim,
                damage: manhack::cut_damage(per_second, interval, false),
                position: middle,
                direction: (middle - origin).normalize_or_zero(),
            }),
        }
    }
}

/// The class of a map's manhack.
pub(super) fn class_for(flags: u32, id: usize) -> AiClass {
    let gagged = flags & super::SF_NPC_GAG != 0;
    let mut manhack = Manhack::new(gagged, 0x3c6e_f372_fe94_f82b ^ id as u64);
    if flags & manhack::SF_PACKED_UP != 0 {
        manhack.pack(flags & SF_NPC_WAIT_FOR_SCRIPT != 0);
    }
    AiClass::Manhack(Box::new(manhack))
}
