//! npc_barnacle (DESIGN 14o session 46): hangs from a ceiling, lets its tongue down to just above
//! the floor and hauls up whatever touches it. It has no schedules, memory or mover, so it lives
//! beside the schedule actors: `AiRuntime` gathers [`Facts`], [`Barnacle::think`] answers with
//! [`Command`]s. The rope is a visual chain between the root and the tip, written over the
//! model's tongue bones in `Scene::actor_matrices`.

use glam::{Mat4, Quat, Vec3};
use serde::Serialize;

/// The think interval near the player; out of the player's PVS it rests 1 to 1.5 s.
pub const THINK_INTERVAL: f64 = 0.1;
/// Tongue speed, u/s.
const PULL_SPEED: f32 = 80.;
/// A lift never takes less than this: a short pull slows down.
const MIN_PULL_TIME: f32 = 3.;
/// One bite on the player (`attack_player` loops, so bites repeat).
pub const PLAYER_BITE_DAMAGE: f32 = 15.;
/// Half the width of the box around the tongue that counts as touching it.
pub const TOUCH_HALF_WIDTH: f32 = 12.;
/// Heavier props are left alone.
pub const MAX_LIFT_MASS: f32 = 70.;
/// How far the floor trace reaches.
pub const FLOOR_TRACE: f32 = 2048.;
/// The tongue's shortest length.
const MIN_TONGUE: f32 = 8.;
/// Height under the barnacle at which it bites; the player's eyes stop lower to keep the view
/// out of the body, a prop's tip higher.
const BITE_HEIGHT: f32 = 60.;
const PLAYER_BITE_EXTRA: f32 = 25.;
const PROP_BITE_EXTRA: f32 = -5.;
/// Prey shorter than this is eaten with the small-things bite.
const SMALL_PREY: f32 = 40.;
/// How long a stalled lift holds on.
const STUCK_TIME: f64 = 3.;
/// Time to digest a meal.
const DIGEST_TIME: f64 = 10.;
/// Thinks a prop is shaken at the mouth before it is tasted, and how far it jumps.
const PROP_SHAKES: u8 = 6;
const PROP_SHAKE_HEIGHT: f32 = 15.;
/// A dead tongue's length, and the room it leaves over the floor.
const DEAD_TONGUE: f32 = 164.;
const DEAD_CLEARANCE: f32 = 78.;
/// `spawnflags`: start with the tongue up until the DropTongue input.
pub const SF_AMBUSH: u32 = 1 << 17;
/// `spawnflags`: skip the gibs half of the time.
pub const SF_CHEAP_DEATH: u32 = 1 << 16;
/// Where the player's tongue is drawn relative to the eyes, so it stays out of the camera.
const PLAYER_TIP_DRAW: Vec3 = Vec3::new(24., 0., -8.);
/// Rope points, one per tongue bone of the installed model.
pub const NODES: usize = 8;
/// How fast the drawn tip catches up with the real one, u/s.
const TIP_SNAP: f32 = 200.;
/// Rope gravity, u/s^2.
const ROPE_GRAVITY: f32 = 1000.;
/// Tip gravity while it hangs free, u/s^2.
const TIP_GRAVITY: f32 = 600.;
/// Below this a rope point counts as still.
const AT_REST: f32 = 0.01;

/// Every cue this class plays; a cue that is not preloaded is dropped.
pub const SOUNDS: &[&str] = &[
    "NPC_Barnacle.Digest",
    "NPC_Barnacle.Scream",
    "NPC_Barnacle.PullPant",
    "NPC_Barnacle.TongueStretch",
    "NPC_Barnacle.FinalBite",
    "NPC_Barnacle.Die",
    "NPC_Barnacle.BreakNeck",
];

/// What the tongue holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Prey {
    Player,
    Npc(usize),
    Prop(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Phase {
    /// Nothing held: the tongue hangs and feels for prey.
    Hanging,
    /// Hauling prey up to the mouth.
    Lifting,
    /// The bite animation runs; the model's event does the damage.
    Biting,
    /// Chewing until this time.
    Digesting(f64),
    /// Playing the death animation while the tongue moves to its dead length.
    Dying,
    /// The tongue is at rest; nothing thinks any more.
    Dead,
}

/// The held prey as the world has it this think.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreyFacts {
    pub position: Vec3,
    /// Eye height in world space (the player's bite test).
    pub eye_z: f32,
    pub alive: bool,
    /// Something solid is between the barnacle and the prey.
    pub blocked: bool,
    /// A flying player cannot be held.
    pub noclip: bool,
    /// The prey's height (`SMALL_PREY`).
    pub height: f32,
}

