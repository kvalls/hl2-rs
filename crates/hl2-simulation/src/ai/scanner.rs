//! npc_cscanner (DESIGN 14o session 46, step 2): a hovering Combine scanner. This first step holds
//! what it decides between: patrolling, flinching, and closing on an enemy from high up. Its
//! flight is `ai::hover`; the spotlight, photographs and inspection are step 3, the death dive
//! step 4, and until then a scanner with something to inspect patrols.
use super::conditions::{COND_HEAVY_DAMAGE, COND_LIGHT_DAMAGE, COND_LOST_ENEMY};
use super::hover::Hover;
use super::schedule::{AiNpc, Context};
use super::state::NpcState;
use glam::Vec3;
use serde::Serialize;

/// m_flFieldOfView.
pub const FIELD_OF_VIEW: f32 = 0.2;
/// SetViewOffset's height.
pub const EYE_HEIGHT: f32 = 10.;
/// SCANNER_ATTACK_FAR_DIST: past this it stays high and approaches with its spotlight.
const ATTACK_FAR: f32 = 300.;
/// How long a patrol's random route can be.
pub const PATROL_ROUTE: f32 = 2000.;
/// The share of its health below which an alert scanner hurt in the open takes cover.
const COVER_HEALTH: f32 = 0.75;
/// Out of ten: how often a hit in combat makes it flinch.
const COMBAT_FLINCH: f32 = 0.4;

/// What a scanner does next. The names follow its schedules, which step 2 adds to the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Plan {
    Script,
    Flinch,
    TakeCover,
    Patrol,
    SpotlightHover,
    AttackHover,
}

/// A scanner under the AI.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Scanner {
    pub hover: Hover,
    /// m_flNextAttack.
    pub next_attack: f64,
    /// m_bShouldInspect: the InputShouldInspect switch.
    pub ignores_enemies: bool,
}
impl Scanner {
    /// The order of its choices. `health` is the share it has left, `roll` in [0, 1).
    pub fn plan(&self, npc: &AiNpc, ctx: &Context, health: f32, roll: f32) -> Plan {
        if npc.state == NpcState::Script {
            return Plan::Script;
        }
        let hurt = npc.conditions.has(COND_LIGHT_DAMAGE) || npc.conditions.has(COND_HEAVY_DAMAGE);
        if hurt {
            match npc.state {
                NpcState::Idle => return Plan::Flinch,
                NpcState::Alert if health < COVER_HEALTH => return Plan::TakeCover,
                NpcState::Alert => return Plan::Flinch,
                _ if roll < COMBAT_FLINCH => return Plan::Flinch,
                _ => {}
            }
        }
        let Some((_, enemy)) = ctx.enemy.filter(|_| !self.ignores_enemies) else {
            return Plan::Patrol;
        };
        if npc.conditions.has(COND_LOST_ENEMY) {
            return Plan::Patrol;
        }
        if ctx.now < self.next_attack {
            return Plan::SpotlightHover;
        }
        let across = ctx.enemy_lkp.unwrap_or(enemy) - ctx.origin;
        if Vec3::new(across.x, across.y, 0.).length() > ATTACK_FAR {
            Plan::SpotlightHover
        } else {
            Plan::AttackHover
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::conditions::{Cond, Conditions};

    fn npc(state: NpcState, conditions: &[Cond]) -> AiNpc {
        let mut npc = AiNpc::new(1);
        npc.state = state;
        npc.conditions = Conditions::of(conditions);
        npc
    }
    fn hunting(distance: f32, now: f64) -> Context {
        Context {
            now,
            enemy: Some((0, Vec3::X * distance)),
            ..Default::default()
        }
    }

    #[test]
    fn alone_it_patrols() {
        let scanner = Scanner::default();
        let plan = scanner.plan(&npc(NpcState::Idle, &[]), &Context::default(), 1., 0.5);
        assert_eq!(plan, Plan::Patrol);
    }

    #[test]
    fn a_script_owns_it_before_anything_else() {
        let scanner = Scanner::default();
        let scripted = npc(NpcState::Script, &[COND_LIGHT_DAMAGE]);
        let plan = scanner.plan(&scripted, &hunting(100., 1.), 1., 0.);
        assert_eq!(plan, Plan::Script);
    }

    #[test]
    fn a_hit_flinches_or_sends_it_to_cover_by_state_and_health() {
        let s = Scanner::default();
        let ctx = Context::default();
        let hit = |state| npc(state, &[COND_LIGHT_DAMAGE]);
        assert_eq!(s.plan(&hit(NpcState::Idle), &ctx, 0.1, 0.9), Plan::Flinch);
        assert_eq!(s.plan(&hit(NpcState::Alert), &ctx, 0.9, 0.9), Plan::Flinch);
        assert_eq!(
            s.plan(&hit(NpcState::Alert), &ctx, 0.5, 0.9),
            Plan::TakeCover
        );
        assert_eq!(s.plan(&hit(NpcState::Combat), &ctx, 1., 0.1), Plan::Flinch);
        assert_eq!(s.plan(&hit(NpcState::Combat), &ctx, 1., 0.9), Plan::Patrol);
    }

    #[test]
    fn it_stays_high_when_far_and_comes_down_when_near() {
        let s = Scanner::default();
        let combat = npc(NpcState::Combat, &[]);
        assert_eq!(
            s.plan(&combat, &hunting(500., 1.), 1., 0.5),
            Plan::SpotlightHover
        );
        assert_eq!(
            s.plan(&combat, &hunting(200., 1.), 1., 0.5),
            Plan::AttackHover
        );
    }

    #[test]
    fn height_does_not_count_toward_the_attack_distance() {
        let s = Scanner::default();
        let combat = npc(NpcState::Combat, &[]);
        let above = Context {
            now: 1.,
            enemy: Some((0, Vec3::new(100., 0., -900.))),
            ..Default::default()
        };
        assert_eq!(s.plan(&combat, &above, 1., 0.5), Plan::AttackHover);
    }

    #[test]
    fn between_attacks_it_only_hovers() {
        let s = Scanner {
            next_attack: 10.,
            ..Default::default()
        };
        let combat = npc(NpcState::Combat, &[]);
        assert_eq!(
            s.plan(&combat, &hunting(100., 5.), 1., 0.5),
            Plan::SpotlightHover
        );
    }

    #[test]
    fn a_lost_enemy_or_the_switch_sends_it_back_on_patrol() {
        let s = Scanner::default();
        let lost = npc(NpcState::Combat, &[COND_LOST_ENEMY]);
        assert_eq!(s.plan(&lost, &hunting(100., 1.), 1., 0.5), Plan::Patrol);
        let told = Scanner {
            ignores_enemies: true,
            ..Default::default()
        };
        let combat = npc(NpcState::Combat, &[]);
        assert_eq!(
            told.plan(&combat, &hunting(100., 1.), 1., 0.5),
            Plan::Patrol
        );
    }
}
