//! Developer NPC spawning: the F4 overlay's data model (class and weapon choice, spawn
//! at the crosshair) and the Source console commands npc_create / npc_create_aimed /
//! npc_create_equipment (ai_concommands.cpp CC_NPC_Create, CC_NPC_Create_Aimed). The
//! class/weapon lists are developer choices, not SDK data. Placement follows the SDK;
//! creating the entity in the running world is host work (not implemented yet).
use glam::Vec3;
use serde::Serialize;

/// Classes offered by the overlay with their additionalequipment choices ("" = none).
pub const SPAWN_CLASSES: &[(&str, &[&str])] = &[
    (
        "npc_metropolice",
        &["weapon_stunstick", "weapon_pistol", "weapon_smg1", ""],
    ),
    (
        "npc_combine_s",
        &["weapon_smg1", "weapon_ar2", "weapon_shotgun"],
    ),
    (
        "npc_citizen",
        &[
            "",
            "weapon_pistol",
            "weapon_smg1",
            "weapon_ar2",
            "weapon_shotgun",
            "weapon_rpg",
        ],
    ),
    (
        "npc_barney",
        &[
            "weapon_pistol",
            "weapon_smg1",
            "weapon_ar2",
            "weapon_shotgun",
        ],
    ),
    ("npc_alyx", &["weapon_alyxgun", "weapon_shotgun", ""]),
    ("npc_vortigaunt", &[""]),
    ("npc_zombie", &[""]),
    ("npc_headcrab", &[""]),
    ("npc_antlion", &[""]),
    ("npc_manhack", &[""]),
    ("npc_cscanner", &[""]),
];
/// Flying classes (bits_CAP_MOVE_FLY) are placed 36 units back along the aim.
fn flies(class: &str) -> bool {
    matches!(class, "npc_manhack" | "npc_cscanner" | "npc_clawscanner")
}
/// MAX_TRACE_LENGTH.
pub const MAX_TRACE_LENGTH: f32 = 56755.84;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpawnRequest {
    pub classname: String,
    pub name: Option<String>,
    pub equipment: String,
    /// npc_create_aimed: face the player's aim yaw (pitch and roll 0).
    pub aimed: bool,
}

/// Console state: the npc_create_equipment cvar.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Console {
    pub equipment: String,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Command {
    Spawn(SpawnRequest),
    SetEquipment(String),
}
impl Console {
    /// Parse one console line: `npc_create <class> [name]`, `npc_create_aimed <class>
    /// [name]`, `npc_create_equipment <weapon>`. Ok(None) for other commands.
    pub fn parse(&mut self, line: &str) -> Result<Option<Command>, String> {
        let args: Vec<&str> = line.split_whitespace().collect();
        let Some(command) = args.first() else {
            return Ok(None);
        };
        match command.to_ascii_lowercase().as_str() {
            "npc_create_equipment" => {
                let value = args
                    .get(1)
                    .copied()
                    .unwrap_or("")
                    .trim_matches('"')
                    .to_owned();
                self.equipment = value.clone();
                Ok(Some(Command::SetEquipment(value)))
            }
            c @ ("npc_create" | "npc_create_aimed") => {
                let class = args
                    .get(1)
                    .ok_or_else(|| format!("{c}: missing npc class"))?;
                if !class.starts_with("npc_") {
                    // CreateEntityByName succeeds only for CAI_BaseNPC classes.
                    return Err(format!("{c}: {class} is not an NPC class"));
                }
                Ok(Some(Command::Spawn(SpawnRequest {
                    classname: (*class).to_owned(),
                    // CC_NPC_Create names the NPC when given exactly two arguments.
                    name: (args.len() == 3).then(|| args[2].to_owned()),
                    equipment: self.equipment.clone(),
                    aimed: c == "npc_create_aimed",
                })))
            }
            _ => Ok(None),
        }
    }
}

/// Where the spawned NPC goes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Placement {
    pub origin: Vec3,
    /// Yaw in degrees (npc_create keeps the spawn angles, 0).
    pub yaw: f32,
}
/// World queries placement needs (MASK_NPCSOLID).
pub trait PlacementWorld {
    /// AI_TraceLine: the end point, or None when nothing is hit.
    fn trace_line(&self, start: Vec3, end: Vec3) -> Option<Vec3>;
    /// UTIL_DropToFloor: the floor position below, if any.
    fn drop_to_floor(&self, class: &str, origin: Vec3) -> Option<Vec3>;
    /// The 1-unit upward hull check: true when the NPC's hull fits.
    fn hull_fits(&self, class: &str, origin: Vec3) -> bool;
}
/// CC_NPC_Create / CC_NPC_Create_Aimed placement. Err for "Bad Position" (the SDK
/// removes the NPC). A miss leaves the NPC at its spawn origin (world origin here).
pub fn place(
    request: &SpawnRequest,
    eye: Vec3,
    forward: Vec3,
    world: &dyn PlacementWorld,
) -> Result<Placement, String> {
    let forward = forward.normalize_or_zero();
    let yaw = if request.aimed {
        forward.y.atan2(forward.x).to_degrees()
    } else {
        0.
    };
    let Some(end) = world.trace_line(eye, eye + forward * MAX_TRACE_LENGTH) else {
        return Ok(Placement {
            origin: Vec3::ZERO,
            yaw,
        });
    };
    let origin = if flies(&request.classname) {
        end - forward * 36.
    } else {
        let raised = end + Vec3::Z * 12.;
        world
            .drop_to_floor(&request.classname, raised)
            .unwrap_or(raised)
    };
    if !world.hull_fits(&request.classname, origin) {
        return Err(format!(
            "Can't create {}.  Bad Position!",
            request.classname
        ));
    }
    Ok(Placement { origin, yaw })
}

