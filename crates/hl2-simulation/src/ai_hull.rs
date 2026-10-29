//! NPC hull types (SDK ai_hull.h/ai_hull.cpp, the non-HL1 table): the bounds each class
//! sets with SetHullType, and the index that selects a node's per-hull offset and a
//! link's per-hull accepted move types in the node graph.
use crate::npc_probe::Hull;
use glam::Vec3;

/// Hull_t, in the order of the graph's per-hull arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum HullType {
    Human,
    SmallCentered,
    WideHuman,
    Tiny,
    WideShort,
    Medium,
    TinyCentered,
    Large,
    LargeCentered,
    MediumTall,
}
pub const HULL_TYPES: [HullType; 10] = [
    HullType::Human,
    HullType::SmallCentered,
    HullType::WideHuman,
    HullType::Tiny,
    HullType::WideShort,
    HullType::Medium,
    HullType::TinyCentered,
    HullType::Large,
    HullType::LargeCentered,
    HullType::MediumTall,
];
impl HullType {
    /// The graph's per-hull array index (NUM_HULLS order).
    pub fn index(self) -> usize {
        self as usize
    }
    /// NAI_Hull::Mins/Maxs.
    pub fn bounds(self) -> Hull {
        let (mins, maxs) = match self {
            Self::Human => ((-13., -13., 0.), (13., 13., 72.)),
            Self::SmallCentered => ((-20., -20., -20.), (20., 20., 20.)),
            Self::WideHuman => ((-15., -15., 0.), (15., 15., 72.)),
            Self::Tiny => ((-12., -12., 0.), (12., 12., 24.)),
            Self::WideShort => ((-35., -35., 0.), (35., 35., 32.)),
            Self::Medium => ((-16., -16., 0.), (16., 16., 64.)),
            Self::TinyCentered => ((-8., -8., -4.), (8., 8., 4.)),
            Self::Large => ((-40., -40., 0.), (40., 40., 100.)),
            Self::LargeCentered => ((-38., -38., -38.), (38., 38., 38.)),
            Self::MediumTall => ((-18., -18., 0.), (18., 18., 100.)),
        };
        Hull {
            mins: Vec3::from(mins),
            maxs: Vec3::from(maxs),
        }
    }
    /// The bounds scaled by the model scale (SetHullSizeNormal on a scaled model).
    pub fn scaled(self, scale: f32) -> Hull {
        let Hull { mins, maxs } = self.bounds();
        Hull {
            mins: mins * scale,
            maxs: maxs * scale,
        }
    }
    /// The table hull with these (scaled) bounds.
    pub fn from_bounds(hull: Hull, scale: f32) -> Option<Self> {
        HULL_TYPES.into_iter().find(|t| {
            let h = t.scaled(scale);
            h.mins.distance(hull.mins) <= 0.001 && h.maxs.distance(hull.maxs) <= 0.001
        })
    }
}
/// The hull each reviewed npc_* class's Spawn sets (SetHullType), for those done so far.
pub fn class_hull(class: &str) -> Option<HullType> {
    match class {
        "npc_metropolice" | "npc_combine_s" | "npc_citizen" | "npc_alyx" | "npc_barney"
        | "npc_kleiner" | "npc_eli" | "npc_breen" | "npc_mossman" | "npc_magnusson"
        | "npc_gman" | "npc_monk" | "npc_zombie" | "npc_zombine" | "npc_poisonzombie"
        | "npc_fastzombie" | "npc_stalker" | "npc_assassin" => Some(HullType::Human),
        // CZombie::SetZombieModel / CFastZombie::SetZombieModel with m_fIsTorso.
        "npc_zombie_torso" | "npc_fastzombie_torso" => Some(HullType::Tiny),
        // CBaseHeadcrab::Spawn.
        "npc_headcrab" | "npc_headcrab_fast" | "npc_headcrab_black" | "npc_headcrab_poison" => {
            Some(HullType::Tiny)
        }
        _ => None,
    }
}

/// WorldSpaceCenter's height above the origin: the middle of the class hull (36 for the
/// human hull and classes not reviewed yet).
pub fn center_height(class: &str) -> f32 {
    class_hull(class).map_or(36., |t| {
        let hull = t.bounds();
        (hull.mins.z + hull.maxs.z) * 0.5
    })
}
/// EyePosition's height: the model's $eyeposition (SetDefaultEyeOffset). 64 for the human
/// models reviewed so far; classic_torso.mdl's is 24 (owned check in ai::zombie).
pub fn eye_height(class: &str) -> f32 {
    match class {
        "npc_zombie_torso" => 24.,
        // CBaseHeadcrab::Spawn: SetViewOffset(Vector(6, 0, 11)).
        "npc_headcrab" | "npc_headcrab_fast" | "npc_headcrab_black" | "npc_headcrab_poison" => {
            crate::ai::headcrab::EYE_HEIGHT
        }
        _ => 64.,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hull_table_follows_ai_hull_cpp() {
        assert_eq!(HullType::Tiny.index(), 3);
        assert_eq!(HullType::MediumTall.index(), 9);
        assert_eq!(HullType::Tiny.bounds().maxs, Vec3::new(12., 12., 24.));
        // The non-HL1 wide human hull.
        assert_eq!(HullType::WideHuman.bounds().maxs.x, 15.);
        assert_eq!(
            HullType::from_bounds(HullType::Human.scaled(1.2), 1.2),
            Some(HullType::Human)
        );
        assert_eq!(HullType::from_bounds(HullType::Tiny.bounds(), 1.2), None);
        assert_eq!(class_hull("npc_zombie_torso"), Some(HullType::Tiny));
        assert_eq!(class_hull("npc_zombie"), Some(HullType::Human));
    }
}
