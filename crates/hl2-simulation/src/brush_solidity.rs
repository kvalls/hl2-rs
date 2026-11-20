//! Which brush entities are solid (SDK: SOLID_BSP without FSOLID_NOT_SOLID): their colliders,
//! and the brush models static decals attach to.
use modkit_core::Entity;

/// A solid brush entity: one of the solid classes, not made non-solid by its keys
/// (`solid 0`, func_brush `Solidity 1`, a func_tracktrain's SF_TRACKTRAIN_PASSABLE).
pub fn solid(e: &Entity) -> bool {
    // func_lod (SDK func_lod.cpp CFunc_LOD): SOLID_BSP, and its "Solid" key is inverted:
    // any non-zero value adds FSOLID_NOT_SOLID (d1_trainstation_01's lod_turnstyle_frame,
    // Solid 0, is the turnstile's solid frame).
    if e.class() == "func_lod" {
        let value = e.get("Solid").map_or("0", str::trim);
        let digits = value.trim_start_matches(['-', '+']);
        let end = digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(digits.len());
        return digits[..end].trim_start_matches('0').is_empty();
    }
    let class = matches!(
        e.class(),
        "func_brush"
            | "func_wall"
            | "func_wall_toggle"
            | "func_door"
            | "func_door_rotating"
            | "func_movelinear"
            | "func_rotating"
            | "func_breakable"
            | "func_physbox"
            | "func_detail"
            | "func_tracktrain"
    );
    let passable_train = e.class() == "func_tracktrain"
        && e.get("spawnflags")
            .and_then(|f| f.trim().parse::<u32>().ok())
            .unwrap_or(0)
            & crate::trains::PASSABLE
            != 0;
    class && !passable_train && e.get("solid") != Some("0") && e.get("Solidity") != Some("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn func_lod_is_solid_unless_its_inverted_solid_key_is_set() {
        let lod = |solid: Option<&str>| {
            let mut pairs = vec![("classname", "func_lod")];
            pairs.extend(solid.map(|v| ("Solid", v)));
            super::solid(&entity(&pairs))
        };
        assert!(lod(Some("0")));
        assert!(lod(None));
        assert!(!lod(Some("1")));
        assert!(!lod(Some("2 ")));
        // The generic keys still apply to the other classes.
        assert!(!super::solid(&entity(&[
            ("classname", "func_brush"),
            ("solid", "0")
        ])));
        assert!(super::solid(&entity(&[("classname", "func_wall")])));
    }
}
