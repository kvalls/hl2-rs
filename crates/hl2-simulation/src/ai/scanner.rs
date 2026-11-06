//! npc_cscanner (DESIGN 14o session 46): a hovering Combine scanner. This module holds what it
//! decides between, its schedules and where it wants to be; `ai::hover` is its flight arithmetic
//! and `runtime::scanners` gives it the world and carries the body. It patrols, follows
//! path_corners, photographs and follows what maps name, hovers near an enemy, and dies in a dive
//! or a burst of gibs. Its eye warns and flashes (`flash`); the spotlight is not modeled yet.
use super::assault::Assault;
use super::conditions::{
    Cond, Conditions, COND_CAN_MELEE_ATTACK1, COND_HEAVY_DAMAGE, COND_LIGHT_DAMAGE,
    COND_LOST_ENEMY, LAST_SHARED_CONDITION,
};
use super::hover::Hover;
use super::metropolice::{CopRequest, PLAYER};
use super::schedule::TaskStatus::{Complete, Failed, Running};
use super::schedule::{
    base_select_schedule, AiNpc, Behavior, Context, Motor, Schedule, Schedules, TaskArg, TaskStatus,
};
use super::senses::sound;
use super::state::NpcState;
use crate::sounds::PatchOp;
use flash::Eye;
use glam::Vec3;
use modkit_core::animation::simple_spline;
use pose::Pose;
use serde::Serialize;
use spotlight::Spotlight;

/// m_flFieldOfView.
pub const FIELD_OF_VIEW: f32 = 0.2;
/// SetViewOffset's height.
pub const EYE_HEIGHT: f32 = 10.;
/// SCANNER_MAX_SPEED: LimitSpeed's cap, with or without an enemy.
pub const MAX_SPEED: f32 = 250.;
/// SCANNER_SPOTLIGHT_FLY_HEIGHT and SCANNER_NOSPOTLIGHT_FLY_HEIGHT, which are equal.
pub const GROUND_CLEARANCE: f32 = 72.;
/// AddNoiseToVelocity's scale, so that it never looks bolted in place.
pub const NOISE: f32 = 3.;
/// SCANNER_ATTACK_FAR_DIST: past this it stays high and approaches with its spotlight.
const ATTACK_FAR: f32 = 300.;
/// SCANNER_ATTACK_NEAR_DIST: how high it attacks from, and with the far distance how far.
const ATTACK_NEAR: f32 = 150.;
/// SCANNER_SPOTLIGHT_NEAR_DIST, how high it hovers with its light, and the middle of that
/// distance and SCANNER_SPOTLIGHT_FAR_DIST, how far off.
const SPOT_HEIGHT: f32 = 64.;
const SPOT_RANGE: f32 = 160.;
/// SCANNER_FOLLOW_DIST.
const FOLLOW_RANGE: f32 = 128.;
/// How high it hovers for a photograph, and the range a photograph is taken in.
const PHOTO_HEIGHT: f32 = 32.;
const PHOTO_NEAR: f32 = 64.;
const PHOTO_FAR: f32 = 128.;
/// Faster than this it cannot take a photograph, and the aim it needs, degrees.
const PHOTO_SPEED: f32 = 64.;
const PHOTO_AIM: f32 = 4.;
/// How long the flash lasts, and the pause before another photograph of the same target.
const FLASH: f64 = 0.1;
const NEXT_PHOTO: (f64, f64) = (5., 10.);
/// SCANNER_CIT_INSPECT_LENGTH, and an enemy's inspection, which does not run out.
const INSPECT_LENGTH: f64 = 5.;
const ENEMY_INSPECT: f64 = 9999.;
/// How close to a point along a route counts as there, and to its end by default.
const LEG_TOLERANCE: f32 = 64.;
const END_TOLERANCE: f32 = 32.;
/// GetHeadTurnRate, degrees a second.
const TURN_RATE: f32 = 350.;
const ENEMY_TURN_RATE: f32 = 800.;
/// The lean toward a target straight to one side at full speed, degrees.
const LEAN: f32 = 120.;
/// The share of its health below which an alert scanner hurt in the open takes cover.
const COVER_HEALTH: f32 = 0.75;
/// How often a hit in combat makes it flinch: 4 of the 11 values of RandomInt(0, 10).
const COMBAT_FLINCH: f32 = 4. / 11.;
/// How often a think speaks: 1 of the 26 values of RandomInt(0, 25).
const IDLE_CHANCE: f32 = 1. / 26.;
/// Killed this high above its enemy and this far from it, it dives at it.
const DIVE_HEIGHT: f32 = 120.;
const DIVE_DISTANCE: f32 = 360.;
/// SCANNER_MAX_DIVE_BOMB_SPEED, and the noise of a dive against a flight's.
pub const DIVE_SPEED: f32 = 2500.;
pub const DIVE_NOISE: f32 = 4.;
/// The spin of a dive, degrees a second. The SDK's values are a torque on a physics body: these
/// are that range read as a rate, not measured.
const DIVE_SPIN: (f32, f32) = (20., 420.);
/// A change of speed share smaller than this leaves the engine sound as it is.
const ENGINE_STEP: f32 = 0.02;

