//! The blended motor's speed plan for ground moves (Source SDK `game/server/ai_blended_movement.cpp`
//! CAI_BlendedMotor::BuildVelocityScript, InsertSlowdown and GetMoveScriptDist; `ai_motor.cpp`
//! DeltaV; `baseanimating.cpp` GetIdealAccel). HL2's humanoids (CAI_BaseHumanoid: citizens,
//! vortigaunts, Barney, metrocops, soldiers...), zombies, antlions and striders neither start nor
//! stop at full speed. Every move their motor lays a script along the path ahead: the current
//! speed where the NPC stands, the point one move interval ahead, every waypoint at the ideal
//! speed scaled by how sharply the path turns there (the dot of its two legs plus 0.2, clamped to
//! 0..1), the arrival speed at the end (an idle arrival: 0) and, when the move stops short of
//! its goal (an arrival distance), 1 u/s there. The speeds are then limited by the acceleration
//! forward and backward, the stretches where the ideal speed is reached get its nodes, and this
//! tick's distance is read off the script. Retail server.dll carries the same three `ai_path_*`
//! cvars (their defaults, 1, are assumed). Not modeled: the slowdown at an obstruction the direct
//! trace meets, the slope speed adjustment (npc_height_adjust), the companions' doubled
//! acceleration when stepping out of the player's way, and the idle layer the motor blends in
//! by 1 - speed/ideal (ours slows the move's cycle instead).
use super::{Actor, Goal, GoalBlockReason};
use glam::{Vec2, Vec3};
use modkit_core::animation::RootMotion;

/// GetIdealAccel per class (speed change per second).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Accel {
    /// CBaseAnimating: the ideal speed + 50.
    PlusFifty,
    /// CNPC_MetroPolice and CNPC_Antlion: twice the ideal speed.
    Twice,
    /// CNPC_Strider: the ideal speed.
    Same,
}

impl Accel {
    /// The blended motor's classes (CAI_BlendingHost: CAI_BaseHumanoid and everything under
    /// CAI_BaseActor, CNPC_BaseZombie and its kin, CNPC_Antlion, CAI_BlendedNPC's antlion
    /// guard, CNPC_Strider); every other ground NPC moves at full speed at once (CAI_Motor).
    pub fn for_class(class: &str) -> Option<Self> {
        match class {
            "npc_metropolice" | "npc_antlion" => Some(Self::Twice),
            "npc_strider" => Some(Self::Same),
            "npc_citizen" | "npc_barney" | "npc_alyx" | "npc_vortigaunt" | "npc_fisherman"
            | "npc_kleiner" | "npc_eli" | "npc_breen" | "npc_mossman" | "npc_gman" | "npc_monk"
            | "npc_magnusson" | "npc_dog" | "npc_hunter" | "npc_combine_s" | "npc_antlionguard" => {
                Some(Self::PlusFifty)
            }
            _ if crate::ai::zombie::is_zombie_class(class) => Some(Self::PlusFifty),
            _ => None,
        }
    }

    fn of(self, ideal: f32) -> f32 {
        match self {
            Self::PlusFifty => ideal + 50.,
            Self::Twice => ideal * 2.,
            Self::Same => ideal,
        }
    }
}

/// The AI's move interval: the navigator expects one interval of travel at the ground speed
/// (MoveCalcBaseGoal's curExpectedDist).
const MOVE_INTERVAL: f32 = 0.1;
/// Added to a turn's dot before it scales the ideal speed.
const TURN_DOT_BIAS: f32 = 0.2;
/// The speed of a slowdown point.
const SLOWDOWN_SPEED: f32 = 1.;
/// Accelerations below this are treated as none (a still actor).
const MIN_ACCEL: f32 = 0.001;

/// A move's state as the motor sees it.
pub(super) struct Plan<'a> {
    pub feet: Vec3,
    /// The current velocity's direction (zero when still).
    pub heading: Vec2,
    pub speed: f32,
    /// The waypoints left, the goal last.
    pub waypoints: &'a [Vec3],
    /// The look-ahead's steered direction this think: the script's first point lies along it
    /// (BuildVelocityScript's origin + move.dir x curExpectedDist), not toward the waypoint.
    pub steered: Option<Vec2>,
    pub ideal: f32,
    pub accel: f32,
    /// Distance short of the goal where the move ends (0: at the goal).
    pub arrival_distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Node {
    at: Vec2,
    speed: f32,
}

