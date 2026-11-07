//! Swarming (DESIGN 14o session 49). Between bursts, or with its squad's two attack slots taken, a
//! manhack flies with its squad instead of at its enemy, and an idle squad keeps together. This
//! holds the choices and the arithmetic; `ai::runtime::manhacks` knows the squad's members.
use super::super::schedule::TaskStatus;
use glam::Vec3;
use serde::Serialize;

pub const SWARM: &str = "SCHED_MANHACK_SWARM";
pub const SWARM_IDLE: &str = "SCHED_MANHACK_SWARM_IDLE";
pub const SWARM_FAILURE: &str = "SCHED_MANHACK_SWARM_FAILURE";

pub const FIND_CENTER: &str = "TASK_MANHACK_FIND_SQUAD_CENTER";
pub const FIND_MEMBER: &str = "TASK_MANHACK_FIND_SQUAD_MEMBER";
pub const MOVE_AT: &str = "TASK_MANHACK_MOVEAT_SAVEPOSITION";

/// The class's swarm schedules. SWARM_FAILURE reaches its member by flying at it: the class has no
/// route yet, so the path and wait tasks are one move here.
pub const SCHEDULES: &[(&str, &str)] = &[
    (
        SWARM,
        "Tasks TASK_STOP_MOVING 0 TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_MANHACK_SWARM_FAILURE \
         TASK_MANHACK_FIND_SQUAD_CENTER 0 TASK_MANHACK_MOVEAT_SAVEPOSITION 1 \
         Interrupts COND_NEW_ENEMY COND_CAN_MELEE_ATTACK1 COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE",
    ),
    (
        SWARM_IDLE,
        "Tasks TASK_STOP_MOVING 0 TASK_SET_FAIL_SCHEDULE SCHEDULE:SCHED_MANHACK_SWARM_FAILURE \
         TASK_MANHACK_FIND_SQUAD_CENTER 0 TASK_MANHACK_MOVEAT_SAVEPOSITION 5 \
         Interrupts COND_NEW_ENEMY COND_SEE_ENEMY COND_SEE_FEAR COND_LIGHT_DAMAGE \
         COND_HEAVY_DAMAGE COND_SMELL COND_PROVOKED COND_GIVE_WAY COND_HEAR_PLAYER \
         COND_HEAR_DANGER COND_HEAR_COMBAT COND_HEAR_BULLET_IMPACT",
    ),
    (
        SWARM_FAILURE,
        "Tasks TASK_STOP_MOVING 0 TASK_WAIT 2 TASK_WAIT_RANDOM 2 \
         TASK_MANHACK_FIND_SQUAD_MEMBER 0 TASK_MANHACK_MOVEAT_SAVEPOSITION 5 \
         Interrupts COND_SEE_ENEMY COND_NEW_ENEMY",
    ),
];

/// An attacker counts ten times in the squad's middle, and the enemy pulls with this weight.
const ATTACKER_WEIGHT: f32 = 10.;
const ENEMY_WEIGHT: f32 = 4.;
/// Closer than this it circles the point instead of closing on it.
const CIRCLE_RANGE: f32 = 64.;
/// Still this far off when time runs out, the move has failed.
const LOST_RANGE: f32 = 256.;
/// A move ends this long before its time.
const LAST_MOMENT: f64 = 0.1;
/// How far ahead it aims, closing and circling: small, so it drifts rather than darts.
const CLOSING_STEP: f32 = 10.;
const CIRCLING_STEP: f32 = 4.;
/// A move lasts its task's time, give or take this share.
const TIME_SPREAD: f32 = 0.2;
/// An attack needs its enemy this far across, and its height this far above or below the eyes.
const MELEE_ACROSS: (f32, f32) = (24., 45.);
const MELEE_HEIGHT: (f32, f32) = (-24., 12.);

/// One member of the squad, as the swarm reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Member {
    pub origin: Vec3,
    pub attacking: bool,
    pub has_goal: bool,
}

/// The squad's middle: attackers weigh more, and the enemy's last known place pulls it their way.
pub fn center(members: &[Member], enemy: Option<Vec3>) -> Option<Vec3> {
    let weighted = members.iter().map(|member| {
        let weight = if member.attacking {
            ATTACKER_WEIGHT
        } else {
            1.
        };
        (member.origin * weight, weight)
    });
    let pull = enemy.map(|place| (place * ENEMY_WEIGHT, ENEMY_WEIGHT));
    let (sum, weight) = weighted
        .chain(pull)
        .fold((Vec3::ZERO, 0.), |(sum, total), (point, weight)| {
            (sum + point, total + weight)
        });
    (weight > 0.).then(|| sum / weight)
}

/// The member worth joining: the first attacking or going somewhere, else the first.
pub fn member_to_join(members: &[Member]) -> Option<Vec3> {
    members
        .iter()
        .find(|member| member.attacking || member.has_goal)
        .or(members.first())
        .map(|member| member.origin)
}

/// Whether an attack is in reach: rested, near across the ground and level with the eyes.
pub fn in_melee(across: f32, above_eyes: f32) -> bool {
    (MELEE_ACROSS.0..=MELEE_ACROSS.1).contains(&across)
        && (MELEE_HEIGHT.0..=MELEE_HEIGHT.1).contains(&above_eyes)
}

/// What a task asked of the squad, for the runtime to answer on this tick.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Ask {
    Center,
    Member,
}

