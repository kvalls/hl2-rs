//! Scanners in the world (DESIGN 14o session 46): `ai::scanner` decides; this stage steers once per
//! think and carries the body every tick, the part a physics motor does in the SDK. Bodies go
//! through NPCs and the player here, as birds do: a scanner bumping into them is not modeled.

use super::super::conditions::{
    clear_range_complaints, Conditions, COND_HEAVY_DAMAGE, COND_LIGHT_DAMAGE, COND_SEE_ENEMY,
};
use super::super::host::PhysicsSight;
use super::super::host::{clamp_yaw, yaw_of};
use super::super::metropolice::{CopRequest, PLAYER};
use super::super::relationships::{classify, Class};
use super::super::scanner::damage;
use super::super::scanner::flash::{self, EyeFlare};
use super::super::scanner::inspect::{self, Candidate};
use super::super::scanner::spotlight::{self, Beam, Spotlight};
use super::super::scanner::{self, Mode, Scanner, COND_FLY_BLOCKED, COND_FLY_CLEAR};
use super::super::senses::SightWorld;
use super::super::state::NpcState;
use super::birds::{self, Air};
use super::crabs::{fall_hull, fall_player_box, step_fraction};
use super::{overlaps, AiActor, AiClass, AiRuntime, PlayerHit, PlayerState, THINK_INTERVAL};
use crate::entities::Scene;
use crate::gibs::PropBreak;
use crate::physics::{angles, Physics};
use crate::player_damage::ScreenFade;
use crate::projectiles::Attacker;
use crate::projectiles::EffectKind;
use crate::sounds::{entity_emitter, PatchOp, SoundRequest};
use crate::view_control::angles_toward;
use glam::Vec3;
use modkit_core::World;

/// SF_CSCANNER_STRIDER_SCOUT.
const STRIDER_SCOUT: u32 = 1 << 17;
/// sk_scanner_dmg_dive where skill.cfg leaves it out.
const DIVE_DAMAGE: f32 = 25.;
/// Stopped this close to what it flies at, the way still counts as clear.
const CLEAR_WITHIN: f32 = 50.;
/// The lift of being low: this much speed over the share of the clearance left, which stops
/// counting below `LOWEST_SHARE`.
const GROUND_LIFT: f32 = 50.;
const LOWEST_SHARE: f32 = 0.1;
/// How fast the lean eases, degrees a second. Not from the SDK, whose motor reaches it in a moment.
const LEAN_RATE: f32 = 120.;

/// Whether something stops a straight flight well short of `to`.
fn blocked(air: &Air, from: Vec3, to: Vec3) -> bool {
    let hit = air.trace(from, to);
    (1. - step_fraction(&hit)) * from.distance(to) >= CLEAR_WITHIN
}

/// `point`, raised as far as the ground under it is too near.
fn above_ground(air: &Air, point: Vec3) -> Vec3 {
    let clearance = Vec3::Z * scanner::GROUND_CLEARANCE;
    let hit = air.trace(point, point - clearance);
    point + clearance * (1. - step_fraction(&hit))
}

/// VelocityToAvoidObstacles: it bounces off what it is about to run into, else rises off the
/// ground under it.
fn avoid(s: &mut Scanner, air: &Air, origin: Vec3, interval: f32) {
    let ahead = air.trace(origin, origin + s.hover.velocity * interval);
    if ahead.trace.fraction < 1. {
        s.hover.bounce(ahead.trace.normal);
        return;
    }
    let below = air.trace(origin, origin - Vec3::Z * scanner::GROUND_CLEARANCE);
    if below.trace.fraction < 1. {
        let share = below.trace.fraction.max(LOWEST_SHARE);
        s.hover.velocity.z += GROUND_LIFT / share;
    }
}

/// The corner after `corner`, when its `target` names one.
fn next_corner(world: &World, corner: usize) -> Option<usize> {
    let name = world.entities.get(corner)?.get("target")?;
    let next = birds::named(world, name)?;
    (world.entities[next].class() == "path_corner").then_some(next)
}

/// The start of a placed scanner's path, read once it lives.
fn start_path(s: &mut Scanner, world: &World, id: usize) {
    if !std::mem::replace(&mut s.path_read, true) {
        s.corner = next_corner(world, id);
        s.route.extend(s.corner.map(|c| world.entities[c].origin()));
    }
}

