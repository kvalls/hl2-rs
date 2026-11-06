//! npc_manhack (DESIGN 14o session 47, step 1): a flying blade. This module holds what it decides
//! and its flight arithmetic; `runtime::manhacks` gives it the world and carries the body. It flies
//! at its enemy's eyes, bursts from close by, cuts what it touches and bounces off. Not yet: packed
//! manhacks, swarming, the gravity gun, its sprites and panels, a crash in place of gibs, water.
use super::assault::Assault;
use super::hover::{Hover, Thrust};
use super::metropolice::CopRequest;
use super::schedule::TaskStatus::Running;
use super::schedule::{
    base_select_schedule, AiNpc, Behavior, Context, Motor, Schedule, Schedules, TaskArg, TaskStatus,
};
use super::senses::sound;
use super::state::NpcState;
use crate::sounds::PatchOp;
use glam::Vec3;
use serde::Serialize;

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
pub const SOUNDS: &[&str] = &[ENGINE_1, ENGINE_2, DIE, SLICE, GRIND];

const ATTACK: &str = "SCHED_MANHACK_ATTACK_HOVER";
const IDLE: &str = "SCHED_MANHACK_IDLE";

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
];

pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
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
    /// SF_NPC_GAG: it flies without its engine sound.
    gagged: bool,
    seed: u64,
}

impl Manhack {
    pub fn new(gagged: bool, seed: u64) -> Self {
        Self {
            bob: Vec3::splat(NOISE_RATE),
            gagged,
            seed: seed | 1,
            ..Default::default()
        }
    }
    /// The next number of its own dice.
    pub fn dice(&mut self) -> f32 {
        super::roll(&mut self.seed)
    }
    pub fn bursting(&self, now: f64) -> bool {
        self.burst_until > now
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
    /// StartBurst: no steering to speak of for a second, and no other for two.
    pub fn start_burst(&mut self, direction: Vec3, now: f64) {
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

/// A cut's damage for `interval` seconds of contact, at `per_second` on an NPC.
pub fn cut_damage(per_second: f32, interval: f32, on_player: bool) -> f32 {
    let share = if on_player { PLAYER_SHARE } else { 1. };
    (per_second * interval * share).max(LEAST_CUT)
}

impl Behavior for Manhack {
    fn select_schedule(&mut self, npc: &mut AiNpc, ctx: &Context) -> String {
        if npc.state == NpcState::Script {
            return base_select_schedule(npc, ctx, false);
        }
        if ctx.enemy.is_some() { ATTACK } else { IDLE }.into()
    }
    fn translate_schedule(&mut self, name: &str, _npc: &AiNpc, _ctx: &Context) -> String {
        match name {
            "SCHED_IDLE_STAND" | "SCHED_ALERT_STAND" => IDLE.into(),
            other => other.into(),
        }
    }
    fn start_task(
        &mut self,
        _npc: &mut AiNpc,
        task: &str,
        _argument: &TaskArg,
        _ctx: &Context,
        _motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        (task == "TASK_MANHACK_HOVER").then_some(Running)
    }
    fn run_task(
        &mut self,
        _npc: &mut AiNpc,
        task: &str,
        _value: &TaskArg,
        _ctx: &Context,
        _motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        (task == "TASK_MANHACK_HOVER").then_some(Running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_cut_does_twice_as_much_to_the_player_and_never_less_than_one() {
        assert_eq!(cut_damage(20., 0.1, false), 2.);
        assert_eq!(cut_damage(20., 0.1, true), 4.);
        assert_eq!(cut_damage(5., 0.1, false), 1.);
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
