//! Studio model geometry (MDL + VVD + DX90 VTX): the triangles of one skin and body at each
//! level of detail. Source SDK `public/optimize.h` (VTX LODs and their switch points) and
//! `public/studio.h` (`studiohwdata_t::GetLODForMetric`: a trailing LOD with a negative switch
//! point is the shadow LOD, never drawn for the view).

use super::{body_parts, body_selection, studio_submodels};
use crate::{bytes, i32le, u32le, vpk::Vfs};
use anyhow::{bail, Context, Result};
use glam::{Vec2, Vec3};
use modkit_core::{ModelLod, Surface, Vertex};
use std::collections::BTreeMap;

/// Index of the LOD drawn for `root_lod`: clamped to the authored LODs, excluding a
/// trailing shadow LOD (negative switch point).
pub fn root_lod_index(switch_points: &[f32], root_lod: usize) -> usize {
    root_lod.min(view_lods(switch_points).saturating_sub(1))
}

/// LODs that can be drawn for the view: all but a trailing shadow LOD.
fn view_lods(switch_points: &[f32]) -> usize {
    if switch_points.last().is_some_and(|s| *s < 0.0) {
        switch_points.len().saturating_sub(1)
    } else {
        switch_points.len()
    }
}

/// A model's levels of detail from the root LOD down: the root's surfaces and each coarser
/// level with its switch point.
pub struct ModelLevels {
    pub root: Vec<Surface>,
    pub lower: Vec<ModelLod>,
}

/// A world's root surfaces and coarser levels per model asset key.
type Tables<'a> = (
    &'a mut BTreeMap<String, Vec<Surface>>,
    &'a mut BTreeMap<String, Vec<ModelLod>>,
);

impl ModelLevels {
    /// Keeps the root surfaces of an instance's model and, for an entity, its coarser levels (static
    /// props are baked into the world and keep the root LOD).
    pub fn store(&self, instance: &modkit_core::ModelInstance, (assets, lods): Tables) {
        let key = instance.asset_key();
        if instance.entity.is_some() && !self.lower.is_empty() {
            lods.entry(key.clone())
                .or_insert_with(|| self.lower.clone());
        }
        assets.entry(key).or_insert_with(|| self.root.clone());
    }
}

/// The model's meshes for a skin and a body value (each body part draws the submodel
/// the body selects; body 0 is every part's first submodel) at a root LOD.
pub fn read_model_variant(
    vfs: &Vfs,
    path: &str,
    skin: usize,
    body: i32,
    root_lod: usize,
) -> Result<Vec<Surface>> {
    let model = Parsed::read(vfs, path, body)?;
    let mut materials = BTreeMap::new();
    model.surfaces(vfs, skin, root_lod, &mut materials)
}

/// The model's meshes for a skin and a body value at the root LOD and every coarser one.
pub fn read_model_levels(
    vfs: &Vfs,
    path: &str,
    skin: usize,
    body: i32,
    root_lod: usize,
) -> Result<ModelLevels> {
    let model = Parsed::read(vfs, path, body)?;
    let mut materials = BTreeMap::new();
    let root = model.surfaces(vfs, skin, root_lod, &mut materials)?;
    let switch_points = model.switch_points();
    let first = root_lod_index(&switch_points, root_lod);
    let mut lower = Vec::new();
    let drawable = &switch_points[..view_lods(&switch_points)];
    for (level, &switch_point) in drawable.iter().enumerate().skip(first + 1) {
        lower.push(ModelLod {
            switch_point,
            surfaces: model.surfaces(vfs, skin, level, &mut materials)?,
        });
    }
    Ok(ModelLevels { root, lower })
}

/// The files of one model and the submodels a body value selects.
struct Parsed {
    mdl: vmdl::Mdl,
    vvd: vmdl::Vvd,
    vtx: vmdl::Vtx,
    selection: Vec<usize>,
    submodels: Vec<Vec<super::StudioSubmodel>>,
    skin_reference_count: usize,
}

