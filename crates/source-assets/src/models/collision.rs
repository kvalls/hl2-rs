//! A static prop's VPhysics collision as the map loader reads it.
use crate::vpk::Vfs;
use anyhow::{Context, Result};

/// A static prop's convex pieces (identity root) and its first solid's surfaceprop (the .phy
/// "solid" block, "default" without one): the static prop's VPhysics object reports it to
/// traces, whatever texture is drawn where the hull is hit.
pub fn read_static_collision(
    vfs: &Vfs,
    model: &str,
) -> Result<Option<(Vec<modkit_core::ConvexPiece>, String)>> {
    let stem = model.trim_end_matches(".mdl");
    let Some(phy) = vfs.read(&format!("{stem}.phy"))? else {
        return Ok(None);
    };
    let mdl = vfs.read(model)?.context("PHY companion MDL missing")?;
    let checksum = crate::phy::identity_root_checksum(&mdl)?;
    let pieces = crate::phy::read(&phy, checksum)?.pieces;
    let surfaceprop = crate::phy::read_keyvalues(&phy)
        .ok()
        .and_then(|entries| crate::phy::first_solid(&entries))
        .map_or_else(|| "default".into(), |solid| solid.surfaceprop);
    Ok(Some((pieces, surfaceprop)))
}

/// A static prop's collision for the map loader, counted in `report`: None without a .phy or
/// when it cannot be read (the render collision fallback, with a warning).
pub(super) fn static_collision(
    vfs: &Vfs,
    model: &str,
    report: &mut super::ModelReport,
) -> Option<(Vec<modkit_core::ConvexPiece>, String)> {
    match read_static_collision(vfs, model) {
        Ok(Some((pieces, surfaceprop))) => {
            report.collision_models_loaded += 1;
            report.collision_pieces_loaded += pieces.len();
            Some((pieces, surfaceprop))
        }
        Ok(None) => {
            report.collision_models_missing += 1;
            None
        }
        Err(error) => {
            report.collision_warnings.push(format!(
                "{model}: {error:#}; using render collision fallback"
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    /// Owned files: the station bench and the plaza tree report their .phy surfaceprops.
    #[test]
    #[ignore]
    fn owned_static_props_read_their_phy_surfaceprops() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for (model, expected) in [
            (
                "models/props_trainstation/bench_indoor001a.mdl",
                "wood_crate",
            ),
            ("models/props_foliage/tree_deciduous_01a.mdl", "wood_solid"),
        ] {
            let (pieces, surfaceprop) = super::read_static_collision(&vfs, model).unwrap().unwrap();
            assert!(!pieces.is_empty(), "{model}");
            assert_eq!(surfaceprop, expected, "{model}");
        }
    }
}
