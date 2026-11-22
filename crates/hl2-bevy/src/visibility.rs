//! Conservative Source PVS rejection, after current animated bounds and transforms.
use bevy::{
    camera::{primitives::Aabb, visibility::RenderLayers},
    prelude::*,
};
use source_assets::visibility::{PvsRow, VisibilityIndex};
use std::{collections::BTreeMap, sync::Arc};

/// The last PVS test of one draw. `clusters` lists the leaf clusters its padded bounds touch,
/// filled on the first view change while the bounds stay put, so later view changes cost only
/// bitset lookups instead of a tree walk; it keeps its capacity when the bounds move.
#[derive(Default)]
struct Tested {
    epoch: u64,
    center: glam::Vec3,
    extent: glam::Vec3,
    culled: bool,
    listed: bool,
    clusters: Vec<i16>,
}
#[derive(Component, Default)]
pub struct PvsDraw {
    pub culled: bool,
    tested: Option<Tested>,
}
#[derive(Resource)]
pub struct SourceVisibility {
    index: Option<VisibilityIndex>,
    rows: BTreeMap<i16, Option<Arc<PvsRow>>>,
    views: Vec<Arc<PvsRow>>,
    stack: Vec<i32>,
    error: Option<String>,
    checked: usize,
    culled: usize,
    clusters: Vec<i16>,
    disabled: bool,
    view_epoch: u64,
    unrestricted: bool,
    reused: usize,
    listed: usize,
}
impl SourceVisibility {
    pub fn new(index: anyhow::Result<VisibilityIndex>, disabled: bool) -> Self {
        let (index, error) = match index {
            Ok(index) => (Some(index), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        Self {
            index,
            rows: BTreeMap::new(),
            views: vec![],
            stack: vec![],
            error,
            checked: 0,
            culled: 0,
            clusters: vec![],
            disabled,
            view_epoch: 0,
            unrestricted: true,
            reused: 0,
            listed: 0,
        }
    }
    fn row(&mut self, eye: glam::Vec3) -> anyhow::Result<Option<Arc<PvsRow>>> {
        let Some(index) = &self.index else {
            return Ok(None);
        };
        let cluster = index.cluster(eye)?;
        self.clusters.push(cluster);
        if let Some(row) = self.rows.get(&cluster) {
            return Ok(row.clone());
        }
        let row = index.row(cluster)?.map(Arc::new);
        if self.rows.len() >= 8 {
            self.rows.clear();
        }
        self.rows.insert(cluster, row.clone());
        Ok(row)
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"enabled":!self.disabled,"index_available":self.index.is_some(),"view_clusters":self.clusters,"cached_rows":self.rows.len(),"checked_draws":self.checked,"culled_draws":self.culled,"reused_bounds_tests":self.reused,"listed_bounds_tests":self.listed,"error":self.error,"policy":"current animated bounds; union of active world/monitor views, fail open without valid PVS; sky/viewmodels excluded; area portals and occluders remain incomplete"})
    }
}

/// How a draw's PVS answer was found this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Answer {
    /// Same bounds and views as last frame.
    Reused,
    /// Same bounds, new views: the cached cluster list against the new rows.
    Listed,
    /// New bounds: an early-exit tree walk.
    Walked,
}

