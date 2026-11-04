//! CFastZombie (npc_fastzombie; Source SDK 2013 game/server/hl2/npc_fastzombie.cpp) on
//! top of the zombie AI (DESIGN 14o session 43 fast zombie plan): its leap and claw
//! rules, schedules and tasks, its two sound patches and what it does around navigation
//! climbs (npc::links moves it). The torso (no fast_torso.mdl in HL2) is not modeled.
use super::super::conditions::{
    Cond, COND_CAN_MELEE_ATTACK1, COND_CAN_RANGE_ATTACK1, COND_ENEMY_UNREACHABLE, COND_LOST_ENEMY,
    COND_NOT_FACING_ATTACK, COND_TOO_FAR_TO_ATTACK,
};
use super::super::metropolice::CopRequest;
use super::super::schedule::{
    AiNpc, Context, Goal, Motor, Schedule, Schedules, TaskArg, TaskStatus,
};
use super::super::state::NpcState;
use super::Zombie;
use crate::sounds::PatchOp;
use glam::Vec3;
use serde::Serialize;

/// FASTZOMBIE_MAXLEAP_Z: no leap at an enemy this much higher or lower.
pub const MAX_LEAP_Z: f32 = 128.;
/// FASTZOMBIE_MINLEAP and FASTZOMBIE_MAXLEAP (InnateRange1MaxRange).
pub const LEAP_RANGE: (f32, f32) = (200., 300.);
/// RangeAttack1Conditions' facing cone and LeapAttack's speed clamp.
pub const LEAP_CONE: f32 = 0.8;
pub const LEAP_SPEED_CLAMP: f32 = 1000.;
/// GetClawAttackRange, the claws' damage (AE_ZOMBIE_ATTACK_RIGHT/LEFT) and the leap's.
pub const CLAW_REACH: f32 = 50.;
pub const CLAW_DAMAGE: f32 = 3.;
pub const LEAP_CLAW_DAMAGE: f32 = 5.;
/// The moan's pitch: FASTZOMBIE_IDLE_PITCH, MIN/MAX_PITCH, FASTZOMBIE_EXCITE_DIST and
/// FASTZOMBIE_SOUND_UPDATE_FREQ.
pub const IDLE_PITCH: f32 = 35.;
pub const MIN_PITCH: f32 = 70.;
pub const MAX_PITCH: f32 = 130.;
pub const EXCITE_DIST: f32 = 480.;
pub const SOUND_UPDATE_FREQ: f64 = 0.5;
/// m_pMoanSound (pMoanSounds) and m_pLayer2.
pub const MOAN: &str = "NPC_FastZombie.Moan1";
pub const GURGLE: &str = "NPC_FastZombie.Gurgle";
pub const AE_LEAP: &str = "AE_FASTZOMBIE_LEAP";
pub const AE_CLIMB: [&str; 2] = ["AE_FASTZOMBIE_CLIMB_LEFT", "AE_FASTZOMBIE_CLIMB_RIGHT"];
/// sv_gravity.
const SV_GRAVITY: f32 = 600.;
/// CFastZombie::Precache's cues.
pub const SOUNDS: &[&str] = &[
    "NPC_FastZombie.LeapAttack",
    "NPC_FastZombie.FootstepRight",
    "NPC_FastZombie.FootstepLeft",
    "NPC_FastZombie.AttackHit",
    "NPC_FastZombie.AttackMiss",
    "NPC_FastZombie.Attack",
    "NPC_FastZombie.Idle",
    "NPC_FastZombie.AlertFar",
    "NPC_FastZombie.AlertNear",
    "NPC_FastZombie.GallopLeft",
    "NPC_FastZombie.GallopRight",
    "NPC_FastZombie.Scream",
    "NPC_FastZombie.RangeAttack",
    "NPC_FastZombie.Frenzy",
    "NPC_FastZombie.Die",
    GURGLE,
    MOAN,
];
const SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_FASTZOMBIE_RANGE_ATTACK1",
        "Tasks TASK_PLAY_SEQUENCE ACTIVITY:ACT_RANGE_ATTACK1 \
         TASK_SET_ACTIVITY ACTIVITY:ACT_FASTZOMBIE_LEAP_STRIKE TASK_RANGE_ATTACK1 0 \
         TASK_WAIT 0.1 TASK_FASTZOMBIE_LAND_RECOVER 0 TASK_FACE_ENEMY 0 Interrupts",
    ),
    // Ours: Source's navigator plans from mid-air, the movement controller needs the ground.
    (
        "SCHED_FASTZOMBIE_LAND_WAIT",
        "Tasks TASK_FASTZOMBIE_LAND_WAIT 0 Interrupts",
    ),
    (
        "SCHED_FASTZOMBIE_CLIMBING_UNSTICK_JUMP",
        "Tasks TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_FASTZOMBIE_UNSTICK_JUMP 0 Interrupts",
    ),
    (
        "SCHED_FASTZOMBIE_UNSTICK_JUMP",
        "Tasks TASK_FASTZOMBIE_UNSTICK_JUMP 0 Interrupts",
    ),
    (
        "SCHED_FASTZOMBIE_MELEE_ATTACK1",
        "Tasks TASK_STOP_MOVING 0 TASK_FACE_ENEMY 0 TASK_MELEE_ATTACK1 0 \
         TASK_MELEE_ATTACK1 0 TASK_PLAY_SEQUENCE ACTIVITY:ACT_FASTZOMBIE_FRENZY \
         TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_CHASE_ENEMY TASK_FASTZOMBIE_VERIFY_ATTACK 0 \
         TASK_PLAY_SEQUENCE_FACE_ENEMY ACTIVITY:ACT_FASTZOMBIE_BIG_SLASH Interrupts \
         COND_NEW_ENEMY COND_ENEMY_DEAD COND_ENEMY_OCCLUDED",
    ),
];
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
        let schedule = Schedule::parse(name, text).expect("fast zombie schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// CFastZombie's state beside the zombie's.
#[derive(Clone, Debug, Serialize)]
pub struct FastState {
    /// m_flNextMeleeAttack and m_flNextAttack (the leap).
    pub next_melee_attack: f64,
    pub next_attack: f64,
    /// m_fHasScreamed: the over-the-top scream once per combat state.
    pub has_screamed: bool,
    /// m_fJustJumped (BeginAttackJump; TASK_WAIT_FOR_MOVEMENT clears it).
    pub just_jumped: bool,
    /// LeapAttackTouch is set, and when the last leap left the ground.
    pub leap_touch: bool,
    pub leap_time: f64,
    /// SoundInit ran (PostNPCInit), m_flDistFactor, m_flTimeUpdateSound and the state the
    /// last think saw (OnStateChange).
    pub sound_init: bool,
    pub dist_factor: f32,
    pub next_sound_update: f64,
    pub last_state: NpcState,
    /// Leaps and leap touches that clawed (diagnostics).
    pub leaps: u32,
    pub leap_claws: u32,
    /// In a navigation climb, ClimbTouch set (SetTouch), the player touched it this climb
    /// (COND_FASTZOMBIE_CLIMB_TOUCH) and m_iClimbCount.
    pub climbing: bool,
    pub climb_touch_armed: bool,
    pub climb_touch: bool,
    pub climb_count: u8,
}
impl Default for FastState {
    fn default() -> Self {
        Self {
            next_melee_attack: 0.,
            next_attack: 0.,
            has_screamed: false,
            just_jumped: false,
            leap_touch: false,
            leap_time: f64::NEG_INFINITY,
            sound_init: false,
            dist_factor: 1.,
            next_sound_update: 0.,
            last_state: NpcState::None,
            leaps: 0,
            leap_claws: 0,
            climbing: false,
            climb_touch_armed: false,
            climb_touch: false,
            climb_count: 0,
        }
    }
}

/// CFastZombie::MaxYawSpeed.
pub fn max_yaw_speed(activity: &str) -> f32 {
    match activity {
        "ACT_TURN_LEFT" | "ACT_TURN_RIGHT" => 120.,
        "ACT_RUN" => 160.,
        "ACT_WALK" | "ACT_IDLE" => 25.,
        _ => 20.,
    }
}
/// The fast zombie's cue for a CZombie cue name; "" where it is silent (FootscuffSound,
/// PainSound only envelopes its patches).
pub fn cue(classic: &'static str) -> &'static str {
    match classic {
        "Zombie.FootstepLeft" => "NPC_FastZombie.FootstepLeft",
        "Zombie.FootstepRight" => "NPC_FastZombie.FootstepRight",
        "Zombie.Attack" => "NPC_FastZombie.Attack",
        "Zombie.AttackHit" => "NPC_FastZombie.AttackHit",
        "Zombie.AttackMiss" => "NPC_FastZombie.AttackMiss",
        "Zombie.Idle" => "NPC_FastZombie.Idle",
        "Zombie.Alert" => "NPC_FastZombie.AlertNear",
        "Zombie.Die" => "NPC_FastZombie.Die",
        _ => "",
    }
}
/// CFastZombie::AlertSound: how far the player is decides the cue.
pub fn alert_cue(player_distance: Option<f32>) -> &'static str {
    if player_distance.is_some_and(|d| d > 512.) {
        "NPC_FastZombie.AlertFar"
    } else {
        "NPC_FastZombie.AlertNear"
    }
}
/// AE_ZOMBIE_ATTACK_RIGHT/LEFT: the view punch (Source degrees) and velocity punch from
/// the facing yaw (degrees).
pub fn claw_punch(event: &str, yaw: f32) -> Option<(Vec3, Vec3)> {
    let y = yaw.to_radians();
    let right = Vec3::new(y.sin(), -y.cos(), 0.);
    match event.to_ascii_uppercase().as_str() {
        "AE_ZOMBIE_ATTACK_RIGHT" => Some((Vec3::new(-3., -5., -3.), right * -50.)),
        "AE_ZOMBIE_ATTACK_LEFT" => Some((Vec3::new(-3., 5., -3.), right * 50.)),
        _ => None,
    }
}
/// CFastZombie::MeleeAttack1Conditions over the base zombie's result: only on the ground
/// once m_flNextMeleeAttack has passed, and never the base's complaints.
pub fn melee_condition(base: Cond, on_ground: bool, ready: bool) -> Option<Cond> {
    if !on_ground || !ready || matches!(base, COND_TOO_FAR_TO_ATTACK | COND_NOT_FACING_ATTACK) {
        None
    } else {
        Some(base)
    }
}
/// What RangeAttack1Conditions reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeapCheck {
    pub on_ground: bool,
    /// m_flNextAttack has passed.
    pub ready: bool,
    /// |enemy origin z - my origin z|.
    pub height: f32,
    /// EnemyDistance and the 2D facing dot (GatherAttackConditions).
    pub distance: f32,
    pub dot: f32,
    /// IsMoving: a navigation goal is active.
    pub moving: bool,
    /// The enemy's FInViewCone(me).
    pub seen: bool,
    /// A 32-unit cube from my center halfway to the enemy's center hits nothing.
    pub clear: bool,
}
/// CFastZombie::RangeAttack1Conditions.
pub fn range_condition(c: LeapCheck) -> Option<Cond> {
    if !c.on_ground || !c.ready {
        None
    } else if c.height > MAX_LEAP_Z || c.distance > LEAP_RANGE.1 {
        Some(COND_TOO_FAR_TO_ATTACK)
    } else if c.distance < LEAP_RANGE.0 || c.dot < LEAP_CONE || !c.moving || !c.seen || !c.clear {
        None
    } else {
        Some(COND_CAN_RANGE_ATTACK1)
    }
}
/// CFastZombie::LeapAttack's velocity from the origin (already 1 unit up) to the enemy's
/// center: the apex height clamped to 16..120, its time of flight for the sideways speed,
/// the whole clamped to 1000.
pub fn leap_velocity(origin: Vec3, enemy_center: Vec3, gravity: f32) -> Vec3 {
    let gravity = gravity.max(1.);
    let height = (enemy_center.z - origin.z).clamp(16., 120.);
    let speed = (2. * gravity * height).sqrt();
    let mut velocity = (enemy_center - origin) / (speed / gravity);
    velocity.z = speed;
    let length = velocity.length();
    if length > LEAP_SPEED_CLAMP {
        velocity *= LEAP_SPEED_CLAMP / length;
    }
    velocity
}
/// PrescheduleThink's moan pitch from the enemy's distance less the hull width (None:
/// calm): back to the idle pitch once, or 70..130 as it closes in.
pub fn moan_pitch(dist_factor: &mut f32, distance: Option<f32>) -> Option<f32> {
    let distance = distance.unwrap_or(EXCITE_DIST);
    if distance >= EXCITE_DIST {
        (*dist_factor != 1.).then(|| {
            *dist_factor = 1.;
            IDLE_PITCH
        })
    } else {
        *dist_factor = (1. - distance / EXCITE_DIST).min(1.);
        Some((MIN_PITCH + (MAX_PITCH - MIN_PITCH) * *dist_factor).trunc())
    }
}