/// The activity that opens its eye for a photograph.
const FLARE: &str = "ACT_SCANNER_FLARE_START";
/// What closes it again: the model's sequence graph goes from flare to idle through retract, which
/// native plays after every flare (0.67 s, then 1 s).
const RETRACT: &str = "ACT_SCANNER_FLARE_END";

const ENGINE: &str = "NPC_CScanner.FlyLoop";
const IDLE: &str = "NPC_CScanner.Idle";
const COMBAT: &str = "NPC_CScanner.Combat";
pub const PAIN: &str = "NPC_CScanner.Pain";
pub const DIE: &str = "NPC_CScanner.Die";
const TAKE_PHOTO: &str = "NPC_CScanner.TakePhoto";
const ATTACK_FLASH: &str = "NPC_CScanner.AttackFlash";
pub const DIVE_BOMB: &str = "NPC_CScanner.DiveBomb";
/// Every cue this class plays; a cue that is not preloaded is dropped.
pub const SOUNDS: &[&str] = &[
    crate::impacts::RICOCHET_SOUND,
    ENGINE,
    IDLE,
    COMBAT,
    PAIN,
    DIE,
    TAKE_PHOTO,
    ATTACK_FLASH,
    DIVE_BOMB,
];

/// COND_SCANNER_FLY_CLEAR and COND_SCANNER_FLY_BLOCKED: the way to its enemy.
pub const COND_FLY_CLEAR: Cond = Cond(LAST_SHARED_CONDITION + 1);
pub const COND_FLY_BLOCKED: Cond = Cond(LAST_SHARED_CONDITION + 2);
/// COND_CSCANNER_HAVE_INSPECT_TARGET, COND_CSCANNER_INSPECT_DONE, COND_CSCANNER_CAN_PHOTOGRAPH.
pub const COND_HAVE_TARGET: Cond = Cond(LAST_SHARED_CONDITION + 3);
pub const COND_INSPECT_DONE: Cond = Cond(LAST_SHARED_CONDITION + 4);
pub const COND_CAN_PHOTOGRAPH: Cond = Cond(LAST_SHARED_CONDITION + 5);

const PATROL: &str = "SCHED_CSCANNER_PATROL";
const CHASE: &str = "SCHED_SCANNER_CHASE_ENEMY";
const SPOTLIGHT_HOVER: &str = "SCHED_CSCANNER_SPOTLIGHT_HOVER";
const ATTACK_HOVER: &str = "SCHED_SCANNER_ATTACK_HOVER";
const ATTACK: &str = "SCHED_CSCANNER_ATTACK_FLASH";
const PHOTOGRAPH_HOVER: &str = "SCHED_CSCANNER_PHOTOGRAPH_HOVER";
const PHOTOGRAPH: &str = "SCHED_CSCANNER_PHOTOGRAPH";
const MOVE_TO_INSPECT: &str = "SCHED_CSCANNER_MOVE_TO_INSPECT";
const FOLLOW_HOVER: &str = "SCHED_SCANNER_FOLLOW_HOVER";
/// Along its path_corners; the SDK's NPCs walk them with SCHED_IDLE_WALK.
const CORNERS: &str = "SCHED_SCANNER_CORNERS";

