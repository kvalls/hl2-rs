//! Steering before contact (Source SDK `game/server/ai_localnavigator.cpp` MoveCalcDirect and
//! MoveCalcSteer, `ai_navigator.cpp` OnObstructionPreSteer, `ai_planesolver.cpp`
//! CAI_PlaneSolver::Solve, GenerateObstacleSuggestions, RunMoveSolver and DetectUnsolvable).
//! Once per think the mover looks ahead along its waypoint direction; when an obstruction
//! lies within the look-ahead and before the waypoint, the plane solver picks a direction
//! around it for this think. A failed solve leaves the plain direction, and contact is then
//! handled by the parent module's deflection, detour and waypoint skip.

use super::arcs::{self, Circle, Mover};
use super::plane::{self, Arc, Favor, Kind};
use super::{links, Actor, Goal, Step};
use crate::npc_probe::{self, ActorHull, NpcCollisionWorld, Query};
use glam::{Vec2, Vec3};
use std::collections::BTreeMap;

/// The NPC think interval the motor's decisions follow (CAI_BaseNPC thinks every 0.1 s).
const THINK: f64 = 0.1;
/// CalcProbeDist: one second of travel (GetLookaheadTime), within a hull width and
/// MAX_PROBE_DIST (10 feet with ai_strong_optimizations 0).
const LOOKAHEAD_SECONDS: f32 = 1.;
const MAX_PROBE: f32 = 120.;
/// TIME_DELAY_FULL_DIRECT_PROBE: a clear look-ahead is repeated this often.
const FULL_PROBE_EVERY: f64 = 0.25;
/// DEGREES_POSITIVE_ARC and _CLOSE_OBSTRUCTION, over NUM_PROBES probes.
const ARC: f32 = 270.;
const ARC_CLOSE: f32 = 340.;
const PROBES: i32 = 5;
/// DetectUnsolvable: no progress toward the same waypoint for this long gives up.
const UNSOLVABLE_AFTER: f64 = 0.75;
/// MoveCalcSteer ignores a solution this close to the heading (cos 15 degrees) when a
/// waypoint is near and the way mostly clear.
const SMALL_TURN_DOT: f32 = 0.966;
/// GenerateObstacleNpcs counts actors whose height overlaps within this margin.
const HEIGHT_MARGIN: f32 = 12.;

/// The last full look-ahead: its waypoint, direction, outcome and when it is due again.
#[derive(Clone, Copy, Debug)]
struct Look {
    target: Vec3,
    direction: Vec3,
    clear: bool,
    due: f64,
}

/// One actor's look-ahead and solver memory between thinks.
#[derive(Clone, Debug, Default)]
pub(super) struct Memory {
    next_think: f64,
    look: Option<Look>,
    /// The solved direction this think walks, for its waypoint.
    current: Option<(Vec3, Vec3)>,
    /// RunMoveSolver's previous answer (yaw) and whether the last solve succeeded
    /// (m_PrevSolution, m_fSolvedPrev), for `previous_target`.
    previous_yaw: f32,
    solved: bool,
    previous_target: Option<Vec3>,
    /// DetectUnsolvable: the nearest the mover came to the target, when, and a give-up.
    closest: f32,
    progress_at: f64,
    unsolvable: bool,
}

impl Memory {
    /// The steered direction in force for `target`.
    pub(super) fn current(&self, target: Vec3) -> Option<Vec2> {
        self.current
            .filter(|(at, _)| *at == target)
            .map(|(_, direction)| direction.truncate())
    }
}

/// Every actor's velocity since the last tick and its steering memory (`Controller::motion`).
#[derive(Clone, Debug, Default)]
pub(in crate::npc) struct Motion {
    feet: BTreeMap<usize, Vec3>,
    velocity: BTreeMap<usize, Vec2>,
    memory: BTreeMap<usize, Memory>,
}