impl Zombie {
    fn patch(&mut self, cue: &'static str, op: PatchOp) {
        self.requests.push(CopRequest::Patch { cue, op });
    }
    /// SetIdleSoundState: the moan to the idle pitch and 0.75, the gurgle silent.
    fn idle_sound_state(&mut self) {
        if self.moan_patch.is_some() {
            self.patch(
                MOAN,
                PatchOp::Pitch {
                    pitch: IDLE_PITCH,
                    seconds: 1.,
                },
            );
            self.patch(
                MOAN,
                PatchOp::Volume {
                    volume: 0.75,
                    seconds: 1.,
                },
            );
        }
        self.quiet_gurgle();
    }
    /// The gurgle back to pitch 100 and silence over 1 s (both sound states).
    fn quiet_gurgle(&mut self) {
        let ops = [
            PatchOp::Pitch {
                pitch: 100.,
                seconds: 1.,
            },
            PatchOp::Volume {
                volume: 0.,
                seconds: 1.,
            },
        ];
        for op in ops {
            self.patch(GURGLE, op);
        }
    }
    /// The fast zombie's sounds each think: SoundInit (PostNPCInit: MoanSound's kick-start
    /// unless gagged, half the time IdleSound, the silent gurgle), OnStateChange (combat:
    /// SetAngrySoundState; idle or alert: the scream again and SetIdleSoundState), the
    /// moan's pitch every 0.5 s, and ShouldPlayIdleSound (idle or alert, 1 in 100).
    pub fn fast_sounds(&mut self, now: f64, state: NpcState, enemy_distance: Option<f32>) {
        if !self.fast.sound_init {
            self.fast.sound_init = true;
            self.fast.next_sound_update = now;
            if !self.gagged {
                self.moan_patch = Some(MOAN);
                self.patch(
                    MOAN,
                    PatchOp::Play {
                        volume: 1.,
                        pitch: 100.,
                    },
                );
                if self.random() < 0.5 {
                    self.requests.push(CopRequest::Sound("NPC_FastZombie.Idle"));
                }
            }
            self.patch(
                GURGLE,
                PatchOp::Play {
                    volume: 0.,
                    pitch: 100.,
                },
            );
            self.idle_sound_state();
        }
        if state != self.fast.last_state {
            self.fast.last_state = state;
            if state == NpcState::Combat && self.moan_patch.is_some() {
                self.requests
                    .push(CopRequest::Sound("NPC_FastZombie.LeapAttack"));
                self.patch(
                    MOAN,
                    PatchOp::Pitch {
                        pitch: MIN_PITCH,
                        seconds: 0.5,
                    },
                );
                self.patch(
                    MOAN,
                    PatchOp::Volume {
                        volume: 1.,
                        seconds: 0.5,
                    },
                );
                self.quiet_gurgle();
            } else if matches!(state, NpcState::Idle | NpcState::Alert) && self.moan_patch.is_some()
            {
                self.fast.has_screamed = false;
                self.idle_sound_state();
            }
        }
        if self.moan_patch.is_some() && now > self.fast.next_sound_update {
            let distance = enemy_distance.filter(|_| state == NpcState::Combat);
            if let Some(pitch) = moan_pitch(&mut self.fast.dist_factor, distance) {
                let seconds = SOUND_UPDATE_FREQ as f32;
                self.patch(MOAN, PatchOp::Pitch { pitch, seconds });
            }
            self.fast.next_sound_update = now + SOUND_UPDATE_FREQ;
        }
        if matches!(state, NpcState::Idle | NpcState::Alert)
            && !self.gagged
            && (self.random() * 100.) < 1.
        {
            self.requests.push(CopRequest::Sound("NPC_FastZombie.Idle"));
        }
    }
    /// CFastZombie::PainSound: the gurgle swells and fades while the moan dips.
    pub fn fast_pain(&mut self) {
        self.patch(
            GURGLE,
            PatchOp::Envelope {
                points: [(1., 0.1), (0., 1.), (0., 0.)],
            },
        );
        if self.moan_patch.is_some() {
            self.patch(
                MOAN,
                PatchOp::Envelope {
                    points: [(0., 0.1), (1., 1.), (1., 0.)],
                },
            );
        }
    }
    /// OnChangeActivity: a climb starting silences the moan and arms ClimbTouch, its end
    /// brings the moan back and disarms it.
    pub fn fast_climb_edge(&mut self, climbing: bool) {
        self.fast.climbing = climbing;
        self.fast.climb_touch_armed = climbing;
        if self.moan_patch.is_some() {
            let volume = if climbing { 0. } else { 1. };
            let seconds = 0.2;
            self.patch(MOAN, PatchOp::Volume { volume, seconds });
        }
    }
    /// HandleAnimEvent's climb steps: every third one the gurgle layer rasps at a new pitch.
    pub(super) fn fast_climb_step(&mut self, roll: f32) {
        self.fast.climb_count = self.fast.climb_count.wrapping_add(1);
        if self.fast.climb_count.is_multiple_of(3) {
            let pitch = 100. + 50. * roll;
            self.patch(GURGLE, PatchOp::Pitch { pitch, seconds: 0. });
            let points = [(1., 0.1), (0., 0.2), (0., 0.)];
            self.patch(GURGLE, PatchOp::Envelope { points });
        }
    }
    /// AE_FASTZOMBIE_LEAP: LeapAttack at the enemy's center (BeginAttackJump, the sound,
    /// the next leap in 2 s).
    pub fn fast_leap(&mut self, now: f64, origin: Vec3, enemy_center: Option<Vec3>) {
        self.fast.just_jumped = true;
        self.requests
            .push(CopRequest::Sound("NPC_FastZombie.LeapAttack"));
        if let Some(target) = enemy_center {
            let velocity = leap_velocity(origin + Vec3::Z, target, SV_GRAVITY);
            self.requests.push(CopRequest::Leap(velocity));
            self.fast.next_attack = now + 2.;
            self.fast.leap_time = now;
            self.fast.leaps += 1;
        }
    }
    /// LeapAttackTouch on any solid: both hands claw (range 50, 5 damage, punch
    /// (15, +-5, +-5), velocity forward x 500) and the touch is cleared.
    pub fn fast_leap_touch(&mut self, center: Vec3, yaw: f32) {
        self.fast.leap_touch = false;
        self.fast.leap_claws += 1;
        let rolls = (self.random(), self.random());
        let y = yaw.to_radians();
        let forward = Vec3::new(y.cos(), y.sin(), 0.);
        self.requests.push(CopRequest::Claw {
            start: center,
            forward,
            punch: Vec3::new(
                15.,
                (rolls.0 * 11.).floor() - 5.,
                (rolls.1 * 11.).floor() - 5.,
            ),
            impulse: forward * 500.,
            reach: CLAW_REACH,
            bite: false,
            damage: Some(LEAP_CLAW_DAMAGE),
        });
    }
    /// OnScheduleChange: melee again within 0.5 s once a leap's hold has been cleared.
    fn fast_schedule_change(&mut self, now: f64) {
        if self.fast.next_melee_attack > now + 1. {
            self.fast.next_melee_attack = now + 0.5;
        }
    }
    /// CFastZombie::SelectSchedule before the base zombie's: a lost (or unreachable) enemy
    /// in combat or alert drops to alert, forgets the enemy and wanders.
    pub(super) fn fast_select(&mut self, npc: &mut AiNpc, ctx: &Context) -> Option<String> {
        self.fast_schedule_change(ctx.now);
        let c = &npc.conditions;
        if c.has(super::COND_FASTZOMBIE_CLIMB_TOUCH) {
            return Some("SCHED_FASTZOMBIE_UNSTICK_JUMP".into());
        }
        // Still falling after a leap: a chase planned now would fail and mark the enemy
        // unreachable, which a path from the air does not do in Source.
        if ctx.airborne {
            return Some("SCHED_FASTZOMBIE_LAND_WAIT".into());
        }
        let lost = c.has(COND_LOST_ENEMY) || c.has(COND_ENEMY_UNREACHABLE);
        if !lost || !matches!(npc.state, NpcState::Combat | NpcState::Alert) {
            return None;
        }
        npc.state = NpcState::Alert;
        npc.conditions.clear(COND_LOST_ENEMY);
        npc.conditions.clear(COND_ENEMY_UNREACHABLE);
        npc.enemy = None;
        Some("SCHED_ZOMBIE_WANDER_MEDIUM".into())
    }
    /// CFastZombie::TranslateSchedule before the base zombie's.
    pub(super) fn fast_translate(&mut self, name: &str, ctx: &Context) -> Option<&'static str> {
        Some(match name {
            "SCHED_RANGE_ATTACK1" => {
                let scream = !std::mem::replace(&mut self.fast.has_screamed, true);
                self.requests.push(CopRequest::Sound(if scream {
                    "NPC_FastZombie.Scream"
                } else {
                    "NPC_FastZombie.RangeAttack"
                }));
                "SCHED_FASTZOMBIE_RANGE_ATTACK1"
            }
            "SCHED_MELEE_ATTACK1" => "SCHED_FASTZOMBIE_MELEE_ATTACK1",
            "SCHED_FASTZOMBIE_UNSTICK_JUMP" if self.fast.climbing => {
                "SCHED_FASTZOMBIE_CLIMBING_UNSTICK_JUMP"
            }
            "SCHED_MOVE_TO_WEAPON_RANGE"
                if ctx
                    .enemy
                    .is_some_and(|(_, e)| (e.z - ctx.origin.z).abs() > MAX_LEAP_Z) =>
            {
                "SCHED_ZOMBIE_CHASE_ENEMY"
            }
            _ => return None,
        })
    }
    /// SelectFailSchedule: a failure right after a jump means a bad landing spot.
    pub(super) fn fast_fail(&mut self, now: f64) -> Option<String> {
        self.fast_schedule_change(now);
        std::mem::take(&mut self.fast.just_jumped).then(|| "SCHED_FASTZOMBIE_UNSTICK_JUMP".into())
    }
    /// CFastZombie::StartTask (None: the base zombie's or the shared one).
    pub(super) fn fast_start_task(
        &mut self,
        npc: &mut AiNpc,
        name: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        Some(match name {
            "TASK_FASTZOMBIE_VERIFY_ATTACK" if npc.conditions.has(COND_CAN_MELEE_ATTACK1) => {
                TaskStatus::Complete
            }
            "TASK_FASTZOMBIE_VERIFY_ATTACK" => TaskStatus::Failed,
            "TASK_FASTZOMBIE_LAND_WAIT" if ctx.airborne => TaskStatus::Running,
            "TASK_FASTZOMBIE_LAND_WAIT" => TaskStatus::Complete,
            // Backwards off the pipe (the activity is still the climb's at this point).
            "TASK_FASTZOMBIE_UNSTICK_JUMP" if self.fast.climbing => {
                self.fast.just_jumped = true;
                let yaw = ctx.yaw.to_radians();
                let forward = Vec3::new(yaw.cos(), yaw.sin(), 0.);
                self.requests.push(CopRequest::Leap(forward * -200.));
                self.fast.leap_time = ctx.now;
                TaskStatus::Running
            }
            // Toward the enemy at 300 and 200 up (on the ground until it lands).
            "TASK_FASTZOMBIE_UNSTICK_JUMP" => match ctx.enemy {
                Some((_, enemy)) => {
                    self.fast.just_jumped = true;
                    let away = (enemy - ctx.origin)
                        .truncate()
                        .normalize_or_zero()
                        .extend(0.);
                    self.requests
                        .push(CopRequest::Leap(away * 300. + Vec3::Z * 200.));
                    self.fast.leap_time = ctx.now;
                    TaskStatus::Running
                }
                None => TaskStatus::Complete,
            },
            "TASK_WAIT_FOR_MOVEMENT" => {
                self.fast.just_jumped = false;
                return None;
            }
            // No turn activity: it would stomp the landing scrabble.
            "TASK_FACE_ENEMY" => match ctx.enemy_lkp.or(ctx.enemy.map(|e| e.1)) {
                Some(p) if motor.face(Goal::Position(p)) => TaskStatus::Complete,
                Some(_) => TaskStatus::Running,
                None => TaskStatus::Failed,
            },
            "TASK_FASTZOMBIE_LAND_RECOVER" => {
                let target = ctx
                    .enemy_lkp
                    .or(ctx.enemy.map(|e| e.1))
                    .unwrap_or(ctx.origin);
                let to = target - ctx.origin;
                let ideal = to.y.atan2(to.x).to_degrees();
                let delta = (ideal - ctx.yaw + 540.).rem_euclid(360.) - 180.;
                let activity = if delta < 0. {
                    "ACT_FASTZOMBIE_LAND_RIGHT"
                } else {
                    "ACT_FASTZOMBIE_LAND_LEFT"
                };
                npc.activity = activity.into();
                motor.set_activity(activity);
                TaskStatus::Complete
            }
            "TASK_RANGE_ATTACK1" => {
                self.fast.next_melee_attack = ctx.now + 60.;
                self.fast.leap_touch = true;
                TaskStatus::Running
            }
            "TASK_SOUND_WAKE" => {
                let distance = ctx.player.map(|p| (p - ctx.origin).length());
                self.requests.push(CopRequest::Sound(alert_cue(distance)));
                TaskStatus::Complete
            }
            "TASK_PLAY_SEQUENCE" if matches!(arg, TaskArg::Activity(a) if a == "ACT_FASTZOMBIE_FRENZY") =>
            {
                self.requests
                    .push(CopRequest::Sound("NPC_FastZombie.Frenzy"));
                return None;
            }
            _ => return None,
        })
    }
    /// CFastZombie::RunTask: the jumps end on the ground (or once the leap's touch fired).
    pub(super) fn fast_run_task(&mut self, task: &str, ctx: &Context) -> Option<TaskStatus> {
        // The leap leaves the ground after this think's requests: give it a moment.
        let landed = !ctx.airborne && ctx.now >= self.fast.leap_time + 0.15;
        match task {
            "TASK_FASTZOMBIE_LAND_WAIT" if ctx.airborne => Some(TaskStatus::Running),
            "TASK_FASTZOMBIE_LAND_WAIT" => Some(TaskStatus::Complete),
            "TASK_FASTZOMBIE_UNSTICK_JUMP" => Some(if landed {
                TaskStatus::Complete
            } else {
                TaskStatus::Running
            }),
            "TASK_RANGE_ATTACK1" if landed || !self.fast.leap_touch => {
                self.fast.next_melee_attack = ctx.now + 0.5;
                Some(TaskStatus::Complete)
            }
            "TASK_RANGE_ATTACK1" => Some(TaskStatus::Running),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Kind;
    use super::*;
    #[test]
    fn fast_zombie_leaps_from_two_to_three_hundred_units_while_seen_and_running() {
        let check = LeapCheck {
            on_ground: true,
            ready: true,
            height: 0.,
            distance: 250.,
            dot: 0.9,
            moving: true,
            seen: true,
            clear: true,
        };
        assert_eq!(range_condition(check), Some(COND_CAN_RANGE_ATTACK1));
        let far = LeapCheck {
            distance: 301.,
            ..check
        };
        assert_eq!(range_condition(far), Some(COND_TOO_FAR_TO_ATTACK));
        let high = LeapCheck {
            height: 129.,
            ..check
        };
        assert_eq!(range_condition(high), Some(COND_TOO_FAR_TO_ATTACK));
        for no in [
            LeapCheck {
                distance: 199.,
                ..check
            },
            LeapCheck { dot: 0.79, ..check },
            LeapCheck {
                moving: false,
                ..check
            },
            LeapCheck {
                seen: false,
                ..check
            },
            LeapCheck {
                clear: false,
                ..check
            },
            LeapCheck {
                on_ground: false,
                ..far
            },
            LeapCheck {
                ready: false,
                ..far
            },
        ] {
            assert_eq!(range_condition(no), None, "{no:?}");
        }
        // The enemy must look toward it (the player's 0.766 = 40 degrees, 2D).
        let viewer = crate::ai::senses::Viewer {
            entity: 0,
            class: crate::ai::relationships::Class::Player,
            eye: Vec3::new(0., 0., 64.),
            facing: Vec3::X,
            field_of_view: 0.766,
            look_distance: f32::MAX,
        };
        use crate::ai::senses::in_view_cone;
        assert!(in_view_cone(&viewer, Vec3::new(100., 80., 0.)));
        assert!(!in_view_cone(&viewer, Vec3::new(100., 90., 0.)));
        // Claws: only on the ground once ready, and without the base's complaints.
        assert_eq!(
            melee_condition(COND_CAN_MELEE_ATTACK1, true, true),
            Some(COND_CAN_MELEE_ATTACK1)
        );
        assert_eq!(melee_condition(COND_TOO_FAR_TO_ATTACK, true, true), None);
        assert_eq!(melee_condition(COND_CAN_MELEE_ATTACK1, false, true), None);
        assert_eq!(melee_condition(COND_CAN_MELEE_ATTACK1, true, false), None);
    }
    #[test]
    fn fast_zombie_leap_reaches_the_enemy_center_at_the_apex_time() {
        let g = 600.;
        let v = leap_velocity(Vec3::ZERO, Vec3::new(250., 0., 36.), g);
        // Apex height 36: up at sqrt(2 g h), there after speed / g.
        let speed = (2. * g * 36f32).sqrt();
        assert!((v.z - speed).abs() < 1e-3);
        assert!((v.x - 250. / (speed / g)).abs() < 1e-2);
        // The apex height is at least 16 and the whole at most 1000.
        let low = leap_velocity(Vec3::ZERO, Vec3::new(100., 0., -50.), g);
        assert!((low.z - (2. * g * 16f32).sqrt()).abs() < 1e-3);
        let far = leap_velocity(Vec3::ZERO, Vec3::new(2000., 0., 0.), g);
        assert!((far.length() - LEAP_SPEED_CLAMP).abs() < 1e-2);
    }
    #[test]
    fn a_falling_fast_zombie_waits_for_the_ground_before_it_plans() {
        let mut z = Zombie::new(super::super::Kind::Fast, false, 1);
        let mut npc = AiNpc::new(1);
        npc.state = NpcState::Combat;
        let mut ctx = Context {
            airborne: true,
            ..Default::default()
        };
        assert_eq!(
            z.fast_select(&mut npc, &ctx).as_deref(),
            Some("SCHED_FASTZOMBIE_LAND_WAIT")
        );
        assert!(matches!(
            z.fast_run_task("TASK_FASTZOMBIE_LAND_WAIT", &ctx),
            Some(TaskStatus::Running)
        ));
        ctx.airborne = false;
        assert!(matches!(
            z.fast_run_task("TASK_FASTZOMBIE_LAND_WAIT", &ctx),
            Some(TaskStatus::Complete)
        ));
        assert_eq!(z.fast_select(&mut npc, &ctx), None);
    }

    #[test]
    fn fast_zombie_moan_pitch_climbs_as_the_enemy_closes() {
        let mut factor = 1.;
        assert_eq!(moan_pitch(&mut factor, None), None);
        assert_eq!(moan_pitch(&mut factor, Some(240.)), Some(100.));
        assert_eq!(moan_pitch(&mut factor, Some(10.)), Some(128.));
        assert_eq!(moan_pitch(&mut factor, Some(600.)), Some(IDLE_PITCH));
        assert_eq!(moan_pitch(&mut factor, Some(600.)), None);
    }
    #[test]
    fn fast_zombie_schedules_parse_and_translate() {
        let mut schedules = Schedules(Default::default());
        add_schedules(&mut schedules);
        let leap = &schedules.0["SCHED_FASTZOMBIE_RANGE_ATTACK1"];
        assert_eq!(leap.tasks.len(), 6);
        let melee = &schedules.0["SCHED_FASTZOMBIE_MELEE_ATTACK1"];
        assert_eq!(melee.tasks.len(), 8);
        let mut z = Zombie::new(Kind::Fast, false, 7);
        let ctx = Context::default();
        assert_eq!(
            z.fast_translate("SCHED_RANGE_ATTACK1", &ctx),
            Some("SCHED_FASTZOMBIE_RANGE_ATTACK1")
        );
        assert_eq!(
            z.fast_translate("SCHED_RANGE_ATTACK1", &ctx),
            Some("SCHED_FASTZOMBIE_RANGE_ATTACK1")
        );
        let cues: Vec<_> = z
            .requests
            .iter()
            .filter_map(|r| match r {
                CopRequest::Sound(c) => Some(*c),
                _ => None,
            })
            .collect();
        assert_eq!(
            cues,
            ["NPC_FastZombie.Scream", "NPC_FastZombie.RangeAttack"]
        );
        assert_eq!(cue("Zombie.ScuffLeft"), "");
        assert_eq!(alert_cue(Some(600.)), "NPC_FastZombie.AlertFar");
        assert_eq!(max_yaw_speed("ACT_RUN"), 160.);
    }

    #[test]
    #[ignore = "needs the owned installation"]
    fn owned_fast_zombie_model_has_its_leap_claws_frenzy_and_climb() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let path = "models/zombie/fast.mdl";
        let names = source_assets::animation::sequence_names(&vfs, path).unwrap();
        let rig =
            source_assets::animation::load(&vfs, path, &names.iter().cloned().collect()).unwrap();
        let events = |activity: &str| {
            let s = rig.sequences.iter().find(|s| s.activity == activity);
            let s = s.unwrap_or_else(|| panic!("{activity}"));
            let clip = &rig.clips[&s.name];
            let events: Vec<_> = clip
                .events
                .iter()
                .map(|e| (e.name.clone(), e.cycle))
                .collect();
            println!(
                "{activity} {} {} frames {events:?}",
                s.name,
                clip.frames.len()
            );
            events
        };
        assert_eq!(events("ACT_RANGE_ATTACK1"), [(AE_LEAP.into(), 0.8)]);
        let claws = events("ACT_MELEE_ATTACK1");
        assert_eq!(claws[0].0, "AE_ZOMBIE_ATTACK_RIGHT");
        assert_eq!(claws[1].0, "AE_ZOMBIE_ATTACK_LEFT");
        let gallop = events("ACT_RUN");
        assert!(gallop.iter().any(|e| e.0 == "AE_FASTZOMBIE_GALLOP_LEFT"));
        for activity in [
            "ACT_FASTZOMBIE_LEAP_STRIKE",
            "ACT_FASTZOMBIE_LAND_RIGHT",
            "ACT_FASTZOMBIE_LAND_LEFT",
            "ACT_FASTZOMBIE_FRENZY",
            "ACT_FASTZOMBIE_BIG_SLASH",
            "ACT_CLIMB_UP",
            "ACT_CLIMB_DISMOUNT",
            "ACT_JUMP",
            "ACT_WALK",
            "ACT_IDLE",
        ] {
            events(activity);
        }
        let eye = source_assets::models::read_eye_position(&vfs, path).unwrap();
        assert_eq!(eye.z, 64.);
        // HL2 ships no fast zombie torso (an episodic model).
        assert!(
            source_assets::animation::sequence_names(&vfs, "models/zombie/fast_torso.mdl").is_err()
        );
    }
}
