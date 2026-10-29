//! Insertion of components for map-spawned entity and model draws.
use super::{
    AnimatedMesh, DrawTriangles, EyeMesh, NpcWeaponDraw, Owner, SourceEntity, WeaponMesh,
    WeaponSkin, lod,
};
use crate::assets::{LoadedMap, MaterialData};
use bevy::{
    asset::{Assets, Handle},
    ecs::system::EntityCommands,
    image::Image,
    prelude::Visibility,
    render::mesh::Mesh,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type TwoTextures = BTreeMap<String, (Vec<Handle<Image>>, Vec<Handle<Image>>)>;

pub(crate) struct MapDrawContext<'a> {
    pub owner: &'a Owner,
    pub name: &'a str,
    pub triangles: usize,
    pub definition: &'a MaterialData,
    pub world: &'a modkit_core::World,
    pub loaded: &'a LoadedMap,
    pub two_textures: &'a TwoTextures,
    pub weapon_skins: &'a BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>,
}

fn insert_entity_draw(
    draw: &mut EntityCommands,
    cx: &MapDrawContext,
    id: usize,
    handle: &Handle<Mesh>,
    animation: Option<&AnimatedMesh>,
    meshes: &mut Assets<Mesh>,
) {
    draw.insert(SourceEntity(id));
    let eye = cx
        .world
        .model_instances
        .iter()
        .find(|i| i.entity == Some(id))
        .and_then(|instance| {
            cx.loaded
                .eyes
                .get(&(instance.asset_key(), cx.name.to_string()))
                .map(|eye| (instance, eye))
        });
    if let Some((instance, eye)) = eye {
        draw.insert(EyeMesh {
            key: instance.asset_key(),
            eye: eye.clone(),
            scale: instance.scale,
        });
    } else if let Some(levels) = lod::levels_for_draw(
        cx.world,
        id,
        cx.name,
        cx.definition,
        (handle, animation),
        meshes,
    ) {
        draw.insert(levels);
    }
    let hidden = cx
        .loaded
        .gameplay
        .scene
        .states
        .get(id)
        .is_none_or(|s| !s.visible || s.killed);
    if hidden {
        draw.insert(Visibility::Hidden);
    }
}

pub(crate) fn insert_map_draw_components(
    draw: &mut EntityCommands,
    cx: &MapDrawContext,
    handle: &Handle<Mesh>,
    animation: Option<&AnimatedMesh>,
    meshes: &mut Assets<Mesh>,
) {
    if matches!(cx.owner, Owner::SkyWorld)
        || cx
            .owner
            .entity()
            .is_some_and(|id| cx.world.background_entities.contains(&id))
    {
        draw.insert(bevy::camera::visibility::RenderLayers::layer(4));
    }
    if matches!(
        cx.owner,
        Owner::World | Owner::Entity(_) | Owner::NpcWeapon(..)
    ) && !cx
        .owner
        .entity()
        .is_some_and(|id| cx.world.background_entities.contains(&id))
    {
        draw.insert((
            crate::visibility::PvsDraw::default(),
            DrawTriangles(cx.triangles),
        ));
    }
    if cx.definition.camera {
        draw.insert(crate::monitors::MonitorMaterial {
            name: cx.name.to_string(),
            animation: cx.definition.camera_animation.clone(),
            color: cx.definition.tint,
            color2: cx.definition.camera_color2,
        });
        draw.insert(bevy::camera::visibility::RenderLayers::layer(5));
    }
    let two_texture = cx
        .definition
        .two_texture
        .as_ref()
        .zip(cx.two_textures.get(cx.name));
    if let Some((textures, (base, second))) = two_texture {
        let center = cx
            .owner
            .entity()
            .and_then(|id| cx.world.entities[id].get("model"))
            .and_then(|model| model.strip_prefix('*'))
            .and_then(|id| id.parse::<usize>().ok())
            .and_then(|id| cx.world.brush_models.iter().find(|m| m.id == id))
            .map_or(glam::Vec3::ZERO, |model| (model.mins + model.maxs) * 0.5);
        draw.insert(crate::material_proxies::TwoTextureDraw {
            entity: cx.owner.entity(),
            center,
            frames: textures.frames.clone(),
            scroll: textures.scroll.clone(),
            base: base.clone(),
            second: second.clone(),
        });
    }
    if let Owner::NpcWeapon(_, key) = cx.owner {
        draw.insert(NpcWeaponDraw(key.clone()));
    }
    if let Some(id) = cx.owner.entity() {
        insert_entity_draw(draw, cx, id, handle, animation, meshes);
    }
    if let Owner::Weapon(weapon) = cx.owner {
        draw.insert((
            WeaponMesh(weapon.clone()),
            bevy::camera::visibility::RenderLayers::layer(1),
            Visibility::Hidden,
        ));
        if let Some((skin0, skin1)) = cx.weapon_skins.get(weapon) {
            if skin1.contains(cx.name) {
                draw.insert(WeaponSkin(1));
            } else if skin0.contains(cx.name) {
                draw.insert(WeaponSkin(0));
            }
        }
    }
}