impl Motion {
    /// The actors' planar velocities from where they stood at the previous tick.
    pub fn update(&mut self, transients: &[ActorHull], dt: f32) {
        if dt.is_nan() || dt <= 0. {
            return;
        }
        self.velocity.clear();
        for actor in transients {
            let Some(entity) = actor.entity else { continue };
            if let Some(last) = self.feet.insert(entity, actor.feet) {
                self.velocity
                    .insert(entity, (actor.feet - last).truncate() / dt);
            }
        }
    }

    /// The steering context of `actor`'s move this tick at movement time `now`.
    pub fn think(&mut self, actor: usize, now: f64, dt: f32) -> super::Think<'_> {
        let Self {
            velocity, memory, ..
        } = self;
        super::Think::new(actor, (now, dt), memory.entry(actor).or_default(), velocity)
    }
}

/// What the direct look-ahead found.
struct Direct {
    target: Vec3,
    probe: f32,
    max_dist: f32,
    clear_dist: f32,
    by_npc: bool,
    speed: f32,
}

/// This think's steered direction for `step`, None to walk the plain direction.
pub(super) fn steer<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &Goal,
    step: &Step,
    query: Query<'_>,
    think: &mut super::Think<'_>,
) -> Option<Vec3> {
    let target = *goal.route.feet.get(goal.next)?;
    let memory = &mut *think.memory;
    if think.now < memory.next_think {
        return memory
            .current
            .filter(|(at, _)| *at == target)
            .map(|(_, direction)| direction);
    }
    memory.next_think = think.now + THINK;
    memory.current = None;
    let direct = look_ahead(world, actor, goal, step, query, think, target)?;
    let direction = solve(world, actor, goal, step, query, think, &direct)?;
    think.memory.current = Some((target, direction));
    Some(direction)
}

/// MoveCalcDirect and OnObstructionPreSteer: the look-ahead along the plain direction; Some
/// when it is blocked before the waypoint, so the solver should run.
fn look_ahead<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &Goal,
    step: &Step,
    query: Query<'_>,
    think: &mut super::Think<'_>,
    target: Vec3,
) -> Option<Direct> {
    if step.consumed <= 0. {
        return None;
    }
    let speed = step.amount / step.consumed;
    let hull = actor.ground.hull;
    let width = hull.maxs.x - hull.mins.x;
    let max_dist = (target - step.start).truncate().length();
    // The goal or the last ground waypoint before a jump or climb caps the look-ahead.
    let capped = goal.next + 1 >= links::ground_end(&goal.route, goal.next);
    let mut probe = (speed * LOOKAHEAD_SECONDS).clamp(width, MAX_PROBE);
    if capped {
        probe = probe.min(max_dist);
    }
    let memory = &mut *think.memory;
    let same = memory.look.is_some_and(|look| {
        look.clear
            && think.now < look.due
            && look.target.distance(target) <= 0.1
            && (look.direction - step.direction).abs().max_element() <= 0.1
    });
    if same || probe <= 0. {
        return None;
    }
    let end = step.start + step.direction * probe;
    let moved = npc_probe::ground_move(world, step.start, end, actor.ground, query);
    let clear_dist = (moved.end - step.start).truncate().length();
    let clear = moved.completed || (capped && max_dist < clear_dist);
    memory.look = Some(Look {
        target,
        direction: step.direction,
        clear,
        due: think.now + FULL_PROBE_EVERY,
    });
    let is_goal = goal.next + 1 >= goal.route.feet.len();
    // Beyond the waypoint, or a goal already within its tolerance: nothing to steer around.
    if clear || clear_dist > max_dist || (is_goal && max_dist < goal.tolerance) {
        return None;
    }
    Some(Direct {
        target,
        probe,
        max_dist,
        clear_dist,
        by_npc: moved.transient && moved.blocker.is_some(),
        speed,
    })
}

