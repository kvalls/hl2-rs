//! Dynamic shadows of studio-model entities on the world, as Options > Video > Advanced > Shadow
//! detail sets them (`r_shadowrendertotexture`; Source SDK `game/client/clientshadowmgr.cpp`):
//! at Low every caster gets a blob, the installed `decals/simpleshadow` projected straight down
//! ([`Frame::blob`]); at Medium and High each caster's shadow is projected along the map's shadow
//! direction ([`Frame::render_to_texture`]) from its silhouette. Each frame the shadows in view are
//! ranked by screen area; the 32 largest get a fragment of the shadow page as big as their area
//! asks (`CTextureAllocator`) and their silhouette is drawn there when they moved, animate or got a
//! new fragment; the rest show the blob picture on the same projection (`DrawRenderToTextureShadowLOD`).
//! The geometry is [`hl2_simulation::shadows`]: the receivers are gathered on the map worker
//! ([`receivers`]); each caster keeps one decal mesh, rewritten in place only when it moved a
//! quarter shadow texel, turned, changed sequence or changed look, drawn by [`ShadowMaterial`].
//!
//! Casters ([`caster`]): entities with a studio model and its rig, without `disableshadows`, drawn,
//! and with no caster up their parent chain; bounds as `C_BaseAnimating::GetRenderBounds`.
//! Silhouettes come from the model's coarsest level, posed; casters with pose parameters redraw
//! theirs every frame they are ranked. Only NPC shadows reach static props, and only at
//! render-to-texture detail.
mod caster;
mod draw;
pub mod material;
#[cfg(test)]
mod owned_tests;
mod texture;

use crate::{
    FlyCamera,
    assets::{LoadedMap, MaterialData},
    gameplay::Gameplay,
    rendering::SourceEntity,
    simulation::Simulation,
    ui_host::Console,
    video::GameFrame,
    visibility::PvsDraw,
};
use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::Assets,
    camera::Projection,
    ecs::{
        component::Component,
        query::With,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, Query, Res, ResMut},
    },
    image::Image,
    math::Vec2,
    mesh::Mesh,
    pbr::MaterialPlugin,
    transform::{TransformSystems, components::Transform},
};
use caster::{Caster, Look};
use draw::Draws;
use hl2_simulation::shadows::{
    ShadowControl, TEXELS_PER_UNIT,
    allocator::uv_rect,
    linear_color,
    receivers::{Cast, Receivers, Scratch},
};
use hl2_ui::config::ShadowDetail;
pub use material::ShadowMaterial;
use modkit_core::World;
use std::{collections::BTreeMap, sync::Arc};
use texture::Textures;

/// The blob shadows' material.
pub const BLOB_MATERIAL: &str = "decals/simpleshadow";
/// Shadow geometry sits this far off its receivers (units): above impact decals (0.06), which it
/// darkens as the engine's shadow pass does.
const LIFT: f32 = 0.1;
/// A shadow is rebuilt once its caster moved a quarter of a shadow texel...
const MOVE: f32 = 0.25 / TEXELS_PER_UNIT;
/// ...or turned this far (radians; a quarter texel at 64 units from the origin).
const TURN: f32 = MOVE / 64.;
/// Silhouettes drawn per frame, largest on screen first (`r_shadowmaxrendered`).
const MAX_RENDERED: usize = 32;

/// The receivers among the world's surfaces: not translucent, alpha-tested, additive or flagged
/// `SURF_NOSHADOWS` (the engine's surface flags for these keep shadows off them).
pub fn receivers(world: &World, materials: &BTreeMap<String, MaterialData>) -> Receivers {
    Receivers::new(&world.surfaces, |name| {
        !world.no_shadow_materials.contains(name)
            && materials.get(name).is_none_or(|m| {
                !(m.translucent || m.additive || m.opacity < 1. || m.alpha_cutoff.is_some())
            })
    })
}

/// The shadow material and the per-frame update, after entities are posed.
pub struct ShadowPlugin;

impl Plugin for ShadowPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ShadowMaterial>::default())
            .add_systems(
                PostUpdate,
                update
                    .after(crate::rendering::present_entities)
                    .before(TransformSystems::Propagate),
            );
    }
}

/// A caster's shadow draw.
#[derive(Component)]
pub struct ShadowDraw;

