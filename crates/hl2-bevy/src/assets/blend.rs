//! Secondary textures a material names by key: the eyes' `$iris`, and WorldVertexTransition's
//! `$basetexture2` (Source SDK `lightmappedgeneric_ps2_3_x.h`: the two bases are lerped by
//! vertex alpha; displacements carry that alpha per vertex).
use super::{Definition, MaterialData, asset_path, cached_texture};
use anyhow::{Context, Result};
use source_assets::{vpk::Vfs, vtf::GpuImage};
use std::{collections::BTreeMap, sync::Arc};

/// The texture cache and the map's decoded-byte budget.
pub(super) type Textures<'a> = (&'a mut BTreeMap<String, Arc<GpuImage>>, &'a mut usize);

/// The texture the definition names under `key`, or `None` when it names none.
pub(super) fn named_texture(
    vfs: &Vfs,
    definition: &Definition,
    key: &str,
    (cache, decoded_bytes): Textures,
    quality: (i32, bool),
) -> Result<Option<(String, Arc<GpuImage>)>> {
    let Some(name) = definition.properties.get(key) else {
        return Ok(None);
    };
    let path = asset_path(name, ".vtf")?;
    let image = cached_texture(cache, decoded_bytes, &path, quality, || {
        vfs.read(&path)?
            .with_context(|| format!("VTF absent: {path}"))
    })?;
    Ok(Some((path, image)))
}

/// Loads `$basetexture2` into `material` when the definition names one.
pub(super) fn load_second_base(
    vfs: &Vfs,
    definition: &Definition,
    material: &mut MaterialData,
    textures: Textures,
    quality: (i32, bool),
) -> Result<()> {
    material.base2 = named_texture(vfs, definition, "$basetexture2", textures, quality)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_canal_displacement_material_has_its_second_base() {
        let root = source_assets::install::discover().unwrap();
        let loaded = crate::assets::load(&root, "hl2", "d1_canals_01a").unwrap();
        // The map's cubemap patch of nature/blendmudmud001a (WorldVertexTransition).
        let (name, material) = loaded
            .materials
            .iter()
            .find(|(name, _)| name.contains("blendmudmud001a") && !name.contains("wvt_patch"))
            .expect("the canal's blended mud material");
        let (path, image) = material.base2.as_ref().expect("its $basetexture2");
        assert!(path.ends_with("nature/mudfloor002a.vtf"), "{name}: {path}");
        assert!(image.width > 0 && material.base.is_some());
        // The compiler's brush version drops the second texture.
        let patched = loaded
            .materials
            .iter()
            .find(|(n, _)| n.contains("blendmudmud001a_wvt_patch"));
        assert!(patched.is_some_and(|(_, m)| m.base2.is_none()));
    }
}