/// How a manhack stands with its squad and where its swarming takes it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Swarm {
    /// InputDisableSwarm: it chases, whatever its squad's slots say.
    pub disabled: bool,
    /// In a squad, with swarming on.
    pub squadded: bool,
    /// The squad's attack slot it holds, and the enemy that slot is for.
    pub slot: Option<(Option<usize>, i32)>,
    /// A slot is held, or it needs none.
    pub may_attack: bool,
    pub ask: Option<Ask>,
    /// m_vSavePosition.
    pub point: Vec3,
    /// m_fSwarmMoveTime and m_vSwarmMoveTarget.
    pub until: f64,
    pub target: Vec3,
    /// A new move's line has not been traced yet.
    pub unchecked: bool,
    /// The world stands between it and the point.
    pub blocked: bool,
}

impl Swarm {
    /// Where it should fly now, while a move lasts.
    pub fn flying_at(&self, now: f64) -> Option<Vec3> {
        (self.until > now).then_some(self.target)
    }
    /// The answer to a task's question.
    pub fn answer(&mut self, point: Vec3) {
        self.point = point;
        self.ask = None;
    }
    /// A move starts: its time, and a check of the line to the point.
    pub fn start_move(&mut self, seconds: f32, dice: f32, now: f64) {
        let spread = 1. - TIME_SPREAD + dice * 2. * TIME_SPREAD;
        self.until = now + f64::from(seconds * spread);
        self.unchecked = true;
        self.blocked = false;
    }
    /// One think of a move: where to aim, or how it ended.
    pub fn step(&mut self, origin: Vec3, now: f64) -> TaskStatus {
        if self.blocked {
            self.until = now;
            return TaskStatus::Failed;
        }
        let to_point = self.point - origin;
        let distance = to_point.length();
        let direction = to_point.normalize_or_zero();
        if self.until - now < LAST_MOMENT {
            return if distance > LOST_RANGE {
                TaskStatus::Failed
            } else {
                TaskStatus::Complete
            };
        }
        self.target = if distance < CIRCLE_RANGE {
            origin - Vec3::new(-direction.y, direction.x, 0.) * CIRCLING_STEP
        } else {
            origin + direction * CLOSING_STEP
        };
        TaskStatus::Running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(x: f32, attacking: bool) -> Member {
        Member {
            origin: Vec3::X * x,
            attacking,
            has_goal: false,
        }
    }

    #[test]
    fn an_attacker_counts_ten_times_in_the_middle() {
        let members = [member(0., false), member(110., true)];
        assert_eq!(center(&members, None), Some(Vec3::X * 100.));
    }

    #[test]
    fn the_enemy_pulls_the_middle_with_a_weight_of_four() {
        let members = [member(0., false)];
        assert_eq!(center(&members, Some(Vec3::X * 50.)), Some(Vec3::X * 40.));
    }

    #[test]
    fn nobody_at_all_has_no_middle() {
        assert_eq!(center(&[], None), None);
    }

    #[test]
    fn it_joins_an_attacker_or_one_going_somewhere_else_the_first() {
        let idle = [member(1., false), member(2., false)];
        assert_eq!(member_to_join(&idle), Some(Vec3::X));
        let fighting = [member(1., false), member(2., true)];
        assert_eq!(member_to_join(&fighting), Some(Vec3::X * 2.));
        let going = [
            member(1., false),
            Member {
                has_goal: true,
                ..member(3., false)
            },
        ];
        assert_eq!(member_to_join(&going), Some(Vec3::X * 3.));
        assert_eq!(member_to_join(&[]), None);
    }

    #[test]
    fn melee_needs_it_near_and_level_with_the_eyes() {
        assert!(in_melee(30., 0.));
        assert!(in_melee(24., 12.));
        assert!(in_melee(45., -24.));
        assert!(!in_melee(23., 0.));
        assert!(!in_melee(46., 0.));
        assert!(!in_melee(30., 13.));
        assert!(!in_melee(30., -25.));
    }

    #[test]
    fn a_move_lasts_its_time_give_or_take_a_fifth() {
        let mut swarm = Swarm::default();
        swarm.start_move(5., 0., 10.);
        assert!((swarm.until - 14.).abs() < 1e-5);
        swarm.start_move(5., 1., 10.);
        assert!((swarm.until - 16.).abs() < 1e-5);
    }

    #[test]
    fn far_from_the_point_it_closes_on_it() {
        let mut swarm = Swarm {
            point: Vec3::X * 500.,
            until: 5.,
            ..Default::default()
        };
        assert_eq!(swarm.step(Vec3::ZERO, 1.), TaskStatus::Running);
        assert_eq!(swarm.target, Vec3::X * 10.);
        assert_eq!(swarm.flying_at(1.), Some(Vec3::X * 10.));
    }

    #[test]
    fn near_the_point_it_circles_it() {
        let mut swarm = Swarm {
            point: Vec3::X * 30.,
            until: 5.,
            ..Default::default()
        };
        assert_eq!(swarm.step(Vec3::ZERO, 1.), TaskStatus::Running);
        assert_eq!(swarm.target, Vec3::NEG_Y * 4.);
    }

    #[test]
    fn at_the_end_of_its_time_only_a_far_one_has_failed() {
        let mut near = Swarm {
            point: Vec3::X * 200.,
            until: 5.,
            ..Default::default()
        };
        assert_eq!(near.step(Vec3::ZERO, 4.95), TaskStatus::Complete);
        let mut far = Swarm {
            point: Vec3::X * 300.,
            until: 5.,
            ..Default::default()
        };
        assert_eq!(far.step(Vec3::ZERO, 4.95), TaskStatus::Failed);
    }

    #[test]
    fn a_blocked_line_fails_the_move_and_ends_its_flight() {
        let mut swarm = Swarm {
            blocked: true,
            until: 5.,
            ..Default::default()
        };
        assert_eq!(swarm.step(Vec3::ZERO, 1.), TaskStatus::Failed);
        assert_eq!(swarm.flying_at(1.), None);
    }
}
