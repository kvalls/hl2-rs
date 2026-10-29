//! Optional bounded CPU stage measurements; disabled for ordinary play.
mod gpu_completion;
mod pacing;
use bevy::prelude::*;
pub(crate) use pacing::background_interval;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
#[derive(Resource, Default, Clone)]
pub struct Performance(
    Arc<Mutex<BTreeMap<&'static str, Samples>>>,
    Arc<Mutex<Option<gpu_completion::Stats>>>,
);
#[derive(Default)]
struct Samples {
    calls: u64,
    values: Vec<f64>,
}
pub struct Scope<'a> {
    performance: Option<&'a Performance>,
    name: &'static str,
    start: Instant,
}
pub fn scope<'a>(performance: Option<&'a Performance>, name: &'static str) -> Scope<'a> {
    Scope {
        performance,
        name,
        start: Instant::now(),
    }
}
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        let Some(performance) = self.performance else {
            return;
        };
        let mut all = performance.0.lock().expect("CPU timing");
        let samples = all.entry(self.name).or_default();
        samples.calls += 1;
        if samples.calls > 30 && samples.values.len() < 4096 {
            samples
                .values
                .push(self.start.elapsed().as_secs_f64() * 1000.);
        }
    }
}
impl Performance {
    pub(crate) fn record_reduction(&self, world: bool, processed: u64, full: u64) {
        if let Some(stats) = self.1.lock().expect("GPU profiling state").as_mut() {
            stats.reduction(world, processed, full);
        }
    }
    pub(crate) fn set_camera_drawing(&self, direct: bool) {
        if let Some(stats) = self.1.lock().expect("GPU profiling state").as_mut() {
            stats.camera_indirect_drawing_disabled = direct;
        }
    }
    pub fn write_report(
        &self,
        output: &mut serde_json::Value,
        diagnostics: Option<&bevy::diagnostic::DiagnosticsStore>,
    ) {
        output["cpu_stages"] = self.report();
        output["background_profile_interval_ms"] = self
            .1
            .lock()
            .expect("GPU profiling state")
            .as_ref()
            .map(|_| background_interval(true).as_secs_f64() * 1000.)
            .into();
        output["gpu_completion"] =
            serde_json::to_value(&*self.1.lock().expect("GPU profiling state"))
                .expect("GPU profiling report");
        if let Some(diagnostics) = diagnostics {
            output["render_diagnostics"] = render_report(diagnostics);
        }
    }
    pub fn report(&self) -> serde_json::Value {
        let all = self.0.lock().expect("CPU timing");
        serde_json::Value::Object(all.iter().map(|(name,samples)| {
            let mut sorted=samples.values.clone();sorted.sort_by(f64::total_cmp);
            let mean=sorted.iter().sum::<f64>()/sorted.len().max(1) as f64;
            let p95=sorted.get(sorted.len().saturating_sub(1)*95/100).copied().unwrap_or(0.);
            ((*name).into(),serde_json::json!({"calls":samples.calls,"samples":sorted.len(),"mean_ms":mean,"p95_ms":p95}))
        }).collect())
    }
}

/// Bevy's render CPU/GPU timings and pipeline counters, with their original units.
pub fn render_report(diagnostics: &bevy::diagnostic::DiagnosticsStore) -> serde_json::Value {
    serde_json::Value::Object(
        diagnostics
            .iter()
            .map(|d| {
                (
                    d.path().to_string(),
                    serde_json::json!({"mean":d.average(),"latest":d.value(),"unit":d.suffix}),
                )
            })
            .collect(),
    )
}