/// A corner reached: its OnPass fires and the next one comes, until the chain ends.
fn pass_corner(s: &mut Scanner, world: &World, scene: &mut Scene, id: usize) {
    let Some(corner) = s.corner.filter(|_| s.route.is_empty()) else {
        return;
    };
    scene.fire(corner, "OnPass", id);
    s.corner = next_corner(world, corner);
    s.route.extend(s.corner.map(|c| world.entities[c].origin()));
}

/// Whether it can see what it is placed to photograph.
fn can_photograph(s: &Scanner, air: &Air, scene: &Scene) -> bool {
    let (Some(target), Some(subject)) = (s.inspect_at, s.inspect) else {
        return false;
    };
    let origin = scene.states[air.id].origin;
    let aim = angles_toward(target - origin).1;
    let sight = PhysicsSight {
        physics: air.physics,
        world: air.world,
    };
    s.photo_placed(origin, target, yaw_of(scene, air.id), aim)
        && sight.visible(origin, target, &[air.id, subject])
}

/// Who a scanner could inspect: the player, and citizens who are still passive.
fn candidates(world: &World, scene: &Scene, ids: &[usize], player: PlayerState) -> Vec<Candidate> {
    let precriminal = scene.globals.is_on("gordon_precriminal");
    let mut found = Vec::new();
    if !player.dead && !player.noclip {
        found.push(Candidate {
            subject: PLAYER,
            feet: player.feet,
            eyes: player.eye(),
        });
    }
    let citizens = ids.iter().copied().filter(|&id| {
        let class = world.entities[id].class();
        let alive = !scene.states[id].killed && !scene.states[id].corpse;
        alive && classify(class, precriminal) == Class::CitizenPassive
    });
    found.extend(citizens.map(|id| {
        let feet = scene.states[id].origin;
        Candidate {
            subject: id,
            feet,
            eyes: feet + Vec3::Z * crate::ai_hull::eye_height(world.entities[id].class()),
        }
    }));
    found
}

/// Looks for someone to inspect, as GatherConditions does when it has nothing to do.
fn search(s: &mut Scanner, air: &Air, scene: &Scene, list: &[Candidate]) {
    let (id, now) = (air.id, scene.time);
    let origin = scene.states[id].origin;
    let sight = PhysicsSight {
        physics: air.physics,
        world: air.world,
    };
    let wanted = |c: &&Candidate| {
        if c.subject == PLAYER {
            !s.never_players
        } else {
            !s.only_players
        }
    };
    let list: Vec<_> = list.iter().filter(wanted).copied().collect();
    let reach = inspect::search_from(origin, s.spotlight.as_ref().map(|light| light.end));
    let sees = |c: &Candidate| sight.visible(origin, c.eyes, &[id, c.subject]);
    if let Some(subject) = inspect::nearest(&list, origin, reach, sees) {
        s.found(subject, now);
    }
}

/// One think of flight: where it wants to be, the thrust that takes it there, and its sounds.
fn steer(s: &mut Scanner, air: &Air, scene: &mut Scene, state: NpcState) {
    let origin = scene.states[air.id].origin;
    let interval = THINK_INTERVAL as f32;
    let after = s.inspect_at.or(s.enemy_at);
    s.fly_blocked = after.map(|at| blocked(air, origin, at));
    s.can_photograph = can_photograph(s, air, scene);
    let hover = |s: &Scanner| s.hover_point(origin).map(|p| above_ground(air, p));
    let mut wanted = s.next_leg(origin);
    if wanted.is_none() {
        pass_corner(s, air.world, scene, air.id);
        wanted = s.next_leg(origin).or_else(|| hover(s));
    }
    match wanted {
        Some(point) => s.hover.fly_scanner(point - origin, interval),
        None => s.hover.coast(interval),
    }
    avoid(s, air, origin, interval);
    s.hover.add_noise(s.bob, scanner::NOISE, scene.time as f32);
    s.hover.limit(scanner::MAX_SPEED);
    if let Some(look) = s.enemy_eye.or(s.inspect_at).or(wanted) {
        s.look_yaw = Some(angles_toward(look - origin).1);
    }
    let right = scene.states[air.id].rotation * Vec3::NEG_Y;
    let speed = s.hover.velocity.length();
    s.lean = wanted.map_or(0., |point| scanner::lean(right, point - origin, speed));
    let body = scene.states[air.id].rotation * Vec3::X;
    let eyes = origin + Vec3::Z * scanner::EYE_HEIGHT;
    let alert = matches!(state, NpcState::Alert | NpcState::Combat);
    s.think_pose(body, eyes, alert, scene.time);
    spotlight_think(s, air, scene);
    trail_smoke(s, scene, air.id);
    s.engine_sound();
    if let Some(cue) = s.idle_cue(state == NpcState::Combat) {
        s.requests.push(CopRequest::Sound(cue));
    }
}

