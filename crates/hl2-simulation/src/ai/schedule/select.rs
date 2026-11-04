//! CAI_BaseNPC's schedule selection for classes that inherit it.
use super::super::conditions::{
    COND_CAN_MELEE_ATTACK1, COND_CAN_MELEE_ATTACK2, COND_CAN_RANGE_ATTACK1, COND_CAN_RANGE_ATTACK2,
    COND_ENEMY_DEAD, COND_ENEMY_OCCLUDED, COND_HEAR_BULLET_IMPACT, COND_HEAR_COMBAT,
    COND_HEAR_DANGER, COND_HEAR_PLAYER, COND_HEAR_WORLD, COND_LOW_PRIMARY_AMMO, COND_NEW_ENEMY,
    COND_NOT_FACING_ATTACK, COND_NO_PRIMARY_AMMO, COND_SEE_ENEMY, COND_TOO_CLOSE_TO_ATTACK,
    COND_TOO_FAR_TO_ATTACK, COND_WEAPON_BLOCKED_BY_FRIEND, COND_WEAPON_PLAYER_IN_SPREAD,
    COND_WEAPON_SIGHT_OCCLUDED,
};
use super::super::state::NpcState;
use super::{AiNpc, Context};

/// The base CAI_BaseNPC::SelectSchedule (idle/alert/combat/dead), without flinch,
/// interactions, weapons/shot regulator and squad attack slots, which classes add.
pub fn default_select_schedule(npc: &AiNpc, ctx: &Context) -> String {
    let c = &npc.conditions;
    let hearing = [
        COND_HEAR_DANGER,
        COND_HEAR_COMBAT,
        COND_HEAR_WORLD,
        COND_HEAR_BULLET_IMPACT,
        COND_HEAR_PLAYER,
    ];
    match npc.state {
        NpcState::Idle => {
            if hearing.iter().any(|h| c.has(*h)) {
                "SCHED_ALERT_FACE_BESTSOUND"
            } else {
                "SCHED_IDLE_STAND"
            }
        }
        NpcState::Alert => {
            if hearing.iter().any(|h| c.has(*h)) {
                "SCHED_ALERT_FACE_BESTSOUND"
            } else if ctx.now - npc.enemies.last_time_seen(None)
                < super::super::state::TIME_CARE_ABOUT_DAMAGE
            {
                "SCHED_ALERT_FACE"
            } else {
                "SCHED_ALERT_STAND"
            }
        }
        NpcState::Combat => {
            if npc.wakes_angry(ctx.now) {
                "SCHED_WAKE_ANGRY"
            } else if !c.has(COND_SEE_ENEMY) {
                if !c.has(COND_ENEMY_OCCLUDED) {
                    "SCHED_COMBAT_FACE"
                } else {
                    "SCHED_CHASE_ENEMY"
                }
            } else if c.has(COND_CAN_RANGE_ATTACK1) {
                "SCHED_RANGE_ATTACK1"
            } else if c.has(COND_CAN_MELEE_ATTACK1) {
                "SCHED_MELEE_ATTACK1"
            } else if c.has(COND_NOT_FACING_ATTACK) {
                "SCHED_COMBAT_FACE"
            } else {
                "SCHED_CHASE_ENEMY"
            }
        }
        NpcState::Dead => "SCHED_DIE",
        NpcState::Script => "SCHED_AISCRIPT",
        _ => "SCHED_IDLE_STAND",
    }
    .into()
}

/// CAI_BaseNPC::SelectSchedule for a class that inherits it: the idle/alert/dead/script
/// branches of `default_select_schedule` and, in combat, `base_combat_schedule`.
pub fn base_select_schedule(npc: &mut AiNpc, ctx: &Context, armed: bool) -> String {
    if npc.state == NpcState::Combat {
        return base_combat_schedule(npc, ctx, armed);
    }
    default_select_schedule(npc, ctx)
}
/// CAI_BaseNPC::SelectCombatSchedule for an NPC with a gun (`armed`: GetActiveWeapon)
/// or an innate range attack (`ctx.innate_range`, the headcrabs); innate melee
/// capabilities are not modeled (the zombies select their own).
/// Not modeled: forced interactions, flinches, D_FR enemies (FearSound/RUN_FROM_ENEMY)
/// and squad attack slots (no squads: every slot is free).
pub fn base_combat_schedule(npc: &mut AiNpc, ctx: &Context, armed: bool) -> String {
    let c = npc.conditions;
    if c.has(COND_NEW_ENEMY)
        && npc
            .enemy
            .and_then(|e| npc.enemies.find(Some(e)))
            .is_some_and(|m| ctx.now - m.time_first_seen < 2.)
    {
        return "SCHED_WAKE_ANGRY".into();
    }
    if c.has(COND_ENEMY_DEAD) {
        // SetEnemy(NULL) + ChooseEnemy happened in the runtime's gather; another enemy
        // reselects, none goes alert.
        npc.conditions.clear(COND_ENEMY_DEAD);
        if npc.enemy.is_some() {
            return base_combat_schedule(npc, ctx, armed);
        }
        npc.state = NpcState::Alert;
        return default_select_schedule(npc, ctx);
    }
    if c.has(COND_LOW_PRIMARY_AMMO) || c.has(COND_NO_PRIMARY_AMMO) {
        return "SCHED_HIDE_AND_RELOAD".into();
    }
    if !c.has(COND_SEE_ENEMY) {
        return if !c.has(COND_ENEMY_OCCLUDED) {
            "SCHED_COMBAT_FACE"
        } else if armed || ctx.innate_range.is_some() {
            "SCHED_ESTABLISH_LINE_OF_FIRE"
        } else {
            "SCHED_TAKE_COVER_FROM_ENEMY"
        }
        .into();
    }
    if c.has(COND_TOO_CLOSE_TO_ATTACK) {
        return "SCHED_BACK_AWAY_FROM_ENEMY".into();
    }
    if c.has(COND_WEAPON_PLAYER_IN_SPREAD)
        || c.has(COND_WEAPON_BLOCKED_BY_FRIEND)
        || c.has(COND_WEAPON_SIGHT_OCCLUDED)
    {
        return "SCHED_ESTABLISH_LINE_OF_FIRE".into();
    }
    if npc
        .shot_regulator
        .as_ref()
        .is_some_and(|r| r.is_in_rest_interval(ctx.now))
        && c.has(COND_CAN_RANGE_ATTACK1)
    {
        return "SCHED_COMBAT_FACE".into();
    }
    for (cond, schedule) in [
        (COND_CAN_RANGE_ATTACK1, "SCHED_RANGE_ATTACK1"),
        (COND_CAN_RANGE_ATTACK2, "SCHED_RANGE_ATTACK2"),
        (COND_CAN_MELEE_ATTACK1, "SCHED_MELEE_ATTACK1"),
        (COND_CAN_MELEE_ATTACK2, "SCHED_MELEE_ATTACK2"),
        (COND_NOT_FACING_ATTACK, "SCHED_COMBAT_FACE"),
    ] {
        if c.has(cond) {
            return schedule.into();
        }
    }
    // An innate range attack too far away gets line of sight; nothing else to do hides.
    if armed || c.has(COND_TOO_FAR_TO_ATTACK) && ctx.innate_range.is_some() {
        "SCHED_MOVE_TO_WEAPON_RANGE"
    } else {
        "SCHED_TAKE_COVER_FROM_ENEMY"
    }
    .into()
}
