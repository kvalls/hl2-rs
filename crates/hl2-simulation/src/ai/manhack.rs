//! npc_manhack (DESIGN 14o session 47, step 1): a flying blade. This module holds what it decides
//! and its flight arithmetic; `runtime::manhacks` gives it the world and carries the body. It flies
//! at its enemy's eyes, bursts from close by, cuts what it touches and bounces off. Packed manhacks
//! (step 2), swarming (step 3), panels and sprites (step 5) and the gravity gun (`physgun`, step 4)
//! followed. Not yet: carried manhacks, a crash in place of gibs, water.
use super::assault::Assault;
use super::conditions::{COND_HEAVY_DAMAGE, COND_LIGHT_DAMAGE};
use super::hover::{Hover, Thrust};
use super::metropolice::CopRequest;
use super::schedule::TaskStatus::{Complete, Running};
use super::schedule::{
    base_select_schedule, AiNpc, Behavior, Context, Motor, Schedule, Schedules, TaskArg, TaskStatus,
};
use super::senses::sound;
use super::state::NpcState;
use crate::sounds::PatchOp;
use glam::Vec3;
use serde::Serialize;

mod damage;
mod physgun;
mod swarm;
pub use damage::CLUB_SHARE;
pub use physgun::Victim;
pub use swarm::{center, member_to_join, Ask, Member, Swarm};

/// VIEW_FIELD_FULL: it sees all round.
pub const FIELD_OF_VIEW: f32 = -1.;
/// SetViewOffset's height.
pub const EYE_HEIGHT: f32 = 10.;
/// MANHACK_MAX_SPEED, and the most it climbs or falls.
pub const MAX_SPEED: f32 = 500.;
pub const MAX_CLIMB: f32 = 200.;
/// Its thrust at a target ahead of it and at one it has to turn for, and upward.
const THRUST_AHEAD: f32 = 300.;
const THRUST_TURNING: f32 = 200.;
const THRUST_UP: f32 = 300.;
const DECAY: f32 = 0.3;
/// The target counts as ahead past this cosine.
const AHEAD: f32 = 0.25;
/// A burst's thrust and most climb, how long it lasts and the pause before another.
const BURST_THRUST: f32 = 400.;
const BURST_UP: f32 = 500.;
const BURST_LENGTH: f64 = 1.;
const BURST_PAUSE: f64 = 2.;
/// A burst's steering: how much of the way to the enemy a think of 0.1 s adds.
const BURST_STEER: f32 = 4.;
/// MANHACK_CHARGE_MIN_DIST, and the cosine of how far off its line a burst may start.
const BURST_RANGE: f32 = 200.;
const BURST_AIM: f32 = 0.75;
/// The noise on its flight, away and close to its enemy, and the noise's rate.
const NOISE_FAR: f32 = 7.;
const NOISE_NEAR: f32 = 2.;
pub const NOISE_RATE: f32 = 5000.;
/// How fast a cut sends it back.
const REBOUND: f32 = 200.;
/// A cut's damage on the player against an NPC, and the least a cut does.
const PLAYER_SHARE: f32 = 2.;
const LEAST_CUT: f32 = 1.;
/// Engine pitch: how near the enemy it starts to rise, and between what values.
const PITCH_NEAR_1: f32 = 512.;
const PITCH_1: (f32, f32) = (100., 160.);
const PITCH_NEAR_2: f32 = 400.;
const PITCH_2: (f32, f32) = (85., 190.);
const ENGINE_VOLUME: f32 = 0.55;
/// A change of pitch smaller than this leaves the engine as it is.
const PITCH_STEP: f32 = 2.;

const ENGINE_1: &str = "NPC_Manhack.EngineSound1";
const ENGINE_2: &str = "NPC_Manhack.EngineSound2";
pub const DIE: &str = "NPC_Manhack.Die";
pub const SLICE: &str = "NPC_Manhack.Slice";
pub const GRIND: &str = "NPC_Manhack.Grind";
/// Every cue this class plays; a cue that is not preloaded is dropped.
const CHARGE_ANNOUNCE: &str = "NPC_Manhack.ChargeAnnounce";
const CHARGE_END: &str = "NPC_Manhack.ChargeEnd";

pub const SOUNDS: &[&str] = &[
    ENGINE_1,
    ENGINE_2,
    DIE,
    SLICE,
    GRIND,
    CHARGE_ANNOUNCE,
    CHARGE_END,
    STUNNED,
];

