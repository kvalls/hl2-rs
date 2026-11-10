//! Manhacks in the world (DESIGN 14o session 47): `ai::manhack` decides; this stage steers once per
//! think and carries the body every tick. It flies straight at what it is after: there is no route
//! round what stands in the way yet, so a wall stops it until its enemy moves.

use super::super::host::PhysicsSight;
use super::super::manhack::{self, body, Bumped, Manhack};
use super::super::metropolice::{CopRequest, PLAYER};
use super::super::relationships::{classify, Class, Disposition, Relationships};
use super::super::scanner::damage::effect as scanner_effect;
use super::super::senses::SightWorld;
use super::super::squad::{Squad, SLOT_ATTACK1, SLOT_ATTACK2};
use super::birds::Air;
use super::crabs::fall_hull;
use super::scanners::{break_body, strikes_player};
use super::{AiActor, AiClass, AiRuntime, MeleeHit, PlayerHit, PlayerState, THINK_INTERVAL};
use crate::entities::Scene;
use crate::npc_probe::{NpcCollisionWorld, Query};
use crate::physics::Physics;
use crate::sounds::{entity_emitter, PatchOp, SoundRequest};
use crate::view_control::angles_toward;
use crate::weapon_physcannon::NpcPhysgun;
use glam::Vec3;
use modkit_core::World;
use std::collections::BTreeMap;

mod contact;

/// sk_manhack_melee_dmg where skill.cfg leaves it out.
const CUT_DAMAGE: f32 = 20.;
/// The smoke trail's twenty puffs a second, at a think of 0.1 s.
const PUFFS_PER_THINK: usize = 2;
/// Bump grinds when the surface's normal is this close to across the blades' plane.
const GRIND_ACROSS: f32 = 0.25;
/// MaintainGroundHeight's 52 units and its 32 u/s climb.
const GROUND_HEIGHT: f32 = 52.;
const GROUND_RISE: f32 = 32.;
/// The bodygroup that shows its blade.
const BLADE_GROUP: usize = 1;
const BLUR_GROUP: usize = 2;
/// How close to an NPC's middle counts as touching it.
const CUT_REACH: f32 = 32.;
/// CheckCollisions: a held manhack this close to its carrier traces along the aim this far.
const CARRIER_NEAR: f32 = 30.;
const CARRIER_REACH: f32 = 40.;

/// What a think cut, for the stage to pay out once the actors are free again.
enum Cut {
    Player,
    Npc(usize, Vec3),
    /// Thrown into something: this share of its max health.
    Smash(f32),
    /// CheckCollisions met an entity: who and what it met, still to be judged.
    Touch(usize, Bumped),
}

