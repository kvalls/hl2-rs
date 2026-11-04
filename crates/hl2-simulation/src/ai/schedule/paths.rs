//! The line-of-sight and cover searches of the generic path tasks.
use super::{Context, Motor, TaskStatus};
use glam::Vec3;
use TaskStatus::{Complete, Failed};

pub(super) fn enemy_los(task: &str, ctx: &Context, motor: &mut dyn Motor) -> TaskStatus {
    // FindLos from the enemy's origin (LKP for the _LKP_ task) and eye, within
    // the weapon's range (2000 without one) capped by m_flDistTooFar; the
    // lateral LOS probe is not modeled.
    let Some((_, origin)) = ctx.enemy else {
        return Failed;
    };
    let (min, max) = ctx.weapon_range.or(ctx.innate_range).unwrap_or((0., 2000.));
    let max = max.min(if ctx.dist_too_far > 0. {
        ctx.dist_too_far
    } else {
        1024.
    });
    let threat = if task == "TASK_GET_PATH_TO_ENEMY_LOS" {
        origin
    } else {
        ctx.enemy_lkp.unwrap_or(origin)
    };
    let eye = threat + (ctx.enemy_eye.unwrap_or(origin + Vec3::Z * 64.) - origin);
    match motor.find_los(threat, eye, min, max) {
        Some(goal) if motor.path_to(goal, true) => Complete,
        _ => Failed,
    }
}

pub(super) fn cover(task: &str, ctx: &Context, motor: &mut dyn Motor) -> TaskStatus {
    // FindCoverFromEnemy -> FindCoverPos(enemy origin, enemy eye, 0,
    // CoverRadius 1024) (self when there is no enemy; FROM_ORIGIN: from the
    // NPC's own origin and eye). FindLateralCover is not modeled.
    let (threat, eye) = match (task, ctx.enemy) {
        ("TASK_FIND_COVER_FROM_ENEMY", Some((_, p))) => {
            (p, ctx.enemy_eye.unwrap_or(p + Vec3::Z * 64.))
        }
        _ => (ctx.origin, ctx.origin + Vec3::Z * 64.),
    };
    match motor.find_cover(threat, eye, 0., 1024.) {
        Some(goal) if motor.path_to(goal, true) => {
            // m_flMoveWaitFinished = now + flTaskData.
            Complete
        }
        _ => Failed,
    }
}

pub(super) fn cover_from_sound(ctx: &Context, motor: &mut dyn Motor) -> TaskStatus {
    // FindCoverPos(sound react origin, same, MIN(volume, 120), 1024).
    match ctx.best_sound {
        Some(sound) => {
            let min = ctx.best_sound_volume.min(120.);
            match motor.find_cover(sound, sound, min, 1024.) {
                Some(goal) if motor.path_to(goal, true) => Complete,
                _ => Failed,
            }
        }
        None => Failed,
    }
}