/// The F4 overlay: open state and the selected class/weapon.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Overlay {
    pub open: bool,
    pub class: usize,
    pub equipment: usize,
}
impl Overlay {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
    pub fn next_class(&mut self, step: isize) {
        self.class = (self.class as isize + step).rem_euclid(SPAWN_CLASSES.len() as isize) as usize;
        self.equipment = 0;
    }
    pub fn next_equipment(&mut self, step: isize) {
        let n = SPAWN_CLASSES[self.class].1.len() as isize;
        self.equipment = (self.equipment as isize + step).rem_euclid(n) as usize;
    }
    /// The overlay spawns at the crosshair like npc_create_aimed (facing the aim).
    pub fn request(&self) -> SpawnRequest {
        let (class, equipment) = SPAWN_CLASSES[self.class];
        SpawnRequest {
            classname: class.into(),
            name: None,
            equipment: equipment[self.equipment].into(),
            aimed: true,
        }
    }
    /// Lines for the host to draw.
    pub fn lines(&self) -> Vec<String> {
        let (class, equipment) = SPAWN_CLASSES[self.class];
        vec![
            "F4 NPC spawn (left/right class, up/down weapon, Enter spawn at crosshair)".into(),
            format!("class: {class}"),
            format!(
                "weapon: {}",
                if equipment[self.equipment].is_empty() {
                    "none"
                } else {
                    equipment[self.equipment]
                }
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Floor;
    impl PlacementWorld for Floor {
        fn trace_line(&self, start: Vec3, end: Vec3) -> Option<Vec3> {
            // Ground plane z = 0, a wall at x = 1000.
            let d = end - start;
            let t_floor = if d.z < 0. {
                -start.z / d.z
            } else {
                f32::INFINITY
            };
            let t_wall = if d.x > 0. {
                (1000. - start.x) / d.x
            } else {
                f32::INFINITY
            };
            let t = t_floor.min(t_wall);
            (t <= 1.).then(|| start + d * t)
        }
        fn drop_to_floor(&self, _: &str, origin: Vec3) -> Option<Vec3> {
            Some(Vec3::new(origin.x, origin.y, 0.))
        }
        fn hull_fits(&self, _: &str, origin: Vec3) -> bool {
            origin.x < 990.
        }
    }
    #[test]
    fn console_commands_follow_source_semantics() {
        let mut console = Console::default();
        assert_eq!(
            console
                .parse("npc_create_equipment weapon_stunstick")
                .unwrap(),
            Some(Command::SetEquipment("weapon_stunstick".into()))
        );
        let Some(Command::Spawn(r)) = console.parse("npc_create npc_metropolice cop1").unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (
                r.classname.as_str(),
                r.name.as_deref(),
                r.equipment.as_str(),
                r.aimed
            ),
            ("npc_metropolice", Some("cop1"), "weapon_stunstick", false)
        );
        let Some(Command::Spawn(r)) = console.parse("npc_create_aimed npc_citizen").unwrap() else {
            panic!()
        };
        assert!(r.aimed && r.name.is_none());
        assert!(console.parse("npc_create prop_physics").is_err());
        assert!(console.parse("npc_create").is_err());
        assert_eq!(console.parse("sv_cheats 1").unwrap(), None);
    }
    #[test]
    fn placement_drops_raised_hits_to_the_floor_and_rejects_bad_positions() {
        let eye = Vec3::new(0., 0., 64.);
        let request = |class: &str, aimed| SpawnRequest {
            classname: class.into(),
            name: None,
            equipment: String::new(),
            aimed,
        };
        let down = Vec3::new(1., 0., -0.25).normalize();
        let p = place(&request("npc_metropolice", true), eye, down, &Floor).unwrap();
        assert!(
            (p.origin - Vec3::new(256., 0., 0.)).length() < 1e-2,
            "{p:?}"
        );
        assert_eq!(p.yaw, 0.);
        let p = place(
            &request("npc_metropolice", false),
            eye,
            Vec3::new(0., 1., -0.25),
            &Floor,
        )
        .unwrap();
        assert_eq!(p.yaw, 0.);
        let p = place(&request("npc_manhack", true), eye, down, &Floor).unwrap();
        assert!((p.origin - (Vec3::new(256., 0., 0.) - down * 36.)).length() < 1e-2);
        // Into the wall: the hull does not fit.
        assert!(place(&request("npc_metropolice", true), eye, Vec3::X, &Floor).is_err());
        // Straight up: no hit, the NPC stays at its spawn origin.
        assert_eq!(
            place(&request("npc_citizen", true), eye, Vec3::Z, &Floor)
                .unwrap()
                .origin,
            Vec3::ZERO
        );
    }
    #[test]
    fn overlay_cycles_classes_and_weapons() {
        let mut o = Overlay::default();
        o.toggle();
        o.next_equipment(1);
        assert_eq!(o.request().equipment, "weapon_pistol");
        o.next_class(-1);
        assert_eq!(o.request().classname, "npc_cscanner");
        assert_eq!(o.equipment, 0);
        o.next_class(1);
        o.next_class(1);
        assert_eq!(o.request().classname, "npc_combine_s");
        assert!(o.lines()[1].contains("npc_combine_s"));
    }
}
