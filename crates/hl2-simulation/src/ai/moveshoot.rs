//! CAI_MoveAndShootOverlay (ai_moveshoot.cpp): NPCs with bits_CAP_MOVE_SHOOT shoot
//! while a continuous-move task runs. The movement switches to the aiming walk/run, the
//! body faces the enemy's last known position (AddFacingTarget 0.8 s) while the legs
//! follow the path (move_yaw), and the shot regulator times the shots. Native restarts
//! the ACT_GESTURE_RANGE_ATTACK1 gesture per shot; AI gesture layers are not modeled
//! yet, so the shot is fired directly (one weapon fire event per regulator shot).
use super::conditions::{
    Conditions, COND_CAN_RANGE_ATTACK1, COND_ENEMY_DEAD, COND_ENEMY_OCCLUDED, COND_ENEMY_TOO_FAR,
    COND_TOO_FAR_TO_ATTACK,
};
use crate::npc_weapons::ShotRegulator;
use serde::Serialize;

/// CAI_BaseNPC::IsCurTaskContinuousMove.
pub fn continuous_move_task(task: &str) -> bool {
    matches!(
        task,
        "TASK_WAIT_FOR_MOVEMENT"
            | "TASK_MOVE_TO_TARGET_RANGE"
            | "TASK_MOVE_TO_GOAL_RANGE"
            | "TASK_WEAPON_RUN_PATH"
            | "TASK_PLAY_SCENE"
            | "TASK_RUN_PATH_TIMED"
            | "TASK_WALK_PATH_TIMED"
            | "TASK_RUN_PATH_FOR_UNITS"
            | "TASK_WALK_PATH_FOR_UNITS"
            | "TASK_RUN_PATH_FLEE"
            | "TASK_WALK_PATH_WITHIN_DIST"
            | "TASK_RUN_PATH_WITHIN_DIST"
    )
}

