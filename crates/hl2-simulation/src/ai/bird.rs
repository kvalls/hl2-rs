//! npc_crow, npc_pigeon and npc_seagull (DESIGN 14o session 46): ambient birds that never attack.
//! On the ground they keep away from what they hate; scared, they take off for a hint or along
//! path_corners. This module decides and holds the flight's arithmetic; `runtime::birds` gives it
//! the world and moves the body. One class in the SDK, with three models and sets of sounds.
//! The SDK's hop schedule and morale are not here: nothing selects the one or reads the other.
use super::assault::Assault;
use super::conditions::{
    Cond, Conditions, COND_HEAR_COMBAT, COND_HEAR_DANGER, COND_HEAVY_DAMAGE, COND_LIGHT_DAMAGE,
    LAST_SHARED_CONDITION,
};
use super::metropolice::CopRequest;
use super::schedule::TaskStatus::{Complete, Failed, Running};
use super::schedule::{
    base_select_schedule, AiNpc, Behavior, Context, Goal, Motor, Schedule, Schedules, TaskArg,
    TaskStatus,
};
use super::senses::sound;
use super::state::NpcState;
use glam::Vec3;
use serde::Serialize;

/// CROW_TAKEOFF_SPEED and CROW_AIRSPEED.
pub const TAKEOFF_SPEED: f32 = 170.;
pub const AIR_SPEED: f32 = 220.;
/// VIEW_FIELD_FULL: birds see all around them.
pub const FIELD_OF_VIEW: f32 = -1.;
/// MaxYawSpeed.
pub const MAX_YAW_SPEED: f32 = 120.;
/// SetViewOffset's height.
pub const EYE_HEIGHT: f32 = 11.;
/// SF_CROW_FLYING: in the air from the start.
pub const SF_FLYING: u32 = 16;
/// HINT_CROW_FLYTO_POINT.
pub const HINT_TYPE: i32 = 700;
/// How far a hint can be (the FindHint of TASK_FIND_HINTNODE).
const HINT_RANGE: f32 = 10_000.;
/// An enemy closer than these: too close, way too close (GatherEnemyConditions).
const TOO_CLOSE: f32 = 1024.;
const WAY_TOO_CLOSE: f32 = 512.;
/// A bird too close to something walks off past the first distance, runs past the second.
const WALK_AWAY_PAST: f32 = 400.;
const RUN_AWAY_PAST: f32 = 300.;
/// Gunfire scares a grounded bird at most this often; a flight deafens it this long after.
const DANGER_SOUND_DELAY: f64 = 10.;
const FLIGHT_SOUND_DELAY: f64 = 5.;
/// Past this distance a flight turns a fixed share toward its goal; inside, the share falls
/// from `FAR_TURN`'s neighbourhood to all of it as the goal nears.
const TURN_RANGE: f32 = 256.;
const FAR_TURN: f32 = 0.25;
/// The steepest bank, degrees.
const MAX_BANK: f32 = 45.;
/// How often a flying bird checks that it still moves.
const STUCK_CHECK: f64 = 1.;
/// The reach of a stroll, and of a step away from an enemy (at least, and random on top).
const STROLL: f32 = 48.;
const EVADE: f32 = 32.;

/// Every cue this class plays; a cue that is not preloaded is dropped.
pub const SOUNDS: &[&str] = &[
    "NPC_Crow.Idle",
    "NPC_Crow.Alert",
    "NPC_Crow.Flap",
    "NPC_Crow.Die",
    "NPC_Pigeon.Idle",
    "NPC_Seagull.Idle",
    "NPC_Seagull.Pain",
];

/// GatherEnemyConditions; COND_CROW_FORCED_FLY (the FlyAway input).
pub const COND_ENEMY_TOO_CLOSE: Cond = Cond(LAST_SHARED_CONDITION + 1);
pub const COND_ENEMY_WAY_TOO_CLOSE: Cond = Cond(LAST_SHARED_CONDITION + 2);
pub const COND_FORCED_FLY: Cond = Cond(LAST_SHARED_CONDITION + 3);