/// How shadows are drawn at the current detail.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Blob,
    Texture,
}

/// The map's shadows: its receivers and settings, each caster's state and the textures.
#[derive(Resource)]
pub struct MapShadows {
    receivers: Arc<Receivers>,
    control: ShadowControl,
    textures: Textures,
    casters: Vec<Caster>,
    /// Model instances already looked at (runtime spawns append to the list).
    seen: usize,
    scratch: Scratch,
    /// Casters in view, largest first (kept between frames for its buffer).
    order: Vec<usize>,
    /// Entities with a draw the view's PVS kept last frame, by entity index.
    potentially_visible: Vec<bool>,
    detail: Option<ShadowDetail>,
    /// Decals rebuilt and silhouettes drawn since the map was installed.
    rebuilds: u64,
    silhouettes: u64,
}

impl MapShadows {
    pub fn new(loaded: &LoadedMap) -> Self {
        let control = ShadowControl::from_world(&loaded.world);
        Self {
            receivers: loaded.shadow_receivers.clone(),
            textures: Textures::new(loaded, linear_color(control.color)),
            control,
            casters: Vec::new(),
            seen: 0,
            scratch: Scratch::default(),
            order: Vec::new(),
            potentially_visible: Vec::new(),
            detail: None,
            rebuilds: 0,
            silhouettes: 0,
        }
    }

    /// Receivers, casters, shadows with geometry and their triangles, page fragments, work done.
    pub fn report(&self) -> serde_json::Value {
        let built = self.casters.iter().filter(|c| !c.decal.is_empty());
        let cells = self
            .casters
            .iter()
            .filter(|c| matches!(c.look, Some(Look::Cell(..))));
        serde_json::json!({
            "detail": self.detail.map(|d| format!("{d:?}")),
            "receiver_triangles": self.receivers.len(),
            "casters": self.casters.len(),
            "with_geometry": built.clone().count(),
            "decal_triangles": built.map(|c| c.decal.indices.len() / 3).sum::<usize>(),
            "in_view": self.order.len(),
            "textured": cells.count(),
            "rebuilds": self.rebuilds,
            "silhouettes": self.silhouettes,
            "control": {"direction": self.control.direction.to_array(), "color": self.control.color,
                "distance": self.control.distance, "disabled": self.control.disabled},
        })
    }

    /// Adds a caster for every model instance new since the last call.
    fn find_casters(&mut self, world: &World) {
        for instance in &world.model_instances[self.seen.min(world.model_instances.len())..] {
            if let Some(id) = instance.entity
                && let Some(caster) = Caster::of(world, id, (instance.asset_key(), instance.scale))
            {
                self.casters.push(caster);
            }
        }
        self.seen = world.model_instances.len();
    }
}

type ShadowAssets<'w> = (
    ResMut<'w, Assets<Mesh>>,
    ResMut<'w, Assets<ShadowMaterial>>,
    ResMut<'w, Assets<Image>>,
);
type CameraView<'w, 's> = (
    Query<'w, 's, (&'static Transform, &'static Projection), With<FlyCamera>>,
    Option<Res<'w, GameFrame>>,
);
type EntityDraws<'w, 's> = Query<'w, 's, (&'static SourceEntity, &'static PvsDraw)>;

/// The world camera, for screen areas: origin and forward in Source axes, and pixels per unit of
/// size at unit depth (viewport height over the tangent of half the vertical field of view).
struct View {
    origin: glam::Vec3,
    forward: glam::Vec3,
    right: glam::Vec3,
    up: glam::Vec3,
    /// Tangents of half the field of view, horizontal and vertical.
    tan: glam::Vec2,
    pixels: f32,
}

impl View {
    fn of((camera, frame): &CameraView) -> Option<Self> {
        let (transform, Projection::Perspective(p)) = camera.single().ok()? else {
            return None;
        };
        let source =
            |v: bevy::math::Vec3| glam::Vec3::from_array(crate::bevy_to_source(v).to_array());
        let size = frame.as_ref().map_or(glam::Vec2::new(1024., 768.), |f| {
            glam::Vec2::new(f.resolution.x as f32, f.resolution.y as f32)
        });
        let tan = (p.fov / 2.).tan();
        Some(Self {
            origin: source(transform.translation),
            forward: source(transform.forward().as_vec3()),
            right: source(transform.right().as_vec3()),
            up: source(transform.up().as_vec3()),
            tan: glam::Vec2::new(tan * size.x / size.y.max(1.), tan),
            pixels: size.y / tan,
        })
    }