/// How far under a point the floor is looked for.
const FLOOR_SEARCH: f32 = 2048.;

/// The point under `point` on the floor, or the point itself over nothing.
fn on_floor(sight: &PhysicsSight, id: usize, point: Vec3) -> Vec3 {
    let below = point - Vec3::Z * FLOOR_SEARCH;
    sight.first_solid(point, below, &[id]).unwrap_or(point)
}

/// What the beam looks for: a player's eyes when seen, else the floor at its enemy, what it
/// inspects, or a sweep ahead of its flight.
fn spotlight_target(s: &Scanner, sight: &PhysicsSight, id: usize, origin: Vec3, now: f32) -> Vec3 {
    if s.sees_enemy && s.enemy_is_player {
        if let Some(eyes) = s.enemy_eye {
            return eyes;
        }
    }
    if let Some(at) = s.enemy_at {
        return on_floor(sight, id, at);
    }
    let sweep = || spotlight::sweep_target(origin, s.hover.velocity, s.bob, now);
    s.inspect_at.unwrap_or_else(sweep)
}

/// The way a new beam starts: at the floor by what it is after, else straight down.
fn first_direction(s: &Scanner, sight: &PhysicsSight, id: usize, origin: Vec3) -> Vec3 {
    let after = s.enemy_at.or(s.inspect_at);
    after.map_or(Vec3::NEG_Z, |at| {
        (on_floor(sight, id, at) - origin).normalize_or(Vec3::NEG_Z)
    })
}

/// Where a beam from `origin` lands; NPCs do not stop it.
fn landing(sight: &PhysicsSight, id: usize, origin: Vec3, direction: Vec3, reach: f32) -> Vec3 {
    let end = origin + direction * reach;
    sight.first_solid(origin, end, &[id]).unwrap_or(end)
}

/// One think of the spotlight: lit when it may be, swung at what it wants and put out when that
/// swings it behind the body. A mode without a light puts it out at once.
fn spotlight_think(s: &mut Scanner, air: &Air, scene: &Scene) {
    let (id, now) = (air.id, scene.time);
    if s.spotlight_disabled || !matches!(s.mode, Mode::Spot | Mode::Patrol) {
        s.spotlight = None;
        return;
    }
    let sight = PhysicsSight {
        physics: air.physics,
        world: air.world,
    };
    let origin = scene.states[id].origin;
    if s.spotlight.is_none() {
        if now < s.next_spotlight {
            return;
        }
        let direction = first_direction(s, &sight, id, origin);
        let end = landing(&sight, id, origin, direction, spotlight::FIRST_TRACE);
        s.spotlight = Some(Spotlight::new(direction, end, s.spotlight_reach));
    }
    let target = spotlight_target(s, &sight, id, origin, now as f32);
    let Some(light) = s.spotlight.as_mut() else {
        return;
    };
    light.swing(origin, target);
    let lands = landing(&sight, id, origin, light.direction, light.trace_reach());
    let forward = scene.states[id].rotation * Vec3::X;
    if forward.dot(lands - origin) < 0. {
        s.spotlight = None;
        s.next_spotlight = now + spotlight::OFF_WHEN_BEHIND;
        return;
    }
    light.land(origin, lands);
    let frame = scene.attachment_frames(air.world, id, &[flash::EYE_ATTACHMENT]);
    if let Some(frame) = frame.first().copied().flatten() {
        let from_body = frame.w_axis.truncate() - origin;
        s.light_offset = scene.states[id].rotation.inverse() * from_body;
    }
}