/// What the runtime found for this think.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facts {
    pub now: f64,
    pub dt: f32,
    pub origin: Vec3,
    /// How far the tongue can reach down (the floor less the rest height).
    pub reach: f32,
    /// Distance from the tongue's root to the floor under it, if the trace found one.
    pub floor_distance: Option<f32>,
    /// Prey inside the touch box, with the point the tongue takes it by.
    pub touching: Option<(Prey, Vec3)>,
    pub prey: Option<PreyFacts>,
    pub sequence_finished: bool,
    /// The tip's height.
    pub tip_z: f32,
}

/// What the barnacle asks of the runtime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Sound(&'static str),
    Activity(&'static str),
    Grab(Prey, Vec3),
    /// The prey goes free, and the tip goes back to hanging.
    Release(Prey),
    /// Bite this prey: the player for `PLAYER_BITE_DAMAGE`, anything else dies.
    Bite(Prey),
    /// The tongue throws the prop away from the mouth.
    Spit(Prey),
    /// One rib from the mouth.
    CoughGib,
}

/// Barnacle state.
#[derive(Clone, Debug, Serialize)]
pub struct Barnacle {
    pub phase: Phase,
    pub prey: Option<Prey>,
    /// The tongue's length.
    pub altitude: f32,
    /// `RestDist`: how far above the floor the tongue stops.
    pub rest_above_ground: f32,
    pub pull_speed: f32,
    /// Waiting for the DropTongue input.
    pub ambush: bool,
    pub cheap_death: bool,
    pub next_think: f64,
    pub last_think: f64,
    /// Ribs left to cough up.
    gibs: u8,
    /// The last thing thrown away or dropped, not grabbed again right away.
    ignored: Option<Prey>,
    /// The |sin| pull's clock.
    pull_clock: f32,
    last_pull: f32,
    last_prey_position: Vec3,
    stuck_until: f64,
    screamed: bool,
    /// Shakes left before a prop is tasted.
    shakes: u8,
    /// What follows the bite animation, until the first chew ends.
    chew: Option<&'static str>,
    /// Seed of `roll`.
    seed: u64,
}

impl Barnacle {
    pub fn new(flags: u32, rest_above_ground: f32, seed: u64) -> Self {
        Self {
            phase: Phase::Hanging,
            prey: None,
            altitude: MIN_TONGUE,
            rest_above_ground,
            pull_speed: PULL_SPEED,
            ambush: flags & SF_AMBUSH != 0,
            cheap_death: flags & SF_CHEAP_DEATH != 0,
            next_think: 0.,
            last_think: 0.,
            gibs: 0,
            ignored: None,
            pull_clock: 0.,
            last_pull: 0.,
            last_prey_position: Vec3::ZERO,
            stuck_until: 0.,
            screamed: false,
            shakes: 0,
            chew: None,
            seed: seed | 1,
        }
    }
    /// A number in [0, 1).
    pub fn roll(&mut self) -> f32 {
        super::roll(&mut self.seed)
    }
    /// The tongue's reach to a floor `distance` below the root.
    pub fn reach(&self, distance: f32) -> f32 {
        (distance - self.rest_above_ground).max(MIN_TONGUE)
    }
    /// An ambushing tongue does not change until it drops.
    fn set_altitude(&mut self, altitude: f32) {
        if !self.ambush {
            self.altitude = altitude;
        }
    }
    /// The DropTongue input.
    pub fn drop_tongue(&mut self) {
        self.ambush = false;
    }
    pub fn holding(&self) -> bool {
        self.prey.is_some()
    }
    pub fn alive(&self) -> bool {
        !matches!(self.phase, Phase::Dying | Phase::Dead)
    }

    pub fn think(&mut self, facts: &Facts, out: &mut Vec<Command>) {
        match self.phase {
            Phase::Hanging => self.hang(facts, out),
            Phase::Lifting => self.lift(facts, out),
            Phase::Biting => self.wait_for_bite(facts, out),
            Phase::Digesting(until) => self.digest(facts, until, out),
            Phase::Dying => self.settle_dead_tongue(facts),
            Phase::Dead => {}
        }
    }

    fn hang(&mut self, f: &Facts, out: &mut Vec<Command>) {
        if f.sequence_finished {
            out.push(Command::Activity("ACT_IDLE"));
        }
        if self.gibs > 0 && self.roll() < 0.01 {
            self.gibs -= 1;
            out.push(Command::CoughGib);
            out.push(Command::Sound("NPC_Barnacle.Digest"));
        }
        if self.altitude < f.reach {
            self.set_altitude(self.altitude + self.pull_speed * f.dt);
        }
        if self.altitude < f.reach {
            return;
        }
        match f.touching.filter(|(prey, _)| Some(*prey) != self.ignored) {
            Some((prey, grab)) => self.grab(f, prey, grab, out),
            None => self.set_altitude(f.reach),
        }
    }

