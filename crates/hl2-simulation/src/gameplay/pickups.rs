// SDK items.h/item_ammo.cpp quantities/aliases; difficulty scaling and crates remain separate.
pub(super) fn ammo_pickup(class: &str) -> Option<(&'static str, i32)> {
    Some(match class {
        "item_ammo_pistol" | "item_box_srounds" => ("Pistol", 20),
        "item_ammo_pistol_large" | "item_large_box_srounds" => ("Pistol", 100),
        "item_ammo_smg1" | "item_box_mrounds" => ("SMG1", 45),
        "item_ammo_smg1_large" | "item_large_box_mrounds" => ("SMG1", 225),
        "item_ammo_ar2" | "item_box_lrounds" => ("AR2", 20),
        "item_ammo_ar2_large" | "item_large_box_lrounds" => ("AR2", 100),
        "item_ammo_357" => ("357", 6),
        "item_ammo_357_large" => ("357", 20),
        "item_box_buckshot" => ("Buckshot", 20),
        "item_ammo_crossbow" => ("XBowBolt", 6),
        "item_rpg_round" | "item_ml_grenade" => ("RPG_Round", 1),
        "item_ammo_smg1_grenade" | "item_ar2_grenade" => ("SMG1_Grenade", 1),
        "item_ammo_ar2_altfire" => ("AR2AltFire", 1),
        _ => return None,
    })
}