    /// Whether a sphere touches the view frustum (`CullBox` on the shadow's box, made a sphere).
    fn sees(&self, (centre, radius): (glam::Vec3, f32)) -> bool {
        let to = centre - self.origin;
        let (x, y, z) = (to.dot(self.right), to.dot(self.up), to.dot(self.forward));
        let side =
            |offset: f32, tan: f32| offset.abs() - z * tan <= radius * (1. + tan * tan).sqrt();
        z > -radius && side(x, self.tan.x) && side(y, self.tan.y)
    }

    /// `ComputeScreenArea`: the squared pixel diameter of a sphere.
    fn area(&self, (centre, radius): (glam::Vec3, f32)) -> f32 {
        let depth = (centre - self.origin).dot(self.forward).max(1.);
        (radius * self.pixels / depth).powi(2)
    }
}

/// Each caster's shadow follows it: placed where it moved, textured by rank, hidden while it is.
pub(crate) fn update(
    mut commands: Commands,
    (ui, game, sim): (Option<Res<Console>>, Res<Gameplay>, Res<Simulation>),
    shadows: Option<ResMut<MapShadows>>,
    (mut meshes, mut materials, mut images): ShadowAssets,
    mut draws: Draws,
    (camera, entity_draws, performance): (
        CameraView,
        EntityDraws,
        Option<Res<crate::performance::Performance>>,
    ),
) {
    let Some(mut shadows) = shadows else {
        return;
    };
    let _timing = crate::performance::scope(performance.as_deref(), "shadows");
    let detail = ui.map(|ui| ui.source.config.video.advanced.shadow_detail);
    if detail != shadows.detail {
        shadows.detail = detail;
        shadows.casters.iter_mut().for_each(Caster::invalidate);
    }
    shadows.find_casters(&game.world);
    shadows.textures.prepare(&mut materials, &mut images);
    let mode = match detail {
        Some(ShadowDetail::Low) => Mode::Blob,
        Some(_) if !shadows.control.disabled => Mode::Texture,
        _ => Mode::Off,
    };
    let view = View::of(&camera).filter(|_| mode == Mode::Texture);
    let shadows = &mut *shadows;
    if view.is_some() {
        let count = game.world.entities.len();
        potentially_visible(&mut shadows.potentially_visible, &entity_draws, count);
    }
    let shown = place(
        &mut shadows.casters,
        (&game, &sim),
        (&shadows.control, mode, view.as_ref()),
        &shadows.potentially_visible,
    );
    if mode == Mode::Texture {
        shadows.textures.page.advance_frame();
        shadows.silhouettes += rank(shadows, &game, &mut images);
    }
    for (caster, shown) in shadows.casters.iter_mut().zip(shown) {
        if !shown {
            draw::show(caster, &mut draws, false);
            continue;
        }
        if caster.look != Some(caster.wanted) || caster.moved {
            lay(caster, (&shadows.receivers, &mut shadows.scratch));
            shadows.rebuilds += 1;
            let rect = match caster.wanted {
                Look::Cell(corner, n) => to_bevy(uv_rect((corner, n))),
                _ => (Vec2::ZERO, Vec2::ONE),
            };
            let material = match caster.wanted {
                Look::Cell(..) => shadows.textures.atlas_material.clone(),
                _ => shadows.textures.blob.clone(),
            };
            if let Some(material) = material.filter(|_| !caster.decal.is_empty()) {
                draw::write(
                    caster,
                    (&mut commands, &mut meshes),
                    (material, rect),
                    &mut draws,
                );
            }
            caster.look = Some(caster.wanted);
        }
        let visible = !caster.decal.is_empty();
        draw::show(caster, &mut draws, visible);
    }
    shadows.textures.refresh(&mut materials);
}

/// Marks the entities with a draw the PVS kept (`EnumerateShadowsInLeaves` walks the view's leaves).
/// Ids past the map's `count` entities (projectile bodies) are not casters.
fn potentially_visible(marks: &mut Vec<bool>, draws: &EntityDraws, count: usize) {
    marks.clear();
    marks.resize(count, false);
    for (owner, pvs) in draws {
        if let Some(mark) = marks.get_mut(owner.0) {
            *mark |= !pvs.culled;
        }
    }
}