    fn grab(&mut self, f: &Facts, prey: Prey, grab: Vec3, out: &mut Vec<Command>) {
        out.push(Command::Sound("NPC_Barnacle.BreakNeck"));
        out.push(Command::Sound(if self.roll() > 0.5 {
            "NPC_Barnacle.PullPant"
        } else {
            "NPC_Barnacle.TongueStretch"
        }));
        out.push(Command::Activity("ACT_BARNACLE_SLURP"));
        out.push(Command::Grab(prey, grab));
        self.phase = Phase::Lifting;
        self.prey = Some(prey);
        self.set_altitude(f.origin.z - grab.z);
        let distance = grab.distance(f.origin);
        self.pull_speed = PULL_SPEED.min(distance / MIN_PULL_TIME);
        // Starts on the rising side of the sine, so the lift shows at once.
        self.pull_clock = 60.;
        self.last_pull = 0.;
        self.last_prey_position = grab;
        self.stuck_until = f.now + STUCK_TIME;
        self.screamed = false;
        self.shakes = PROP_SHAKES;
    }

    fn lift(&mut self, f: &Facts, out: &mut Vec<Command>) {
        let (Some(prey), Some(facts)) = (self.prey, f.prey) else {
            self.let_go(out);
            return;
        };
        if facts.blocked || !facts.alive {
            if prey != Prey::Player {
                self.ignored = Some(prey);
            }
            self.let_go(out);
            return;
        }
        if prey == Prey::Player && facts.noclip {
            self.let_go(out);
            return;
        }
        let (bite, depth) = match prey {
            Prey::Player => (BITE_HEIGHT + PLAYER_BITE_EXTRA, f.origin.z - facts.eye_z),
            Prey::Npc(_) => (BITE_HEIGHT, f.origin.z - f.tip_z),
            Prey::Prop(_) => (BITE_HEIGHT + PROP_BITE_EXTRA, f.origin.z - f.tip_z),
        };
        if !self.screamed && self.altitude < bite + 100. {
            self.screamed = true;
            out.push(Command::Sound("NPC_Barnacle.Scream"));
        }
        if depth >= bite {
            self.pull(f, facts, prey, out);
            return;
        }
        if matches!(prey, Prey::Prop(_)) && self.shakes > 0 {
            self.shakes -= 1;
            let lift = if self.shakes % 2 == 1 {
                PROP_SHAKE_HEIGHT
            } else {
                0.
            };
            self.set_altitude(bite + lift);
            return;
        }
        self.phase = Phase::Biting;
        let small = facts.height < SMALL_PREY;
        self.chew = Some(if small {
            "ACT_BARNACLE_CHEW_SMALL_THINGS"
        } else {
            "ACT_BARNACLE_CHEW_HUMAN"
        });
        out.push(Command::Activity(match prey {
            Prey::Player => "ACT_BARNACLE_BITE_PLAYER",
            Prey::Prop(_) => "ACT_BARNACLE_TASTE_SPIT",
            Prey::Npc(_) if small => "ACT_BARNACLE_BITE_SMALL_THINGS",
            Prey::Npc(_) => "ACT_BARNACLE_BITE_HUMAN",
        }));
    }

    /// One step of the haul. The speed pulses, and prey that stops rising for `STUCK_TIME` while
    /// the tongue pulls is dropped. Props swing on the tongue and skip the stuck test.
    fn pull(&mut self, f: &Facts, facts: PreyFacts, prey: Prey, out: &mut Vec<Command>) {
        self.pull_clock += f.dt;
        let pull = (self.pull_clock * 5.).sin().abs() * self.pull_speed * f.dt;
        self.set_altitude(self.altitude - pull);
        if matches!(prey, Prey::Prop(_)) {
            return;
        }
        if self.last_pull > 1. {
            let moved = facts.position.distance(self.last_prey_position);
            if moved >= self.last_pull - 1. {
                self.stuck_until = f.now + STUCK_TIME;
            } else if f.now >= self.stuck_until {
                self.let_go(out);
                return;
            }
        } else {
            self.stuck_until += f64::from(f.dt);
        }
        self.last_prey_position = facts.position;
        self.last_pull = pull;
    }