/// What the overlay asks of its host this think.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    /// UpdateMoveShootActivity: move with the aiming walk/run (true) or the plain ones.
    pub aim: Option<bool>,
    /// Fire the weapon once (OnRangeAttack1 + the gesture's fire event).
    pub shoot: bool,
    /// AddFacingTarget(enemy LKP, 0.8 s).
    pub face_enemy: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MoveShoot {
    /// m_bMovingAndShooting.
    pub moving_and_shooting: bool,
    /// m_bNoShootWhileMove.
    pub no_shoot: bool,
    /// m_flSuspendUntilTime (None: MOVESHOOT_DO_NOT_SUSPEND).
    pub suspend_until: Option<f64>,
    /// The aiming locomotion is selected.
    pub aiming: bool,
    /// The task the overlay last started on (StartTaskOverlay runs once per task).
    #[serde(skip)]
    started_task: Option<(String, usize)>,
    /// Diagnostics: shots fired while moving.
    pub shots: u32,
}
impl MoveShoot {
    /// StartShootWhileMove: no move-and-shoot in a script, without a ranged weapon or
    /// without both aiming locomotions; else FireNoEarlierThan(now + initial delay 0).
    pub fn start(
        &mut self,
        scripted: bool,
        armed: bool,
        has_aim: bool,
        regulator: Option<&mut ShotRegulator>,
        now: f64,
    ) {
        if scripted || !armed || !has_aim {
            self.no_shoot = true;
            return;
        }
        if let Some(r) = regulator {
            r.fire_no_earlier_than(now);
        }
        self.no_shoot = false;
    }
    /// StartTaskOverlay / RunTaskOverlay / EndTaskOverlay for this think's task: `task`
    /// with its schedule and index identifies a new task start.
    #[allow(clippy::too_many_arguments)]
    pub fn think(
        &mut self,
        task: Option<(&str, &str, usize)>,
        allowed: bool,
        (scripted, armed, has_aim): (bool, bool, bool),
        moving: bool,
        conditions: Conditions,
        has_enemy: bool,
        regulator: Option<&mut ShotRegulator>,
        now: f64,
    ) -> Outcome {
        let mut out = Outcome::default();
        let continuous = task.is_some_and(|(_, t, _)| continuous_move_task(t));
        let key = task.map(|(s, _, i)| (s.to_owned(), i));
        let mut regulator = regulator;
        if self.started_task != key {
            // EndTaskOverlay for the previous task, StartTaskOverlay for this one.
            self.end(regulator.as_deref_mut(), now);
            self.started_task = key;
            if continuous {
                if allowed {
                    self.start(scripted, armed, has_aim, regulator.as_deref_mut(), now);
                } else {
                    self.no_shoot = true;
                }
            }
        }
        if !continuous {
            if self.aiming {
                self.aiming = false;
                out.aim = Some(false);
            }
            return out;
        }
        // RunShootWhileMove.
        if self.no_shoot || self.suspend_until.is_some_and(|t| now < t) {
            return self.stop_aiming(out);
        }
        self.suspend_until = None;
        if !moving || !has_enemy {
            return out;
        }
        // CanAimAtEnemy.
        let can_aim = armed
            && (conditions.has(COND_CAN_RANGE_ATTACK1)
                || ![
                    COND_ENEMY_DEAD,
                    COND_TOO_FAR_TO_ATTACK,
                    COND_ENEMY_TOO_FAR,
                    COND_ENEMY_OCCLUDED,
                ]
                .iter()
                .any(|&c| conditions.has(c)));
        // UpdateMoveShootActivity: a change holds fire for 0.3 s.
        if can_aim != self.aiming {
            self.aiming = can_aim;
            out.aim = Some(can_aim);
            if let Some(r) = regulator.as_deref_mut() {
                r.fire_no_earlier_than(now + 0.3);
            }
        }
        if !can_aim {
            self.end(regulator, now);
            return out;
        }
        if let Some(r) = regulator {
            if conditions.has(COND_CAN_RANGE_ATTACK1) {
                if r.is_in_rest_interval(now) {
                    self.end(Some(r), now);
                } else if r.should_shoot(now) {
                    self.moving_and_shooting = true;
                    out.shoot = true;
                    self.shots += 1;
                }
            }
        }
        out.face_enemy = true;
        out
    }
    fn stop_aiming(&mut self, mut out: Outcome) -> Outcome {
        if self.aiming {
            self.aiming = false;
            out.aim = Some(false);
        }
        out
    }
    /// EndShootWhileMove: a burst cut short starts a rest.
    pub fn end(&mut self, regulator: Option<&mut ShotRegulator>, now: f64) {
        if self.moving_and_shooting {
            if let Some(r) = regulator {
                if !r.is_in_rest_interval(now) {
                    r.reset(false, now);
                }
            }
            self.moving_and_shooting = false;
        }
    }
    /// SuspendMoveAndShoot.
    pub fn suspend(&mut self, duration: f64, regulator: Option<&mut ShotRegulator>, now: f64) {
        self.end(regulator, now);
        self.suspend_until = Some(now + duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::conditions::COND_SEE_ENEMY;
    use crate::npc_weapons::{builtin, RegulatorRule};

    #[test]
    fn moving_npc_aims_and_fires_by_the_regulator_then_rests() {
        let spec = builtin("weapon_smg1").and_then(|d| d.fire).unwrap();
        let mut regulator = ShotRegulator::for_weapon(&spec, RegulatorRule::Weapon, 0., 7);
        let mut overlay = MoveShoot::default();
        let mut seen = Conditions::default();
        seen.set(COND_SEE_ENEMY);
        seen.set(COND_CAN_RANGE_ATTACK1);
        let task = Some(("SCHED_COMBINE_PRESS_ATTACK", "TASK_WAIT_FOR_MOVEMENT", 3));
        // First think on the task: aim (no shot for 0.3 s after the activity change).
        let out = overlay.think(
            task,
            true,
            (false, true, true),
            true,
            seen,
            true,
            Some(&mut regulator),
            1.,
        );
        assert_eq!(out.aim, Some(true));
        assert!(!out.shoot && out.face_enemy);
        let mut shots = 0;
        let mut t = 1.1;
        while t < 4. {
            let out = overlay.think(
                task,
                true,
                (false, true, true),
                true,
                seen,
                true,
                Some(&mut regulator),
                t,
            );
            if out.shoot {
                shots += 1;
                regulator.on_range_attack1(t, None);
            }
            t += 0.1;
        }
        // Bursts of 2-5 every think, separated by 0.3-0.6 s rests.
        assert!((10..=26).contains(&shots), "{shots}");
        // Occluded: stop aiming and end the burst.
        let mut occluded = Conditions::default();
        occluded.set(COND_ENEMY_OCCLUDED);
        let out = overlay.think(
            task,
            true,
            (false, true, true),
            true,
            occluded,
            true,
            Some(&mut regulator),
            4.,
        );
        assert_eq!(out.aim, Some(false));
        assert!(!out.shoot);
        // A non-moving task drops the aim locomotion.
        let out = overlay.think(
            Some(("SCHED_X", "TASK_FACE_ENEMY", 0)),
            true,
            (false, true, true),
            false,
            seen,
            true,
            Some(&mut regulator),
            4.1,
        );
        assert!(!out.shoot);
    }

    #[test]
    fn scripted_or_unarmed_npcs_never_move_and_shoot() {
        let mut overlay = MoveShoot::default();
        let mut seen = Conditions::default();
        seen.set(COND_CAN_RANGE_ATTACK1);
        let task = Some(("S", "TASK_RUN_PATH_TIMED", 1));
        let out = overlay.think(task, true, (true, true, true), true, seen, true, None, 1.);
        assert_eq!(out, Outcome::default());
        let mut overlay = MoveShoot::default();
        let out = overlay.think(task, false, (false, true, true), true, seen, true, None, 1.);
        assert_eq!(out, Outcome::default());
    }
}