/// What a cut comes to: damage on the player, a slice, or the manhack's own smash.
enum Paid {
    Player(f32),
    /// An NPC or a breakable cut there for this much; the player deals it when the manhack is
    /// held or thrown; what does not bleed grinds.
    Slice {
        victim: usize,
        at: Vec3,
        damage: f32,
        by_player: bool,
        npc: bool,
        grinds: bool,
    },
    Smash(f32),
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

/// A think's collisions: a cut if it touches what it hunts, else CheckCollisions' trace along a
/// think of its body's flight. A body pressed on a surface by a motor still pushing into it meets
/// that surface too (inferred: VPhysics keeps such a body in contact; not compared with native).
fn collide(m: &mut Manhack, air: &Air, origin: Vec3, hit: Option<Cut>, now: f64) -> Option<Cut> {
    let pressed = m.contact.take();
    if let Some(cut) = hit {
        return Some(cut);
    }
    let reach = m.body * THINK_INTERVAL as f32;
    let ahead = air.trace(origin, origin + reach);
    let met = if ahead.trace.fraction < 1. || ahead.trace.start_solid {
        Some(Bumped {
            at: origin + reach * ahead.trace.fraction,
            normal: ahead.trace.normal,
            entity: ahead.entity,
            start_solid: ahead.trace.start_solid,
            sparks: None,
        })
    } else {
        pressed.filter(|c| m.hover.velocity.dot(c.normal) < 0.)
    };
    bump(m, air, met?, origin, now).map(Cut::Smash)
}

/// Bump: unless bumps are suppressed, what it wants turns off the surface, its blades slow and
/// its burst ends; on what stands across the blades' plane, a wall and not a floor, they grind
/// and spark; thrown moments ago, it smashes.
fn bump(m: &mut Manhack, air: &Air, mut met: Bumped, origin: Vec3, now: f64) -> Option<f32> {
    if m.bump_quiet(now) {
        return None;
    }
    let speed = m.body.length();
    let before = m.hover.velocity;
    let floor = air.trace(origin, origin - Vec3::Z * body::FLOOR_NEAR);
    m.deflect_off(&met, floor.trace.fraction < 1., now);
    m.jolt(
        (m.hover.velocity - before) * manhack::PHYSICS_MASS,
        manhack::BLADE_HIT,
    );
    m.stop_burst(now);
    m.bumps += 1;
    let up = m.attitude.rotation() * Vec3::Z;
    if up.dot(met.normal).abs() < GRIND_ACROSS {
        met.sparks = Some(body::spark_normal(met.normal, before));
        if !m.gagged() {
            m.requests.push(CopRequest::Sound(manhack::GRIND));
        }
    }
    m.bumped.push(met);
    m.smash(speed, now)
}

/// CheckCollisions held or stalled: a hull trace along a think's travel, or along the aim when
/// its carrier is close, meets NPCs and props as well as the world. The world is bumped at once;
/// an entity is judged by the stage.
fn blade_contact(
    m: &mut Manhack,
    air: &Air,
    origin: Vec3,
    player: &PlayerState,
    now: f64,
) -> Option<Cut> {
    let mut reach = m.body * THINK_INTERVAL as f32;
    if m.held && near_carrier(player, origin) {
        reach = player.facing.normalize_or_zero() * CARRIER_REACH;
    }
    let query = Query {
        excluded_entities: &[air.id],
        ..Default::default()
    };
    let hit = air
        .physics
        .npc_trace_hull(origin, origin + reach, air.hull, query);
    if hit.trace.fraction >= 1. && !hit.trace.start_solid {
        return None;
    }
    let met = Bumped {
        at: origin + reach * hit.trace.fraction,
        normal: hit.trace.normal,
        entity: hit.entity,
        start_solid: hit.trace.start_solid,
        sparks: None,
    };
    match hit.entity {
        Some(entity) => Some(Cut::Touch(entity, met)),
        None => bump(m, air, met, origin, now).map(Cut::Smash),
    }
}

/// The carrier's box (CalcDistanceFromPoint) is within CARRIER_NEAR of its middle.
fn near_carrier(player: &PlayerState, origin: Vec3) -> bool {
    let height = if player.crouched { 36. } else { 72. };
    let low = player.feet - Vec3::new(16., 16., 0.);
    let high = player.feet + Vec3::new(16., 16., height);
    origin.distance(origin.clamp(low, high)) < CARRIER_NEAR
}

/// What CheckCollisions may cut, by the class table: ai_relationship and F4 sides are not read.
struct Blades<'a> {
    world: &'a World,
    relationships: &'a Relationships,
    precriminal: bool,
}

/// What a touch comes to.
enum Verdict {
    Slice(manhack::Victim),
    /// Held, it spares the player's friends.
    Spare,
    Bump,
}