/// The body turns toward what it looks at and eases into its lean.
fn turn(s: &mut Scanner, scene: &mut Scene, id: usize, dt: f32) {
    let current = yaw_of(scene, id);
    let yaw = clamp_yaw(s.turn_rate(), current, s.look_yaw.unwrap_or(current), dt);
    s.roll = clamp_yaw(LEAN_RATE, s.roll, s.lean, dt);
    scene.states[id].rotation = angles(Vec3::new(0., yaw, s.roll));
}

/// A killed scanner's engine stops.
fn stop_engine(s: &mut Scanner, scene: &mut Scene, id: usize) {
    if std::mem::take(&mut s.engine_on) {
        let emitter = entity_emitter(id);
        let cue = Scanner::engine_cue();
        scene.sounds.push(SoundRequest {
            origin: Some(scene.states[id].origin),
            ..SoundRequest::patch(cue, emitter, PatchOp::Destroy)
        });
    }
}

/// One think's puff of its smoke trail, drifting with it.
fn trail_smoke(s: &Scanner, scene: &mut Scene, id: usize) {
    if s.smoking {
        let (origin, velocity) = (scene.states[id].origin, s.hover.velocity);
        let puff = damage::effect(EffectKind::TrailSmoke, origin, velocity, scene.time);
        scene.impact_effects.push(puff);
    }
}

/// Its end: the engine stops, sparks and a blast that hurts nothing cover it and its gibs fly. The
/// light and the chance of a battery are not modeled yet.
fn gib(s: &mut Scanner, air: &Air, scene: &mut Scene) {
    let id = air.id;
    stop_engine(s, scene, id);
    let origin = scene.states[id].origin;
    for _ in 0..damage::GIB_SPARKS {
        let reach = Vec3::new(s.roll(), s.roll(), s.roll());
        let at = origin + (reach * 2. - Vec3::ONE) * damage::GIB_SPARK_REACH;
        let spark = damage::effect(EffectKind::Sparks, at, Vec3::ZERO, scene.time);
        scene.impact_effects.push(spark);
    }
    scene.sounds.push(SoundRequest {
        origin: Some(origin),
        ..scanner::DIE.into()
    });
    let blast = (origin, 1., 2., false, Attacker::Entity(id));
    scene.prop_explosions.push(blast);
    break_body(air, scene, s.hover.velocity);
    scene.scanner_poses.remove(&id);
}

/// The end of a flying body: its pieces take its speed and it is gone.
pub(super) fn break_body(air: &Air, scene: &mut Scene, velocity: Vec3) {
    let id = air.id;
    let (origin, rotation) = (scene.states[id].origin, scene.states[id].rotation);
    let instance = air.world.model_instances.iter();
    if let Some(instance) = instance.into_iter().find(|i| i.entity == Some(id)) {
        scene.prop_breaks.push(PropBreak {
            entity: id,
            model_key: instance.asset_key(),
            origin,
            rotation,
            velocity,
            angular_velocity: Vec3::ZERO,
            center_of_mass: origin,
            skin: instance.skin as i32,
        });
    }
    scene.states[id].killed = true;
    scene.states[id].visible = false;
}

/// Whether its body touches the player's.
pub(super) fn strikes_player(air: &Air, origin: Vec3, player: PlayerState) -> bool {
    fall_player_box(player).is_some_and(|actor| {
        let (lo, hi) = (origin + air.hull.mins, origin + air.hull.maxs);
        overlaps(
            lo,
            hi,
            actor.feet + actor.hull.mins,
            actor.feet + actor.hull.maxs,
        )
    })
}

/// A flash that reached the player, who looks at it with nothing in between: what it leaves on
/// their view. Only what stops light stops it, so NPCs in the way do not.
fn flash_on_player(
    s: &mut Scanner,
    air: &Air,
    scene: &Scene,
    player: PlayerState,
) -> Option<ScreenFade> {
    s.blinds.take().filter(|&subject| subject == PLAYER)?;
    let origin = scene.states[air.id].origin;
    let fade = flash::blinding(origin, player.eye(), player.facing)?;
    let sight = PhysicsSight {
        physics: air.physics,
        world: air.world,
    };
    sight
        .visible(origin, player.eye(), &[air.id])
        .then_some(fade)
}