    fn wait_for_bite(&mut self, f: &Facts, out: &mut Vec<Command>) {
        if f.prey.is_none_or(|p| !p.alive) {
            self.let_go(out);
        }
    }

    /// AE_BARNACLE_BITE, or the next turn of the looping player bite.
    pub fn bite(&mut self, now: f64, out: &mut Vec<Command>) {
        let Some(prey) = self.prey else {
            return;
        };
        out.push(Command::Sound("NPC_Barnacle.FinalBite"));
        out.push(Command::Bite(prey));
        self.gibs = 3;
        if prey != Prey::Player {
            self.prey = None;
            self.phase = Phase::Digesting(now + DIGEST_TIME);
        }
    }

    /// AE_BARNACLE_SPIT.
    pub fn spit(&mut self, out: &mut Vec<Command>) {
        if let Some(prey) = self.prey {
            out.push(Command::Spit(prey));
            self.ignored = Some(prey);
        }
        self.let_go(out);
    }

    fn digest(&mut self, f: &Facts, until: f64, out: &mut Vec<Command>) {
        if f.now >= until {
            self.phase = Phase::Hanging;
            return;
        }
        if f.sequence_finished {
            out.push(Command::Activity(self.chew.take().unwrap_or("ACT_IDLE")));
        }
        if self.roll() < 1. / 26. {
            out.push(Command::Sound("NPC_Barnacle.Digest"));
        }
    }

    /// The prey goes free, and the tongue goes back to hanging.
    pub fn let_go(&mut self, out: &mut Vec<Command>) {
        if let Some(prey) = self.prey.take() {
            out.push(Command::Release(prey));
        }
        self.pull_speed = PULL_SPEED;
        if self.alive() {
            self.phase = Phase::Hanging;
        }
    }

    /// The LetGo input: a dropped prop or NPC is not grabbed again at once.
    pub fn input_let_go(&mut self, out: &mut Vec<Command>) {
        if let Some(prey) = self.prey.filter(|p| *p != Prey::Player) {
            self.ignored = Some(prey);
        }
        self.let_go(out);
    }

    /// Event_Killed: drops whatever it holds; the runtime keeps the body on the ceiling.
    pub fn die(&mut self, out: &mut Vec<Command>) {
        self.let_go(out);
        self.phase = Phase::Dying;
        self.pull_speed = PULL_SPEED;
        out.push(Command::Sound("NPC_Barnacle.Die"));
        out.push(Command::Activity("ACT_DIESIMPLE"));
    }

    fn settle_dead_tongue(&mut self, f: &Facts) {
        let goal = dead_altitude(f.floor_distance);
        let step = self.pull_speed * f.dt;
        self.altitude = if self.altitude >= goal {
            (self.altitude - step).max(goal)
        } else {
            (self.altitude + step).min(goal)
        };
        if self.altitude == goal && f.sequence_finished {
            self.phase = Phase::Dead;
        }
    }
}

/// The length a dead tongue settles at: long, but clear of anyone walking under it.
pub fn dead_altitude(floor_distance: Option<f32>) -> f32 {
    match floor_distance {
        Some(floor) if floor - DEAD_TONGUE < DEAD_CLEARANCE => {
            (floor - DEAD_CLEARANCE).max(floor * 0.5)
        }
        _ => DEAD_TONGUE,
    }
}

/// The rope: a chain of points from the root to the tip, and the tip it follows. The tip swings
/// like a weight on a line as long as the tongue, so held prey drifts in under the barnacle.
#[derive(Clone, Debug, PartialEq)]
pub struct Tongue {
    pub tip: Vec3,
    tip_velocity: Vec3,
    /// The drawn tip: it trails the real one so a grab does not snap across the room.
    drawn_tip: Vec3,
    /// Added to the tip when drawn, to keep the player's tongue out of the camera.
    draw_offset: Vec3,
    pub nodes: [Vec3; NODES],
    previous: [Vec3; NODES],
    /// Nothing moved: the chain is skipped until the tongue changes.
    pub asleep: bool,
}