impl Blades<'_> {
    fn class(&self, id: usize) -> Class {
        let name = self.world.entities.get(id).map_or("", |e| e.class());
        classify(name, self.precriminal)
    }
    /// Slice's and CheckCollisions' rules: what takes damage and is no manhack is cut; held, an
    /// NPC that likes the player is spared.
    fn judge(&self, m: &Manhack, scene: &Scene, victim: usize) -> Verdict {
        let name = self.world.entities.get(victim).map_or("", |e| e.class());
        if name.starts_with("npc_") {
            let alive = scene
                .states
                .get(victim)
                .is_some_and(|s| !s.killed && !s.corpse);
            if !alive || name == "npc_manhack" {
                return Verdict::Bump;
            }
            let likes = self.relationships.get(self.class(victim), Class::Player).0;
            if m.held && likes == Disposition::LI {
                return Verdict::Spare;
            }
            return Verdict::Slice(self.npc_victim(m, victim));
        }
        match crate::gameplay::breakable_health(self.world, scene, victim) {
            Some(health) => Verdict::Slice(manhack::Victim::Breakable(health)),
            None => Verdict::Bump,
        }
    }
    /// An NPC as Slice sees it: an npc_combine_s it hates takes six times as much. Held, it is the
    /// player's ally (Classify).
    fn npc_victim(&self, m: &Manhack, victim: usize) -> manhack::Victim {
        let mine = if m.held {
            Class::PlayerAlly
        } else {
            Class::Manhack
        };
        let hates = self.relationships.get(mine, self.class(victim)).0 == Disposition::HT;
        let soldier = self
            .world
            .entities
            .get(victim)
            .is_some_and(|e| e.class() == "npc_combine_s");
        manhack::Victim::Npc {
            hated_soldier: hates && soldier,
        }
    }
    /// What does not bleed (BloodColor DONT_BLEED) grinds: robots and props.
    fn grinds(&self, victim: usize) -> bool {
        let name = self.world.entities.get(victim).map_or("", |e| e.class());
        !name.starts_with("npc_") || crate::gameplay::blood_color(name).is_none()
    }
}