/// The schedules a scanner can reach, with the interrupts every NPC has; the class's own conditions
/// are in `custom_interrupts`. The spotlight hover's ACT_WALK is not in the model, which makes it
/// ACT_RUN: native plays inspect1 and inspect2 through a hunt and idles only for the flash.
const SCHEDULES: &[(&str, &str)] = &[
    (
        PATROL,
        "Tasks TASK_CSCANNER_CLEAR_INSPECT_TARGET 0 TASK_SCANNER_SET_FLY_PATROL 0 \
         TASK_SET_TOLERANCE_DISTANCE 32 TASK_GET_PATH_TO_RANDOM_NODE 2000 TASK_RUN_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 \
         Interrupts COND_GIVE_WAY COND_NEW_ENEMY COND_SEE_ENEMY COND_SEE_FEAR COND_HEAR_COMBAT \
         COND_HEAR_DANGER COND_HEAR_PLAYER COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE COND_PROVOKED",
    ),
    (
        CHASE,
        "Tasks TASK_SCANNER_SET_FLY_CHASE 0 TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CSCANNER_PATROL \
         TASK_SET_TOLERANCE_DISTANCE 120 TASK_GET_PATH_TO_ENEMY 0 TASK_RUN_PATH 0 \
         TASK_WAIT_FOR_MOVEMENT 0 \
         Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD COND_LOST_ENEMY",
    ),
    (
        SPOTLIGHT_HOVER,
        "Tasks TASK_CSCANNER_SET_FLY_SPOT 0 TASK_SET_ACTIVITY ACTIVITY:ACT_WALK TASK_WAIT 1 \
         Interrupts COND_NEW_ENEMY",
    ),
    (
        ATTACK_HOVER,
        "Tasks TASK_SCANNER_SET_FLY_ATTACK 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_WAIT 0.1 \
         Interrupts COND_TOO_FAR_TO_ATTACK COND_NEW_ENEMY",
    ),
    (
        ATTACK,
        "Tasks TASK_SCANNER_SET_FLY_ATTACK 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE \
         TASK_CSCANNER_ATTACK_PRE_FLASH 0 TASK_CSCANNER_ATTACK_FLASH 0 TASK_WAIT 0.5 \
         Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD",
    ),
    (
        PHOTOGRAPH_HOVER,
        "Tasks TASK_CSCANNER_SET_FLY_PHOTO 0 TASK_WAIT 2 Interrupts COND_NEW_ENEMY",
    ),
    (
        PHOTOGRAPH,
        "Tasks TASK_CSCANNER_SET_FLY_PHOTO 0 TASK_CSCANNER_PHOTOGRAPH 0 \
         Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD",
    ),
    (
        MOVE_TO_INSPECT,
        "Tasks TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CSCANNER_PATROL \
         TASK_SET_TOLERANCE_DISTANCE 128 TASK_CSCANNER_GET_PATH_TO_INSPECT_TARGET 0 \
         TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 Interrupts COND_NEW_ENEMY",
    ),
    (
        FOLLOW_HOVER,
        "Tasks TASK_STOP_MOVING 0 TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_WAIT 0.1",
    ),
    (
        CORNERS,
        "Tasks TASK_RUN_PATH 0 TASK_WAIT_FOR_MOVEMENT 0 \
         Interrupts COND_GIVE_WAY COND_NEW_ENEMY COND_SEE_ENEMY COND_SEE_FEAR COND_HEAR_COMBAT \
         COND_HEAR_DANGER COND_HEAR_PLAYER COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE COND_PROVOKED",
    ),
];

pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
        let schedule = Schedule::parse(name, text).expect("scanner schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// What a scanner does next. The names follow its schedules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Plan {
    Script,
    Flinch,
    TakeCover,
    Patrol,
    Corners,
    Chase,
    SpotlightHover,
    AttackHover,
    /// Close and facing its enemy with an attack due: the coin decides what comes of it.
    Melee,
    MoveToInspect,
    PhotographHover,
    Photograph,
    FollowHover,
}

/// m_nFlyMode, as far as the modes are modeled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Mode {
    #[default]
    Patrol,
    Chase,
    Spot,
    Attack,
    Photo,
    Follow,
}

/// A killed scanner's fall.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Dive {
    pub direction: Vec3,
    /// Degrees a second around its length.
    pub spin: f32,
}

