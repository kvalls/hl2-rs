//! Static map decals (infodecal) applied at map load.
use super::{asset_path, definition, scalar};
use anyhow::{Context, Result, bail};
use modkit_core::World;
use source_assets::vpk::Vfs;
use std::collections::BTreeMap;

/// CDecal::StaticDecal: infodecals without a targetname are applied at map load (named ones
/// wait for an input and are not handled yet). Size is the decal's base texture in world
/// units times `$decalscale`.
pub(super) fn add_static_decals(
    world: &mut World,
    vfs: &Vfs,
    errors: &mut Vec<String>,
) -> Vec<(modkit_core::Surface, super::DecalLayer)> {
    let decals: Vec<_> = world
        .entities
        .iter()
        .filter(|e| e.class() == "infodecal" && e.get("targetname").is_none_or(str::is_empty))
        .filter_map(|e| {
            Some((
                e.origin(),
                e.get("texture")?.replace('\\', "/").to_lowercase(),
            ))
        })
        .collect();
    let mut specs = BTreeMap::new();
    let mut ordinary = Vec::new();
    let mut added = Vec::new();
    for (placement, (origin, material)) in decals.into_iter().enumerate() {
        let size =
            *specs
                .entry(material.clone())
                .or_insert_with(|| match decal_spec(vfs, &material) {
                    Ok(size) => Some(size),
                    Err(e) => {
                        errors.push(format!("infodecal {material}: {e:#}"));
                        None
                    }
                });
        if let Some((size, sorted)) = size {
            let pieces =
                modkit_core::decals::static_decal(&world.surfaces, origin, size, 5., &material);
            if pieces.is_empty() && !entity_decal(world, origin, size, &material) {
                errors.push(format!(
                    "infodecal {material} at {origin}: no surface in reach"
                ));
            }
            if sorted {
                added.extend(pieces.into_iter().map(|piece| (placement, piece)));
            } else {
                ordinary.extend(pieces);
            }
        }
    }
    world.surfaces.extend(ordinary);
    super::decal_order::separate(added, &mut world.surfaces)
}
fn decal_spec(vfs: &Vfs, material: &str) -> Result<(glam::Vec2, bool)> {
    let definition = definition(vfs, material, 0, true)?;
    let base = definition
        .properties
        .get("$basetexture")
        .context("decal has no $basetexture")?;
    let path = asset_path(base, ".vtf")?;
    let data = vfs
        .read(&path)?
        .with_context(|| format!("VTF absent: {path}"))?;
    if data.get(..4) != Some(b"VTF\0") || data.len() < 20 {
        bail!("not a VTF: {path}");
    }
    let width = f32::from(u16::from_le_bytes([data[16], data[17]]));
    let height = f32::from(u16::from_le_bytes([data[18], data[19]]));
    let scale = scalar(&definition.properties, "$decalscale", 1.)?;
    let sorted = super::metadata(&definition).is_ok_and(|m| {
        matches!(
            crate::rendering::alpha_mode(&m),
            bevy::prelude::AlphaMode::Blend
                | bevy::prelude::AlphaMode::Add
                | bevy::prelude::AlphaMode::Multiply
        )
    });
    Ok((glam::Vec2::new(width, height) * scale, sorted))
}

