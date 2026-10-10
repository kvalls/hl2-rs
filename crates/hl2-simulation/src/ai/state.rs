//! NPC_STATE (ai_npcstate.h) and CAI_BaseNPC::SelectIdealState with the idle, alert
//! and script selectors (ai_basenpc.cpp). The yaw side effects (SetIdealYawToTarget
//! toward the enemy/sound) are returned for the motor. SDK behavior.
use super::conditions::*;
use glam::Vec3;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum NpcState {
    #[default]
    None,
    Idle,
    Alert,
    Combat,
    Script,
    PlayDead,
    Prone,
    Dead,
}
/// TIME_CARE_ABOUT_DAMAGE.
pub const TIME_CARE_ABOUT_DAMAGE: f64 = 3.;

/// What SelectIdealState looks at.
pub struct StateInputs {
    pub conditions: Conditions,
    pub has_enemy: bool,
    /// Seconds since an unknown enemy (danger memory) was last seen.
    pub since_unknown_enemy: f64,
    /// Where to face for damage/an enemy (enemy LKP or the danger memory).
    pub threat: Option<Vec3>,
    /// The best sound's react origin and whether it is combat/danger/bullet impact.
    pub best_sound: Option<(Vec3, bool)>,
    /// ShouldGoToIdleState (false in the base class).
    pub go_idle: bool,
    pub ideal: NpcState,
}

/// Returns the ideal state and an optional ideal-yaw target.
pub fn select_ideal_state(state: NpcState, i: &StateInputs) -> (NpcState, Option<Vec3>) {
    let c = &i.conditions;
    let damaged = c.has(COND_LIGHT_DAMAGE)
        || c.has(COND_HEAVY_DAMAGE)
        || !i.has_enemy && i.since_unknown_enemy < TIME_CARE_ABOUT_DAMAGE;
    match state {
        NpcState::Idle => {
            if c.has(COND_NEW_ENEMY) || c.has(COND_SEE_ENEMY) {
                return (NpcState::Combat, None);
            }
            if damaged {
                return (NpcState::Alert, i.threat);
            }
            let hearing = [
                COND_HEAR_DANGER,
                COND_HEAR_COMBAT,
                COND_HEAR_WORLD,
                COND_HEAR_PLAYER,
                COND_HEAR_THUMPER,
                COND_HEAR_BULLET_IMPACT,
            ];
            if hearing.iter().any(|h| c.has(*h)) {
                if let Some((origin, alarming)) = i.best_sound {
                    if alarming {
                        return (NpcState::Alert, Some(origin));
                    }
                    return (i.ideal, Some(origin));
                }
            }
            if c.has(COND_SMELL) {
                return (NpcState::Alert, None);
            }
            (i.ideal, None)
        }
        NpcState::Alert => {
            if c.has(COND_NEW_ENEMY) || c.has(COND_SEE_ENEMY) || i.has_enemy {
                return (NpcState::Combat, None);
            }
            if damaged {
                return (NpcState::Alert, i.threat);
            }
            if c.has(COND_HEAR_DANGER) || c.has(COND_HEAR_COMBAT) {
                return (NpcState::Alert, i.best_sound.map(|s| s.0));
            }
            if i.go_idle {
                return (NpcState::Idle, None);
            }
            (i.ideal, None)
        }
        NpcState::Combat if !i.has_enemy => (NpcState::Alert, None),
        NpcState::Dead => (NpcState::Dead, None),
        // SelectScriptIdealState only leaves when the script ends (ExitScriptedSequence).
        _ => (i.ideal, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs(conditions: &[Cond]) -> StateInputs {
        StateInputs {
            conditions: Conditions::of(conditions),
            has_enemy: false,
            since_unknown_enemy: 999.,
            threat: None,
            best_sound: None,
            go_idle: false,
            ideal: NpcState::Idle,
        }
    }
    #[test]
    fn ideal_state_follows_the_sdk_selectors() {
        assert_eq!(
            select_ideal_state(NpcState::Idle, &inputs(&[COND_SEE_ENEMY])).0,
            NpcState::Combat
        );
        let mut i = inputs(&[COND_LIGHT_DAMAGE]);
        i.threat = Some(Vec3::X);
        assert_eq!(
            select_ideal_state(NpcState::Idle, &i),
            (NpcState::Alert, Some(Vec3::X))
        );
        let mut i = inputs(&[COND_HEAR_PLAYER]);
        i.best_sound = Some((Vec3::Y, false));
        assert_eq!(
            select_ideal_state(NpcState::Idle, &i),
            (NpcState::Idle, Some(Vec3::Y))
        );
        i.best_sound = Some((Vec3::Y, true));
        assert_eq!(select_ideal_state(NpcState::Idle, &i).0, NpcState::Alert);
        let mut i = inputs(&[]);
        i.has_enemy = false;
        assert_eq!(select_ideal_state(NpcState::Combat, &i).0, NpcState::Alert);
        i.has_enemy = true;
        assert_eq!(select_ideal_state(NpcState::Alert, &i).0, NpcState::Combat);
        let mut i = inputs(&[]);
        i.go_idle = true;
        i.ideal = NpcState::Alert;
        assert_eq!(select_ideal_state(NpcState::Alert, &i).0, NpcState::Idle);
    }
}