/// A scanner under the AI.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Scanner {
    #[serde(skip)]
    pub requests: Vec<CopRequest>,
    pub assault: Option<Assault>,
    pub script_goal: Option<(Vec3, bool)>,
    pub hover: Hover,
    pub mode: Mode,
    /// m_flNextAttack.
    pub next_attack: f64,
    /// m_bShouldInspect, inverted: the InputShouldInspect switch.
    pub ignores_enemies: bool,
    /// The points of its route still to reach, the next one first.
    pub route: Vec<Vec3>,
    /// How close the route's end must come.
    pub tolerance: f32,
    /// The enemy's last known position and its eyes, from this think's gather.
    pub enemy_at: Option<Vec3>,
    pub enemy_eye: Option<Vec3>,
    /// IsEnemyPlayerInSuit.
    pub enemy_in_suit: bool,
    /// What it inspects or follows (`PLAYER` for the player), its eyes from this think's gather,
    /// and where that thing faces when it marks a direction to hover from.
    pub inspect: Option<usize>,
    pub inspect_at: Option<Vec3>,
    pub inspect_facing: Option<Vec3>,
    /// m_fInspectEndTime.
    pub inspect_end: f64,
    /// A target given since the last schedule choice.
    pub new_target: bool,
    /// m_fCheckCitizenTime, set by its first think; OnlyInspectPlayers and NeverInspectPlayers.
    pub next_search: Option<f64>,
    pub only_players: bool,
    pub never_players: bool,
    /// m_bPhotoTaken, and when the next step of a photograph or an attack comes.
    pub photo_taken: bool,
    photo_wait: f64,
    /// What lights its eye now.
    pub eye: Option<Eye>,
    /// Its spotlight when lit, m_fNextSpotlightTime, SpotlightDisabled and SpotlightLength.
    pub spotlight: Option<Spotlight>,
    pub next_spotlight: f64,
    pub spotlight_disabled: bool,
    pub spotlight_reach: f32,
    /// Where the light sits on the body, from the last think that had a lit spotlight.
    pub light_offset: Vec3,
    /// It sees its enemy, and that enemy is the player.
    pub sees_enemy: bool,
    pub enemy_is_player: bool,
    /// Who a flash just reached, and where an attack's light goes, until the world has seen them.
    pub blinds: Option<usize>,
    pub light_at: Option<Vec3>,
    /// How many of its flashes whitened the player's view.
    pub blinded: u32,
    /// SF_CSCANNER_STRIDER_SCOUT: its attacks come twice as often.
    pub strider_scout: bool,
    /// Everything a photograph needs but the time, from the last steer.
    pub can_photograph: bool,
    /// m_flGoalOverrideDistance.
    pub distance_override: f32,
    /// Its dive, and that its death has been looked at.
    pub dive: Option<Dive>,
    /// Hurt enough to trail smoke, which then lasts as long as it does.
    pub smoking: bool,
    pub death_seen: bool,
    /// The path_corner it is flying to, and that its `target` key has been read.
    pub corner: Option<usize>,
    pub path_read: bool,
    /// Whether the way to its enemy is blocked, from the last steer.
    pub fly_blocked: Option<bool>,
    /// The share of its health left, and the health that is all of it.
    pub health: f32,
    pub max_health: f32,
    /// The noise's three rates (SetNoiseMod).
    pub bob: Vec3,
    /// Its pose from the last think, the one before it, and when the last think was.
    pub pose: Pose,
    #[serde(skip)]
    pose_before: Pose,
    pose_at: f64,
    /// The yaw it turns to and the lean it eases to; the lean it has.
    pub look_yaw: Option<f32>,
    pub lean: f32,
    pub roll: f32,
    pub next_steer: f64,
    pub last_step: f64,
    /// The engine sound runs, and the speed share it was last set for.
    pub engine_on: bool,
    engine_share: f32,
    /// SF_NPC_GAG.
    gagged: bool,
    seed: u64,
}
impl Scanner {
    pub fn new(gagged: bool, seed: u64) -> Self {
        let mut scanner = Self {
            tolerance: END_TOLERANCE,
            health: 1.,
            spotlight_reach: spotlight::DEFAULT_REACH,
            gagged,
            seed: seed | 1,
            ..Default::default()
        };
        let across = |roll: f32| roll * 4. - 2.;
        let x = across(scanner.roll());
        let y = across(scanner.roll());
        let z = 2. + 2. * scanner.roll();
        let sign = if scanner.roll() < 0.5 { 1. } else { -1. };
        scanner.bob = Vec3::new(x, y, z * sign);
        scanner
    }
    pub fn roll(&mut self) -> f32 {
        super::roll(&mut self.seed)
    }
    /// The next number of its own dice.
    pub fn dice(&mut self) -> f32 {
        self.roll()
    }
    /// Told to inspect or follow something: whatever it was flying to is dropped.
    pub fn inspect_target(&mut self, subject: usize, mode: Mode, now: f64) {
        self.inspect = Some(subject);
        self.inspect_end = now + INSPECT_LENGTH;
        self.mode = mode;
        self.new_target = true;
        self.photo_taken = self.photo_taken && mode != Mode::Photo;
        self.drop_route();
    }
    /// Whether it should look for someone now: nothing to do and its time has come.
    pub fn search_due(&mut self, now: f64) -> bool {
        let due = *self.next_search.get_or_insert(now + inspect::SEARCH_DELAY);
        now > due && self.inspect.is_none() && self.enemy_at.is_none() && !self.ignores_enemies
    }
    /// It found someone: mostly a photograph, sometimes the spotlight.
    pub fn found(&mut self, subject: usize, now: f64) {
        let spot = self.roll() < inspect::SPOT_SHARE;
        let mode = if spot { Mode::Spot } else { Mode::Photo };
        self.inspect_target(subject, mode, now);
    }
    /// InputClearFollowTarget.
    pub fn clear_follow(&mut self) {
        self.inspect = None;
        self.mode = Mode::Patrol;
    }
    fn drop_route(&mut self) {
        self.route.clear();
        self.corner = None;
    }
    /// Event_Killed: high above its enemy and far from it, it dives; true when it does.
    pub fn start_dive(&mut self, at: Vec3) -> bool {
        let Some(enemy) = self.enemy_at else {
            return false;
        };
        let above = at - enemy;
        if above.z <= DIVE_HEIGHT || above.length() <= DIVE_DISTANCE {
            return false;
        }
        let (least, most) = DIVE_SPIN;
        let sign = if self.roll() < 0.5 { 1. } else { -1. };
        let spin = sign * (least + (most - least) * self.roll());
        let direction = -above.normalize();
        self.dive = Some(Dive { direction, spin });
        self.drop_route();
        true
    }
    /// The inspection's part of GatherConditions; with an enemy none of it applies. The time
    /// runs only once it is there: a photograph not yet taken or a route not finished keep it
    /// full. A followed target has no end.
    pub fn sense_inspection(&mut self, conditions: &mut Conditions, enemy: bool, now: f64) {
        if std::mem::take(&mut self.new_target) {
            conditions.set(COND_HAVE_TARGET);
        }
        if enemy {
            return;
        }
        let waiting_for_photo = self.mode == Mode::Photo && !self.photo_taken;
        let on_the_way = self.inspect.is_some() && !self.route.is_empty();
        if waiting_for_photo || on_the_way {
            self.inspect_end = now + INSPECT_LENGTH;
        }
        if self.inspect.is_some() && now > self.inspect_end && self.mode != Mode::Follow {
            conditions.set(COND_INSPECT_DONE);
            self.inspect = None;
            self.next_search = Some(now + inspect::SEARCH_DELAY);
        }
        if self.can_photograph {
            conditions.set(COND_CAN_PHOTOGRAPH);
        }
    }
    /// Whether it is placed for a photograph of what is at `target`: slow enough, in range across
    /// and aimed. How far it is off to one side only counts one way, as in the SDK.
    pub fn photo_placed(&self, at: Vec3, target: Vec3, yaw: f32, target_yaw: f32) -> bool {
        let across = (target - at).truncate().length();
        self.mode == Mode::Photo
            && self.inspect.is_some()
            && self.hover.velocity.length() < PHOTO_SPEED
            && (PHOTO_NEAR..PHOTO_FAR).contains(&across)
            && across > PHOTO_NEAR
            && super::host::angle_diff(yaw, target_yaw) < PHOTO_AIM
    }
    /// The order of its choices. `health` is the share it has left, `roll` in [0, 1). An alert
    /// scanner in good health does not flinch: its model has no ACT_SMALL_FLINCH.
    pub fn plan(&self, npc: &AiNpc, ctx: &Context, health: f32, roll: f32) -> Plan {
        if npc.state == NpcState::Script {
            return Plan::Script;
        }
        let hurt = npc.conditions.has(COND_LIGHT_DAMAGE) || npc.conditions.has(COND_HEAVY_DAMAGE);
        if hurt {
            match npc.state {
                NpcState::Idle => return Plan::Flinch,
                NpcState::Alert if health < COVER_HEALTH => return Plan::TakeCover,
                NpcState::Alert => {}
                _ if roll < COMBAT_FLINCH => return Plan::Flinch,
                _ => {}
            }
        }
        let Some((_, enemy)) = ctx.enemy.filter(|_| !self.ignores_enemies) else {
            return self.plan_without_enemy(npc);
        };
        if npc.conditions.has(COND_LOST_ENEMY) {
            return Plan::Patrol;
        }
        if npc.conditions.has(COND_FLY_BLOCKED) {
            return Plan::Chase;
        }
        if ctx.now < self.next_attack {
            return Plan::SpotlightHover;
        }
        if npc.conditions.has(COND_CAN_MELEE_ATTACK1) {
            return Plan::Melee;
        }
        let across = ctx.enemy_lkp.unwrap_or(enemy) - ctx.origin;
        if Vec3::new(across.x, across.y, 0.).length() > ATTACK_FAR {
            Plan::SpotlightHover
        } else {
            Plan::AttackHover
        }
    }
    /// Something to inspect, a path, or a patrol.
    fn plan_without_enemy(&self, npc: &AiNpc) -> Plan {
        if self.inspect.is_none() {
            return if self.corner.is_some() {
                Plan::Corners
            } else {
                Plan::Patrol
            };
        }
        if npc.conditions.has(COND_FLY_BLOCKED) {
            return Plan::MoveToInspect;
        }
        match self.mode {
            Mode::Spot => Plan::SpotlightHover,
            Mode::Photo if npc.conditions.has(COND_CAN_PHOTOGRAPH) => Plan::Photograph,
            Mode::Photo => Plan::PhotographHover,
            Mode::Follow => Plan::FollowHover,
            Mode::Patrol | Mode::Chase | Mode::Attack => Plan::Patrol,
        }
    }
    /// The point of its route to fly at, past those it has reached.
    pub fn next_leg(&mut self, at: Vec3) -> Option<Vec3> {
        while let Some(&point) = self.route.first() {
            let last = self.route.len() == 1;
            let tolerance = if last { self.tolerance } else { LEG_TOLERANCE };
            if at.distance(point) > tolerance {
                return Some(point);
            }
            self.route.remove(0);
        }
        None
    }
    /// Where it wants to hover in the mode it is in: above what it is after and short of it on the
    /// side it comes from, or on the side that thing marks. A suit brings it closer.
    pub fn hover_point(&self, at: Vec3) -> Option<Vec3> {
        let suit = if self.enemy_in_suit { 0.5 } else { 1. };
        let (range, height) = match self.mode {
            Mode::Spot => (SPOT_RANGE * suit, SPOT_HEIGHT * suit),
            Mode::Follow => (FOLLOW_RANGE, SPOT_HEIGHT * suit),
            Mode::Photo => ((PHOTO_NEAR + PHOTO_FAR) * 0.5, PHOTO_HEIGHT),
            Mode::Attack => ((ATTACK_NEAR + ATTACK_FAR) * 0.5 * suit, ATTACK_NEAR),
            Mode::Patrol | Mode::Chase => return None,
        };
        let target = match self.mode {
            Mode::Attack => self.enemy_at,
            _ => self.inspect_at.or(self.enemy_at),
        }?;
        let range = if self.distance_override != 0. {
            self.distance_override
        } else {
            range
        };
        let toward = (target - at).truncate().normalize_or_zero().extend(0.);
        let side = self.inspect_facing.unwrap_or(toward);
        Some(target + Vec3::Z * height - side * range)
    }
    /// A think's pose: what it watches from `eyes`, its climb and speed, and whether it is on
    /// its guard, which opens its flaps.
    pub fn think_pose(&mut self, body: Vec3, eyes: Vec3, alert: bool, now: f64) {
        self.pose_before = self.pose;
        self.pose_at = now;
        let watched = self.inspect_at.or(self.enemy_eye);
        self.pose.watch(body, watched.map(|at| at - eyes));
        self.pose.fly(self.hover.velocity, Some(MAX_SPEED));
        self.pose.alert = if alert { 1. } else { 0. };
    }
    /// A dive's pose: only the tail and the dynamo go on.
    pub fn dive_pose(&mut self, now: f64) {
        self.pose_before = self.pose;
        self.pose_at = now;
        self.pose.fly(self.hover.velocity, None);
    }
    /// Its pose at `now`, between the last two thinks.
    pub fn pose_now(&self, now: f64, think: f64) -> Pose {
        let share = ((now - self.pose_at) / think).clamp(0., 1.) as f32;
        self.pose_before.between(&self.pose, share)
    }
    /// How fast the body turns, degrees a second.
    pub fn turn_rate(&self) -> f32 {
        if self.enemy_eye.is_some() {
            ENEMY_TURN_RATE
        } else {
            TURN_RATE
        }
    }
    /// This think's chirp, if it makes one.
    pub fn idle_cue(&mut self, combat: bool) -> Option<&'static str> {
        if self.gagged || self.roll() >= IDLE_CHANCE {
            return None;
        }
        Some(if combat { COMBAT } else { IDLE })
    }
    /// PlayFlySound: the engine starts once, then its pitch and volume follow the speed.
    pub fn engine_sound(&mut self) {
        let share = self.hover.velocity.length() / MAX_SPEED;
        if !self.engine_on {
            self.engine_on = true;
            let op = PatchOp::Play {
                volume: 0.,
                pitch: 100.,
            };
            self.requests.push(CopRequest::Patch { cue: ENGINE, op });
        } else if (share - self.engine_share).abs() < ENGINE_STEP {
            return;
        }
        self.engine_share = share;
        let seconds = 0.1;
        let pitch = (80. + 20. * share).min(255.);
        let volume = 0.25 + 0.75 * share;
        for op in [
            PatchOp::Pitch { pitch, seconds },
            PatchOp::Volume { volume, seconds },
        ] {
            self.requests.push(CopRequest::Patch { cue: ENGINE, op });
        }
    }
    /// The engine sound's end, for a host that stops it.
    pub fn engine_cue() -> &'static str {
        ENGINE
    }
    pub fn sound_interests(&self) -> u32 {
        sound::WORLD | sound::COMBAT | sound::PLAYER | sound::DANGER
    }
    fn set_mode(&mut self, mode: Mode) -> TaskStatus {
        self.mode = mode;
        self.drop_route();
        Complete
    }
    /// TakePhoto: the sound, the output, the eye opening and the flash on a player. What it does
    /// to an NPC it catches is not modeled yet.
    fn take_photo(&mut self, npc: &mut AiNpc, motor: &mut dyn Motor) {
        self.requests.push(CopRequest::Sound(TAKE_PHOTO));
        self.requests
            .push(CopRequest::Output(photo_output(self.inspect)));
        self.blinds = self.inspect.filter(|&subject| subject == PLAYER);
        npc.activity = FLARE.into();
        motor.set_activity(FLARE);
        self.photo_taken = true;
        self.eye = Some(Eye::PHOTO);
    }
    /// One beat of the red warning: the eye changes and the shutter clicks.
    fn warn(&mut self) {
        self.requests.push(CopRequest::Sound(TAKE_PHOTO));
        self.eye = match self.eye {
            Some(_) => None,
            None => Some(Eye::WARNING),
        };
    }
    /// The flash: its sound, the white eye, a light at its enemy and the output.
    fn attack_flash(&mut self, ctx: &Context) {
        self.requests.push(CopRequest::Sound(ATTACK_FLASH));
        self.eye = Some(Eye::ATTACK);
        if let Some((enemy, at)) = ctx.enemy {
            self.light_at = ctx.enemy_lkp.or(Some(at));
            self.requests
                .push(CopRequest::Output(photo_output(Some(enemy))));
        }
        self.photo_wait = ctx.now + flash::BLIND_AFTER;
    }
    /// The flash reaches its enemy, the eye closes and the next attack is set.
    fn end_flash(&mut self, ctx: &Context) -> TaskStatus {
        self.blinds = ctx.enemy.map(|(enemy, _)| enemy);
        self.eye = None;
        let (least, most) = flash::ATTACK_DELAY;
        let delay = least + (most - least) * f64::from(self.roll());
        let share = if self.strider_scout { 0.5 } else { 1. };
        self.next_attack = ctx.now + delay * share;
        self.next_spotlight = ctx.now + spotlight::OFF_AFTER_FLASH;
        Complete
    }
    /// A photograph's steps: the flash goes off, then another comes unless the time is up.
    fn run_photograph(
        &mut self,
        npc: &mut AiNpc,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> TaskStatus {
        if ctx.now < self.photo_wait {
            return Running;
        }
        if self.eye.take().is_none() {
            self.take_photo(npc, motor);
            self.photo_wait = ctx.now + FLASH;
            return Running;
        }
        if ctx.now > self.inspect_end {
            self.inspect = None;
            return Complete;
        }
        let (least, most) = NEXT_PHOTO;
        self.photo_wait = ctx.now + least + (most - least) * f64::from(self.roll());
        Running
    }
    /// PrescheduleThink's return to idling: a played flare closes through retract, then idles.
    fn settle_flare(npc: &mut AiNpc, motor: &mut dyn Motor) {
        let next = match npc.activity.as_str() {
            FLARE => RETRACT,
            RETRACT => "ACT_IDLE",
            _ => return,
        };
        if motor.activity_finished() {
            npc.activity = next.into();
            motor.set_activity(next);
        }
    }
    fn follow(&mut self, route: Option<Vec<Vec3>>) -> TaskStatus {
        match route {
            Some(route) => {
                self.route = route;
                Complete
            }
            None => Failed,
        }
    }
    fn flight(&self) -> TaskStatus {
        if self.route.is_empty() && self.corner.is_none() {
            Complete
        } else {
            Running
        }
    }
}