/// A cancelled moving goal leaves its speed on the actor: the AI cancels and re-requests in
/// the same tick when it re-paths, and the motor's velocity outlives the path (only MoveStop,
/// at arrival or a failure, zeroes it).
pub(super) fn carry(actor: Option<&mut Actor>, goal: &Goal, clock: f64) {
    if let Some(actor) = actor.filter(|_| goal.state == super::GoalState::Moving) {
        actor.carried = Some((goal.speed, clock));
    }
}

/// A new goal's starting speed: the one it replaces if that is moving, else a speed carried
/// from a goal cancelled on this clock, else at rest.
pub(super) fn starting_speed(
    previous: Option<&Goal>,
    actor: Option<&mut Actor>,
    clock: f64,
) -> f32 {
    if let Some(goal) = previous.filter(|g| g.state == super::GoalState::Moving) {
        return goal.speed;
    }
    actor
        .and_then(|a| a.carried.take())
        .filter(|&(_, at)| at == clock)
        .map_or(0., |(speed, _)| speed)
}

/// The share of this tick's full-speed root-motion travel the motor allows; updates the goal's
/// speed. 1 for an actor without the blended motor.
pub(super) fn throttle(
    actor: &Actor,
    goal: &mut Goal,
    track: &RootMotion,
    think: &super::steer::Think<'_>,
) -> Result<f32, GoalBlockReason> {
    let Some(rule) = actor.accel else {
        return Ok(1.);
    };
    let dt = think.dt();
    let duration = track
        .duration()
        .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?;
    let cycle = track
        .movement(0., 1.)
        .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?
        .position
        .x;
    let ideal = cycle * actor.scale * actor.playback_rate / duration;
    if !ideal.is_finite() || ideal <= 0. || dt <= 0. {
        return Ok(1.);
    }
    let yaw = goal.yaw.to_radians();
    let plan = Plan {
        feet: goal.feet,
        heading: Vec2::new(yaw.cos(), yaw.sin()) * if goal.speed > 0. { 1. } else { 0. },
        speed: goal.speed,
        waypoints: goal.route.feet.get(goal.next..).unwrap_or(&[]),
        steered: think.steered(goal),
        ideal,
        accel: rule.of(ideal),
        arrival_distance: goal.request.event_distance,
    };
    let (distance, speed) = step(&plan, dt);
    goal.speed = speed;
    Ok((distance / (ideal * dt)).clamp(0., 1.))
}

/// This tick's distance along the path and the speed at its end.
pub(super) fn step(plan: &Plan, dt: f32) -> (f32, f32) {
    let mut nodes = waypoint_nodes(plan);
    if nodes.len() < 2 || plan.accel < MIN_ACCEL {
        return (plan.ideal * dt, plan.ideal);
    }
    let total: f32 = nodes.windows(2).map(|w| w[0].at.distance(w[1].at)).sum();
    if plan.arrival_distance > 0. {
        insert_slowdown(&mut nodes, total - plan.arrival_distance);
    }
    // The current speed, by how far the path's first leg turns from the velocity.
    let first = (nodes[1].at - nodes[0].at).normalize_or_zero();
    nodes[0].speed *= first.dot(plan.heading).clamp(0., 1.);
    clamp_speeds(&mut nodes, plan.accel);
    insert_ideal_nodes(&mut nodes, plan.ideal, plan.accel);
    integrate(&mut nodes, dt)
}

/// The script's nodes: here, one move interval ahead, each waypoint by its turn, the goal at
/// the arrival speed (0: an idle arrival). Waypoints under a unit before the next are dropped.
fn waypoint_nodes(plan: &Plan) -> Vec<Node> {
    let here = plan.feet.truncate();
    let mut nodes = vec![Node {
        at: here,
        speed: plan.speed,
    }];
    let Some(first) = plan.waypoints.first().map(|w| w.truncate()) else {
        return nodes;
    };
    let toward = plan
        .steered
        .unwrap_or_else(|| (first - here).normalize_or_zero());
    let ahead = toward * (plan.ideal * MOVE_INTERVAL).min(here.distance(first));
    let points: Vec<Vec2> = std::iter::once(here + ahead)
        .chain(plan.waypoints.iter().map(|w| w.truncate()))
        .collect();
    for (i, &at) in points.iter().enumerate() {
        let speed = match points.get(i + 1) {
            Some(&next) => {
                if at.distance(next) < 1. {
                    continue;
                }
                let out = (next - at).normalize_or_zero();
                let into = (at - nodes[nodes.len() - 1].at).normalize_or_zero();
                plan.ideal * (out.dot(into) + TURN_DOT_BIAS).clamp(0., 1.)
            }
            // The arrival sequence's entry speed: the actors' arrivals here are idles.
            None => 0.,
        };
        nodes.push(Node { at, speed });
    }
    nodes
}