impl Parsed {
    fn read(vfs: &Vfs, path: &str, body: i32) -> Result<Self> {
        let stem = path.trim_end_matches(".mdl");
        let mdl_bytes = vfs.read(path)?.context("MDL missing")?;
        let vvd_bytes = vfs.read(&format!("{stem}.vvd"))?.context("VVD missing")?;
        let vtx_bytes = vfs
            .read(&format!("{stem}.dx90.vtx"))?
            .context("DX90 VTX missing")?;
        if bytes(&mdl_bytes, 0, 4)? != b"IDST" || bytes(&vvd_bytes, 0, 4)? != b"IDSV" {
            bail!("model magic mismatch");
        }
        if !(44..=49).contains(&u32le(&mdl_bytes, 4)?)
            || u32le(&vvd_bytes, 4)? != 4
            || u32le(&vtx_bytes, 0)? != 7
        {
            bail!("unsupported MDL/VVD/VTX version");
        }
        let checksum = u32le(&mdl_bytes, 8)?;
        if checksum != u32le(&vvd_bytes, 8)? || checksum != u32le(&vtx_bytes, 16)? {
            bail!("model companion checksum mismatch");
        }
        // Geometry loading must not enter vmdl's unimplemented external animation-block path.
        let mut geometry_bytes = mdl_bytes.clone();
        bytes(&geometry_bytes, 180, 4)?;
        geometry_bytes[180..184].copy_from_slice(&0i32.to_le_bytes());
        Ok(Self {
            mdl: vmdl::Mdl::read(&geometry_bytes)?,
            skin_reference_count: usize::try_from(i32le(&mdl_bytes, 220)?)?,
            vvd: vmdl::Vvd::read(&vvd_bytes)?,
            vtx: vmdl::Vtx::read(&vtx_bytes)?,
            selection: if body == 0 {
                Vec::new()
            } else {
                body_selection(&body_parts(&mdl_bytes)?, body)
            },
            submodels: studio_submodels(&mdl_bytes)?,
        })
    }

    /// The switch points of the first selected submodel's LODs (every submodel of a model
    /// shares the header's LOD count and switch points).
    fn switch_points(&self) -> Vec<f32> {
        self.vtx
            .body_parts
            .first()
            .and_then(|part| {
                part.models
                    .get(self.selection.first().copied().unwrap_or(0))
            })
            .map(|m| m.lods.iter().map(|l| l.switch_point).collect())
            .unwrap_or_default()
    }

    /// The material a mesh draws for a skin, resolved once per texture index.
    fn material(
        &self,
        vfs: &Vfs,
        skin: usize,
        mesh_material: i32,
        cache: &mut BTreeMap<usize, String>,
    ) -> Result<String> {
        let material = usize::try_from(mesh_material)?;
        let texture_index = if self.skin_reference_count > 0 {
            let index = skin
                .checked_mul(self.skin_reference_count)
                .and_then(|i| i.checked_add(material))
                .context("skin overflow")?;
            *self
                .mdl
                .skin_table
                .get(index)
                .or_else(|| self.mdl.skin_table.get(material))
                .context("skin material missing")? as usize
        } else {
            material
        };
        if let Some(name) = cache.get(&texture_index) {
            return Ok(name.clone());
        }
        let texture = self
            .mdl
            .textures
            .get(texture_index)
            .context("model texture index outside table")?;
        let candidates = self
            .mdl
            .texture_paths
            .iter()
            .map(|p| format!("{}{}", p.replace('\\', "/"), texture.name))
            .chain(std::iter::once(texture.name.clone()));
        let mut material_name = texture.name.clone();
        if let Some(resolved) = vfs.resolve_material_name(&texture.name) {
            material_name = resolved;
        }
        for candidate in candidates {
            if vfs.read(&format!("materials/{candidate}.vmt"))?.is_some() {
                material_name = candidate;
                break;
            }
        }
        cache.insert(texture_index, material_name.clone());
        Ok(material_name)
    }