/// The output a photograph of `subject` fires.
fn photo_output(subject: Option<usize>) -> &'static str {
    match subject {
        Some(PLAYER) => "OnPhotographPlayer",
        _ => "OnPhotographNPC",
    }
}

/// How far the body leans toward what it flies at: the more to one side and the faster, the more.
pub fn lean(right: Vec3, to_target: Vec3, speed: f32) -> f32 {
    let share = simple_spline((speed / MAX_SPEED).clamp(0., 1.));
    LEAN * right.dot(to_target.normalize_or_zero()) * share
}

impl Behavior for Scanner {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        // A schedule that ends or is interrupted leaves no eye lit.
        self.eye = None;
        if let Some((enemy, _)) = ctx.enemy.filter(|_| !self.ignores_enemies) {
            self.inspect = Some(enemy);
            self.inspect_end = ctx.now + ENEMY_INSPECT;
        } else if self.inspect.is_some() && self.mode == Mode::Chase {
            self.mode = if self.roll() < 0.5 {
                Mode::Spot
            } else {
                Mode::Photo
            };
        }
        match self.plan(npc, ctx, self.health, ctx.random) {
            Plan::Script => base_select_schedule(npc, ctx, false),
            Plan::Flinch => "SCHED_SMALL_FLINCH".into(),
            // SCHED_TAKE_COVER_FROM_ORIGIN looks for cover among nodes this flight has no search
            // for yet: it flies off on patrol instead.
            Plan::TakeCover | Plan::Patrol => PATROL.into(),
            Plan::Chase => CHASE.into(),
            Plan::SpotlightHover => SPOTLIGHT_HOVER.into(),
            Plan::AttackHover => ATTACK_HOVER.into(),
            Plan::Melee if self.roll() < 0.5 => ATTACK.into(),
            Plan::Melee => CHASE.into(),
            Plan::Corners => CORNERS.into(),
            Plan::MoveToInspect => MOVE_TO_INSPECT.into(),
            Plan::PhotographHover => PHOTOGRAPH_HOVER.into(),
            Plan::Photograph => PHOTOGRAPH.into(),
            Plan::FollowHover => FOLLOW_HOVER.into(),
        }
    }
    fn translate_schedule(&mut self, name: &str, _npc: &AiNpc, _ctx: &Context) -> String {
        match name {
            "SCHED_IDLE_STAND" => PATROL.into(),
            other => other.into(),
        }
    }
    /// The class's conditions in the schedules that listen for them.
    fn custom_interrupts(&self, npc: &AiNpc) -> Conditions {
        match npc.schedule.as_deref() {
            Some(PATROL | CORNERS) => Conditions::of(&[COND_HAVE_TARGET]),
            Some(ATTACK_HOVER | FOLLOW_HOVER) => Conditions::of(&[COND_FLY_BLOCKED]),
            Some(SPOTLIGHT_HOVER) => Conditions::of(&[COND_FLY_BLOCKED, COND_INSPECT_DONE]),
            Some(PHOTOGRAPH_HOVER) => {
                Conditions::of(&[COND_FLY_BLOCKED, COND_INSPECT_DONE, COND_CAN_PHOTOGRAPH])
            }
            Some(PHOTOGRAPH) => Conditions::of(&[COND_INSPECT_DONE]),
            Some(CHASE | MOVE_TO_INSPECT) => Conditions::of(&[COND_FLY_CLEAR]),
            _ => Conditions::default(),
        }
    }
    fn start_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        argument: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        Some(match task {
            "TASK_SCANNER_SET_FLY_PATROL" => self.set_mode(Mode::Patrol),
            "TASK_SCANNER_SET_FLY_CHASE" => self.set_mode(Mode::Chase),
            "TASK_SCANNER_SET_FLY_ATTACK" => self.set_mode(Mode::Attack),
            "TASK_CSCANNER_SET_FLY_SPOT" => self.set_mode(Mode::Spot),
            "TASK_CSCANNER_SET_FLY_PHOTO" => {
                self.photo_taken = false;
                self.next_spotlight = ctx.now + spotlight::OFF_FOR_PHOTO;
                self.set_mode(Mode::Photo)
            }
            "TASK_CSCANNER_CLEAR_INSPECT_TARGET" => {
                self.inspect = None;
                Complete
            }
            "TASK_CSCANNER_PHOTOGRAPH" => {
                self.take_photo(npc, motor);
                self.photo_wait = ctx.now + FLASH;
                Running
            }
            // A task also runs in the pass that starts it in the SDK, and only from the next
            // think here: the first think has both beats.
            "TASK_CSCANNER_ATTACK_PRE_FLASH" => {
                self.warn();
                self.warn();
                self.photo_wait = ctx.now + flash::WARNING;
                Running
            }
            "TASK_CSCANNER_ATTACK_FLASH" => {
                self.attack_flash(ctx);
                Running
            }
            "TASK_CSCANNER_GET_PATH_TO_INSPECT_TARGET" => {
                let route = self.inspect_at.and_then(|goal| motor.fly_route(goal));
                self.follow(route)
            }
            "TASK_SET_TOLERANCE_DISTANCE" => {
                self.tolerance = argument.number();
                Complete
            }
            "TASK_GET_PATH_TO_RANDOM_NODE" => {
                let route = motor.fly_patrol(argument.number(), &mut self.seed);
                self.follow(route)
            }
            "TASK_GET_PATH_TO_ENEMY" => {
                let goal = ctx.enemy_lkp.or(ctx.enemy.map(|(_, at)| at));
                self.follow(goal.and_then(|goal| motor.fly_route(goal)))
            }
            "TASK_RUN_PATH" => {
                npc.activity = "ACT_RUN".into();
                motor.set_activity("ACT_RUN");
                Complete
            }
            "TASK_STOP_MOVING" => {
                self.drop_route();
                Complete
            }
            // The model has no flinch to play.
            "TASK_SMALL_FLINCH" => Complete,
            "TASK_WAIT_FOR_MOVEMENT" => self.flight(),
            _ => return None,
        })
    }
    fn run_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        _value: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        Self::settle_flare(npc, motor);
        match task {
            "TASK_WAIT_FOR_MOVEMENT" => Some(self.flight()),
            "TASK_CSCANNER_PHOTOGRAPH" => Some(self.run_photograph(npc, ctx, motor)),
            "TASK_CSCANNER_ATTACK_PRE_FLASH" => {
                self.warn();
                Some(if ctx.now < self.photo_wait {
                    Running
                } else {
                    Complete
                })
            }
            "TASK_CSCANNER_ATTACK_FLASH" if ctx.now < self.photo_wait => Some(Running),
            "TASK_CSCANNER_ATTACK_FLASH" => Some(self.end_flash(ctx)),
            _ => None,
        }
    }
}

pub mod damage;
pub mod flash;
pub mod inspect;
pub mod pose;
pub mod spotlight;
#[cfg(test)]
mod tests;