/// A dead scanner's tick: its dive, ended by the first thing it meets, or its end at once. True
/// when it struck the player. NPCs in its way are not struck yet: flights pass through them.
fn die(s: &mut Scanner, air: &Air, scene: &mut Scene, player: PlayerState) -> bool {
    let id = air.id;
    let origin = scene.states[id].origin;
    if !std::mem::replace(&mut s.death_seen, true) && s.start_dive(origin) {
        scene.sounds.push(SoundRequest {
            origin: Some(origin),
            ..scanner::DIVE_BOMB.into()
        });
    }
    let Some(dive) = s.dive else {
        gib(s, air, scene);
        return false;
    };
    let now = scene.time;
    if now >= s.next_steer {
        s.next_steer = now + THINK_INTERVAL;
        s.hover.fly_dive(dive.direction, THINK_INTERVAL as f32);
        s.hover
            .add_noise(s.bob, scanner::NOISE * scanner::DIVE_NOISE, now as f32);
        s.hover.limit(scanner::DIVE_SPEED);
        s.dive_pose(now);
        s.smoking = true;
        trail_smoke(s, scene, id);
    }
    let dt = (now - s.last_step).clamp(0., 0.1) as f32;
    let hit = air.trace(origin, origin + s.hover.velocity * dt);
    let struck = strikes_player(air, origin, player);
    if hit.trace.fraction < 1. || struck {
        gib(s, air, scene);
        return struck;
    }
    let dt = air.step(&mut s.hover.velocity, &mut s.last_step, scene);
    scene
        .scanner_poses
        .insert(id, s.pose_now(now, THINK_INTERVAL));
    s.roll += dive.spin * dt;
    scene.states[id].rotation = angles(Vec3::new(0., yaw_of(scene, id), s.roll));
    false
}

impl AiRuntime {
    /// This tick's hovering: a steer once per think and each scanner's step.
    pub(super) fn fly_scanners(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        player: PlayerState,
    ) {
        if !self.enabled {
            return;
        }
        let now = scene.time;
        let held = &self.held_npcs;
        let actors: Vec<usize> = self.actors.keys().copied().collect();
        let mut people: Option<Vec<Candidate>> = None;
        let mut strikes = Vec::new();
        let mut flashes = Vec::new();
        for (&id, actor) in self.actors.iter_mut() {
            let state = actor.npc.state;
            let AiClass::Scanner(s) = &mut actor.class else {
                continue;
            };
            actor.fall = None;
            if scene.states[id].killed {
                stop_engine(s, scene, id);
                continue;
            }
            if held.contains(&id) {
                continue;
            }
            let air = Air {
                world,
                physics,
                hull: fall_hull(world, id),
                id,
            };
            if scene.states[id].corpse {
                s.eye = None;
                s.spotlight = None;
                if die(s, &air, scene, player) {
                    strikes.push(id);
                }
                continue;
            }
            start_path(s, world, id);
            if now >= s.next_steer {
                s.next_steer = now + THINK_INTERVAL;
                steer(s, &air, scene, state);
                if s.search_due(now) {
                    let list =
                        people.get_or_insert_with(|| candidates(world, scene, &actors, player));
                    search(s, &air, scene, list);
                }
            }
            if let Some(fade) = flash_on_player(s, &air, scene, player) {
                s.blinded += 1;
                flashes.push((id, fade));
            }
            let dt = air.step(&mut s.hover.velocity, &mut s.last_step, scene);
            turn(s, scene, id, dt);
            if let Some(light) = s.spotlight.as_mut() {
                light.carry(dt);
            }
            scene
                .scanner_poses
                .insert(id, s.pose_now(now, THINK_INTERVAL));
        }
        let damage = scene.skill.get("sk_scanner_dmg_dive", DIVE_DAMAGE);
        let hit = |attacker, damage, fade| PlayerHit {
            attacker,
            damage,
            punch: Vec3::ZERO,
            impulse: Vec3::ZERO,
            fade,
        };
        let struck = strikes.into_iter().map(|id| hit(id, damage, None));
        self.player_hits.extend(struck);
        let blinded = flashes
            .into_iter()
            .map(|(id, fade)| hit(id, 0., Some(fade)));
        self.player_hits.extend(blinded);
    }
    /// The lights of the flashes since the last call.
    pub fn take_scanner_lights(&mut self) -> Vec<Vec3> {
        let lights = self
            .actors
            .values_mut()
            .filter_map(|actor| match &mut actor.class {
                AiClass::Scanner(s) => s.light_at.take(),
                _ => None,
            });
        lights.collect()
    }
    /// The lit spotlights, from where their lights are this frame.
    pub fn scanner_beams<'a>(&'a self, scene: &'a Scene) -> impl Iterator<Item = Beam> + 'a {
        self.actors.iter().filter_map(|(&id, actor)| {
            let AiClass::Scanner(s) = &actor.class else {
                return None;
            };
            let state = &scene.states[id];
            let start = state.origin + state.rotation * s.light_offset;
            Some(s.spotlight.as_ref()?.beam(id, start))
        })
    }
    /// The scanners with a lit eye, placed for this frame.
    pub fn scanner_eyes<'a>(
        &'a self,
        world: &'a World,
        scene: &'a Scene,
    ) -> impl Iterator<Item = EyeFlare> + 'a {
        self.actors.iter().filter_map(|(&id, actor)| {
            let AiClass::Scanner(s) = &actor.class else {
                return None;
            };
            let eye = s.eye?;
            let frame = scene.attachment_frames(world, id, &[flash::EYE_ATTACHMENT]);
            let at = frame.first().copied().flatten()?.w_axis.truncate();
            let forward = scene.states[id].rotation * Vec3::X;
            Some(EyeFlare {
                entity: id,
                at,
                forward,
                eye,
            })
        })
    }
}