#[derive(Resource, Default)]
struct RenderTimers(BTreeMap<&'static str, Instant>);
/// Optional render-stage CPU wall timings. Ordering fences exist only with --profile.
pub fn install(app: &mut App) {
    use bevy::render::{Render, RenderApp, RenderSystems};
    let performance = gpu_completion::prepare(app);
    app.insert_resource(performance.clone())
        .add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .insert_resource(performance)
        .init_resource::<RenderTimers>();
    for (name, previous, set, next) in [
        (
            "render_assets",
            RenderSystems::ExtractCommands,
            RenderSystems::PrepareAssets,
            RenderSystems::PrepareMeshes,
        ),
        (
            "render_meshes",
            RenderSystems::PrepareAssets,
            RenderSystems::PrepareMeshes,
            RenderSystems::CreateViews,
        ),
        (
            "render_views",
            RenderSystems::Specialize,
            RenderSystems::PrepareViews,
            RenderSystems::Queue,
        ),
        (
            "render_queue",
            RenderSystems::PrepareViews,
            RenderSystems::Queue,
            RenderSystems::PhaseSort,
        ),
        (
            "render_prepare",
            RenderSystems::PhaseSort,
            RenderSystems::Prepare,
            RenderSystems::Render,
        ),
        (
            "render_graph",
            RenderSystems::Prepare,
            RenderSystems::Render,
            RenderSystems::Cleanup,
        ),
    ] {
        render.add_systems(
            Render,
            (
                (move |mut timers: ResMut<RenderTimers>| {
                    timers.0.insert(name, Instant::now());
                })
                .after(previous)
                .before(set.clone()),
                (move |mut timers: ResMut<RenderTimers>, performance: Res<Performance>| {
                    if let Some(start) = timers.0.remove(name) {
                        drop(Scope {
                            performance: Some(&performance),
                            name,
                            start,
                        });
                    }
                })
                .after(set)
                .before(next),
            ),
        );
    }
    // Subsets of Prepare are fenced inside their parent, so these intervals
    // measure only their own stage rather than accumulating previous work.
    for (name, previous, set, next) in [
        (
            "render_resource_setup",
            None,
            RenderSystems::PrepareResources,
            Some(RenderSystems::PrepareResourcesBatchPhases),
        ),
        (
            "render_batch",
            Some(RenderSystems::PrepareResources),
            RenderSystems::PrepareResourcesBatchPhases,
            Some(RenderSystems::PrepareResourcesWritePhaseBuffers),
        ),
        (
            "render_write",
            Some(RenderSystems::PrepareResourcesBatchPhases),
            RenderSystems::PrepareResourcesWritePhaseBuffers,
            Some(RenderSystems::PrepareResourcesCollectPhaseBuffers),
        ),
        (
            "render_collect",
            Some(RenderSystems::PrepareResourcesWritePhaseBuffers),
            RenderSystems::PrepareResourcesCollectPhaseBuffers,
            Some(RenderSystems::PrepareResourcesFlush),
        ),
        (
            "render_flush",
            Some(RenderSystems::PrepareResourcesCollectPhaseBuffers),
            RenderSystems::PrepareResourcesFlush,
            Some(RenderSystems::PrepareBindGroups),
        ),
        (
            "render_bindgroups",
            Some(RenderSystems::PrepareResourcesFlush),
            RenderSystems::PrepareBindGroups,
            None,
        ),
    ] {
        let mut begin = (move |mut timers: ResMut<RenderTimers>| {
            timers.0.insert(name, Instant::now());
        })
        .in_set(RenderSystems::Prepare)
        .before(set.clone());
        if let Some(previous) = previous {
            begin = begin.after(previous);
        }
        let mut end = (move |mut timers: ResMut<RenderTimers>, performance: Res<Performance>| {
            if let Some(start) = timers.0.remove(name) {
                drop(Scope {
                    performance: Some(&performance),
                    name,
                    start,
                });
            }
        })
        .in_set(RenderSystems::Prepare)
        .after(set);
        if let Some(next) = next {
            end = end.before(next);
        }
        render.add_systems(Render, (begin, end));
    }
}
/// Draw candidates and their triangles, all and visible in any view.
pub fn render_candidates<'a>(
    geometry: impl Iterator<
        Item = (
            &'a bevy::camera::visibility::ViewVisibility,
            &'a crate::rendering::DrawTriangles,
        ),
    >,
) -> serde_json::Value {
    let mut visible_meshes = 0;
    let mut visible_triangles = 0;
    let mut total_triangles = 0;
    let mut meshes = 0;
    for (visible, triangles) in geometry {
        meshes += 1;
        total_triangles += triangles.0;
        if visible.get() {
            visible_meshes += 1;
            visible_triangles += triangles.0;
        }
    }
    serde_json::json!({
        "tagged_meshes":meshes,"tagged_triangles":total_triangles,
        "visible_any_view_meshes":visible_meshes,"visible_any_view_triangles":visible_triangles,
        "scope":"world/entity candidates after PVS/frustum/layer checks; excludes sky and viewmodels; union of all views, not GPU draw-call counts"
    })
}
