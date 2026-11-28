//! Default world models for loose CItem pickups (Source SDK game/server/hl2/item_*.cpp).

pub fn supply_model(class: &str) -> Option<&'static str> {
    Some(match class {
        "item_healthkit" => "models/items/healthkit.mdl",
        "item_healthvial" => "models/healthvial.mdl",
        "item_battery" => "models/items/battery.mdl",
        "item_suit" => "models/items/hevsuit.mdl",
        "item_ammo_pistol"
        | "item_ammo_pistol_large"
        | "item_box_srounds"
        | "item_large_box_srounds" => "models/items/boxsrounds.mdl",
        "item_ammo_smg1"
        | "item_ammo_smg1_large"
        | "item_box_mrounds"
        | "item_large_box_mrounds" => "models/items/boxmrounds.mdl",
        "item_ammo_ar2" | "item_ammo_ar2_large" | "item_box_lrounds" | "item_large_box_lrounds" => {
            "models/items/combine_rifle_cartridge01.mdl"
        }
        "item_ammo_357" => "models/items/357ammo.mdl",
        "item_ammo_357_large" => "models/items/357ammobox.mdl",
        "item_box_buckshot" => "models/items/boxbuckshot.mdl",
        "item_ammo_crossbow" => "models/items/crossbowrounds.mdl",
        "item_rpg_round" | "item_ml_grenade" => "models/weapons/w_missile_closed.mdl",
        "item_ammo_smg1_grenade" | "item_ar2_grenade" => "models/items/ar2_grenade.mdl",
        "item_ammo_ar2_altfire" => "models/items/combine_rifle_ammo01.mdl",
        _ => return None,
    })
}

pub fn physical_pickup(class: &str) -> bool {
    // Its campaign pickup stays in place (CWeaponBugBait::FallInit).
    (class.starts_with("weapon_") && class != "weapon_bugbait") || supply_model(class).is_some()
}