/// Where no world face is in reach, CDecal::StaticDecal's MASK_SOLID trace (5 units each way)
/// hits a solid brush entity instead, and the decal attaches to its model in the entity's
/// spawn frame (VectorITransform), so it moves with it: d1_trainstation_01's car keeps its
/// trash, stains and window cracks on the func_tracktrain.
fn entity_decal(world: &mut World, origin: glam::Vec3, size: glam::Vec2, material: &str) -> bool {
    let found = world.entities.iter().find_map(|e| {
        let model = e.get("model")?.strip_prefix('*')?.parse::<usize>().ok()?;
        let index = world.brush_models.iter().position(|m| m.id == model)?;
        if !hl2_simulation::brush_solidity::solid(e) {
            return None;
        }
        let angles =
            modkit_core::parse_vec3(e.get("angles").unwrap_or("0 0 0")).unwrap_or_default();
        let rotation = hl2_simulation::grab::source_angles(angles.to_array());
        let local = rotation.inverse() * (origin - e.origin());
        let brush = &world.brush_models[index];
        let reach = glam::Vec3::splat(5.);
        if local.cmplt(brush.mins - reach).any() || local.cmpgt(brush.maxs + reach).any() {
            return None;
        }
        let pieces = modkit_core::decals::static_decal(&brush.surfaces, local, size, 5., material);
        (!pieces.is_empty()).then_some((index, pieces))
    });
    let Some((index, pieces)) = found else {
        return false;
    };
    world.brush_models[index].surfaces.extend(pieces);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::{BrushModel, Entity, Surface, Vertex};

    fn wall() -> Surface {
        // A wall in the model's XZ plane facing -Y, in the brush model's own frame.
        let corner = |x: f32, z: f32| Vertex {
            normal: Default::default(),
            position: glam::Vec3::new(x, 0., z),
            uv: glam::Vec2::new(x / 100., -z / 100.),
            color: [255; 4],
            light_uv: glam::Vec2::new(x / 100., z / 100.),
            skin: None,
        };
        Surface {
            flex_source: None,
            background: false,
            material: "wall".into(),
            lightmap: Some(0),
            vertices: vec![
                corner(0., 0.),
                corner(100., 100.),
                corner(100., 0.),
                corner(0., 100.),
            ],
            indices: vec![0, 1, 2, 0, 3, 1],
        }
    }

    #[test]
    fn decal_groups_connect_overlaps_and_leave_isolated_pieces_batched() {
        let receiver = [wall()];
        let projected = |x: f32| {
            modkit_core::decals::static_decal(
                &receiver,
                glam::Vec3::new(x, 0., 50.),
                glam::Vec2::splat(20.),
                5.,
                "paint",
            )
            .pop()
            .unwrap()
        };
        let pieces = [10., 25., 40., 80.]
            .into_iter()
            .enumerate()
            .map(|(id, x)| (id, projected(x)))
            .collect();
        let mut ordinary = Vec::new();
        let layered = super::super::decal_order::separate(pieces, &mut ordinary);
        assert_eq!(ordinary.len(), 1);
        let layers: Vec<_> = layered.iter().map(|(_, layer)| layer.order).collect();
        assert_eq!(layers, [0, 1, 2]);
        for (_, layer) in &layered[1..] {
            assert_eq!(layer.bounds, layered[0].1.bounds);
        }
    }

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// d1_trainstation_01's car: an infodecal with no world face in reach lands on the solid
    /// brush entity it touches, in that entity's frame; a trigger's model is never decaled.
    #[test]
    fn a_decal_out_of_the_worlds_reach_attaches_to_a_solid_brush_entity() {
        let model = |id| BrushModel {
            id,
            surfaces: vec![wall()],
            brushes: Vec::new(),
            mins: glam::Vec3::ZERO,
            maxs: glam::Vec3::new(100., 0., 100.),
        };
        let mut world = World {
            brush_models: vec![model(1), model(2)],
            ..Default::default()
        };
        world.entities.push(entity(&[
            ("classname", "trigger_once"),
            ("model", "*2"),
            ("origin", "1000 0 0"),
        ]));
        world.entities.push(entity(&[
            ("classname", "func_tracktrain"),
            ("model", "*1"),
            ("origin", "1000 0 0"),
        ]));
        let size = glam::Vec2::new(20., 10.);
        assert!(entity_decal(
            &mut world,
            glam::Vec3::new(1050., 2., 50.),
            size,
            "trash"
        ));
        let train = &world.brush_models[0].surfaces;
        assert_eq!(train.len(), 2);
        assert_eq!(train[1].material, "trash");
        // In the model's frame: around local x 50, not world x 1050.
        assert!(
            train[1]
                .vertices
                .iter()
                .all(|v| (v.position.x - 50.).abs() <= 10.01)
        );
        assert_eq!(world.brush_models[1].surfaces.len(), 1);
        // Out of every model's reach: nothing.
        assert!(!entity_decal(
            &mut world,
            glam::Vec3::new(1050., 30., 50.),
            size,
            "trash"
        ));
    }
}
