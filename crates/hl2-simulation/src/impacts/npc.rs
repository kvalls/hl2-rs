//! A shot on an NPC's model (DESIGN 14o session 48). A trace against a hitbox reports the surfaceprop
//! of the bone it hit, so a scanner rings and sparks like metal and an antlion marks like an antlion;
//! nothing here may assume flesh. Only a model that names no surface falls back to it.
use super::{impact_effect, surface_light, Impacts, FLESH, FLESH_DECALS};
use crate::{
    entities::Scene,
    physics::{Physics, RayHit},
};
use glam::Vec3;
use modkit_core::World;

/// The whine of a bullet glancing off armor (FX_RicochetSound).
pub const RICOCHET_SOUND: &str = "FX_RicochetSound.Ricochet";
/// The surfaces NPC models name, whose bullet sounds are loaded with a map, since a class spawned
/// later brings no loading of its own.
pub(super) const SURFACES: [&str; 8] = [
    "metal",
    "default",
    "zombieflesh",
    "alienflesh",
    "antlion",
    "roller",
    "strider",
    "gunship",
];
/// Owned scripts/decals_subrect.txt: Impact.Metal, Impact.Concrete, Impact.AlienFlesh,
/// Impact.Antlion and Impact.BloodyFlesh.
pub(super) const METAL_DECALS: [&str; 5] = [
    "decals/metal/shot1_subrect",
    "decals/metal/shot2_subrect",
    "decals/metal/shot3_subrect",
    "decals/metal/shot4_subrect",
    "decals/metal/shot5_subrect",
];
pub(super) const CONCRETE_DECALS: [&str; 5] = [
    "decals/concrete/shot1_subrect",
    "decals/concrete/shot2_subrect",
    "decals/concrete/shot3_subrect",
    "decals/concrete/shot4_subrect",
    "decals/concrete/shot5_subrect",
];
pub(super) const ALIEN_DECALS: [&str; 5] = [
    "decals/alienflesh/shot1_subrect",
    "decals/alienflesh/shot2_subrect",
    "decals/alienflesh/shot3_subrect",
    "decals/alienflesh/shot4_subrect",
    "decals/alienflesh/shot5_subrect",
];
pub(super) const ANTLION_DECALS: [&str; 5] = [
    "decals/antlion/shot1_subrect",
    "decals/antlion/shot2_subrect",
    "decals/antlion/shot3_subrect",
    "decals/antlion/shot4_subrect",
    "decals/antlion/shot5_subrect",
];
pub(super) const BLOODY_DECALS: [&str; 5] = [
    "decals/bloodyflesh/blood1_subrect",
    "decals/bloodyflesh/blood2_subrect",
    "decals/bloodyflesh/blood3_subrect",
    "decals/bloodyflesh/blood4_subrect",
    "decals/bloodyflesh/blood5_subrect",
];

/// The TranslationData row for a surface's game material; None for those that take no mark.
fn decals_for(material: &str) -> Option<&'static [&'static str; 5]> {
    match material {
        "-" | "I" | "S" | "X" => None,
        "F" => Some(&FLESH_DECALS),
        "B" => Some(&BLOODY_DECALS),
        "H" => Some(&ALIEN_DECALS),
        "A" | "E" => Some(&ANTLION_DECALS),
        "M" | "V" | "G" => Some(&METAL_DECALS),
        _ => Some(&CONCRETE_DECALS),
    }
}

/// The surfaceprop of the bone a shot hit. A hit on the hull has no bone, and takes the first
/// hitbox's. An empty name is the engine's default surface.
fn bone_surface<'a>(hit: &RayHit, world: &'a World, physics: &Physics) -> Option<&'a str> {
    let rig = world.rigs.get(physics.studio_model(hit.entity)?)?;
    let bone = hit.bone.or(rig.hitboxes.first().map(|h| h.bone))?;
    let name = rig.bone_surfaces.get(bone)?;
    Some(if name.is_empty() { "default" } else { name })
}

impl Impacts {
    /// A shot on an NPC: the surface's sound, its sparks or flecks, and its mark on the model.
    pub(super) fn add_on_npc(
        &mut self,
        hit: &RayHit,
        direction: Option<Vec3>,
        world: &World,
        physics: &Physics,
        scene: &mut Scene,
    ) {
        let prop = bone_surface(hit, world, physics).unwrap_or(FLESH);
        if let Some(sound) = self.property(prop, "bulletimpact") {
            scene.sounds.push(crate::sounds::SoundRequest {
                origin: Some(hit.position),
                ..sound.into()
            });
        }
        let material = self.property(prop, "gamematerial");
        let material = material.as_deref().unwrap_or("C");
        let at = (
            scene.time,
            surface_light(world, hit.position + hit.normal * 2.),
        );
        scene
            .impact_effects
            .extend(direction.and_then(|d| impact_effect(material, hit, d, at)));
        if let Some(decals) = decals_for(material) {
            let decal = decals[self.studio_created % decals.len()];
            self.add_studio_decal(hit, decal, direction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_game_material_finds_its_decal_group() {
        assert_eq!(decals_for("F"), Some(&FLESH_DECALS));
        assert_eq!(decals_for("M"), Some(&METAL_DECALS));
        assert_eq!(decals_for("V"), Some(&METAL_DECALS));
        assert_eq!(decals_for("A"), Some(&ANTLION_DECALS));
        assert_eq!(decals_for("H"), Some(&ALIEN_DECALS));
        assert_eq!(decals_for("B"), Some(&BLOODY_DECALS));
        assert_eq!(decals_for("C"), Some(&CONCRETE_DECALS));
    }

    #[test]
    fn a_surface_that_refuses_marks_gets_none() {
        assert_eq!(decals_for("-"), None);
    }
}