fn to_bevy((origin, size): (glam::Vec2, glam::Vec2)) -> (Vec2, Vec2) {
    (
        Vec2::from_array(origin.to_array()),
        Vec2::from_array(size.to_array()),
    )
}

/// Places every caster's frame; which are shown this frame. Shown casters in view get their
/// screen area; each keeps its look unless the mode changes it.
fn place(
    casters: &mut [Caster],
    (game, sim): (&Gameplay, &Simulation),
    (control, mode, view): (&ShadowControl, Mode, Option<&View>),
    pvs: &[bool],
) -> Vec<bool> {
    let mut shown = vec![false; casters.len()];
    for index in 0..casters.len() {
        let id = casters[index].id;
        let hidden = mode == Mode::Off
            || crate::rendering::draw_hidden(game, id, None)
            || caster::under_caster(game, casters, id);
        let caster = &mut casters[index];
        caster.area = None;
        if hidden {
            continue;
        }
        let Some(moved) = caster.place((game, sim), control, mode == Mode::Blob) else {
            continue;
        };
        caster.moved = moved;
        shown[index] = true;
        caster.wanted = match (mode, caster.look) {
            (Mode::Blob, _) => Look::Blob,
            (_, Some(look)) if look != Look::Blob => look,
            _ => Look::Lod,
        };
        // The shadow reaches its cast distance past the caster: the sphere stretched along it.
        let reach = |(centre, radius): (glam::Vec3, f32)| {
            (
                centre + control.direction * (control.distance * 0.5),
                radius + control.distance * 0.5,
            )
        };
        if let (Some(view), Some(sphere)) = (view, caster.sphere())
            && pvs.get(id).copied().unwrap_or(true)
            && view.sees(reach(sphere))
        {
            caster.area = Some(view.area(sphere));
        }
    }
    shown
}

/// Ranks the casters in view by screen area: the first [`MAX_RENDERED`] get page fragments, their
/// silhouettes drawn when new, moved or animating; the rest take the blob look. Returns how many
/// silhouettes were drawn.
fn rank(shadows: &mut MapShadows, game: &Gameplay, images: &mut Assets<Image>) -> u64 {
    let MapShadows {
        casters,
        order,
        textures,
        ..
    } = shadows;
    order.clear();
    order.extend((0..casters.len()).filter(|&i| casters[i].area.is_some()));
    order.sort_by(|&a, &b| {
        casters[b]
            .area
            .unwrap_or(0.)
            .total_cmp(&casters[a].area.unwrap_or(0.))
    });
    let mut drawn = 0;
    for (place, &index) in order.iter().enumerate() {
        let caster = &mut casters[index];
        caster.wanted = Look::Lod;
        if place >= MAX_RENDERED {
            continue;
        }
        let size = caster.size;
        let id = *caster
            .texture
            .get_or_insert_with(|| textures.page.allocate(size));
        let fresh = textures
            .page
            .use_texture(id, caster.dirty, caster.area.unwrap_or(0.));
        let Some((corner, n)) = textures.page.rect(id) else {
            continue;
        };
        if fresh || caster.dirty {
            caster.silhouette(game, (textures, images), (n, corner));
            caster.dirty = caster.dynamic;
            drawn += 1;
        }
        caster.wanted = Look::Cell(corner, n);
    }
    drawn
}

/// Lays the caster's decal for its wanted look: the blob and cell looks keep the frame's clip
/// planes, the render-to-texture blob drops them (`ClearExtraClipPlanes`).
fn lay(caster: &mut Caster, (receivers, scratch): (&Receivers, &mut Scratch)) {
    let Some(frame) = caster.frame.as_mut() else {
        caster.decal.clear();
        return;
    };
    let clip = if caster.wanted == Look::Lod {
        std::mem::take(&mut frame.clip)
    } else {
        Vec::new()
    };
    let cast = Cast {
        static_props: caster.npc && caster.wanted != Look::Blob,
        bias: 0,
        lift: LIFT,
    };
    receivers.decal(frame, cast, scratch, &mut caster.decal);
    if !clip.is_empty() {
        frame.clip = clip;
    }
    caster.moved = false;
}
