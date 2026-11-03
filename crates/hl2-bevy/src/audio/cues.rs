//! Sounds every map preloads besides those its entities, weapons and rigs name: HUD, player, pickup,
//! impact and effect cues the simulation can ask for at any time.

/// The fixed cues, by soundscript name or wave path.
pub(super) const FIXED: &[&str] = &[
    "Player.WeaponSelectionMoveSlot",
    "Player.WeaponSelectionClose",
    "Player.WeaponSelected",
    "Player.DenyWeaponSelection",
    "HUDQuickInfo.LowAmmo",
    "HUDQuickInfo.LowHealth",
    "NPC_CombineBall.Launch",
    "NPC_CombineBall.KillImpact",
    "NPC_CombineBall.Impact",
    "NPC_CombineBall.WhizFlyby",
    "NPC_CombineBall.Explosion",
    "BaseExplosionEffect.Sound",
    "HealthKit.Touch",
    "HealthVial.Touch",
    "ItemBattery.Touch",
    "BaseCombatCharacter.AmmoPickup",
    "Player.FallDamage",
    "Player.FallGib",
    "Player.Death",
    "ui/buttonrollover.wav",
    "ui/buttonclick.wav",
    "ui/buttonclickrelease.wav",
    "Grenade.Blip",
    "Grenade.ImpactHard",
    "HL2Player.BurnPain",
    // hl2_player.cpp FlashlightTurnOn / FlashlightTurnOff.
    hl2_simulation::flashlight::SOUND_ON,
    hl2_simulation::flashlight::SOUND_OFF,
    "Player.PlasmaDamage",
    "Player.SonicDamage",
    "Flesh.BulletImpact",
    // weapon_crossbow.cpp CCrossbowBolt::BoltTouch.
    "Weapon_Crossbow.BoltHitWorld",
    "Weapon_Crossbow.BoltHitBody",
    "Weapon_Crossbow.BoltSkewer",
    // weapon_rpg.cpp CMissile::IgniteThink.
    "Missile.Ignite",
    // weapon_physcannon.cpp GetMotorSound (a looping CSoundPatch).
    hl2_simulation::weapon_physcannon::MOTOR_SOUND,
    // weapon_bugbait.cpp squeeze, grenade_bugbait.cpp splat.
    "Weapon_Bugbait.Splat",
    "GrenadeBugBait.Splat",
    // Metrocop baton hits (weapon_stunstick.txt SoundData melee_hit/melee_miss).
    "Weapon_StunStick.Melee_Hit",
    "Weapon_StunStick.Melee_Miss",
    // npc_metropolice.cpp OnAnimEventShove (owner report: shoves were silent;
    // the request was dropped because the cue was never preloaded).
    "NPC_Metropolice.Shove",
    // npc_combine.cpp COMBINE_AE_KICK.
    "NPC_Combine.WeaponBash",
    // npc_combine.cpp COMBINE_AE_BEGIN_ALTFIRE (elites).
    "Weapon_CombineGuard.Special1",
    // props.cpp CBreakableProp::Break with PROPINTER_PHYSGUN_BREAK_EXPLODE.
    "PropaneTank.Burst",
];
