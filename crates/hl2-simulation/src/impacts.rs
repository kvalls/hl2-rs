//! Shared owned impact selection and triangle-clipped marks. No GPU or file I/O in add.
use crate::{
    entities::Scene,
    physics::{Physics, RayHit},
};
use glam::Vec3 as GVec3;
use modkit_core::{decals, Surface, World};
use source_assets::{keyvalues, vpk::Vfs, vtf};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
// VPhysics hulls can sit slightly outside a model's visible triangles. Keep
// a bounded, same-receiver projection instead of painting the collision plane.
// This is an approximate render-mesh adapter, not Source's studio decal system.
fn receiver(
    surfaces: &[Surface],
    center: GVec3,
    normal: GVec3,
    gap: f32,
) -> Option<(&Surface, GVec3)> {
    if let Some(surface) = surfaces
        .iter()
        .find(|s| !decals::project(std::slice::from_ref(s), center, normal, 0.15, 0.).is_empty())
    {
        return Some((surface, center));
    }
    let mut nearest = None;
    let mut distance = gap;
    for surface in surfaces.iter().filter(|s| !s.background) {
        for triangle in surface.indices.as_chunks::<3>().0 {
            let p = triangle.map(|i| surface.vertices[i as usize].position);
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            let facing = normal.dot(n);
            if facing.abs() < 0.85 {
                continue;
            }
            let offset = (p[0] - center).dot(n) / facing;
            if offset.abs() >= distance {
                continue;
            }
            let projected = center + normal * offset;
            if !decals::project(std::slice::from_ref(surface), projected, normal, 0.15, 0.)
                .is_empty()
            {
                nearest = Some((surface, projected));
                distance = offset.abs();
            }
        }
    }
    nearest
}
pub struct Mark {
    pub id: usize,
    pub vertices: Vec<decals::DecalVertex>,
    pub material: String,
    pub entity: usize,
    pub scale: f32,
}
pub struct Texture {
    pub image: Arc<vtf::Image>,
    pub scale: f32,
}
#[derive(Default)]
pub struct Impacts {
    pub marks: Vec<Mark>,
    pub textures: HashMap<String, Texture>,
    properties: BTreeMap<String, keyvalues::Entry>,
    surfaceprops: BTreeMap<String, String>,
    pub errors: BTreeMap<String, String>,
    pub created: usize,
    pub unclippable: usize,
}
impl Impacts {
    pub fn new(vfs: &Vfs, world: &World) -> Self {
        let mut properties = BTreeMap::new();
        if let Ok(Some(manifest)) = vfs.read("scripts/surfaceproperties_manifest.txt") {
            if let Ok(entries) = keyvalues::parse(&String::from_utf8_lossy(&manifest)) {
                for file in entries
                    .iter()
                    .flat_map(|e| e.children())
                    .filter(|e| e.key.eq_ignore_ascii_case("file"))
                {
                    if let Some(file) = file.text() {
                        if let Ok(Some(data)) = vfs.read(file) {
                            if let Ok(entries) = keyvalues::parse(&String::from_utf8_lossy(&data)) {
                                for entry in entries {
                                    properties.insert(entry.key.to_lowercase(), entry);
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut result = Self {
            properties,
            ..Default::default()
        };
        for surface in world
            .surfaces
            .iter()
            .chain(world.brush_models.iter().flat_map(|m| &m.surfaces))
            .chain(world.model_assets.values().flatten())
        {
            result
                .surfaceprops
                .entry(surface.material.clone())
                .or_insert_with(|| {
                    vfs.material_value(&surface.material, "$surfaceprop")
                        .ok()
                        .flatten()
                        .unwrap_or("default".into())
                });
        }
        for family in ["metal", "wood", "glass", "sand", "concrete"] {
            for i in 1..=5 {
                let material = format!("decals/{family}/shot{i}");
                match crate::projectile_visuals::load_sprite(vfs, &material) {
                    Ok(sprite) => {
                        let scale = vfs
                            .material_value(&material, "$decalscale")
                            .ok()
                            .flatten()
                            .and_then(|s| s.parse::<f32>().ok())
                            .unwrap_or(0.1);
                        result.textures.insert(
                            material,
                            Texture {
                                image: sprite.image,
                                scale,
                            },
                        );
                    }
                    Err(e) => {
                        result.errors.insert(material, format!("{e:#}"));
                    }
                }
            }
        }
        result
    }
    pub fn required_sound_requests(&self) -> Vec<crate::sounds::SoundRequest> {
        self.surfaceprops
            .values()
            .filter_map(|p| self.property(p, "bulletimpact"))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(Into::into)
            .collect()
    }
    fn property(&self, name: &str, key: &str) -> Option<String> {
        let mut current = name.to_lowercase();
        for _ in 0..32 {
            let e = self.properties.get(&current)?;
            if let Some(value) = e.get(key).and_then(|e| e.text()) {
                return Some(value.into());
            }
            current = e
                .get("base")
                .and_then(|e| e.text())
                .unwrap_or("default")
                .to_lowercase();
            if current == e.key.to_lowercase() {
                break;
            }
        }
        None
    }
    pub fn add(
        &mut self,
        hit: RayHit,
        melee: bool,
        world: &World,
        physics: &Physics,
        scene: &mut Scene,
    ) {
        let (surfaces, origin, rotation, scale): (&[Surface], _, _, _) =
            if let Some(e) = world.entities.get(hit.entity) {
                let state = &scene.states[hit.entity];
                if state.killed {
                    return;
                }
                let (origin, rotation) = physics
                    .entity_pose(hit.entity)
                    .unwrap_or((state.origin, state.rotation));
                if let Some(model) = e
                    .get("model")
                    .and_then(|s| s.strip_prefix('*'))
                    .and_then(|s| s.parse::<usize>().ok())
                    .and_then(|id| world.brush_models.iter().find(|m| m.id == id))
                {
                    (&model.surfaces[..], origin, rotation, 1.)
                } else if let Some(instance) = world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(hit.entity))
                {
                    let Some(surfaces) = world.model_assets.get(&instance.asset_key()) else {
                        return;
                    };
                    (&surfaces[..], origin, rotation, instance.scale)
                } else {
                    return;
                }
            } else {
                (&world.surfaces[..], GVec3::ZERO, glam::Quat::IDENTITY, 1.)
            };
        // Project against the currently rendered skeletal pose. Door idle clips
        // can rotate the panel away from the MDL bind pose used by raw vertices.
        let posed = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(hit.entity))
            .and_then(|instance| world.rigs.get(&instance.asset_key()))
            .zip(scene.states.get(hit.entity))
            .filter(|(rig, state)| rig.clips.contains_key(&state.animation))
            .map(|(rig, _)| {
                let matrices = scene.actor_matrices(rig, hit.entity);
                surfaces
                    .iter()
                    .map(|surface| {
                        let mut surface = surface.clone();
                        for vertex in &mut surface.vertices {
                            if let Some(weights) = &vertex.skin {
                                vertex.position = modkit_core::animation::skin(
                                    vertex.position,
                                    weights,
                                    &matrices,
                                );
                            }
                        }
                        surface
                    })
                    .collect::<Vec<_>>()
            });
        let surfaces = posed.as_deref().unwrap_or(surfaces);
        let center = rotation.inverse() * (hit.position - origin) / scale;
        let normal = rotation.inverse() * hit.normal;
        let gap = if hit.entity < world.entities.len() {
            1.5 / scale
        } else {
            0.
        };
        let Some((receiver, center)) = receiver(surfaces, center, normal, gap) else {
            self.unclippable += 1;
            return;
        };
        let prop = self
            .surfaceprops
            .get(&receiver.material)
            .map(String::as_str)
            .unwrap_or("default");
        if !melee {
            if let Some(sound) = self.property(prop, "bulletimpact") {
                scene.sounds.push(sound.into());
            }
        }
        let family = match self
            .property(prop, "gamematerial")
            .as_deref()
            .unwrap_or("C")
        {
            "M" | "V" | "G" => "metal",
            "W" => "wood",
            "Y" => "glass",
            "N" => "sand",
            _ => "concrete",
        };
        let material = format!("decals/{family}/shot{}", self.created % 5 + 1);
        let Some(texture) = self.textures.get(&material) else {
            return;
        };
        let factor = texture.scale;
        let vertices = decals::project(
            surfaces,
            center,
            normal,
            f32::from(texture.image.width) * factor * 0.5 / scale,
            (self.created as f32 * 2.399963).rem_euclid(std::f32::consts::TAU),
        );
        if vertices.is_empty() {
            self.unclippable += 1;
            return;
        }
        if self.marks.len() >= 256 {
            self.marks.remove(0);
        }
        self.marks.push(Mark {
            id: self.created,
            vertices,
            material,
            entity: hit.entity,
            scale,
        });
        self.created += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn flat_world() -> World {
        World {
            surfaces: vec![Surface {
                flex_source: None,
                background: false,
                material: "wall".into(),
                lightmap: None,
                vertices: [
                    GVec3::new(-10., -10., 0.),
                    GVec3::new(10., -10., 0.),
                    GVec3::new(0., 10., 0.),
                ]
                .into_iter()
                .map(|position| modkit_core::Vertex {
                    normal: Default::default(),
                    position,
                    uv: glam::Vec2::ZERO,
                    color: [255; 4],
                    light_uv: glam::Vec2::ZERO,
                    skin: None,
                })
                .collect(),
                indices: vec![0, 1, 2],
            }],
            ..Default::default()
        }
    }
    #[test]
    fn bounded_marks_keep_unique_ids_and_surface_clipping_without_file_access() {
        let world = flat_world();
        let physics = Physics::new(&world);
        let mut scene = Scene::new(&world);
        let mut impacts = Impacts::default();
        let image = Arc::new(vtf::Image {
            width: 16,
            height: 16,
            rgba: vec![128; 16 * 16 * 4],
            format: 0,
            translucent: false,
        });
        for i in 1..=5 {
            impacts.textures.insert(
                format!("decals/concrete/shot{i}"),
                Texture {
                    image: image.clone(),
                    scale: 0.1,
                },
            );
        }
        for _ in 0..257 {
            impacts.add(
                RayHit {
                    entity: usize::MAX,
                    position: GVec3::ZERO,
                    normal: GVec3::Z,
                },
                false,
                &world,
                &physics,
                &mut scene,
            );
        }
        assert_eq!(impacts.created, 257);
        assert_eq!(impacts.marks.len(), 256);
        assert_eq!(impacts.marks.first().unwrap().id, 1);
        assert_eq!(impacts.marks.last().unwrap().id, 256);
        assert!(impacts
            .marks
            .iter()
            .flat_map(|m| &m.vertices)
            .all(|v| (v.position.z - 0.06).abs() < 1e-6
                && v.uv.cmpge(glam::Vec2::splat(-1e-5)).all()
                && v.uv.cmple(glam::Vec2::splat(1.00001)).all()));
        impacts.add(
            RayHit {
                entity: usize::MAX,
                position: GVec3::Z * 10.,
                normal: GVec3::Z,
            },
            false,
            &world,
            &physics,
            &mut scene,
        );
        assert_eq!(impacts.created, 257);
        assert_eq!(impacts.unclippable, 1);
    }
    #[test]
    fn collision_offset_snaps_only_to_nearest_bounded_visible_receiver() {
        let mut world = flat_world();
        let mut far = world.surfaces[0].clone();
        for v in &mut far.vertices {
            v.position.z = -1.;
        }
        world.surfaces.insert(0, far);
        let (surface, center) = receiver(&world.surfaces, GVec3::Z, GVec3::Z, 1.5).unwrap();
        assert_eq!(surface.vertices[0].position.z, 0.);
        assert_eq!(center, GVec3::ZERO);
        assert!(receiver(&world.surfaces, GVec3::Z * 2., GVec3::Z, 1.5).is_none());
        assert!(receiver(&world.surfaces, GVec3::new(100., 0., 1.), GVec3::Z, 1.5).is_none());
        assert!(receiver(&world.surfaces, GVec3::Z, GVec3::Z, 0.).is_none());
    }
    #[test]
    #[ignore = "requires owned installed HL2"]
    fn owned_marks_use_preloaded_textures_and_surface_sounds() {
        let vfs = Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let data = vfs.read("maps/d1_trainstation_02.bsp").unwrap().unwrap();
        let world = source_assets::bsp::Bsp::parse(&data)
            .unwrap()
            .world("d1_trainstation_02")
            .unwrap();
        let mut impacts = Impacts::new(&vfs, &world);
        assert_eq!(impacts.textures.len(), 20, "{:?}", impacts.errors);
        assert_eq!(impacts.errors.len(), 5);
        assert!(impacts.errors.keys().all(|k| k.starts_with("decals/sand/")));
        assert!(!impacts.required_sound_requests().is_empty());
        drop(vfs);
        let surface = world
            .surfaces
            .iter()
            .find(|s| !s.background && s.indices.len() >= 3)
            .unwrap();
        let p = [0, 1, 2].map(|i| surface.vertices[surface.indices[i] as usize].position);
        let center = (p[0] + p[1] + p[2]) / 3.;
        let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
        let mut scene = Scene::new(&world);
        let physics = Physics::new(&world);
        impacts.add(
            RayHit {
                entity: usize::MAX,
                position: center,
                normal,
            },
            false,
            &world,
            &physics,
            &mut scene,
        );
        assert_eq!(impacts.created, 1);
        assert!(!impacts.marks[0].vertices.is_empty());
        assert_eq!(scene.sounds.len(), 1);
    }
    #[test]
    #[ignore = "requires owned installed HL2"]
    fn owned_door_collision_hit_attaches_to_rendered_leaf() {
        let mut vfs = Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let data = vfs.read("maps/d1_trainstation_02.bsp").unwrap().unwrap();
        let bsp = source_assets::bsp::Bsp::parse(&data).unwrap();
        vfs.mount_pak(bsp.lump(40)).unwrap();
        let mut world = bsp.world("d1_trainstation_02").unwrap();
        source_assets::models::append_models(&mut world, &vfs);
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        for (id, state) in scene.states.iter().enumerate() {
            physics.set_entity(
                id,
                state.origin,
                state.rotation,
                !state.killed && state.visible,
            );
        }
        physics.refresh_queries();
        let hit = physics
            .impact_ray(GVec3::new(-3260., -2024., 128.), GVec3::X, 100.)
            .unwrap();
        let mut impacts = Impacts::new(&vfs, &world);
        impacts.add(hit, false, &world, &physics, &mut scene);
        assert_eq!(impacts.created, 1);
        assert_eq!(impacts.marks[0].entity, hit.entity);
    }
}
