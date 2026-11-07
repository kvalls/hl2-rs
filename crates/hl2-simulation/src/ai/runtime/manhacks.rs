//! Manhacks in the world (DESIGN 14o session 47): `ai::manhack` decides; this stage steers once per
//! think and carries the body every tick. It flies straight at what it is after: there is no route
//! round what stands in the way yet, so a wall stops it until its enemy moves.

use super::super::host::PhysicsSight;
use super::super::manhack::{self, Manhack};
use super::super::metropolice::{CopRequest, PLAYER};
use super::super::senses::SightWorld;
use super::super::squad::{Squad, SLOT_ATTACK1, SLOT_ATTACK2};
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
use std::collections::BTreeMap;

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
    // A burst holds its line at the enemy; otherwise a swarm's move leads.
    let swarming = m.swarm.flying_at(now).filter(|_| !m.bursting(now));
    m.target = swarming.or(enemy.map(|e| e.eyes));
    check_swarm_line(m, air, origin);
    m.update_panels(now);
    if scene.manhack_panels.get(&id) != Some(&m.panels) {
        scene.manhack_panels.insert(id, m.panels);
    }
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

/// A new move's line: the world in the way fails it.
fn check_swarm_line(m: &mut Manhack, air: &Air, origin: Vec3) {
    if std::mem::take(&mut m.swarm.unchecked) {
        let sight = PhysicsSight {
            physics: air.physics,
            world: air.world,
        };
        m.swarm.blocked = !sight.visible(origin, m.swarm.point, &[air.id]);
    }
}

/// Gives up the attack slot it holds. Squads keep a slot until it is given back, so this runs on
/// death too: a slot left behind would keep its squad swarming for good.
fn release_slot(m: &mut Manhack, squads: &mut BTreeMap<String, Squad>, squad: Option<&str>) {
    if let Some((enemy, slot)) = m.swarm.slot.take() {
        if let Some(squad) = squad.and_then(|name| squads.get_mut(name)) {
            squad.vacate(enemy, slot);
        }
    }
}