/// Solve and RunMoveSolver: the direction around the obstruction, None when unsolvable, when
/// the move along it is not clear, or when MoveCalcSteer keeps the plain direction.
fn solve<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &Goal,
    step: &Step,
    query: Query<'_>,
    think: &mut super::Think<'_>,
    direct: &Direct,
) -> Option<Vec3> {
    let (now, me) = (think.now, think.me);
    let mine = think.velocity.get(&me).copied().unwrap_or(Vec2::ZERO);
    let memory = &mut *think.memory;
    let new_target = memory.previous_target != Some(direct.target);
    if unsolvable(memory, direct, now, new_target) {
        return None;
    }
    let fresh = new_target || !memory.solved;
    let close = direct.clear_dist < 1.;
    let span = if close { ARC_CLOSE } else { ARC };
    let heading = step.direction.truncate().normalize_or_zero();
    let heading_yaw = arcs::yaw(heading);
    let sample = Sample {
        world,
        actor,
        step,
        query,
        heading_yaw,
        span,
    };
    let mover = Mover {
        at: step.start.truncate(),
        radius: radius(actor.ground.hull),
        heading,
        velocity: mine,
        probe: direct.probe,
    };
    let mut arcs = sample.obstacles(&mover, goal, think);
    let (probed, found) = sample.probes(direct.probe);
    // Something found (an actor's circle or a blocked probe): unknown behind the arc.
    let found = found || !arcs.is_empty();
    arcs.extend(probed);
    if found {
        arcs.push(Arc::new(Kind::Unknown, 1., heading_yaw - 180., 360. - span));
    }
    let memory = &mut *think.memory;
    memory.previous_target = Some(direct.target);
    let answer = if arcs.is_empty() {
        Some(heading)
    } else {
        decide(
            memory,
            arcs,
            (heading_yaw, span),
            direct.by_npc,
            (close, fresh),
        )
        .map(|yaw| blend(arcs::heading(yaw), mine, fresh))
    };
    let solved = answer.filter(|direction| {
        let end = step.start + direction.extend(0.) * direct.speed * THINK as f32;
        npc_probe::ground_move(world, step.start, end, actor.ground, query).completed
    });
    memory.solved = solved.is_some();
    let direction = solved?;
    // MoveCalcSteer: a slight turn just before a waypoint is not worth taking.
    let slight = direction.dot(heading) > SMALL_TURN_DOT
        && direct.max_dist < direct.probe / 3.
        && direct.clear_dist > direct.probe * 2. / 3.;
    (!slight && direction.distance(heading) > 1e-4).then(|| direction.extend(0.))
}

/// DetectUnsolvable for this target: true once it made no progress for UNSOLVABLE_AFTER.
fn unsolvable(memory: &mut Memory, direct: &Direct, now: f64, new_target: bool) -> bool {
    let distance = direct.max_dist;
    if new_target {
        memory.closest = distance;
        memory.progress_at = now;
        memory.unsolvable = false;
    } else if !memory.unsolvable {
        if memory.closest > distance {
            memory.closest = distance;
            memory.progress_at = now;
        } else if now - memory.progress_at > UNSOLVABLE_AFTER {
            memory.unsolvable = true;
        }
    }
    memory.unsolvable
}

/// The vote with the wanted direction (favouring the left when an NPC blocks) and, after a
/// solve for the same target, a deterrent opposite the previous answer; a near reversal of
/// that answer fails.
fn decide(
    memory: &mut Memory,
    mut arcs: Vec<Arc>,
    (heading_yaw, span): (f32, f32),
    by_npc: bool,
    (close, fresh): (bool, bool),
) -> Option<f32> {
    let mut wanted = Arc::new(Kind::Move, 1., heading_yaw, span);
    if by_npc {
        wanted.favor = Favor::Left;
    }
    arcs.push(wanted);
    if !close && !fresh {
        arcs.push(Arc::new(
            Kind::Deterrent,
            1.,
            memory.previous_yaw - 180.,
            180.,
        ));
    }
    let yaw = plane::solve(&arcs)?;
    if !fresh {
        let turn = (yaw - memory.previous_yaw).rem_euclid(360.);
        if turn > 165. && turn < 195. {
            return None;
        }
    }
    memory.previous_yaw = yaw;
    Some(yaw)
}

/// The answer leaned toward the mover's current velocity (RunMoveSolver's smoothing).
fn blend(answer: Vec2, velocity: Vec2, fresh: bool) -> Vec2 {
    let moving = velocity.normalize_or_zero();
    if moving == Vec2::ZERO {
        return answer;
    }
    let lean = if fresh { 0.2 } else { 0.1 };
    (moving * lean + answer * (1. - lean)).normalize_or_zero()
}