/// The schedules a bird can reach, with the interrupts every NPC has; the class's own conditions
/// are in `custom_interrupts`.
const SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_CROW_IDLE_WALK",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_IDLE_STAND TASK_CROW_PICK_RANDOM_GOAL 0 \
         TASK_GET_PATH_TO_SAVEPOSITION 0 TASK_WALK_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         TASK_WAIT_PVS 0 \
         Interrupts COND_PROVOKED COND_NEW_ENEMY COND_HEAVY_DAMAGE COND_LIGHT_DAMAGE \
         COND_HEAR_DANGER COND_HEAR_COMBAT",
    ),
    (
        "SCHED_CROW_WALK_AWAY",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CROW_FLY_AWAY TASK_CROW_PICK_EVADE_GOAL 0 \
         TASK_GET_PATH_TO_SAVEPOSITION 0 TASK_WALK_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         Interrupts COND_NEW_ENEMY COND_HEAVY_DAMAGE COND_LIGHT_DAMAGE COND_HEAR_DANGER \
         COND_HEAR_COMBAT",
    ),
    (
        "SCHED_CROW_RUN_AWAY",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CROW_FLY_AWAY TASK_CROW_PICK_EVADE_GOAL 0 \
         TASK_GET_PATH_TO_SAVEPOSITION 0 TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         Interrupts COND_NEW_ENEMY COND_HEAVY_DAMAGE COND_LIGHT_DAMAGE COND_HEAR_DANGER \
         COND_HEAR_COMBAT",
    ),
    (
        "SCHED_CROW_IDLE_FLY",
        "Tasks TASK_FIND_HINTNODE 0 TASK_GET_PATH_TO_HINTNODE 0 TASK_WAIT_FOR_MOVEMENT 0",
    ),
    (
        "SCHED_CROW_FLY_AWAY",
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CROW_FLY_FAIL TASK_STOP_MOVING 0 \
         TASK_FIND_HINTNODE 0 TASK_GET_PATH_TO_HINTNODE 0 TASK_CROW_TAKEOFF 0 \
         TASK_WAIT_FOR_MOVEMENT 0",
    ),
    (
        "SCHED_CROW_FLY_FAIL",
        "Tasks TASK_CROW_FALL_TO_GROUND 0 TASK_SET_SCHEDULE SCHEDULE:SCHED_CROW_IDLE_WALK",
    ),
];
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
        let schedule = Schedule::parse(name, text).expect("bird schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// Which bird: the model's class, and so its sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Kind {
    Crow,
    Pigeon,
    Seagull,
}
impl Kind {
    pub fn for_class(class: &str) -> Option<Self> {
        match class {
            "npc_crow" => Some(Self::Crow),
            "npc_pigeon" => Some(Self::Pigeon),
            "npc_seagull" => Some(Self::Seagull),
            _ => None,
        }
    }
    pub fn idle_sound(self) -> &'static str {
        match self {
            Self::Crow => "NPC_Crow.Idle",
            Self::Pigeon => "NPC_Pigeon.Idle",
            Self::Seagull => "NPC_Seagull.Idle",
        }
    }
    /// Only the crow calls out on takeoff.
    fn alert_sound(self) -> Option<&'static str> {
        (self == Self::Crow).then_some("NPC_Crow.Alert")
    }
    /// The pigeon dies silently, and the seagull reuses its pain call.
    pub fn death_sound(self) -> Option<&'static str> {
        match self {
            Self::Crow => Some("NPC_Crow.Die"),
            Self::Pigeon => None,
            Self::Seagull => Some("NPC_Seagull.Pain"),
        }
    }
}

/// A place to fly to (info_node_hint or info_node_air_hint of `HINT_TYPE`).
#[derive(Clone, Debug, PartialEq)]
pub struct FlyHint {
    pub position: Vec3,
    pub group: String,
}
/// The nearest usable hint. A bird with a hint group takes only hints of that group; a bird
/// without one takes any.
pub fn nearest_hint(hints: &[FlyHint], origin: Vec3, group: &str) -> Option<Vec3> {
    hints
        .iter()
        .filter(|h| group.is_empty() || h.group.eq_ignore_ascii_case(group))
        .map(|h| (h.position, h.position.distance(origin)))
        .filter(|(_, distance)| *distance <= HINT_RANGE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(position, _)| position)
}

/// The share of the way a heading turns toward the goal in one steer.
pub fn turn_share(distance: f32) -> f32 {
    if distance <= TURN_RANGE {
        1. - distance / TURN_RANGE
    } else {
        FAR_TURN
    }
}
/// The heading after one steer, from the current one toward `target`.
pub fn heading(forward: Vec3, target: Vec3, distance: f32) -> Vec3 {
    let turned = forward
        .lerp(target, turn_share(distance))
        .normalize_or_zero();
    if turned == Vec3::ZERO {
        forward
    } else {
        turned
    }
}
/// A heading that would hit a surface, laid along that surface. Straight at it leaves nothing:
/// the bird stops, and the stuck check ends it.
pub fn deflect(direction: Vec3, normal: Vec3) -> Vec3 {
    (direction - normal * direction.dot(normal)).normalize_or_zero()
}
/// The roll, degrees, banking a heading of `yaw` degrees into a goal `to_goal` away.
pub fn bank(yaw: f32, to_goal: Vec3) -> f32 {
    let (sin, cos) = yaw.to_radians().sin_cos();
    let right = Vec3::new(sin, -cos, 0.);
    (right.dot(to_goal) * MAX_BANK).clamp(-MAX_BANK, MAX_BANK)
}