/// OccupyStrategySlotRange, once a think: a rested manhack in a squad needs one of the two attack
/// slots on its enemy to chase. Alone or with swarming off it needs none.
fn claim_slot(
    m: &mut Manhack,
    squads: &mut BTreeMap<String, Squad>,
    squad: Option<&str>,
    (enemy, now): (Option<usize>, f64),
) {
    m.swarm.squadded = squad.is_some() && !m.swarm.disabled;
    if !m.swarm.squadded {
        release_slot(m, squads, squad);
        m.swarm.may_attack = true;
        return;
    }
    let rested = enemy.is_some() && now >= m.next_burst;
    if !rested || m.swarm.slot.is_some_and(|(held, _)| held != enemy) {
        release_slot(m, squads, squad);
    }
    let Some(group) = squad
        .and_then(|name| squads.get_mut(name))
        .filter(|_| rested)
    else {
        m.swarm.may_attack = false;
        return;
    };
    let mut slot = m.swarm.slot.map(|(_, slot)| slot);
    m.swarm.may_attack = group.occupy_range(enemy, SLOT_ATTACK1, SLOT_ATTACK2, &mut slot);
    m.swarm.slot = slot.map(|slot| (enemy, slot));
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
    /// InputUnpack and InputDisableSwarm.
    pub(super) fn manhack_input(&mut self, id: usize, input: &str) {
        if let Some(AiClass::Manhack(m)) = self.actors.get_mut(&id).map(|actor| &mut actor.class) {
            match input {
                "unpack" => m.wake(),
                "disableswarm" => m.swarm.disabled = true,
                _ => {}
            }
        }
    }
    /// The living manhacks' eye and light, placed for this frame.
    pub fn manhack_glows<'a>(
        &'a self,
        world: &'a World,
        scene: &'a Scene,
    ) -> impl Iterator<Item = (usize, Vec3)> + 'a {
        let lit = |id: &usize| {
            let state = scene.states.get(*id);
            state.is_some_and(|s| s.visible && !s.killed && !s.corpse)
        };
        self.actors
            .iter()
            .filter(|(_, actor)| matches!(actor.class, AiClass::Manhack(_)))
            .map(|(&id, _)| id)
            .filter(lit)
            .flat_map(move |id| {
                let frames = scene.attachment_frames(world, id, &manhack::GLOW_ATTACHMENTS);
                frames
                    .into_iter()
                    .flatten()
                    .map(move |frame| (id, frame.w_axis.truncate()))
            })
    }
    /// The squad's members as the swarm reads them.
    fn swarm_members(&self, squad: &str, scene: &Scene) -> Vec<manhack::Member> {
        let members = self.squads.get(squad).map_or(&[][..], |s| &s.members);
        let read = |id: &usize| {
            let origin = scene.states.get(*id)?.origin;
            let class = &self.actors.get(id)?.class;
            let manhack = if let AiClass::Manhack(m) = class {
                Some(m)
            } else {
                None
            };
            Some(manhack::Member {
                origin,
                attacking: manhack.is_some_and(|m| m.swarm.slot.is_some()),
                has_goal: manhack.is_some_and(|m| m.target.is_some()),
            })
        };
        members.iter().filter_map(read).collect()
    }
    /// A think's question to the squad, answered once the actors are free again. Alone it stays
    /// where it is.
    fn answer_swarm(&mut self, scene: &Scene, id: usize, ask: manhack::Ask) {
        let origin = scene.states[id].origin;
        let actor = &self.actors[&id];
        let enemy = actor
            .npc
            .enemy
            .and_then(|enemy| actor.npc.enemies.find(Some(enemy)))
            .map(|memory| memory.last_known);
        let point = actor.squad.as_deref().and_then(|squad| {
            let members = self.swarm_members(squad, scene);
            match ask {
                manhack::Ask::Center => manhack::center(&members, enemy),
                manhack::Ask::Member => manhack::member_to_join(&members),
            }
        });
        if let Some(AiClass::Manhack(m)) = self.actors.get_mut(&id).map(|actor| &mut actor.class) {
            m.swarm.answer(point.unwrap_or(origin));
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
        let (held, squads) = (&self.held_npcs, &mut self.squads);
        let mut cuts = Vec::new();
        let mut asks = Vec::new();
        for (&id, actor) in self.actors.iter_mut() {
            let enemy = enemy_of(actor, world, scene, &player);
            let enemy_id = actor.npc.enemy;
            let AiClass::Manhack(m) = &mut actor.class else {
                continue;
            };
            actor.fall = None;
            let squad = actor.squad.as_deref();
            if scene.states[id].killed || scene.states[id].corpse {
                release_slot(m, squads, squad);
            }
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
            if let Some(ask) = m.swarm.ask {
                asks.push((id, ask));
            }
            if now >= m.next_steer {
                m.next_steer = now + THINK_INTERVAL;
                claim_slot(m, squads, squad, (enemy_id, now));
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
            if let Some(look) = enemy.as_ref().map(|e| e.eyes).or(m.target) {
                let yaw = angles_toward(look - scene.states[id].origin).1;
                let current = super::super::host::yaw_of(scene, id);
                let turned = super::super::host::clamp_yaw(TURN_RATE, current, yaw, dt);
                scene.states[id].rotation = angles(Vec3::new(0., turned, 0.));
            }
        }
        for (id, ask) in asks {
            self.answer_swarm(scene, id, ask);
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

#[cfg(test)]
mod tests {
    use super::*;

    const ENEMY: Option<usize> = Some(1);

    fn squads() -> BTreeMap<String, Squad> {
        BTreeMap::from([("pack".to_owned(), Squad::new("pack"))])
    }

    fn claim(m: &mut Manhack, squads: &mut BTreeMap<String, Squad>, enemy: Option<usize>) {
        claim_slot(m, squads, Some("pack"), (enemy, 10.));
    }

    #[test]
    fn two_of_a_squad_attack_and_the_third_swarms() {
        let mut squads = squads();
        let mut pack: Vec<Manhack> = (0..3).map(|seed| Manhack::new(false, seed)).collect();
        for m in &mut pack {
            claim(m, &mut squads, ENEMY);
        }
        let free: Vec<bool> = pack.iter().map(|m| m.swarm.may_attack).collect();
        assert_eq!(free, [true, true, false]);
        assert!(pack.iter().all(|m| m.swarm.squadded));
    }

    #[test]
    fn a_dead_attacker_gives_its_slot_to_the_squad() {
        let mut squads = squads();
        let mut pack: Vec<Manhack> = (0..3).map(|seed| Manhack::new(false, seed)).collect();
        for m in &mut pack {
            claim(m, &mut squads, ENEMY);
        }
        release_slot(&mut pack[0], &mut squads, Some("pack"));
        claim(&mut pack[2], &mut squads, ENEMY);
        assert!(pack[2].swarm.may_attack);
    }

    #[test]
    fn a_resting_manhack_holds_no_slot() {
        let mut squads = squads();
        let (mut first, mut second, mut third) = (
            Manhack::new(false, 1),
            Manhack::new(false, 2),
            Manhack::new(false, 3),
        );
        claim(&mut first, &mut squads, ENEMY);
        claim(&mut second, &mut squads, ENEMY);
        first.next_burst = 12.;
        claim(&mut first, &mut squads, ENEMY);
        assert_eq!(first.swarm.slot, None);
        claim(&mut third, &mut squads, ENEMY);
        assert!(third.swarm.may_attack);
    }

    #[test]
    fn a_new_enemy_moves_the_slot_instead_of_leaving_the_old_one_taken() {
        let mut squads = squads();
        let mut m = Manhack::new(false, 1);
        claim(&mut m, &mut squads, ENEMY);
        claim(&mut m, &mut squads, Some(2));
        assert!(m.swarm.slot.is_some_and(|(enemy, _)| enemy == Some(2)));
        let squad = &squads["pack"];
        assert!(!squad.is_slot_occupied(ENEMY, SLOT_ATTACK1));
        assert!(squad.is_slot_occupied(Some(2), SLOT_ATTACK1));
    }

    #[test]
    fn without_a_squad_or_with_swarming_off_it_needs_no_slot() {
        let mut squads = squads();
        let mut alone = Manhack::new(false, 1);
        claim_slot(&mut alone, &mut squads, None, (ENEMY, 10.));
        assert!(alone.swarm.may_attack && !alone.swarm.squadded);
        let mut solo = Manhack::new(false, 2);
        solo.swarm.disabled = true;
        claim(&mut solo, &mut squads, ENEMY);
        assert!(solo.swarm.may_attack && !solo.swarm.squadded);
        assert_eq!(solo.swarm.slot, None);
    }

    #[test]
    fn swarming_off_gives_back_a_slot_already_held() {
        let mut squads = squads();
        let mut m = Manhack::new(false, 1);
        claim(&mut m, &mut squads, ENEMY);
        m.swarm.disabled = true;
        claim(&mut m, &mut squads, ENEMY);
        assert!(!squads["pack"].is_slot_occupied(ENEMY, SLOT_ATTACK1));
    }
}