/// A point `distance` along the script where the speed falls to 1 u/s.
fn insert_slowdown(nodes: &mut Vec<Node>, mut distance: f32) {
    if distance <= 0. {
        return;
    }
    for i in 0..nodes.len() - 1 {
        let length = nodes[i].at.distance(nodes[i + 1].at);
        if length > 0. && distance < length {
            let at = nodes[i].at.lerp(nodes[i + 1].at, distance / length);
            nodes.insert(
                i + 1,
                Node {
                    at,
                    speed: SLOWDOWN_SPEED,
                },
            );
            return;
        }
        distance -= length;
    }
}

/// Speeds the acceleration cannot reach over a leg are lowered: forward from the start, then
/// backward from the end (deceleration).
fn clamp_speeds(nodes: &mut [Node], accel: f32) {
    let reachable = |from: f32, length: f32| (from * from + 2. * accel * length).sqrt();
    for i in 0..nodes.len() - 1 {
        let length = nodes[i].at.distance(nodes[i + 1].at);
        nodes[i + 1].speed = nodes[i + 1].speed.min(reachable(nodes[i].speed, length));
    }
    for i in (1..nodes.len()).rev() {
        let length = nodes[i - 1].at.distance(nodes[i].at);
        nodes[i - 1].speed = nodes[i - 1].speed.min(reachable(nodes[i].speed, length));
    }
}

/// Nodes where a leg reaches the ideal speed and leaves it again, or, on a leg too short for
/// that, where it peaks.
fn insert_ideal_nodes(nodes: &mut Vec<Node>, ideal: f32, accel: f32) {
    let span = |v: f32, t: f32| v * t + 0.5 * accel * t * t;
    let mut i = 0;
    while i + 1 < nodes.len() {
        let (a, b) = (nodes[i], nodes[i + 1]);
        let length = a.at.distance(b.at);
        let (t1, t2) = ((ideal - a.speed) / accel, (ideal - b.speed) / accel);
        let (d1, d2) = (span(a.speed, t1), span(b.speed, t2));
        // (distance along the leg, speed) of the new nodes.
        let mut inserted = Vec::new();
        if d1 + d2 < length {
            if d1 > 1. && t1 > 0.1 {
                inserted.push((d1, ideal));
            }
            if length - d2 > 1. && t2 > 0.1 {
                inserted.push((length - d2, ideal));
            }
        } else if delta_v(a.speed, b.speed, length).abs() < accel {
            // Accelerate for `up`, then decelerate for `down` onto the next speed.
            let both = accel * length + 0.5 * (a.speed * a.speed + b.speed * b.speed);
            let up = (both.sqrt() - a.speed) / accel;
            let down = (a.speed + accel * up - b.speed) / accel;
            let peak = a.speed + accel * up;
            if up > 0. && down > 0. && peak < ideal {
                inserted.push((span(a.speed, up), peak));
            }
        }
        let count = inserted.len();
        for (k, (d, speed)) in inserted.into_iter().enumerate() {
            let at = a.at.lerp(b.at, d / length);
            nodes.insert(i + 1 + k, Node { at, speed });
        }
        i += count + 1;
    }
}

/// The acceleration that turns speed `from` into `to` over `length` (DeltaV).
fn delta_v(from: f32, to: f32, length: f32) -> f32 {
    0.5 * (to * to - from * from) / length
}

/// Walks the script for `dt` (speed linear in time on each leg): the distance covered and the
/// speed reached.
fn integrate(nodes: &mut [Node], dt: f32) -> (f32, f32) {
    let mut left = dt;
    let mut distance = 0.;
    for i in 0..nodes.len() - 1 {
        let length = nodes[i].at.distance(nodes[i + 1].at);
        if nodes[i].speed <= 0. && nodes[i + 1].speed <= 0. {
            nodes[i + 1].speed = SLOWDOWN_SPEED;
        }
        let (v0, v1) = (nodes[i].speed.max(0.), nodes[i + 1].speed.max(0.));
        let time = length / (0.5 * (v0 + v1));
        if left < time {
            let speed = v0 + (v1 - v0) * left / time;
            return (distance + 0.5 * (v0 + speed) * left, speed);
        }
        distance += length;
        left -= time;
    }
    (distance, nodes[nodes.len() - 1].speed.max(0.))
}

#[cfg(test)]
mod tests;