/// Whether a draw with bounds `center ± extent` (Source units) is outside every view row.
fn test_draw(
    tested: &mut Option<Tested>,
    (center, extent): (glam::Vec3, glam::Vec3),
    (index, views, epoch): (&VisibilityIndex, &[Arc<PvsRow>], u64),
    stack: &mut Vec<i32>,
) -> anyhow::Result<(bool, Answer)> {
    // Keep coplanar surfaces and compensate float basis/bounds rounding.
    let padding = glam::Vec3::splat(1. / 32.);
    let (mins, maxs) = (center - extent - padding, center + extent + padding);
    let same = tested
        .as_ref()
        .is_some_and(|t| t.center == center && t.extent == extent);
    let mut last = tested.take().unwrap_or_default();
    let answer = if same && last.epoch == epoch {
        Answer::Reused
    } else if same {
        if !last.listed {
            index.bounds_clusters(mins, maxs, stack, &mut last.clusters)?;
            last.listed = true;
        }
        let visible = views
            .iter()
            .any(|row| last.clusters.iter().any(|&c| row.contains(c)));
        last.culled = !visible;
        Answer::Listed
    } else {
        let mut visible = false;
        for row in views {
            if index.bounds_visible(mins, maxs, row, stack)? {
                visible = true;
                break;
            }
        }
        last.listed = false;
        last.culled = !visible;
        Answer::Walked
    };
    last.epoch = epoch;
    last.center = center;
    last.extent = extent;
    let culled = last.culled;
    *tested = Some(last);
    Ok((culled, answer))
}
type DrawItem = (
    &'static mut PvsDraw,
    &'static mut Visibility,
    &'static GlobalTransform,
    &'static Aabb,
    Option<&'static crate::rendering::SourceEntity>,
    Option<&'static crate::rendering::NpcWeaponDraw>,
);
pub fn present(
    mut visibility: ResMut<SourceVisibility>,
    game: Res<crate::gameplay::Gameplay>,
    cameras: Query<(&Camera, &GlobalTransform, Option<&RenderLayers>)>,
    mut draws: Query<DrawItem>,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "source_pvs");
    visibility.checked = 0;
    visibility.culled = 0;
    let previous_clusters = std::mem::take(&mut visibility.clusters);
    visibility.reused = 0;
    visibility.listed = 0;
    if visibility.index.is_some() {
        visibility.error = None;
    }
    let world_layers = RenderLayers::layer(0).with(5);
    let mut rows = std::mem::take(&mut visibility.views);
    rows.clear();
    let mut unrestricted = visibility.disabled || visibility.index.is_none();
    if !unrestricted {
        for (camera, transform, layers) in &cameras {
            if !camera.is_active
                || !layers
                    .unwrap_or(&RenderLayers::default())
                    .intersects(&world_layers)
            {
                continue;
            }
            match visibility.row(glam::Vec3::from_array(
                crate::bevy_to_source(transform.translation()).to_array(),
            )) {
                Ok(Some(row)) => rows.push(row),
                Ok(None) => unrestricted = true,
                Err(error) => {
                    visibility.error = Some(format!("{error:#}"));
                    unrestricted = true;
                }
            }
        }
    }
    if rows.is_empty() {
        unrestricted = true;
    }
    if visibility.clusters != previous_clusters || unrestricted != visibility.unrestricted {
        visibility.view_epoch = visibility.view_epoch.wrapping_add(1);
    }
    visibility.unrestricted = unrestricted;
    let state = &mut *visibility;
    for (mut pvs, mut draw, transform, aabb, owner, weapon) in &mut draws {
        let scene_hidden =
            owner.is_some_and(|owner| crate::rendering::draw_hidden(&game, owner.0, weapon));
        state.checked += 1;
        let culled = if unrestricted || scene_hidden {
            false
        } else {
            let index = state
                .index
                .as_ref()
                .expect("restricted views have an index");
            match test_draw(
                &mut pvs.bypass_change_detection().tested,
                source_bounds(transform, aabb),
                (index, &rows, state.view_epoch),
                &mut state.stack,
            ) {
                Ok((culled, answer)) => {
                    state.reused += usize::from(answer == Answer::Reused);
                    state.listed += usize::from(answer == Answer::Listed);
                    culled
                }
                Err(error) => {
                    state.error = Some(format!("{error:#}"));
                    false
                }
            }
        };
        if pvs.culled != culled {
            pvs.culled = culled;
        }
        state.culled += usize::from(culled);
        let desired = if scene_hidden || culled {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *draw != desired {
            *draw = desired;
        }
    }
    state.views = rows;
}