/// The eyes of what a scanner is after: an entity with no eyes has its origin.
fn eyes(world: &World, scene: &Scene, player: &PlayerState, subject: usize) -> Option<Vec3> {
    if subject == PLAYER {
        return Some(player.eye());
    }
    let class = world.entities.get(subject)?.class();
    let height = if class.starts_with("npc_") {
        crate::ai_hull::eye_height(class)
    } else {
        0.
    };
    Some(scene.states.get(subject)?.origin + Vec3::Z * height)
}

/// GetGoalDirection: an info_target or an air hint says which side to hover on.
fn marked_side(world: &World, scene: &Scene, subject: usize) -> Option<Vec3> {
    let class = world.entities.get(subject)?.class();
    matches!(class, "info_target" | "info_hint_air")
        .then(|| scene.states[subject].rotation * Vec3::X)
}

/// The attack conditions of a scanner that sees its enemy within range of being looked at.
fn attack_conditions(
    conditions: &mut Conditions,
    scene: &Scene,
    (id, enemy_at): (usize, Vec3),
    next_attack: f64,
) {
    let state = &scene.states[id];
    let to_enemy = enemy_at - state.origin;
    let checked =
        conditions.has(COND_SEE_ENEMY) && to_enemy.length() < flash::ATTACKS_CHECKED_WITHIN;
    if checked && scene.time >= next_attack {
        conditions.set(flash::attack_condition(to_enemy, state.rotation * Vec3::X));
        clear_range_complaints(conditions);
    }
}

/// Where its enemy stands.
fn enemy_feet(scene: &Scene, player: &PlayerState, enemy: usize) -> Option<Vec3> {
    if enemy == PLAYER {
        return Some(player.feet);
    }
    Some(scene.states.get(enemy)?.origin)
}

/// A scanner's part of GatherConditions: its enemy, its health, what it inspects and the way.
fn gather(actor: &mut AiActor, world: &World, scene: &Scene, player: &PlayerState, id: usize) {
    let enemy = actor.npc.enemy;
    let known = enemy
        .and_then(|e| actor.npc.enemies.find(Some(e)))
        .map(|memory| memory.last_known);
    let AiClass::Scanner(s) = &mut actor.class else {
        return;
    };
    s.enemy_at = known;
    s.enemy_eye = enemy.and_then(|e| eyes(world, scene, player, e));
    s.inspect_at = s.inspect.and_then(|e| eyes(world, scene, player, e));
    s.inspect_facing = s.inspect.and_then(|e| marked_side(world, scene, e));
    s.enemy_in_suit = enemy == Some(PLAYER) && player.suit;
    let health = scene.states[id].value;
    let before = s.health * s.max_health;
    s.max_health = s.max_health.max(health);
    s.health = health / s.max_health.max(1.);
    let conditions = &mut actor.npc.conditions;
    match s.fly_blocked {
        Some(true) => conditions.set(COND_FLY_BLOCKED),
        Some(false) => conditions.set(COND_FLY_CLEAR),
        None => {}
    }
    s.sense_inspection(conditions, enemy.is_some(), scene.time);
    s.sees_enemy = conditions.has(COND_SEE_ENEMY);
    s.enemy_is_player = enemy == Some(PLAYER);
    // Shot, it goes dark for a while, and one already hurt starts to smoke.
    if conditions.has(COND_LIGHT_DAMAGE) || conditions.has(COND_HEAVY_DAMAGE) {
        s.spotlight = None;
        s.next_spotlight = scene.time + spotlight::OFF_AFTER_HIT;
        s.smoking |= damage::starts_smoking(before, s.max_health);
    }
    if let Some(at) = enemy.and_then(|e| enemy_feet(scene, player, e)) {
        attack_conditions(conditions, scene, (id, at), s.next_attack);
    }
}