impl Tongue {
    pub fn hanging(root: Vec3, altitude: f32) -> Self {
        let tip = root - Vec3::Z * altitude;
        let nodes = std::array::from_fn(|i| root.lerp(tip, i as f32 / (NODES as f32 - 1.)));
        Self {
            tip,
            tip_velocity: Vec3::ZERO,
            drawn_tip: tip,
            draw_offset: Vec3::ZERO,
            nodes,
            previous: nodes,
            asleep: false,
        }
    }
    /// Takes hold of prey at `point`.
    pub fn attach(&mut self, point: Vec3, player_yaw: Option<f32>) {
        self.tip = point;
        self.tip_velocity = Vec3::ZERO;
        self.draw_offset = player_yaw.map_or(Vec3::ZERO, |yaw| {
            let turned = Quat::from_rotation_z(yaw.to_radians()) * PLAYER_TIP_DRAW;
            Vec3::new(turned.x, turned.y, PLAYER_TIP_DRAW.z)
        });
        self.asleep = false;
    }
    pub fn detach(&mut self) {
        self.draw_offset = Vec3::ZERO;
        self.asleep = false;
    }
    /// Moves the real tip on a line of length `altitude` from `root`.
    pub fn swing(&mut self, root: Vec3, altitude: f32, dt: f32) {
        let start = self.tip;
        self.tip_velocity.z -= TIP_GRAVITY * dt;
        self.tip_velocity *= (1. - 2. * dt).max(0.);
        let mut next = start + self.tip_velocity * dt;
        let line = next - root;
        let length = line.length();
        if length > altitude && length > 1e-3 {
            next = root + line * (altitude / length);
        }
        self.tip_velocity = (next - start) / dt;
        self.tip = next;
    }
    /// One rope step; true when any point moved. A sleeping rope wakes when the tip or the
    /// length no longer match it.
    pub fn step_rope(&mut self, root: Vec3, altitude: f32, dt: f32) -> bool {
        let ideal = self.tip + self.draw_offset;
        let to_ideal = ideal - self.drawn_tip;
        let reach = TIP_SNAP * dt;
        self.drawn_tip = if to_ideal.length() > reach {
            self.drawn_tip + to_ideal.normalize() * reach
        } else {
            ideal
        };
        if self.asleep && self.nodes[NODES - 1].distance(self.drawn_tip) < AT_REST {
            return false;
        }
        let before = self.nodes;
        let gravity = Vec3::NEG_Z * ROPE_GRAVITY * dt * dt;
        for i in 1..NODES - 1 {
            let carried = (self.nodes[i] - self.previous[i]) * 0.9;
            self.previous[i] = self.nodes[i];
            self.nodes[i] += carried + gravity;
        }
        let rest = altitude / (NODES as f32 - 1.);
        for _ in 0..4 {
            self.nodes[0] = root;
            self.nodes[NODES - 1] = self.drawn_tip;
            for i in 0..NODES - 1 {
                let span = self.nodes[i + 1] - self.nodes[i];
                let length = span.length();
                if length < 1e-4 {
                    continue;
                }
                let correction = span * (0.5 * (length - rest) / length);
                if i > 0 {
                    self.nodes[i] += correction;
                }
                if i + 1 < NODES - 1 {
                    self.nodes[i + 1] -= correction;
                }
            }
        }
        self.nodes[0] = root;
        self.nodes[NODES - 1] = self.drawn_tip;
        let moved = self
            .nodes
            .iter()
            .zip(&before)
            .any(|(a, b)| a.distance(*b) >= AT_REST);
        self.asleep = !moved;
        moved
    }
}

/// The rope's world points, in the form the pose needs; `frame` changes when they do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TonguePose {
    pub nodes: [Vec3; NODES],
    pub frame: u64,
}