/// World-space bounds of a draw's current `Aabb`, as a Source-space center and half extent.
fn source_bounds(transform: &GlobalTransform, aabb: &Aabb) -> (glam::Vec3, glam::Vec3) {
    let affine = transform.affine();
    let center = affine.transform_point3a(aabb.center);
    let matrix = affine.matrix3;
    let extent = matrix.x_axis.abs() * aabb.half_extents.x
        + matrix.y_axis.abs() * aabb.half_extents.y
        + matrix.z_axis.abs() * aabb.half_extents.z;
    (
        glam::Vec3::from_array(crate::bevy_to_source(Vec3::from(center)).to_array()),
        glam::Vec3::from_array(crate::bevy_to_source(Vec3::from(extent)).abs().to_array()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    fn tree(front: i16) -> VisibilityIndex {
        let mut lumps = vec![vec![]; 64];
        lumps[1] = vec![0; 20];
        lumps[1][..4].copy_from_slice(&1f32.to_le_bytes());
        lumps[5] = vec![0; 32];
        lumps[5][4..8].copy_from_slice(&(-1i32).to_le_bytes());
        lumps[5][8..12].copy_from_slice(&(-2i32).to_le_bytes());
        lumps[10] = vec![0; 64];
        lumps[10][4..6].copy_from_slice(&front.to_le_bytes());
        lumps[10][36..38].copy_from_slice(&1i16.to_le_bytes());
        lumps[14] = vec![0; 48];
        lumps[4] = vec![0; 22];
        lumps[4][..4].copy_from_slice(&2i32.to_le_bytes());
        lumps[4][4..8].copy_from_slice(&20i32.to_le_bytes());
        lumps[4][12..16].copy_from_slice(&21i32.to_le_bytes());
        lumps[4][20] = 1;
        lumps[4][21] = 2;
        VisibilityIndex::new(&lumps, 1).unwrap()
    }
    #[test]
    fn active_view_union_uses_current_bounds_preserves_script_visibility_and_fails_open() {
        let mut app = App::new();
        let game = crate::gameplay::Gameplay::synthetic(modkit_core::World {
            entities: vec![modkit_core::Entity {
                properties: vec![("classname".into(), "prop_dynamic".into())],
            }],
            ..default()
        });
        app.insert_resource(game)
            .insert_resource(SourceVisibility::new(Ok(tree(0)), false));
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::from_translation(Vec3::X * 100.),
            RenderLayers::layer(0),
        ));
        let monitor = app
            .world_mut()
            .spawn((
                Camera {
                    is_active: false,
                    ..default()
                },
                GlobalTransform::from_translation(-Vec3::X * 100.),
                RenderLayers::layer(0),
            ))
            .id();
        let front = app
            .world_mut()
            .spawn((
                PvsDraw::default(),
                Visibility::Inherited,
                GlobalTransform::from_translation(Vec3::X * 10.),
                Aabb {
                    center: default(),
                    half_extents: Vec3::ONE.into(),
                },
            ))
            .id();
        let back = app
            .world_mut()
            .spawn((
                PvsDraw::default(),
                Visibility::Inherited,
                GlobalTransform::from_translation(-Vec3::X * 10.),
                Aabb {
                    center: default(),
                    half_extents: Vec3::ONE.into(),
                },
            ))
            .id();
        let boundary = app
            .world_mut()
            .spawn((
                PvsDraw::default(),
                Visibility::Inherited,
                GlobalTransform::IDENTITY,
                Aabb {
                    center: default(),
                    half_extents: Vec3::new(0., 1., 1.).into(),
                },
            ))
            .id();
        app.world_mut().run_system_once(present).unwrap();
        assert_eq!(
            *app.world().get::<Visibility>(front).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(
            *app.world().get::<Visibility>(back).unwrap(),
            Visibility::Hidden
        );
        assert_eq!(
            *app.world().get::<Visibility>(boundary).unwrap(),
            Visibility::Inherited
        );
        app.world_mut().run_system_once(present).unwrap();
        assert_eq!(app.world().resource::<SourceVisibility>().reused, 3);
        app.world_mut()
            .get_mut::<Camera>(monitor)
            .unwrap()
            .is_active = true;
        app.world_mut().run_system_once(present).unwrap();
        // New views, unchanged bounds: cached cluster lists instead of tree walks.
        assert_eq!(app.world().resource::<SourceVisibility>().reused, 0);
        assert_eq!(app.world().resource::<SourceVisibility>().listed, 3);
        assert_eq!(
            *app.world().get::<Visibility>(back).unwrap(),
            Visibility::Inherited
        );
        app.world_mut()
            .entity_mut(front)
            .insert(crate::rendering::SourceEntity(0));
        app.world_mut()
            .resource_mut::<crate::gameplay::Gameplay>()
            .scene
            .states[0]
            .visible = false;
        app.world_mut().run_system_once(present).unwrap();
        assert_eq!(
            *app.world().get::<Visibility>(front).unwrap(),
            Visibility::Hidden
        );
        // A moved current bound is visible immediately, with no prior ViewVisibility dependency.
        app.world_mut()
            .get_mut::<Camera>(monitor)
            .unwrap()
            .is_active = false;
        *app.world_mut().get_mut::<GlobalTransform>(back).unwrap() =
            GlobalTransform::from_translation(Vec3::X * 10.);
        app.world_mut().run_system_once(present).unwrap();
        assert_eq!(
            *app.world().get::<Visibility>(back).unwrap(),
            Visibility::Inherited
        );
        *app.world_mut().get_mut::<GlobalTransform>(back).unwrap() =
            GlobalTransform::from_translation(-Vec3::X * 10.);
        app.world_mut().resource_mut::<SourceVisibility>().index = Some(tree(-1));
        app.world_mut().run_system_once(present).unwrap();
        assert_eq!(
            *app.world().get::<Visibility>(back).unwrap(),
            Visibility::Inherited
        );
        assert_eq!(
            *app.world().get::<Visibility>(front).unwrap(),
            Visibility::Hidden
        );
        assert!(!app.world().get::<PvsDraw>(back).unwrap().culled);
    }
}