/// What the classes in the air add to GatherConditions.
pub(super) fn gather_air(
    actor: &mut AiActor,
    world: &World,
    scene: &Scene,
    player: &PlayerState,
    id: usize,
) {
    birds::gather(actor, scene, player);
    gather(actor, world, scene, player, id);
}

/// FindEntityGeneric: what an input's parameter names.
fn subject(world: &World, name: &str, activator: usize) -> Option<usize> {
    match name.trim().to_ascii_lowercase().as_str() {
        "!player" => Some(PLAYER),
        "!activator" => Some(activator),
        wanted => birds::named(world, wanted).or_else(|| {
            let by_class = |e: &modkit_core::Entity| e.class().eq_ignore_ascii_case(wanted);
            world.entities.iter().position(by_class)
        }),
    }
}

impl AiRuntime {
    /// A think's dice. A scanner throws its own, so that one more scanner on a map leaves the
    /// random choices of every other NPC there as they were.
    pub(super) fn roll_for(&mut self, id: usize) -> f32 {
        match self.actors.get_mut(&id).map(|actor| &mut actor.class) {
            Some(AiClass::Scanner(s)) => s.dice(),
            _ => self.random(),
        }
    }
    /// The inputs maps send a scanner.
    pub(super) fn scanner_input(
        &mut self,
        world: &World,
        now: f64,
        target: usize,
        (input, parameter, activator): (&str, &str, usize),
    ) {
        let Some(actor) = self.actors.get_mut(&target) else {
            return;
        };
        let AiClass::Scanner(s) = &mut actor.class else {
            return;
        };
        let mode = match input {
            "inspecttargetphoto" => Mode::Photo,
            "inspecttargetspotlight" => Mode::Spot,
            "setfollowtarget" => Mode::Follow,
            "clearfollowtarget" => return s.clear_follow(),
            "disablespotlight" => {
                s.spotlight_disabled = true;
                return;
            }
            "setdistanceoverride" => {
                s.distance_override = parameter.trim().parse().unwrap_or(0.);
                return;
            }
            "inputshouldinspect" => {
                s.ignores_enemies = parameter.trim().parse().unwrap_or(0) == 0;
                if s.ignores_enemies {
                    s.inspect = None;
                }
                return;
            }
            _ => return,
        };
        if let Some(found) = subject(world, parameter, activator) {
            s.inspect_target(found, mode, now);
        }
    }
}

/// The class of a map's scanner.
pub(super) fn class_for(e: &modkit_core::Entity, gagged: bool, id: usize) -> AiClass {
    let mut scanner = Scanner::new(gagged, 0xbb67_ae85_84ca_a73b ^ id as u64);
    scanner.ignores_enemies = e.get("ShouldInspect").map(str::trim) == Some("0");
    scanner.spotlight_disabled = e.get("SpotlightDisabled").map(str::trim) == Some("1");
    scanner.only_players = e.get("OnlyInspectPlayers").map(str::trim) == Some("1");
    scanner.never_players = e.get("NeverInspectPlayers").map(str::trim) == Some("1");
    scanner.spotlight_reach = spotlight::reach_from(e.get("SpotlightLength"));
    let flags = e
        .get("spawnflags")
        .and_then(|f| f.trim().parse::<u32>().ok());
    scanner.strider_scout = flags.unwrap_or(0) & STRIDER_SCOUT != 0;
    AiClass::Scanner(Box::new(scanner))
}