/// Half the planar diagonal of a hull (WorldSpaceSurroundingBounds' radius).
fn radius(hull: npc_probe::Hull) -> f32 {
    (hull.maxs.truncate() - hull.mins.truncate()).length() * 0.5
}

/// The mover's surroundings for one solve.
struct Sample<'a, W> {
    world: &'a W,
    actor: &'a Actor,
    step: &'a Step,
    query: Query<'a>,
    heading_yaw: f32,
    span: f32,
}

impl<W: NpcCollisionWorld> Sample<'_, W> {
    /// GenerateObstacleNpcs and GenerateCircleObstacleSuggestions: every other actor whose
    /// height overlaps and whose circle comes within the probe; the player's circle is its
    /// full planar diagonal, as the SDK takes it.
    fn obstacles(&self, mover: &Mover, goal: &Goal, think: &super::Think<'_>) -> Vec<Arc> {
        let hull = self.actor.ground.hull;
        let (low, high) = (
            self.step.start.z + hull.mins.z,
            self.step.start.z + hull.maxs.z,
        );
        self.query
            .transients
            .iter()
            .filter(|other| {
                let id = other.entity;
                id != Some(think.me)
                    && id != Some(goal.request.target_entity)
                    && !id.is_some_and(|e| self.query.excluded_entities.contains(&e))
                    && other.feet.z + other.hull.mins.z < high + HEIGHT_MARGIN
                    && other.feet.z + other.hull.maxs.z > low - HEIGHT_MARGIN
            })
            .filter_map(|other| {
                let radius = match other.entity {
                    Some(_) => radius(other.hull),
                    None => radius(other.hull) * 2.,
                };
                let center = (other.feet + (other.hull.mins + other.hull.maxs) * 0.5).truncate();
                if other.feet.truncate().distance(mover.at) - radius >= mover.radius + mover.probe {
                    return None;
                }
                let velocity = other
                    .entity
                    .map(|e| think.velocity.get(&e).copied().unwrap_or(Vec2::ZERO));
                arcs::circle_arc(
                    mover,
                    &Circle {
                        center,
                        radius,
                        velocity,
                    },
                )
            })
            .collect()
    }

    /// The five world probes across the arc (NPCs ignored): the centre, then each side out
    /// to its first clear probe, the rest of that side unknown; and whether any was blocked.
    fn probes(&self, probe: f32) -> (Vec<Arc>, bool) {
        let per = self.span / PROBES as f32;
        let side = (PROBES - 1) / 2;
        let mut arcs = self.probe(probe, per, 0);
        let mut found = !arcs.is_empty();
        for sign in [1, -1] {
            let mut clear = false;
            for i in 1..=side {
                let offset = i * sign;
                if clear {
                    let (center, _) = arcs::probe_yaws(self.heading_yaw, per, offset);
                    arcs.push(Arc::new(Kind::Unknown, 1., center, per));
                    continue;
                }
                let blocked = self.probe(probe, per, offset);
                clear = blocked.is_empty();
                found |= !clear;
                arcs.extend(blocked);
            }
        }
        (arcs, found)
    }

    /// One world probe (GenerateObstacleSuggestion) and its arcs when blocked.
    fn probe(&self, probe: f32, per: f32, offset: i32) -> Vec<Arc> {
        let (center, test) = arcs::probe_yaws(self.heading_yaw, per, offset);
        let start = self.step.start;
        let end = start + arcs::heading(test).extend(0.) * probe;
        let world_only = Query {
            transients: &[],
            ..self.query
        };
        let moved = npc_probe::ground_move(self.world, start, end, self.actor.ground, world_only);
        if moved.completed {
            return Vec::new();
        }
        let clear = (moved.end - start).truncate().length();
        let favor_left = (moved.normal != Vec3::ZERO).then(|| {
            let along = moved.end - start;
            along.cross(moved.normal).z > 0.
        });
        arcs::trace_arcs(center, per, (probe, probe - clear), favor_left)
    }
}