    /// Every selected submodel's meshes at one LOD (clamped per submodel to its drawable LODs).
    fn surfaces(
        &self,
        vfs: &Vfs,
        skin: usize,
        level: usize,
        materials: &mut BTreeMap<usize, String>,
    ) -> Result<Vec<Surface>> {
        let mut surfaces = Vec::new();
        for (body_index, (part, topology)) in
            self.submodels.iter().zip(&self.vtx.body_parts).enumerate()
        {
            let model_index = self.selection.get(body_index).copied().unwrap_or(0);
            let Some(model) = part.get(model_index) else {
                continue;
            };
            let lod = topology
                .models
                .get(model_index)
                .and_then(|m| {
                    let switch_points: Vec<f32> = m.lods.iter().map(|l| l.switch_point).collect();
                    m.lods
                        .get(root_lod_index(&switch_points, level))
                        .or_else(|| m.lods.first())
                })
                .context("missing model LOD")?;
            for (mesh_index, (&(mesh_material, mesh_vertex_offset), topology)) in
                model.meshes.iter().zip(&lod.meshes).enumerate()
            {
                let material = self.material(vfs, skin, mesh_material, materials)?;
                let base = model
                    .vertex_offset
                    .checked_add(mesh_vertex_offset)
                    .context("model vertex overflow")?;
                let surface = self.mesh(
                    material,
                    base,
                    topology,
                    (body_index, model_index, mesh_index),
                )?;
                if !surface.indices.is_empty() {
                    surfaces.push(surface);
                }
            }
        }
        if surfaces.is_empty() {
            bail!("model has no default-bodygroup triangles");
        }
        Ok(surfaces)
    }

    /// One mesh's triangles at one LOD; `source` is (body part, submodel, mesh) for facial flexes.
    fn mesh(
        &self,
        material: String,
        base: usize,
        topology: &vmdl::vtx::Mesh,
        (bodypart, model, mesh): (usize, usize, usize),
    ) -> Result<Surface> {
        let mut surface = Surface {
            background: false,
            material,
            lightmap: None,
            vertices: Vec::new(),
            indices: Vec::new(),
            flex_source: None,
        };
        let mut vertex_ids = Vec::new();
        for group in &topology.strip_groups {
            for strip in &group.strips {
                let mut indices: Vec<_> = strip.indices().collect();
                // vmdl 0.2 expands two extra triangles for a triangle strip.
                if strip.flags.contains(vmdl::vtx::StripFlags::IS_TRI_STRIP) {
                    indices.truncate(indices.len().saturating_sub(6));
                }
                for triangle in indices.as_chunks::<3>().0 {
                    for &index in triangle {
                        let group_index = *group
                            .indices
                            .get(index)
                            .context("strip index out of range")?
                            as usize;
                        let vertex_id = group
                            .vertices
                            .get(group_index)
                            .context("VTX vertex missing")?
                            .original_mesh_vertex_id
                            as usize;
                        let vertex = self
                            .vvd
                            .vertices
                            .get(base + vertex_id)
                            .context("VVD vertex missing")?;
                        surface.indices.push(surface.vertices.len() as u32);
                        vertex_ids
                            .push(u16::try_from(vertex_id).context("mesh vertex id exceeds u16")?);
                        surface.vertices.push(Vertex {
                            normal: Vec3::new(vertex.normal.x, vertex.normal.y, vertex.normal.z),
                            position: Vec3::new(
                                vertex.position.x,
                                vertex.position.y,
                                vertex.position.z,
                            ),
                            uv: Vec2::from_array(vertex.texture_coordinates),
                            color: [180, 180, 180, 255],
                            light_uv: Vec2::ZERO,
                            skin: Some({
                                let mut bones = [0; 3];
                                let mut weights = [0.; 3];
                                for (i, w) in vertex.bone_weights.weights().enumerate() {
                                    bones[i] = w.bone_id;
                                    weights[i] = w.weight;
                                }
                                modkit_core::animation::Weights { bones, weights }
                            }),
                        });
                    }
                }
            }
        }
        if !surface.indices.is_empty() {
            surface.flex_source = Some(modkit_core::FlexSource {
                bodypart,
                model,
                mesh,
                vertex_ids,
            });
        }
        Ok(surface)
    }
}
