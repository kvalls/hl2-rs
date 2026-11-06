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

/// CNPC_CScanner::Gib's four models, and how they fly: the middle of CGib's 250 to 500 u/s, its
/// 25 s life, as debris.
const SCANNER_GIBS: [&str; 4] = [
    "models/gibs/scanner_gib01.mdl",
    "models/gibs/scanner_gib02.mdl",
    "models/gibs/scanner_gib04.mdl",
    "models/gibs/scanner_gib05.mdl",
];
/// The break list in models/manhack.phy.
const MANHACK_GIBS: [&str; 6] = [
    "models/gibs/manhack_gib01.mdl",
    "models/gibs/manhack_gib02.mdl",
    "models/gibs/manhack_gib03.mdl",
    "models/gibs/manhack_gib04.mdl",
    "models/gibs/manhack_gib05.mdl",
    "models/gibs/manhack_gib06.mdl",
];
const SCANNER_GIB_SPEED: f32 = 375.;
const SCANNER_GIB_LIFE: f32 = 25.;

/// Gib lists of NPC classes that break apart in code, not from their model's .phy: the same
/// table, so the gibs' physics and meshes are preloaded with the map.
pub fn add_class_gibs(world: &mut modkit_core::World) {
    let keys = |world: &modkit_core::World, class: &str| -> Vec<String> {
        let instances = world.model_instances.iter();
        let of_class = instances.filter(|instance| instance.kind == class);
        of_class.map(|instance| instance.asset_key()).collect()
    };
    for key in keys(world, "npc_cscanner") {
        let gibs = class_gibs(&SCANNER_GIBS);
        world.model_breaks.entry(key).or_insert(gibs);
    }
    for key in keys(world, "npc_manhack") {
        let gibs = class_gibs(&MANHACK_GIBS);
        world.model_breaks.entry(key).or_insert(gibs);
    }
}