/// The eye and the light (StartEye, at Activate and when the engine starts): red additive glow1
/// sprites at these attachments, brightness 164 and scale 0.25, lit for as long as it lives. Retail
/// precaches the same sprite.
pub const GLOW_SPRITE: &str = "sprites/glow1";
pub const GLOW_ATTACHMENTS: [&str; 2] = ["Eye", "Light"];
pub const GLOW_SCALE: f32 = 0.25;
pub const GLOW_BRIGHTNESS: f32 = 164. / 255.;

/// The four panels' pose parameters (UpdatePanels): open wide while hostile, else closing by 25
/// degrees a think. Retail reads the same 90 and 25; the owned model's range is 0 to 90.
pub const PANELS: [&str; 4] = ["Panel1", "Panel2", "Panel3", "Panel4"];
pub const GLOW_COLOR: [u8; 3] = [255, 0, 0];

/// The gravity gun (OnPhysGunPickup, PUNTED_BY_CANNON): its VPhysics body is the owned
/// manhack.phy's first solid; the engine stalls for 2 s with the eye stunned (orange, scale
/// 0.15, strobing fast); a hit within 0.35 s of the punt is a smash for max health times speed
/// over 500, at most twice (TakeDamageFromPhyscannon).
pub const PHYSICS_MASS: f32 = 3.693_67;
const STALL: f64 = 2.;
const SMASH_TIME: f64 = 0.35;
const SMASH_SPEED: f32 = 500.;
const STUNNED: &str = "NPC_Manhack.Stunned";
pub const STUNNED_COLOR: [u8; 3] = [255, 128, 0];
pub const STUNNED_SCALE: f32 = 0.15;

/// kRenderFxStrobeFast: lit while sin(16 t + 363 x index) is positive. The SDK's index is the
/// sprite's own entity; here it is the manhack's.
pub fn strobe_lit(now: f64, id: usize) -> bool {
    (now * 16. + id as f64 * 363.).sin() > 0.
}
const PANELS_OPEN: f32 = 90.;
const PANELS_CLOSING: f32 = 25.;

const ATTACK: &str = "SCHED_MANHACK_ATTACK_HOVER";
const IDLE: &str = "SCHED_MANHACK_IDLE";
const DEPLOY: &str = "SCHED_MANHACK_DEPLOY";
/// A packed manhack waiting for a script: SCHED_WAIT_FOR_SCRIPT, which a hit or Unpack ends.
const PACKED: &str = "SCHED_MANHACK_PACKED";

/// SF_MANHACK_PACKED_UP.
pub const SF_PACKED_UP: u32 = 1 << 16;
/// SF_MANHACK_CARRIED: a metrocop carries it.
pub const SF_CARRIED: u32 = 1 << 19;
/// The deploy sequence's events: the engine starts, the blade opens, and it is done.
pub const AE_START_ENGINE: i32 = 50;
pub const AE_DONE_UNPACKING: i32 = 51;
pub const AE_OPEN_BLADE: i32 = 52;
/// How long after unpacking its first burst can come.
const UNPACKED_REST: f64 = 2.;

/// Its schedules, with the interrupts every NPC has. Steering is the runtime's: a hover just runs.
const SCHEDULES: &[(&str, &str)] = &[
    (
        ATTACK,
        "Tasks TASK_SET_ACTIVITY ACTIVITY:ACT_FLY TASK_MANHACK_HOVER 0 \
         Interrupts COND_NEW_ENEMY COND_ENEMY_DEAD COND_LOST_ENEMY",
    ),
    (
        IDLE,
        "Tasks TASK_SET_ACTIVITY ACTIVITY:ACT_FLY TASK_WAIT 1 \
         Interrupts COND_NEW_ENEMY COND_SEE_ENEMY COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE",
    ),
    (
        DEPLOY,
        "Tasks TASK_PLAY_SEQUENCE ACTIVITY:ACT_MANHACK_UNPACK TASK_SET_ACTIVITY ACTIVITY:ACT_FLY",
    ),
    (
        PACKED,
        "Tasks TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_WAIT 1 \
         Interrupts COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE",
    ),
];

pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES.iter().chain(swarm::SCHEDULES) {
        let schedule = Schedule::parse(name, text).expect("manhack schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// A manhack under the AI.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Manhack {
    #[serde(skip)]
    pub requests: Vec<CopRequest>,
    pub assault: Option<Assault>,
    pub script_goal: Option<(Vec3, bool)>,
    pub hover: Hover,
    /// Where it flies: its enemy's eyes, or its last known place, from this think's gather.
    pub target: Option<Vec3>,
    /// m_flBurstDuration, m_flNextBurstTime and m_vecBurstDirection.
    pub burst_until: f64,
    pub next_burst: f64,
    pub burst_direction: Vec3,
    /// How many cuts it has landed.
    pub cuts: u32,
    /// The noise's three rates (SetNoiseMod).
    pub bob: Vec3,
    pub next_steer: f64,
    pub last_step: f64,
    /// The blade is out, and the engine runs at these pitches.
    pub blade_out: bool,
    pub engine_on: bool,
    engine_pitches: (f32, f32),
    /// SF_MANHACK_PACKED_UP: it does not move until its deploy sequence starts the engine.
    pub packed: bool,
    /// Packed, it waits for a script: it stays shut until a hit or Unpack wakes it.
    pub waits: bool,
    /// Unpack came, or a hit.
    pub woken: bool,
    /// The deploy sequence has not opened the blade yet.
    pub blade_shut: bool,
    /// How it stands with its squad.
    pub swarm: Swarm,
    /// m_bShowingHostile: a burst is on. Its panels' opening in degrees.
    pub hostile: bool,
    pub panels: [f32; 4],
    /// m_flEngineStallTime, the time of the gravity gun's last pickup, punt or launch
    /// (m_flLastPhysicsInfluenceTime with the player as its physics attacker), and the eye's
    /// stunned state.
    pub stalled_until: f64,
    pub punted_at: Option<f64>,
    pub stunned: bool,
    /// m_bHeld: the gravity gun holds it, at this yaw in degrees.
    pub held: bool,
    pub held_yaw: Option<f32>,
    /// SF_MANHACK_CARRIED: a metrocop carries it, and it has no physics object yet.
    pub carried: bool,
    /// SF_NPC_GAG: it flies without its engine sound.
    gagged: bool,
    seed: u64,
}

impl Manhack {
    pub fn new(gagged: bool, seed: u64) -> Self {
        Self {
            bob: Vec3::splat(NOISE_RATE),
            // Alone it needs no slot; a squad's claim decides from its first think.
            swarm: Swarm {
                may_attack: true,
                ..Default::default()
            },
            gagged,
            seed: seed | 1,
            ..Default::default()
        }
    }
    /// A manhack placed packed, shut until it deploys.
    pub fn pack(&mut self, waits: bool) {
        self.packed = true;
        self.waits = waits;
        self.blade_shut = true;
    }
    /// InputUnpack, or a hit: only a packed manhack has anything to wake.
    pub fn wake(&mut self) {
        self.woken = self.packed;
    }
    /// HandleAnimEvent: the deploy sequence starts the engine and opens the blade, and no burst
    /// comes until it is done.
    pub fn anim_event(&mut self, event: i32, now: f64) {
        match event {
            AE_START_ENGINE => {
                self.packed = false;
                self.next_burst = f64::MAX;
            }
            AE_OPEN_BLADE => self.blade_shut = false,
            AE_DONE_UNPACKING => self.next_burst = now + UNPACKED_REST,
            _ => {}
        }
    }
    /// The next number of its own dice.
    pub fn dice(&mut self) -> f32 {
        super::roll(&mut self.seed)
    }
    pub fn bursting(&self, now: f64) -> bool {
        self.burst_until > now
    }
    /// MeleeAttack1Conditions: rested, its enemy near across the ground and level with the eyes.
    /// The SDK measures between the bodies' middles; this from its enemy's feet.
    fn melee_ready(&self, ctx: &Context) -> bool {
        let (Some((_, feet)), Some(eyes)) = (ctx.enemy, ctx.enemy_eye) else {
            return false;
        };
        let across = (feet - ctx.origin).truncate().length();
        ctx.now >= self.next_burst && swarm::in_melee(across, ctx.origin.z - eyes.z)
    }
    /// Rather than chase: it rests between bursts, or its squad's attack slots are taken.
    pub fn swarms(&self, now: f64) -> bool {
        self.next_burst > now || !self.swarm.may_attack
    }
    /// One think's thrust: a burst holds its line, steered a little at the enemy; otherwise it
    /// flies at its target, never more in a think than the distance left.
    pub fn fly(&mut self, origin: Vec3, interval: f32, now: f64) {
        if self.bursting(now) {
            self.fly_burst(origin, interval);
            return;
        }
        let Some(target) = self.target else {
            self.hover.push(Vec3::ZERO, thrust(0., 0.), interval);
            return;
        };
        let to_target = target - origin;
        let distance = to_target.length();
        let direction = to_target.normalize_or_zero();
        let heading = self.hover.velocity.normalize_or_zero();
        let across = if direction.dot(heading) > AHEAD {
            THRUST_AHEAD
        } else {
            THRUST_TURNING
        };
        let reach = distance / interval;
        let push = thrust(across.min(reach), THRUST_UP.min(reach));
        self.hover.push(direction, push, interval);
    }
    fn fly_burst(&mut self, origin: Vec3, interval: f32) {
        let mut climb = BURST_UP;
        if let Some(target) = self.target {
            let to_enemy = target - origin;
            climb = climb.min(to_enemy.z.abs() / interval);
            let steer = to_enemy.normalize_or_zero();
            let mut line = self.burst_direction + steer * BURST_STEER;
            line.z = steer.z;
            self.burst_direction = line.normalize_or(self.burst_direction);
        }
        self.hover
            .push(self.burst_direction, thrust(BURST_THRUST, climb), interval);
    }
    /// How much noise its flight takes, by how far across it is from its enemy.
    pub fn noise(&self, across: Option<f32>) -> f32 {
        match across {
            Some(distance) if distance < BURST_RANGE => NOISE_NEAR,
            _ => NOISE_FAR,
        }
    }
    /// Whether it commits to a burst: close, at the enemy's height, flying at it, and rested.
    pub fn burst_ready(&self, across: f32, in_height: bool, to_enemy: Vec3, now: f64) -> bool {
        let heading = self.hover.velocity.normalize_or_zero();
        now >= self.next_burst
            && !self.bursting(now)
            && across < BURST_RANGE
            && in_height
            && heading.dot(to_enemy.normalize_or_zero()) > BURST_AIM
    }
    /// ShowHostile: the panels open, and it says so as it changes.
    fn show_hostile(&mut self, hostile: bool) {
        if std::mem::replace(&mut self.hostile, hostile) != hostile {
            let cue = if hostile { CHARGE_ANNOUNCE } else { CHARGE_END };
            self.requests.push(CopRequest::Sound(cue));
        }
    }
    /// Once a think: a stalled engine flutters each panel at random; otherwise past its burst it
    /// is no longer hostile, and the panels follow.
    pub fn update_panels(&mut self, now: f64) {
        if self.stalled(now) {
            self.panels = std::array::from_fn(|_| self.dice() * PANELS_OPEN);
            return;
        }
        if !self.bursting(now) {
            self.show_hostile(false);
        }
        let open = if self.hostile {
            PANELS_OPEN
        } else {
            (self.panels[0] - PANELS_CLOSING).max(0.)
        };
        self.panels = [open; 4];
    }
    /// A punt by the gravity gun: the speed it adds, up to VPhysics' limit, the burst's end (its
    /// DMG_PHYSGUN), the stall and the stunned eye.
    pub fn punted(&mut self, push: Vec3, now: f64) {
        let speed = self.hover.velocity + push;
        self.hover.velocity = speed.clamp_length_max(crate::physics::MAX_VELOCITY);
        self.stop_burst(now);
        self.held = false;
        self.stun(now);
    }
    /// SF_NPC_GAG: no engine and no grind.
    pub fn gagged(&self) -> bool {
        self.gagged
    }
    /// MoveToTarget does nothing while the engine is stalled.
    pub fn stalled(&self, now: f64) -> bool {
        now < self.stalled_until
    }
    /// A hit soon after a punt or a launch: the share of its max health it takes. Held, it takes
    /// no impact damage.
    pub fn smash(&self, speed: f32, now: f64) -> Option<f32> {
        self.thrown(now)
            .then(|| (speed / SMASH_SPEED).clamp(0., 2.))
    }
    /// StartBurst: no steering to speak of for a second, and no other for two. One already under
    /// way runs on as it is.
    pub fn start_burst(&mut self, direction: Vec3, now: f64) {
        if self.bursting(now) {
            return;
        }
        self.show_hostile(true);
        // SetEyeState(MANHACK_EYE_STATE_CHARGE): the eye is red again.
        self.stunned = false;
        self.burst_direction = direction.normalize_or_zero();
        self.burst_until = now + BURST_LENGTH;
        self.next_burst = now + BURST_PAUSE;
    }
    /// StopBurst: a burst that hit something ends, and the next waits its full pause.
    pub fn stop_burst(&mut self, now: f64) {
        if self.bursting(now) {
            self.next_burst = now + BURST_PAUSE;
            self.burst_until = now - 0.1;
        }
    }
    /// A cut landed: `away` runs from what it cut to itself. It flies back across the ground and
    /// the burst ends.
    pub fn rebound(&mut self, away: Vec3, now: f64) {
        let across = away.truncate().normalize_or_zero();
        self.hover.velocity = across.extend(0.) * REBOUND;
        self.cuts += 1;
        self.stop_burst(now);
    }
    /// PlayFlySound: the engines start once, then rise as it nears its enemy.
    pub fn engine_sound(&mut self, enemy_distance: Option<f32>) {
        if self.gagged {
            return;
        }
        let (one, two) = pitches(enemy_distance);
        let seconds = 0.1;
        if !std::mem::replace(&mut self.engine_on, true) {
            for (cue, pitch) in [(ENGINE_1, one), (ENGINE_2, two)] {
                let volume = ENGINE_VOLUME;
                let op = PatchOp::Play { volume, pitch };
                self.requests.push(CopRequest::Patch { cue, op });
            }
            self.engine_pitches = (one, two);
            return;
        }
        let (was_one, was_two) = self.engine_pitches;
        if (one - was_one).abs() < PITCH_STEP && (two - was_two).abs() < PITCH_STEP {
            return;
        }
        self.engine_pitches = (one, two);
        for (cue, pitch) in [(ENGINE_1, one), (ENGINE_2, two)] {
            let op = PatchOp::Pitch { pitch, seconds };
            self.requests.push(CopRequest::Patch { cue, op });
        }
    }
    /// The engine cues, for a host that stops them.
    pub fn engine_cues() -> [&'static str; 2] {
        [ENGINE_1, ENGINE_2]
    }
    pub fn sound_interests(&self) -> u32 {
        sound::WORLD | sound::COMBAT | sound::PLAYER | sound::DANGER
    }
}

fn thrust(across: f32, up: f32) -> Thrust {
    Thrust {
        across,
        up,
        decay: DECAY,
    }
}

/// The engines' pitches at this distance from its enemy.
fn pitches(enemy_distance: Option<f32>) -> (f32, f32) {
    let rise = |near: f32, (low, high): (f32, f32)| {
        let share = enemy_distance.map_or(0., |d| (1. - d / near).clamp(0., 1.));
        low + (high - low) * share
    };
    (rise(PITCH_NEAR_1, PITCH_1), rise(PITCH_NEAR_2, PITCH_2))
}

impl Behavior for Manhack {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        if npc.state == NpcState::Script {
            return base_select_schedule(npc, ctx, false);
        }
        if npc.conditions.has(COND_LIGHT_DAMAGE) || npc.conditions.has(COND_HEAVY_DAMAGE) {
            self.wake();
        }
        if self.packed {
            let held = self.waits && !self.woken;
            return if held { PACKED } else { DEPLOY }.into();
        }
        match ctx.enemy {
            Some(_) if self.swarms(ctx.now) => swarm::SWARM,
            Some(_) => ATTACK,
            None if self.swarm.squadded => swarm::SWARM_IDLE,
            None => IDLE,
        }
        .into()
    }
    fn translate_schedule(&mut self, name: &str, _npc: &AiNpc, _ctx: &Context) -> String {
        match name {
            "SCHED_IDLE_STAND" | "SCHED_ALERT_STAND" | "SCHED_ALERT_FACE" => if self.swarm.squadded
            {
                swarm::SWARM_IDLE
            } else {
                IDLE
            }
            .into(),
            other => other.into(),
        }
    }
    fn start_task(
        &mut self,
        _npc: &mut AiNpc,
        task: &str,
        argument: &TaskArg,
        ctx: &Context,
        _motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        match task {
            "TASK_MANHACK_HOVER" => {}
            swarm::FIND_CENTER => self.swarm.ask = Some(Ask::Center),
            swarm::FIND_MEMBER => self.swarm.ask = Some(Ask::Member),
            swarm::MOVE_AT => {
                let seconds = if let TaskArg::Number(seconds) = argument {
                    *seconds
                } else {
                    1.
                };
                self.swarm.start_move(seconds, ctx.random, ctx.now);
            }
            _ => return None,
        }
        Some(Running)
    }
    fn run_task(
        &mut self,
        _npc: &mut AiNpc,
        task: &str,
        _value: &TaskArg,
        ctx: &Context,
        _motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        match task {
            // The hover has no end of its own: a rest or a taken slot sends it back to swarm.
            "TASK_MANHACK_HOVER" => {
                let done = self.swarms(ctx.now) && !self.bursting(ctx.now);
                Some(if done { Complete } else { Running })
            }
            swarm::FIND_CENTER | swarm::FIND_MEMBER => Some(if self.swarm.ask.is_some() {
                Running
            } else {
                Complete
            }),
            // SWARM's COND_CAN_MELEE_ATTACK1: an enemy in reach ends the move.
            swarm::MOVE_AT if self.melee_ready(ctx) => Some(Complete),
            swarm::MOVE_AT => Some(self.swarm.step(ctx.origin, ctx.now)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::schedule::tests::FakeMotor;
    use super::*;

    #[test]
    fn a_packed_manhack_waiting_for_a_script_stays_shut_until_woken() {
        let mut m = Manhack::new(false, 3);
        m.pack(true);
        let mut npc = AiNpc::new(1);
        let ctx = Context::default();
        assert_eq!(m.select_schedule(&mut npc, &ctx), PACKED);
        m.wake();
        assert_eq!(m.select_schedule(&mut npc, &ctx), DEPLOY);
    }

    /// A flying manhack free to attack, with an enemy in `ctx`.
    fn hunting(now: f64) -> (Manhack, AiNpc, Context) {
        let m = Manhack::new(false, 3);
        let ctx = Context {
            now,
            enemy: Some((1, Vec3::X * 100.)),
            ..Default::default()
        };
        (m, AiNpc::new(1), ctx)
    }

    #[test]
    fn it_attacks_when_rested_and_free_and_swarms_otherwise() {
        let (mut m, mut npc, ctx) = hunting(10.);
        assert_eq!(m.select_schedule(&mut npc, &ctx), ATTACK);
        m.next_burst = 11.;
        assert_eq!(m.select_schedule(&mut npc, &ctx), swarm::SWARM);
        m.next_burst = 0.;
        m.swarm.may_attack = false;
        assert_eq!(m.select_schedule(&mut npc, &ctx), swarm::SWARM);
    }

    #[test]
    fn an_idle_squad_keeps_together_and_a_lone_manhack_waits() {
        let mut m = Manhack::new(false, 3);
        let mut npc = AiNpc::new(1);
        let ctx = Context::default();
        assert_eq!(m.select_schedule(&mut npc, &ctx), IDLE);
        m.swarm.squadded = true;
        assert_eq!(m.select_schedule(&mut npc, &ctx), swarm::SWARM_IDLE);
        assert_eq!(
            m.translate_schedule("SCHED_ALERT_FACE", &npc, &ctx),
            swarm::SWARM_IDLE
        );
    }

    #[test]
    fn the_hover_ends_when_it_must_swarm_but_never_inside_a_burst() {
        let (mut m, mut npc, ctx) = hunting(10.);
        let task = "TASK_MANHACK_HOVER";
        let none = TaskArg::Number(0.);
        let mut run =
            |m: &mut Manhack| m.run_task(&mut npc, task, &none, &ctx, &mut FakeMotor::default());
        assert_eq!(run(&mut m), Some(Running));
        m.next_burst = 12.;
        m.burst_until = 11.;
        assert_eq!(run(&mut m), Some(Running));
        m.burst_until = 9.;
        assert_eq!(run(&mut m), Some(Complete));
    }

    #[test]
    fn an_enemy_in_reach_ends_a_swarm_move_once_rested() {
        let (mut m, mut npc, mut ctx) = hunting(10.);
        ctx.origin = Vec3::new(0., 0., 60.);
        ctx.enemy = Some((1, Vec3::X * 30.));
        ctx.enemy_eye = Some(Vec3::new(30., 0., 64.));
        m.swarm.until = 20.;
        m.swarm.point = Vec3::X * 500.;
        let none = TaskArg::Number(0.);
        let mut run = |m: &mut Manhack| {
            m.run_task(
                &mut npc,
                swarm::MOVE_AT,
                &none,
                &ctx,
                &mut FakeMotor::default(),
            )
        };
        assert_eq!(run(&mut m), Some(Complete));
        m.next_burst = 11.;
        assert_eq!(run(&mut m), Some(Running));
    }

    #[test]
    fn a_squad_question_holds_its_task_until_the_runtime_answers() {
        let (mut m, mut npc, ctx) = hunting(10.);
        let none = TaskArg::Number(0.);
        let task = swarm::FIND_CENTER;
        m.start_task(&mut npc, task, &none, &ctx, &mut FakeMotor::default());
        assert_eq!(m.swarm.ask, Some(Ask::Center));
        assert_eq!(
            m.run_task(&mut npc, task, &none, &ctx, &mut FakeMotor::default()),
            Some(Running)
        );
        m.swarm.answer(Vec3::X);
        assert_eq!(
            m.run_task(&mut npc, task, &none, &ctx, &mut FakeMotor::default()),
            Some(Complete)
        );
    }

    #[test]
    fn a_burst_opens_the_panels_at_once_and_they_close_by_25_a_think() {
        let mut m = Manhack::new(false, 3);
        m.start_burst(Vec3::X, 10.);
        m.update_panels(10.1);
        assert_eq!(m.panels, [90.; 4]);
        let closing: Vec<f32> = [11.1, 11.2, 11.3, 11.4]
            .map(|now| {
                m.update_panels(now);
                m.panels[0]
            })
            .to_vec();
        assert_eq!(closing, [65., 40., 15., 0.]);
    }

    #[test]
    fn it_announces_a_charge_and_its_end_once_each() {
        let mut m = Manhack::new(false, 3);
        m.start_burst(Vec3::X, 10.);
        m.start_burst(Vec3::X, 10.5);
        for now in [10.6, 11.1, 11.2] {
            m.update_panels(now);
        }
        let cues: Vec<_> = m
            .requests
            .iter()
            .filter_map(|r| match r {
                CopRequest::Sound(cue) => Some(*cue),
                _ => None,
            })
            .collect();
        assert_eq!(cues, [CHARGE_ANNOUNCE, CHARGE_END]);
    }

    #[test]
    fn a_punt_stalls_it_for_two_seconds_and_stuns_its_eye_until_a_charge() {
        let mut m = Manhack::new(false, 3);
        m.punted(Vec3::X * 4661., 10.);
        assert_eq!(m.hover.velocity, Vec3::X * 2000.);
        assert!(m.stalled(11.9) && !m.stalled(12.));
        assert!(m.stunned);
        m.update_panels(10.5);
        assert!(m.panels.iter().all(|p| (0. ..=90.).contains(p)));
        assert!(
            m.panels.windows(2).any(|w| w[0] != w[1]),
            "each panel at random"
        );
        m.start_burst(Vec3::X, 12.5);
        assert!(!m.stunned);
    }

    #[test]
    fn a_smash_counts_only_just_after_a_punt_and_is_at_most_twice_max_health() {
        let mut m = Manhack::new(false, 3);
        assert_eq!(m.smash(1000., 10.), None);
        m.punted(Vec3::X * 100., 10.);
        assert_eq!(m.smash(250., 10.2), Some(0.5));
        assert_eq!(m.smash(2000., 10.35), Some(2.));
        assert_eq!(m.smash(2000., 10.4), None);
    }

    #[test]
    fn the_fast_strobe_is_lit_about_half_the_time() {
        let lit = (0..1000)
            .filter(|i| strobe_lit(f64::from(*i) * 0.01, 7))
            .count();
        assert!((400..600).contains(&lit), "{lit}");
    }

    #[test]
    fn a_hit_wakes_a_packed_manhack() {
        let mut m = Manhack::new(false, 3);
        m.pack(true);
        let mut npc = AiNpc::new(1);
        npc.conditions.set(COND_LIGHT_DAMAGE);
        assert_eq!(m.select_schedule(&mut npc, &Context::default()), DEPLOY);
    }

    #[test]
    fn packed_with_no_script_to_wait_for_it_deploys_at_once() {
        let mut m = Manhack::new(false, 3);
        m.pack(false);
        let mut npc = AiNpc::new(1);
        assert_eq!(m.select_schedule(&mut npc, &Context::default()), DEPLOY);
    }

    #[test]
    fn unpack_does_nothing_to_a_manhack_already_flying() {
        let mut m = Manhack::new(false, 3);
        m.wake();
        assert!(!m.woken);
        let mut npc = AiNpc::new(1);
        assert_eq!(m.select_schedule(&mut npc, &Context::default()), IDLE);
    }

    #[test]
    fn the_deploy_events_start_it_open_the_blade_and_hold_bursts_until_done() {
        let mut m = Manhack::new(false, 3);
        m.pack(false);
        assert!(m.blade_shut);
        m.anim_event(AE_START_ENGINE, 5.);
        assert!(!m.packed);
        assert_eq!(m.next_burst, f64::MAX);
        m.anim_event(AE_OPEN_BLADE, 5.5);
        assert!(!m.blade_shut);
        assert_eq!(m.next_burst, f64::MAX);
        m.anim_event(AE_DONE_UNPACKING, 6.);
        assert_eq!(m.next_burst, 8.);
    }

    fn at_speed(velocity: Vec3) -> Manhack {
        let mut m = Manhack::new(false, 3);
        m.hover.velocity = velocity;
        m
    }

    #[test]
    fn it_flies_at_its_target_harder_when_that_is_ahead() {
        let gain = |velocity: Vec3| {
            let mut m = at_speed(velocity);
            m.target = Some(Vec3::X * 1000.);
            let kept = DECAY.powf(0.1) * velocity.x;
            m.fly(Vec3::ZERO, 0.1, 0.);
            m.hover.velocity.x - kept
        };
        assert!((gain(Vec3::X * 100.) - 30.).abs() < 1e-3);
        assert!((gain(Vec3::NEG_X * 100.) - 20.).abs() < 1e-3);
    }

    #[test]
    fn a_think_never_pushes_it_past_its_target() {
        let mut m = Manhack::new(false, 3);
        m.target = Some(Vec3::X * 2.);
        m.fly(Vec3::ZERO, 0.1, 0.);
        assert!((m.hover.velocity.x - 2.).abs() < 1e-3);
    }

    #[test]
    fn with_no_target_it_only_slows() {
        let mut m = at_speed(Vec3::X * 100.);
        m.fly(Vec3::ZERO, 0.1, 0.);
        assert!(m.hover.velocity.x < 100. && m.hover.velocity.x > 0.);
    }

    #[test]
    fn a_burst_needs_it_close_at_height_on_line_and_rested() {
        let m = at_speed(Vec3::X * 300.);
        assert!(m.burst_ready(150., true, Vec3::X, 1.));
        assert!(!m.burst_ready(250., true, Vec3::X, 1.));
        assert!(!m.burst_ready(150., false, Vec3::X, 1.));
        assert!(!m.burst_ready(150., true, Vec3::Y, 1.));
        let rested = Manhack {
            next_burst: 5.,
            ..at_speed(Vec3::X * 300.)
        };
        assert!(!rested.burst_ready(150., true, Vec3::X, 1.));
    }

    #[test]
    fn a_burst_lasts_a_second_and_the_next_comes_after_two() {
        let mut m = Manhack::new(false, 3);
        m.start_burst(Vec3::X, 10.);
        assert!(m.bursting(10.9));
        assert!(!m.bursting(11.));
        assert_eq!(m.next_burst, 12.);
    }

    #[test]
    fn a_burst_holds_its_line_but_leans_toward_a_moving_enemy() {
        let mut m = Manhack::new(false, 3);
        m.start_burst(Vec3::X, 0.);
        m.target = Some(Vec3::new(100., 100., 0.));
        m.fly(Vec3::ZERO, 0.1, 0.5);
        assert!(m.burst_direction.y > 0. && m.burst_direction.x > 0.);
        assert!(m.hover.velocity.length() > 0.);
    }

    #[test]
    fn a_cut_sends_it_back_across_the_ground_and_ends_the_burst() {
        let mut m = at_speed(Vec3::X * 500.);
        m.start_burst(Vec3::X, 0.);
        m.rebound(Vec3::new(-10., 0., 30.), 0.5);
        assert_eq!(m.hover.velocity, Vec3::NEG_X * 200.);
        assert!(!m.bursting(0.5));
        assert_eq!(m.next_burst, 2.5);
        assert_eq!(m.cuts, 1);
    }

    #[test]
    fn close_to_its_enemy_it_flies_steadier() {
        let m = Manhack::new(false, 3);
        assert_eq!(m.noise(None), NOISE_FAR);
        assert_eq!(m.noise(Some(500.)), NOISE_FAR);
        assert_eq!(m.noise(Some(100.)), NOISE_NEAR);
    }

    #[test]
    fn the_engine_starts_once_and_rises_as_it_nears_its_enemy() {
        let patches = |m: &mut Manhack| std::mem::take(&mut m.requests).len();
        let mut m = Manhack::new(false, 3);
        m.engine_sound(None);
        assert_eq!(patches(&mut m), 2);
        m.engine_sound(None);
        assert_eq!(patches(&mut m), 0);
        m.engine_sound(Some(100.));
        assert_eq!(patches(&mut m), 2);
        assert_eq!(pitches(None), (100., 85.));
        assert_eq!(pitches(Some(0.)), (160., 190.));
    }

    #[test]
    fn a_gagged_manhack_flies_quietly() {
        let mut m = Manhack::new(true, 3);
        m.engine_sound(Some(100.));
        assert!(m.requests.is_empty());
    }

    #[test]
    fn with_an_enemy_it_attacks_and_without_one_it_idles() {
        let mut m = Manhack::new(false, 3);
        let mut npc = AiNpc::new(1);
        let hunting = Context {
            enemy: Some((0, Vec3::X)),
            ..Default::default()
        };
        assert_eq!(m.select_schedule(&mut npc, &hunting), ATTACK);
        assert_eq!(m.select_schedule(&mut npc, &Context::default()), IDLE);
    }

    #[test]
    fn its_schedules_parse() {
        let mut schedules = Schedules::default();
        add_schedules(&mut schedules);
        assert!(schedules.0.contains_key(ATTACK) && schedules.0.contains_key(IDLE));
    }
}