/// A think's cut, reckoned while its manhack is at hand: Slice's damage and the bounce away from
/// what it cut (ComputeSliceBounceVelocity), a spared friend, or a bump.
fn reckon(
    m: &mut Manhack,
    cut: Cut,
    (blades, scene, air): (&Blades, &Scene, &Air),
    (origin, rate, player): (Vec3, (f32, f32), &PlayerState),
) -> Option<Paid> {
    let now = scene.time;
    let guard = scene.skill.get("sk_combine_guard_health", 0.);
    let (victim, at, kind) = match cut {
        Cut::Smash(share) => return Some(Paid::Smash(share)),
        Cut::Player => {
            m.rebound(origin - (player.feet + Vec3::Z * 36.), now);
            let damage = m.slice_damage(rate, manhack::Victim::Player, guard, now);
            return Some(Paid::Player(damage));
        }
        Cut::Npc(victim, middle) => (victim, middle, blades.npc_victim(m, victim)),
        Cut::Touch(victim, met) => match blades.judge(m, scene, victim) {
            Verdict::Slice(kind) => (victim, met.at, kind),
            Verdict::Spare => return None,
            Verdict::Bump => return bump(m, air, met, origin, now).map(Paid::Smash),
        },
    };
    let damage = m.slice_damage(rate, kind, guard, now);
    let by_player = m.physics_attacker(now);
    let middle = scene.states.get(victim).map_or(at, |s| s.origin);
    m.rebound(origin - middle, now);
    Some(Paid::Slice {
        victim,
        at,
        damage,
        by_player,
        npc: matches!(kind, manhack::Victim::Npc { .. }),
        grinds: blades.grinds(victim),
    })
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
    // SpinBlades: the blade shows while the blades are slow, the blur once they are fast.
    if !m.blade_shut {
        m.blade_out = true;
        m.attitude.think(interval, m.stalled(now));
        if let Some((blade, blur)) = m.attitude.blade_groups() {
            scene.set_bodygroup(air.world, id, BLADE_GROUP, blade as i32);
            scene.set_bodygroup(air.world, id, BLUR_GROUP, blur as i32);
        }
    }
    // A burst holds its line at the enemy; otherwise a swarm's move leads.
    let swarming = m.swarm.flying_at(now).filter(|_| !m.bursting(now));
    m.target = swarming.or(enemy.map(|e| e.eyes));
    check_swarm_line(m, air, origin);
    m.update_panels(now);
    if scene.manhack_panels.get(&id) != Some(&m.panels) {
        scene.manhack_panels.insert(id, m.panels);
    }
    // Held, the grab steers it; stalled, MoveToTarget returns and it flies on as it was thrown.
    // CheckCollisions sees everything its hull meets then.
    if m.held {
        return blade_contact(m, air, origin, &player, now);
    }
    keep_off_ground(m, air, origin);
    if m.stalled(now) {
        return blade_contact(m, air, origin, &player, now);
    }
    m.fly(origin, interval, now);
    m.bank(origin, now);
    // TurnHeadToTarget: the yaw its motor turns it to.
    if let Some(look) = enemy.map(|e| e.eyes).or(m.target) {
        m.attitude.head_yaw = angles_toward(look - origin).1;
    }
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

/// MaintainGroundHeight, every think but packed: under 52 units over the ground and not rising
/// faster than 32 u/s, it is pushed up, the harder the nearer. The SDK's trace sees brushes only;
/// this one sees what flight sees.
fn keep_off_ground(m: &mut Manhack, air: &Air, origin: Vec3) {
    let rise = m.hover.velocity.z;
    if rise > GROUND_RISE {
        return;
    }
    let down = air.trace(origin, origin - Vec3::Z * GROUND_HEIGHT);
    if down.trace.fraction < 1. {
        m.hover.velocity.z += (-rise * 0.5).max(16.) * (1. - down.trace.fraction);
    }
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
    break_body(air, scene, m.gib_motion());
}

/// Where a club knocks it: the light props ahead are the candidates. NPCs' physics shadows, which
/// the SDK's box also finds, are not searched.
fn club_target(physics: &Physics, origin: Vec3, along: Vec3) -> Vec3 {
    use crate::grab::{body_mass, center};
    let props = physics.dynamic.keys();
    let candidates = props.filter_map(|&id| Some((center(physics, id)?, body_mass(physics, id)?)));
    manhack::club_aim(origin, along, candidates)
}

/// A think's puffs of its smoke trail, drifting with it.
fn smoke(scene: &mut Scene, id: usize, velocity: Vec3) {
    use crate::projectiles::EffectKind;
    let origin = scene.states[id].origin;
    for _ in 0..PUFFS_PER_THINK {
        let puff = scanner_effect(EffectKind::ManhackSmoke, origin, velocity, scene.time);
        scene.impact_effects.push(puff);
    }
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
    /// The living manhacks' eye and light, placed for this frame, and whether they are stunned.
    /// A stunned eye strobes, so half the time it is not drawn.
    pub fn manhack_glows<'a>(
        &'a self,
        world: &'a World,
        scene: &'a Scene,
    ) -> impl Iterator<Item = (Vec3, bool)> + 'a {
        let alive = |id: usize| {
            let state = scene.states.get(id);
            state.is_some_and(|s| s.visible && !s.killed && !s.corpse)
        };
        self.actors
            .iter()
            .filter_map(move |(&id, actor)| match &actor.class {
                AiClass::Manhack(m) if alive(id) => Some((id, m.stunned)),
                _ => None,
            })
            .filter(move |&(id, stunned)| !stunned || manhack::strobe_lit(scene.time, id))
            .flat_map(move |(id, stunned)| {
                let frames = scene.attachment_frames(world, id, &manhack::GLOW_ATTACHMENTS);
                frames
                    .into_iter()
                    .flatten()
                    .map(move |frame| (frame.w_axis.truncate(), stunned))
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
        events: &[(usize, NpcPhysgun)],
    ) {
        let now = scene.time;
        let per_second = scene.skill.get("sk_manhack_melee_dmg", CUT_DAMAGE);
        let (held, squads) = (&self.held_npcs, &mut self.squads);
        let marks = &mut self.manhack_marks;
        let blades = Blades {
            world,
            relationships: &self.relationships,
            precriminal: scene.globals.is_on("gordon_precriminal"),
        };
        let rate = (per_second, THINK_INTERVAL as f32);
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
            // Its VPhysics object: the gravity gun may take it (a carried one has none yet).
            if !m.carried && scene.states[id].visible {
                scene.physgun_npcs.push(manhack::listing(id));
            }
            // A punt or a launch has it think at once, as the SDK's next think 0.01 s on.
            for &(_, event) in events.iter().filter(|(target, _)| *target == id) {
                if m.physgun(event, now) {
                    m.next_steer = now;
                }
            }
            if let Some((along, base)) = m.batted.take() {
                m.bat(club_target(physics, scene.states[id].origin, along), base);
            }
            if now >= m.next_steer {
                m.next_steer = now + THINK_INTERVAL;
                claim_slot(m, squads, squad, (enemy_id, now));
                if m.smoking {
                    smoke(scene, id, m.body);
                }
                if let Some(cut) = steer(m, &air, scene, enemy.as_ref(), player) {
                    let origin = scene.states[id].origin;
                    let ctx = (origin, rate, &player);
                    if let Some(paid) = reckon(m, cut, (&blades, scene, &air), ctx) {
                        cuts.push((id, paid, origin));
                    }
                }
                contact::show(m, &air, scene, marks);
            }
            let dt = contact::carry(m, &air, scene, marks);
            m.attitude.start(super::super::host::yaw_of(scene, id));
            match m.held_yaw {
                Some(yaw) => m.attitude.hold(yaw, dt),
                None => m.attitude.step(dt),
            }
            scene.states[id].rotation = m.attitude.rotation();
        }
        for (id, ask) in asks {
            self.answer_swarm(scene, id, ask);
        }
        for (id, cut, origin) in cuts {
            self.pay_cut(scene, id, cut, origin);
        }
    }
    /// What a cut does: the damage and its sound.
    fn pay_cut(&mut self, scene: &mut Scene, id: usize, cut: Paid, origin: Vec3) {
        if let Paid::Smash(share) = cut {
            // TakeDamageFromPhyscannon: DMG_GENERIC from the player who threw it.
            let health = scene.skill.get("sk_manhack_health", manhack::MAX_HEALTH);
            scene.entity_damage.push(crate::projectiles::Damage {
                target: crate::projectiles::DamageTarget::Entity(id),
                amount: health * share,
                kind: 0,
                dissolve: false,
                direction: Vec3::ZERO,
                origin,
                attacker: crate::projectiles::Attacker::Player,
            });
            return;
        }
        let grinds = matches!(cut, Paid::Slice { grinds: true, .. });
        let cue = if grinds {
            manhack::GRIND
        } else {
            manhack::SLICE
        };
        scene.sounds.push(SoundRequest {
            origin: Some(origin),
            ..cue.into()
        });
        match cut {
            Paid::Player(damage) => self.player_hits.push(PlayerHit {
                attacker: id,
                damage,
                punch: Vec3::ZERO,
                impulse: Vec3::ZERO,
                fade: None,
            }),
            // An NPC cut by a free manhack is its melee hit; the player's cuts and breakables
            // take DMG_SLASH through the entity path, where the player's friends refuse it.
            Paid::Slice {
                victim,
                at,
                damage,
                by_player: false,
                npc: true,
                ..
            } => self.melee_hits.push(MeleeHit {
                attacker: id,
                victim,
                damage,
                position: at,
                direction: (at - origin).normalize_or_zero(),
            }),
            Paid::Slice {
                victim,
                at,
                damage,
                by_player,
                ..
            } => scene.entity_damage.push(crate::projectiles::Damage {
                target: crate::projectiles::DamageTarget::Entity(victim),
                amount: damage,
                kind: crate::player_damage::DMG_SLASH,
                dissolve: false,
                direction: (at - origin).normalize_or_zero(),
                origin: at,
                attacker: if by_player {
                    crate::projectiles::Attacker::Player
                } else {
                    crate::projectiles::Attacker::Entity(id)
                },
            }),
            Paid::Smash(_) => {}
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
    manhack.carried = flags & manhack::SF_CARRIED != 0;
    manhack.no_damage_effects = flags & manhack::SF_NO_DAMAGE_EFFECTS != 0;
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
    fn with_its_blades_out_every_idle_is_its_flight_and_packed_it_idles_shut() {
        let mut class = class_for(0, 1);
        assert!(class.activity_rules().flies);
        let AiClass::Manhack(m) = &mut class else {
            panic!("a manhack");
        };
        m.pack(false);
        assert!(!class.activity_rules().flies);
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