fn class_gibs(models: &[&str]) -> Vec<modkit_core::BreakModel> {
    models
        .iter()
        .map(|model| modkit_core::BreakModel {
            model: (*model).into(),
            ragdoll: false,
            offset: Default::default(),
            health: 0.,
            fade_time: SCANNER_GIB_LIFE,
            debris: Some(true),
            motion_disabled: false,
            burst: Some(SCANNER_GIB_SPEED),
        })
        .collect()
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
            | "npc_cscanner"
            | "npc_crow"
            | "npc_pigeon"
            | "npc_seagull"
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
    #[ignore = "requires an installed owned HL2 copy; reads the scanner model"]
    fn owned_scanner_sequences() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        let model = "models/combine_scanner.mdl";
        let names = crate::animation::sequence_names(&vfs, model)
            .unwrap()
            .into_iter()
            .collect();
        let rig = crate::animation::load(&vfs, model, &names).unwrap();
        for seq in &rig.sequences {
            let clip = rig.clips.get(&seq.name);
            println!(
                "seq {} act {} w {} frames {} fps {} loop {}",
                seq.name,
                seq.activity,
                seq.weight,
                clip.map_or(0, |c| c.frames.len()),
                clip.map_or(0., |c| c.fps),
                clip.is_some_and(|c| c.looping),
            );
        }
        assert!(rig.sequences.iter().any(|s| s.activity == "ACT_IDLE"));
        assert!(rig.attachment("light").is_some());
        // The faceplate, tail, dynamo and flaps: autoplay layers each blended by a parameter.
        for (sequence, parameter, start, end) in [
            ("dynamo_turn", "dynamo_wheel", -180., 180.),
            ("tail_control", "tail_control", -25., 80.),
            ("alert_control", "alert_control", -1., 1.),
            ("upndown", "flex_vert", -20., 20.),
            ("rightnleft", "flex_horz", -20., 20.),
        ] {
            assert!(rig.autoplay.iter().any(|s| s == sequence), "{sequence}");
            assert!(rig.clips[sequence].layer.blend.is_some(), "{sequence}");
            let index = rig.pose_parameter(parameter).expect(parameter);
            let found = &rig.pose_parameters[index];
            assert_eq!((found.start, found.end), (start, end), "{parameter}");
        }
    }

    /// The scanner's materials as the install has them.
    #[test]
    #[ignore = "requires an installed owned HL2 copy; prints the scanner's materials"]
    fn owned_scanner_materials() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        let data = vfs.read("models/combine_scanner.mdl").unwrap().unwrap();
        let text = String::from_utf8_lossy(&data);
        for name in text.split('\0').filter(|s| s.len() > 3 && s.len() < 60) {
            let lower = name.to_lowercase();
            if lower.contains("scanner") && !lower.contains('.') {
                let path = format!("materials/models/combine_scanner/{lower}.vmt");
                if let Ok(Some(vmt)) = vfs.read(&path) {
                    println!(
                        "VMT {path}: {}",
                        String::from_utf8_lossy(&vmt)
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            }
        }
    }

    /// What the scanner's base texture holds in its alpha, which selects self-illumination.
    #[test]
    #[ignore = "requires an installed owned HL2 copy; prints the scanner texture's alpha"]
    fn owned_scanner_texture_alpha() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        let data = vfs
            .read("materials/models/combine_scanner/scanner_sheet.vtf")
            .unwrap()
            .unwrap();
        let image = crate::vtf::decode(&data, 512).unwrap();
        let alpha: Vec<u8> = image.rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        let lit = alpha.iter().filter(|a| **a > 16).count();
        println!(
            "TEX {}x{} translucent {} lit share {:.3} max {}",
            image.width,
            image.height,
            image.translucent,
            lit as f32 / alpha.len() as f32,
            alpha.iter().max().unwrap(),
        );
    }

    /// The surface rows and decal translation a shot's effects depend on.
    #[test]
    #[ignore = "requires an installed owned HL2 copy; prints surface rows and decal groups"]
    fn owned_surface_impact_rows() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        let text = vfs.read("scripts/decals_subrect.txt").unwrap().unwrap();
        let text = String::from_utf8_lossy(&text);
        let at = text.find("TranslationData").unwrap_or(0);
        println!("{}", &text[at..(at + 900).min(text.len())]);
        for group in [
            "Impact.Metal",
            "Impact.Flesh",
            "Impact.Antlion",
            "Impact.AlienFlesh",
            "Impact.BloodyFlesh",
        ] {
            let heading = format!("\"{group}\"");
            let Some(start) = text.rfind(&heading) else {
                continue;
            };
            let end = text[start..].find('}').map_or(text.len(), |e| start + e);
            println!(
                "GROUP {}",
                text[start..end]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        for file in [
            "scripts/surfaceproperties.txt",
            "scripts/surfaceproperties_hl2.txt",
        ] {
            let Ok(Some(data)) = vfs.read(file) else {
                continue;
            };
            let data = String::from_utf8_lossy(&data);
            for name in [
                "metal",
                "flesh",
                "zombieflesh",
                "alienflesh",
                "antlion",
                "strider",
                "gunship",
                "roller",
                "no_decal",
                "default",
                "bloodyflesh",
                "combine_metal",
                "solidmetal",
            ] {
                if let Some(start) = data.find(&format!("\"{name}\"")) {
                    let end = data[start..].find('}').map_or(data.len(), |e| start + e);
                    println!(
                        "{file} {}",
                        data[start..end]
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            }
        }
    }

    /// Every class's hitbox surfaces: a shot reports the bone's, so none may be assumed.
    #[test]
    #[ignore = "requires an installed owned HL2 copy; prints each NPC model's hitbox surfaces"]
    fn owned_npc_hitbox_surfaces() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        for class in [
            "npc_metropolice",
            "npc_combine_s",
            "npc_citizen",
            "npc_barney",
            "npc_alyx",
            "npc_vortigaunt",
            "npc_dog",
            "npc_kleiner",
            "npc_eli",
            "npc_mossman",
            "npc_breen",
            "npc_gman",
            "npc_monk",
            "npc_zombie",
            "npc_zombie_torso",
            "npc_fastzombie",
            "npc_poisonzombie",
            "npc_headcrab",
            "npc_headcrab_fast",
            "npc_headcrab_black",
            "npc_barnacle",
            "npc_antlion",
            "npc_antlionguard",
            "npc_ichthyosaur",
            "npc_crow",
            "npc_pigeon",
            "npc_seagull",
            "npc_stalker",
            "npc_manhack",
            "npc_cscanner",
            "npc_rollermine",
            "npc_turret_floor",
            "npc_turret_ceiling",
            "npc_combine_camera",
            "npc_strider",
            "npc_combinegunship",
            "npc_helicopter",
            "npc_combinedropship",
        ] {
            let model = default_model(class).unwrap();
            let Ok(Some(data)) = vfs.read(model) else {
                println!("{class}: no {model}");
                continue;
            };
            let surfaces = crate::animation::bone_surfaces(&data).unwrap();
            let boxes = crate::animation::hitboxes(&data).unwrap();
            let mut seen = std::collections::BTreeMap::<&str, usize>::new();
            for hitbox in &boxes {
                *seen.entry(surfaces[hitbox.bone].as_str()).or_default() += 1;
            }
            println!("{class}: {} boxes {seen:?}", boxes.len());
        }
    }

    /// What each sequence of the scanner moves, and how far: evidence for the faceplate work.
    #[test]
    #[ignore = "requires an installed owned HL2 copy; prints the scanner's bone motion"]
    fn owned_scanner_bone_motion() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        let model = "models/combine_scanner.mdl";
        let names = crate::animation::sequence_names(&vfs, model)
            .unwrap()
            .into_iter()
            .collect();
        let rig = crate::animation::load(&vfs, model, &names).unwrap();
        for (i, bone) in rig.bones.iter().enumerate() {
            println!(
                "bone {i} {} parent {:?} surface {}",
                bone.name, bone.parent, rig.bone_surfaces[i]
            );
        }
        println!("hitboxes {:?}", rig.hitboxes);
        println!("pose {:?}", rig.pose_parameters);
        println!("autoplay {:?}", rig.autoplay);
        for seq in &rig.sequences {
            let Some(clip) = rig.clips.get(&seq.name) else {
                continue;
            };
            println!(
                "== {} act {} flags {:#x} frames {} fade {} {} events {:?}",
                seq.name,
                seq.activity,
                clip.layer.flags,
                clip.frames.len(),
                clip.layer.fade_in,
                clip.layer.fade_out,
                clip.events
                    .iter()
                    .map(|e| (e.cycle, e.id, e.name.clone()))
                    .collect::<Vec<_>>(),
            );
            let mut grids = vec![("frames".to_string(), clip.frames.clone())];
            if let Some(blend) = &clip.layer.blend {
                println!("   blend {:?} groups {:?}", blend.axes, blend.groups);
                for (n, anim) in blend.anims.iter().enumerate() {
                    grids.push((format!("blend{n}"), anim.clone()));
                }
            }
            for (name, frames) in grids {
                for (b, bone) in rig.bones.iter().enumerate() {
                    let base = if clip.layer.flags & 4 != 0 {
                        modkit_core::animation::Pose {
                            position: glam::Vec3::ZERO,
                            rotation: glam::Quat::IDENTITY,
                        }
                    } else {
                        bone.bind.clone()
                    };
                    let mut turn = 0f32;
                    let mut slide = 0f32;
                    for frame in &frames {
                        let Some(pose) = frame.get(b) else { continue };
                        turn = turn.max(pose.rotation.angle_between(base.rotation).to_degrees());
                        slide = slide.max(pose.position.distance(base.position));
                    }
                    if turn > 0.5 || slide > 0.05 {
                        println!("   {name} {} turn {turn:.1} slide {slide:.2}", bone.name);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "requires an installed owned HL2 copy; reads the manhack model"]
    fn owned_manhack_flies_and_has_its_blades() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        let model = "models/manhack.mdl";
        let names = crate::animation::sequence_names(&vfs, model)
            .unwrap()
            .into_iter()
            .collect();
        let rig = crate::animation::load(&vfs, model, &names).unwrap();
        for seq in &rig.sequences {
            println!("seq {} act {} w {}", seq.name, seq.activity, seq.weight);
        }
        println!("pose {:?}", rig.pose_parameters);
        println!("autoplay {:?}", rig.autoplay);
        println!(
            "attachments {:?}",
            rig.attachments.iter().map(|a| &a.name).collect::<Vec<_>>()
        );
        assert!(rig.sequences.iter().any(|s| s.activity == "ACT_FLY"));
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
