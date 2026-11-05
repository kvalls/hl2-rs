//! The models NPC classes set in code, which map entities do not name.

/// The model each class's Spawn/Precache sets when the map gives none (SDK 2013
/// hl2/npc_*.cpp; citizens pick from groups at random in the SDK, one is used here).
pub fn default_model(class: &str) -> Option<&'static str> {
    Some(match class {
        "npc_metropolice" => "models/police.mdl",
        "npc_combine_s" | "npc_sniper" => "models/combine_soldier.mdl",
        "npc_citizen" => "models/humans/group01/male_07.mdl",
        "npc_barney" => "models/barney.mdl",
        "npc_alyx" => "models/alyx.mdl",
        "npc_vortigaunt" => "models/vortigaunt.mdl",
        "npc_dog" => "models/dog.mdl",
        "npc_kleiner" => "models/kleiner.mdl",
        "npc_eli" => "models/eli.mdl",
        "npc_mossman" => "models/mossman.mdl",
        "npc_breen" => "models/breen.mdl",
        "npc_gman" => "models/gman.mdl",
        "npc_monk" => "models/monk.mdl",
        "npc_zombie" => "models/zombie/classic.mdl",
        "npc_zombie_torso" => "models/zombie/classic_torso.mdl",
        "npc_fastzombie" => "models/zombie/fast.mdl",
        "npc_poisonzombie" => "models/zombie/poison.mdl",
        "npc_headcrab" => "models/headcrabclassic.mdl",
        "npc_headcrab_fast" => "models/headcrab.mdl",
        "npc_headcrab_black" | "npc_headcrab_poison" => "models/headcrabblack.mdl",
        "npc_barnacle" => "models/barnacle.mdl",
        "npc_antlion" => "models/antlion.mdl",
        "npc_antlionguard" => "models/antlion_guard.mdl",
        "npc_ichthyosaur" => "models/ichthyosaur.mdl",
        "npc_crow" => "models/crow.mdl",
        "npc_pigeon" => "models/pigeon.mdl",
        "npc_seagull" => "models/seagull.mdl",
        "npc_stalker" => "models/stalker.mdl",
        "npc_manhack" => "models/manhack.mdl",
        "npc_cscanner" | "npc_clawscanner" => "models/combine_scanner.mdl",
        "npc_rollermine" => "models/roller.mdl",
        "npc_turret_floor" => "models/combine_turrets/floor_turret.mdl",
        "npc_turret_ceiling" => "models/combine_turrets/ceiling_turret.mdl",
        "npc_combine_camera" => "models/combine_camera/combine_camera.mdl",
        "npc_strider" => "models/combine_strider.mdl",
        "npc_combinegunship" => "models/gunship.mdl",
        "npc_helicopter" => "models/combine_helicopter.mdl",
        "npc_combinedropship" => "models/combine_dropship.mdl",
        _ => return None,
    })
}

/// The class model of a map entity without a `model` key. Only classes with AI here: the rest of
/// `default_model` would stand on the map as statues.
pub fn map_default_model(class: &str) -> Option<&'static str> {
    let has_ai = matches!(
        class,
        "npc_barney"
            | "npc_metropolice"
            | "npc_citizen"
            | "npc_combine_camera"
            | "npc_barnacle"
            | "npc_zombie"
            | "npc_zombie_torso"
            | "npc_fastzombie"
            | "npc_poisonzombie"
            | "npc_headcrab"
            | "npc_headcrab_fast"
            | "npc_headcrab_black"
            | "npc_headcrab_poison"
    );
    has_ai.then(|| default_model(class)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_zombies_and_headcrabs_get_their_class_model_and_statues_do_not() {
        assert_eq!(
            map_default_model("npc_fastzombie"),
            Some("models/zombie/fast.mdl")
        );
        assert_eq!(
            map_default_model("npc_headcrab"),
            Some("models/headcrabclassic.mdl")
        );
        assert_eq!(map_default_model("npc_strider"), None);
        assert_eq!(
            default_model("npc_strider"),
            Some("models/combine_strider.mdl")
        );
    }

    #[test]
    #[ignore = "requires an installed owned HL2 copy; reads the barnacle model"]
    fn owned_barnacle_has_its_tongue_bones_and_activities() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        let names = crate::animation::sequence_names(&vfs, "models/barnacle.mdl")
            .unwrap()
            .into_iter()
            .collect();
        let rig = crate::animation::load(&vfs, "models/barnacle.mdl", &names).unwrap();
        for (i, b) in rig.bones.iter().enumerate() {
            println!(
                "bone {i} {} parent {:?} bind {:?}",
                b.name, b.parent, b.bind.position
            );
        }
        for seq in &rig.sequences {
            let name = &seq.name;
            let clip = rig.clips.get(name);
            println!(
                "seq {name} act {} w {} frames {} fps {} loop {} events {:?}",
                seq.activity,
                seq.weight,
                clip.map_or(0, |c| c.frames.len()),
                clip.map_or(0., |c| c.fps),
                clip.is_some_and(|c| c.looping),
                clip.map(|c| c
                    .events
                    .iter()
                    .map(|e| (e.cycle, e.id, e.name.clone()))
                    .collect::<Vec<_>>())
            );
        }
        let first = rig
            .bones
            .iter()
            .position(|b| b.name == "Barnacle.tongue1")
            .unwrap();
        assert!(rig.bones.iter().any(|b| b.name == "Barnacle.base"));
        assert_eq!(rig.bones.len(), first + 8);
    }

    #[test]
    #[ignore = "requires an installed owned HL2 copy; reads the bird models"]
    fn owned_birds_have_their_flight_activities() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        for model in ["models/crow.mdl", "models/pigeon.mdl", "models/seagull.mdl"] {
            let names = crate::animation::sequence_names(&vfs, model)
                .unwrap()
                .into_iter()
                .collect();
            let rig = crate::animation::load(&vfs, model, &names).unwrap();
            println!("== {model}");
            for seq in &rig.sequences {
                let clip = rig.clips.get(&seq.name);
                println!(
                    "seq {} act {} w {} frames {} fps {} loop {} events {:?}",
                    seq.name,
                    seq.activity,
                    seq.weight,
                    clip.map_or(0, |c| c.frames.len()),
                    clip.map_or(0., |c| c.fps),
                    clip.is_some_and(|c| c.looping),
                    clip.map(|c| c
                        .events
                        .iter()
                        .map(|e| (e.cycle, e.id, e.name.clone()))
                        .collect::<Vec<_>>())
                );
            }
            for activity in ["ACT_IDLE", "ACT_WALK", "ACT_RUN", "ACT_FLY", "ACT_HOP"] {
                assert!(
                    rig.sequences.iter().any(|s| s.activity == activity),
                    "{model} has no {activity}"
                );
            }
        }
    }
}