/// Writes the rope over the model's root-level tongue bones. Each keeps its bind turn, swung from
/// straight down onto its stretch of rope, so the tongue cannot twist.
pub fn pose_tongue(
    rig: &modkit_core::animation::Rig,
    pose: &TonguePose,
    to_model: Mat4,
    matrices: &mut [Mat4],
) {
    let Some(first) = rig.bones.iter().position(|b| b.name == "Barnacle.tongue1") else {
        return;
    };
    let points = pose.nodes.map(|p| to_model.transform_point3(p));
    let mut along = Vec3::NEG_Z;
    for (i, point) in points.iter().enumerate() {
        let (Some(bone), Some(matrix)) = (rig.bones.get(first + i), matrices.get_mut(first + i))
        else {
            break;
        };
        if let Some(next) = points.get(i + 1) {
            along = (*next - *point).try_normalize().unwrap_or(along);
        }
        let turn = Quat::from_rotation_arc(Vec3::NEG_Z, along);
        *matrix =
            Mat4::from_rotation_translation(turn * bone.bind.rotation, *point) * bone.inverse_bind;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: Vec3 = Vec3::new(0., 0., 300.);

    fn facts(now: f64) -> Facts {
        Facts {
            now,
            dt: 0.1,
            origin: ORIGIN,
            reach: 284.,
            floor_distance: Some(300.),
            touching: None,
            prey: None,
            sequence_finished: false,
            tip_z: 16.,
        }
    }
    fn player_at(z: f32) -> PreyFacts {
        PreyFacts {
            position: Vec3::Z * z,
            eye_z: z + 64.,
            alive: true,
            blocked: false,
            noclip: false,
            height: 72.,
        }
    }
    fn hanging() -> Barnacle {
        let mut b = Barnacle::new(4, 16., 7);
        b.altitude = 284.;
        b
    }
    /// A barnacle holding the player, eyes `depth` under it.
    fn lifting(depth: f32) -> Barnacle {
        let mut b = hanging();
        let mut out = Vec::new();
        let mut f = facts(0.);
        f.touching = Some((Prey::Player, ORIGIN - Vec3::Z * depth));
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Lifting);
        b
    }

    #[test]
    fn the_tongue_lowers_at_the_pull_speed_and_stops_at_the_rest_height() {
        let mut b = Barnacle::new(0, 16., 7);
        let mut out = Vec::new();
        b.altitude = 100.;
        b.think(&facts(0.), &mut out);
        assert!((b.altitude - 108.).abs() < 1e-3);
        b.altitude = 283.;
        b.think(&facts(0.1), &mut out);
        assert_eq!(b.altitude, 284.);
        assert_eq!(b.reach(300.), 284.);
        assert_eq!(b.reach(10.), MIN_TONGUE);
    }

    #[test]
    fn a_tongue_grabs_only_when_fully_down() {
        let mut b = hanging();
        b.altitude = 100.;
        let mut f = facts(0.);
        f.touching = Some((Prey::Player, Vec3::Z * 64.));
        let mut out = Vec::new();
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Hanging);
        b.altitude = 284.;
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Lifting);
        assert_eq!(b.prey, Some(Prey::Player));
        assert!(out.contains(&Command::Sound("NPC_Barnacle.BreakNeck")));
        assert!(out.contains(&Command::Activity("ACT_BARNACLE_SLURP")));
        assert_eq!(b.altitude, 236.);
    }

    #[test]
    fn a_short_lift_still_takes_three_seconds() {
        let b = lifting(120.);
        assert!((b.pull_speed - 40.).abs() < 1e-3);
        assert_eq!(lifting(280.).pull_speed, 80.);
    }

    #[test]
    fn the_pull_pulses_and_rises_about_two_thirds_of_full_speed() {
        let mut b = lifting(280.);
        let start = b.altitude;
        let mut out = Vec::new();
        let mut z = 20.;
        for i in 1..=30 {
            let mut f = facts(i as f64 * 0.1);
            f.prey = Some(player_at(z));
            b.think(&f, &mut out);
            z += 4.;
        }
        let per_second = (start - b.altitude) / 3.;
        assert!((40.0..62.).contains(&per_second), "{per_second}");
    }

    #[test]
    fn the_player_is_bitten_with_his_eyes_short_of_the_mouth() {
        let mut b = lifting(100.);
        let mut f = facts(1.);
        let mut out = Vec::new();
        f.prey = Some(player_at(ORIGIN.z - 64. - 86.));
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Lifting);
        f.prey = Some(player_at(ORIGIN.z - 64. - 84.));
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Biting);
        assert!(out.contains(&Command::Activity("ACT_BARNACLE_BITE_PLAYER")));
    }

    #[test]
    fn biting_the_player_repeats_and_keeps_him() {
        let mut b = lifting(100.);
        b.phase = Phase::Biting;
        let mut out = Vec::new();
        b.bite(1., &mut out);
        b.bite(2.5, &mut out);
        assert_eq!(
            out.iter()
                .filter(|c| matches!(c, Command::Bite(Prey::Player)))
                .count(),
            2
        );
        assert_eq!(b.phase, Phase::Biting);
        assert_eq!(b.prey, Some(Prey::Player));
    }

    #[test]
    fn a_dead_player_is_dropped() {
        let mut b = lifting(100.);
        b.phase = Phase::Biting;
        let mut f = facts(1.);
        f.prey = Some(PreyFacts {
            alive: false,
            ..player_at(150.)
        });
        let mut out = Vec::new();
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Hanging);
        assert!(out.contains(&Command::Release(Prey::Player)));
    }

    #[test]
    fn an_npc_is_eaten_and_digested_then_the_tongue_returns() {
        let mut b = hanging();
        let mut out = Vec::new();
        let mut f = facts(0.);
        f.touching = Some((Prey::Npc(9), Vec3::Z * 64.));
        b.think(&f, &mut out);
        f.touching = None;
        f.tip_z = ORIGIN.z - 50.;
        f.prey = Some(player_at(200.));
        b.think(&f, &mut out);
        assert!(out.contains(&Command::Activity("ACT_BARNACLE_BITE_HUMAN")));
        b.bite(5., &mut out);
        assert!(out.contains(&Command::Bite(Prey::Npc(9))));
        assert_eq!(b.phase, Phase::Digesting(15.));
        assert_eq!(b.prey, None);
        f.prey = None;
        f.now = 6.;
        f.sequence_finished = true;
        b.think(&f, &mut out);
        assert!(out.contains(&Command::Activity("ACT_BARNACLE_CHEW_HUMAN")));
        out.clear();
        b.think(&f, &mut out);
        assert!(out.contains(&Command::Activity("ACT_IDLE")));
        f.now = 15.;
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Hanging);
    }

    #[test]
    fn small_prey_takes_the_small_bite() {
        let mut b = hanging();
        let mut out = Vec::new();
        let mut f = facts(0.);
        f.touching = Some((Prey::Npc(9), Vec3::Z * 12.));
        b.think(&f, &mut out);
        f.touching = None;
        f.tip_z = ORIGIN.z - 50.;
        f.prey = Some(PreyFacts {
            height: 24.,
            ..player_at(200.)
        });
        b.think(&f, &mut out);
        assert!(out.contains(&Command::Activity("ACT_BARNACLE_BITE_SMALL_THINGS")));
    }

    #[test]
    fn a_prop_is_shaken_then_spat_and_not_taken_again() {
        let mut b = hanging();
        let mut out = Vec::new();
        let mut f = facts(0.);
        f.touching = Some((Prey::Prop(5), Vec3::Z * 20.));
        b.think(&f, &mut out);
        f.touching = None;
        f.tip_z = ORIGIN.z - 50.;
        f.prey = Some(player_at(200.));
        let mut heights = Vec::new();
        for _ in 0..PROP_SHAKES {
            b.think(&f, &mut out);
            assert_eq!(b.phase, Phase::Lifting);
            heights.push(b.altitude);
        }
        assert_eq!(heights, [70., 55., 70., 55., 70., 55.]);
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Biting);
        assert!(out.contains(&Command::Activity("ACT_BARNACLE_TASTE_SPIT")));
        b.spit(&mut out);
        assert!(out.contains(&Command::Spit(Prey::Prop(5))));
        assert_eq!(b.phase, Phase::Hanging);
        f.prey = None;
        f.touching = Some((Prey::Prop(5), Vec3::Z * 20.));
        b.altitude = 284.;
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Hanging);
    }

    #[test]
    fn blocked_prey_is_dropped_and_ignored_but_the_player_can_be_retaken() {
        let mut b = hanging();
        let mut out = Vec::new();
        let mut f = facts(0.);
        f.touching = Some((Prey::Prop(5), Vec3::Z * 20.));
        b.think(&f, &mut out);
        f.prey = Some(PreyFacts {
            blocked: true,
            ..player_at(20.)
        });
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Hanging);
        assert_eq!(b.ignored, Some(Prey::Prop(5)));
        let mut b = lifting(280.);
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Hanging);
        assert_eq!(b.ignored, None);
    }

    #[test]
    fn a_flying_player_is_released() {
        let mut b = lifting(280.);
        let mut f = facts(1.);
        f.prey = Some(PreyFacts {
            noclip: true,
            ..player_at(20.)
        });
        let mut out = Vec::new();
        b.think(&f, &mut out);
        assert!(out.contains(&Command::Release(Prey::Player)));
    }

    #[test]
    fn prey_that_stops_rising_is_dropped_after_the_stuck_time() {
        let mut b = lifting(280.);
        let mut out = Vec::new();
        let mut t = 0.;
        while b.phase == Phase::Lifting && t < 10. {
            t += 0.1;
            let mut f = facts(t);
            f.prey = Some(player_at(20.));
            b.think(&f, &mut out);
        }
        assert_eq!(b.phase, Phase::Hanging);
        assert!((3.0..6.).contains(&t), "{t}");
    }

    #[test]
    fn an_ambush_tongue_stays_up_until_it_drops() {
        let mut b = Barnacle::new(SF_AMBUSH, 16., 7);
        let mut out = Vec::new();
        b.think(&facts(0.), &mut out);
        assert_eq!(b.altitude, MIN_TONGUE);
        b.drop_tongue();
        b.think(&facts(0.1), &mut out);
        assert!(b.altitude > MIN_TONGUE);
    }

    #[test]
    fn death_drops_the_prey_and_leaves_a_tongue_clear_of_the_floor() {
        let mut b = lifting(100.);
        let mut out = Vec::new();
        b.die(&mut out);
        assert!(out.contains(&Command::Release(Prey::Player)));
        assert!(out.contains(&Command::Activity("ACT_DIESIMPLE")));
        assert_eq!(b.phase, Phase::Dying);
        assert_eq!(dead_altitude(None), 164.);
        assert_eq!(dead_altitude(Some(400.)), 164.);
        assert_eq!(dead_altitude(Some(200.)), 122.);
        assert_eq!(dead_altitude(Some(100.)), 50.);
        let mut f = facts(1.);
        f.floor_distance = Some(400.);
        f.sequence_finished = true;
        b.altitude = 164.;
        b.think(&f, &mut out);
        assert_eq!(b.phase, Phase::Dead);
    }

    #[test]
    fn a_rope_settles_straight_and_sleeps() {
        let root = Vec3::Z * 300.;
        let mut t = Tongue::hanging(root, 200.);
        for _ in 0..400 {
            t.swing(root, 200., 0.015);
            t.step_rope(root, 200., 0.015);
        }
        assert!((t.tip.z - 100.).abs() < 0.5, "{}", t.tip);
        assert!(t.asleep);
        assert!(!t.step_rope(root, 200., 0.015));
        for (i, node) in t.nodes.iter().enumerate() {
            let want = 300. - 200. * i as f32 / 7.;
            assert!((node.z - want).abs() < 1.5, "{i} {node}");
        }
    }

    #[test]
    fn a_tip_held_to_one_side_swings_in_as_the_line_shortens() {
        let root = Vec3::Z * 300.;
        let mut t = Tongue::hanging(root, 200.);
        t.attach(Vec3::new(60., 0., 100.), None);
        let mut length = 208.9;
        for _ in 0..300 {
            length -= 0.2;
            t.swing(root, length, 0.015);
        }
        assert!(t.tip.distance(root) <= length + 0.01);
        assert!(t.tip.x.abs() < 60.);
    }

    #[test]
    fn the_drawn_tip_trails_a_grab_and_keeps_out_of_the_camera() {
        let root = Vec3::Z * 300.;
        let mut t = Tongue::hanging(root, 200.);
        t.attach(Vec3::new(30., 0., 100.), Some(90.));
        t.step_rope(root, 200., 0.015);
        assert!(t.nodes[NODES - 1].distance(Vec3::new(0., 0., 100.)) <= 3.01);
        for _ in 0..60 {
            t.step_rope(root, 200., 0.015);
        }
        let last = t.nodes[NODES - 1];
        assert!(last.distance(Vec3::new(30., 24., 92.)) < 0.01, "{last}");
    }

    #[test]
    fn tongue_bones_follow_the_rope_and_keep_their_bind_turn_when_straight() {
        use modkit_core::animation::{Bone, Pose, Rig};
        let bind = Quat::from_rotation_y(90f32.to_radians());
        let bone = |name: &str, z: f32| {
            let matrix = Mat4::from_rotation_translation(bind, Vec3::Z * z);
            Bone {
                name: name.into(),
                parent: None,
                bind: Pose {
                    position: Vec3::Z * z,
                    rotation: bind,
                },
                inverse_bind: matrix.inverse(),
            }
        };
        let mut bones = vec![bone("Barnacle.base", 0.)];
        for i in 0..NODES {
            bones.push(bone(&format!("Barnacle.tongue{}", i + 1), -10. * i as f32));
        }
        let rig = Rig {
            bones,
            ..Default::default()
        };
        let straight = TonguePose {
            nodes: std::array::from_fn(|i| Vec3::Z * (-10. * i as f32)),
            frame: 1,
        };
        let mut matrices = vec![Mat4::IDENTITY; NODES + 1];
        pose_tongue(&rig, &straight, Mat4::IDENTITY, &mut matrices);
        for matrix in &matrices {
            assert!(matrix.abs_diff_eq(Mat4::IDENTITY, 1e-4), "{matrix}");
        }
        let stretched = TonguePose {
            nodes: std::array::from_fn(|i| Vec3::new(5. * i as f32, 0., -10. * i as f32)),
            frame: 2,
        };
        pose_tongue(&rig, &stretched, Mat4::IDENTITY, &mut matrices);
        let last = matrices[NODES] * rig.bones[NODES].inverse_bind.inverse();
        let moved = last.transform_point3(Vec3::ZERO);
        assert!(moved.distance(stretched.nodes[NODES - 1]) < 1e-3, "{moved}");
    }
}