/// Where a flight is heading now. A corner's chain goes on from `corner`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Waypoint {
    pub position: Vec3,
    pub corner: Option<usize>,
}
/// One steer's outcome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steer {
    pub direction: Vec3,
    pub to_goal: Vec3,
    pub distance: f32,
    /// Close enough to count as there this steer.
    pub arrived: bool,
}

/// A bird under the AI.
#[derive(Clone, Debug, Serialize)]
pub struct Bird {
    pub kind: Kind,
    #[serde(skip)]
    pub requests: Vec<CopRequest>,
    pub assault: Option<Assault>,
    pub script_goal: Option<(Vec3, bool)>,
    /// The `deaf` keyvalue: it hears nothing.
    pub deaf: bool,
    /// The `hintgroup` keyvalue.
    pub group: String,
    /// In the air under its own power (NAV_FLY).
    pub flying: bool,
    pub velocity: Vec3,
    /// Where the flight is heading; None on the ground or when it has arrived.
    pub waypoint: Option<Waypoint>,
    /// The legs after that one, the next one last.
    pub route: Vec<Vec3>,
    /// The hint it holds. It keeps it across flights, so a bird that reaches a hint in mid-air
    /// drops, takes off and comes back to the same one.
    pub hint: Option<Vec3>,
    /// The corner FlyAway named, advanced as the flight passes corners.
    pub goal_corner: Option<Waypoint>,
    /// FlyAway's parameter, until the flight stage can see whether the corner is in sight.
    pub fly_request: Option<String>,
    pub forced_fly: bool,
    /// Asked to drop out of the air: the flight stage hands it to the step fall.
    pub drop: bool,
    /// The enemy's distance at the last think.
    pub enemy_distance: f32,
    /// The next stroll no sooner.
    pub next_ground_move: f64,
    /// Danger sounds and nearby enemies are ignored until then.
    pub danger_sound_time: f64,
    pub soaring: bool,
    pub soar_until: f64,
    /// The goal of the last steer, and the direction to it as last taken.
    pub steered_goal: Vec3,
    pub target: Vec3,
    pub next_steer: f64,
    pub last_step: f64,
    /// Where it was at the last stuck check, and when the next one is.
    pub stuck_origin: Vec3,
    pub next_stuck_check: f64,
    /// Its own dice: a flight's rolls leave the other NPCs' stream alone.
    seed: u64,
}
impl Bird {
    pub fn new(kind: Kind, flying: bool, deaf: bool, group: String, seed: u64) -> Self {
        let mut bird = Self {
            kind,
            requests: Vec::new(),
            assault: None,
            script_goal: None,
            deaf,
            group,
            flying,
            velocity: Vec3::ZERO,
            waypoint: None,
            route: Vec::new(),
            hint: None,
            goal_corner: None,
            fly_request: None,
            forced_fly: false,
            drop: false,
            enemy_distance: f32::MAX,
            next_ground_move: 0.,
            danger_sound_time: 0.,
            soaring: false,
            soar_until: 0.,
            steered_goal: Vec3::ZERO,
            target: Vec3::ZERO,
            next_steer: 0.,
            last_step: 0.,
            stuck_origin: Vec3::ZERO,
            next_stuck_check: 3.,
            seed: seed | 1,
        };
        bird.next_ground_move = f64::from(bird.roll()) * 5.;
        bird
    }
    /// A number in [0, 1).
    pub fn roll(&mut self) -> f32 {
        super::roll(&mut self.seed)
    }
    /// The check that ends a wedged bird: true when it has not moved and has no speed left. A
    /// scrape along a wall leaves a trace of speed, so neither has to be exactly nothing.
    pub fn stuck(&mut self, origin: Vec3, now: f64) -> bool {
        if now < self.next_stuck_check {
            return false;
        }
        if origin.distance(self.stuck_origin) < 1. && self.velocity.length() < 1. {
            return true;
        }
        self.stuck_origin = origin;
        self.next_stuck_check = now + STUCK_CHECK;
        false
    }
    /// GetSoundInterests, none for a deaf bird.
    pub fn sound_interests(&self) -> u32 {
        if self.deaf {
            0
        } else {
            sound::WORLD | sound::COMBAT | sound::PLAYER | sound::DANGER
        }
    }
    /// The enemy's distance this think, as conditions.
    pub fn sense_enemy(&mut self, conditions: &mut Conditions, distance: f32) {
        self.enemy_distance = distance;
        if distance < WAY_TOO_CLOSE {
            conditions.set(COND_ENEMY_WAY_TOO_CLOSE);
        }
        if distance < TOO_CLOSE {
            conditions.set(COND_ENEMY_TOO_CLOSE);
        }
    }
    /// A route to fly, or a failed task and no hint when there is none: the bird stays down.
    fn follow(&mut self, legs: Option<Vec<Vec3>>) -> TaskStatus {
        let Some(mut legs) = legs else {
            self.hint = None;
            return Failed;
        };
        legs.reverse();
        self.route = legs;
        if self.next_leg() {
            Complete
        } else {
            Failed
        }
    }
    /// On to the route's next leg; false when it has none left.
    pub fn next_leg(&mut self) -> bool {
        let Some(position) = self.route.pop() else {
            return false;
        };
        self.waypoint = Some(Waypoint {
            position,
            corner: None,
        });
        true
    }
    pub fn last_leg(&self) -> bool {
        self.route.is_empty()
    }
    /// SetFlyingState(FlyState_Flying).
    pub fn start_flying(&mut self, origin: Vec3, now: f64) {
        self.flying = true;
        self.drop = false;
        self.stuck_origin = origin;
        self.next_stuck_check = now + 3.;
        self.next_ground_move = now + 5. + 5. * f64::from(self.roll());
    }
    /// Steering toward the waypoint, every think in the air. The first steer after a new goal
    /// keeps the last target's direction.
    pub fn steer(&mut self, origin: Vec3, forward: Vec3, interval: f32, now: f64) -> Option<Steer> {
        let goal = self.waypoint?.position;
        self.danger_sound_time = now + FLIGHT_SOUND_DELAY;
        let to_goal = goal - origin;
        let distance = to_goal.length();
        if goal == self.steered_goal {
            self.target = to_goal.normalize_or_zero();
        } else {
            self.steered_goal = goal;
        }
        Some(Steer {
            direction: heading(forward, self.target, distance),
            to_goal,
            distance,
            arrived: distance < AIR_SPEED * interval,
        })
    }
    /// The flapping or soaring activity. A bird on the way down may soar for 1 to 4 s; each
    /// spell of either lasts until `soar_until`. `rolls` are in [0, 1).
    pub fn fly_activity(&mut self, now: f64, rolls: [f32; 2]) -> &'static str {
        if self.soar_until < now {
            let glide = rolls[0] <= 0.5 && !self.soaring && self.velocity.z < 0.;
            self.soaring = glide;
            let (least, spread) = if glide { (1., 3.) } else { (3., 2.) };
            self.soar_until = now + least + spread * f64::from(rolls[1]);
        }
        if self.soaring {
            "ACT_CROW_SOAR"
        } else {
            "ACT_FLY"
        }
    }
    /// The takeoff under way: it turns to its goal until the event lifts it or the clip ends.
    fn run_takeoff(&mut self, npc: &mut AiNpc, motor: &mut dyn Motor) -> TaskStatus {
        let Some(waypoint) = self.waypoint else {
            return Failed;
        };
        if !self.flying {
            motor.face(Goal::Position(waypoint.position));
        }
        if !self.flying && !motor.activity_finished() {
            return Running;
        }
        npc.activity = "ACT_FLY".into();
        motor.set_activity("ACT_FLY");
        Complete
    }
    fn scared(&mut self, npc: &AiNpc, now: f64) -> bool {
        let c = &npc.conditions;
        if std::mem::take(&mut self.forced_fly)
            || c.has(COND_LIGHT_DAMAGE)
            || c.has(COND_HEAVY_DAMAGE)
        {
            return true;
        }
        if self.danger_sound_time <= now && (c.has(COND_HEAR_DANGER) || c.has(COND_HEAR_COMBAT)) {
            self.danger_sound_time = now + DANGER_SOUND_DELAY;
            return true;
        }
        c.has(COND_ENEMY_WAY_TOO_CLOSE)
    }
}
impl Behavior for Bird {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        if self.flying {
            return "SCHED_CROW_IDLE_FLY".into();
        }
        // In the air without flying (dropped off a hint): take off, but not while still falling
        // to the ground at the start of the map.
        let adrift = ctx.airborne && ctx.now > 2.;
        if adrift || self.scared(npc, ctx.now) {
            return "SCHED_CROW_FLY_AWAY".into();
        }
        if npc.conditions.has(COND_ENEMY_TOO_CLOSE) && self.danger_sound_time <= ctx.now {
            if self.enemy_distance > WALK_AWAY_PAST {
                return "SCHED_CROW_WALK_AWAY".into();
            }
            if self.enemy_distance > RUN_AWAY_PAST {
                return "SCHED_CROW_RUN_AWAY".into();
            }
        }
        if !matches!(
            npc.state,
            NpcState::Idle | NpcState::Alert | NpcState::Combat
        ) {
            return base_select_schedule(npc, ctx, false);
        }
        if ctx.now > self.next_ground_move {
            self.next_ground_move = ctx.now + 10. + 10. * f64::from(ctx.random);
            return "SCHED_CROW_IDLE_WALK".into();
        }
        "SCHED_IDLE_STAND".into()
    }
    /// The class's conditions in the schedules that listen for them.
    fn custom_interrupts(&self, npc: &AiNpc) -> Conditions {
        match npc.schedule.as_deref() {
            Some("SCHED_CROW_IDLE_WALK") => {
                Conditions::of(&[COND_FORCED_FLY, COND_ENEMY_TOO_CLOSE])
            }
            Some("SCHED_CROW_WALK_AWAY" | "SCHED_CROW_RUN_AWAY") => {
                Conditions::of(&[COND_FORCED_FLY, COND_ENEMY_WAY_TOO_CLOSE])
            }
            _ => Conditions::default(),
        }
    }
    fn start_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        _: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        Some(match task {
            "TASK_FIND_HINTNODE" => {
                if self.goal_corner.is_none() && self.hint.is_none() {
                    let hints = ctx.fly_hints.as_deref().unwrap_or(&[]);
                    self.hint = nearest_hint(hints, ctx.origin, &self.group);
                }
                if self.goal_corner.is_some() || self.hint.is_some() {
                    Complete
                } else {
                    Failed
                }
            }
            "TASK_GET_PATH_TO_HINTNODE" => {
                if let Some(corner) = self.goal_corner {
                    // A named path needs no takeoff: it is flying from here on.
                    self.start_flying(ctx.origin, ctx.now);
                    self.route.clear();
                    self.waypoint = Some(corner);
                    self.hint = None;
                    Complete
                } else {
                    let legs = self.hint.and_then(|hint| motor.fly_route(hint));
                    self.follow(legs)
                }
            }
            "TASK_CROW_TAKEOFF" => {
                if let Some(cue) = self.kind.alert_sound().filter(|_| ctx.random < 0.25) {
                    self.requests.push(CopRequest::Sound(cue));
                }
                self.requests.push(CopRequest::Sound("NPC_Crow.Flap"));
                npc.activity = "ACT_CROW_TAKEOFF".into();
                motor.set_activity("ACT_CROW_TAKEOFF");
                Running
            }
            "TASK_CROW_PICK_RANDOM_GOAL" => {
                let second = (ctx.random * 97.).fract();
                let spread = Vec3::new(ctx.random * 2. - 1., second * 2. - 1., 0.);
                npc.save_position = Some(ctx.origin + spread * STROLL);
                Complete
            }
            "TASK_CROW_PICK_EVADE_GOAL" => {
                let Some((_, enemy)) = ctx.enemy else {
                    return Some(Failed);
                };
                let away = (ctx.origin - enemy)
                    .truncate()
                    .normalize_or_zero()
                    .extend(0.);
                let goal = ctx.origin + away * (EVADE + EVADE * ctx.random);
                npc.save_position = Some(goal);
                motor.face(Goal::Position(goal));
                Complete
            }
            "TASK_CROW_FALL_TO_GROUND" => {
                self.flying = false;
                self.waypoint = None;
                self.route.clear();
                self.drop = true;
                Running
            }
            "TASK_WAIT_FOR_MOVEMENT" if self.waypoint.is_some() => Running,
            _ => return None,
        })
    }
    fn run_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        _argument: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        if task == "TASK_CROW_TAKEOFF" {
            return Some(self.run_takeoff(npc, motor));
        }
        Some(match task {
            "TASK_CROW_FALL_TO_GROUND" if self.drop || ctx.airborne => Running,
            "TASK_CROW_FALL_TO_GROUND" => Complete,
            "TASK_WAIT_FOR_MOVEMENT" if self.waypoint.is_some() => Running,
            // A flight that has ended; a ground move goes on to the generic task.
            "TASK_WAIT_FOR_MOVEMENT" if self.flying => Complete,
            _ => return None,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
