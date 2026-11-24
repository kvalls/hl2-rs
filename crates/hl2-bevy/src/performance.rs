//! Optional bounded CPU stage measurements; disabled for ordinary play.
mod frame_stages;
mod gpu_completion;
mod pacing;
pub mod status;
#[cfg(feature = "trace")]
pub(crate) mod trace_writer;
use bevy::prelude::*;
pub(crate) use pacing::background_interval;
/// Samples kept per stage; later frames only count calls.
pub(crate) const SAMPLE_LIMIT: usize = 4096;
/// The whole-frame stage (`frame_stages`).
pub(crate) const FRAME_TOTAL: &str = "frame_total";
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
fn push_sample(all: &mut BTreeMap<&'static str, Samples>, name: &'static str, ms: f64) {
    let samples = all.entry(name).or_default();
    samples.calls += 1;
    if samples.calls > 30 && samples.values.len() < SAMPLE_LIMIT {
        samples.values.push(ms);
    }
}
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        if let Some(performance) = self.performance {
            performance.record_sample(self.name, self.start.elapsed().as_secs_f64() * 1000.);
        }
    }
}
impl Performance {
    pub(crate) fn set_camera_drawing(&self, direct: bool) {
        if let Some(stats) = self.1.lock().expect("GPU profiling state").as_mut() {
            stats.camera_indirect_drawing_disabled = direct;
        }
    }
    fn record_sample(&self, name: &'static str, ms: f64) {
        let mut all = self.0.lock().expect("CPU timing");
        push_sample(&mut all, name, ms);
    }
    pub fn record_ms(&self, name: &'static str, ms: f64) {
        self.record_sample(name, ms);
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
            // The slowest samples with their sample index (frame hitches, e.g. a map copy).
            let mut slowest:Vec<(usize,f64)>=samples.values.iter().copied().enumerate().collect();
            slowest.sort_by(|a,b| b.1.total_cmp(&a.1));slowest.truncate(3);
            ((*name).into(),serde_json::json!({"calls":samples.calls,"samples":sorted.len(),"mean_ms":mean,"p95_ms":p95,"max_ms":sorted.last().copied().unwrap_or(0.),"slowest":slowest}))
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
    frame_stages::install(app);
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
            "render_create_views",
            RenderSystems::PrepareMeshes,
            RenderSystems::CreateViews,
            RenderSystems::Specialize,
        ),
        (
            "render_specialize",
            RenderSystems::CreateViews,
            RenderSystems::Specialize,
            RenderSystems::PrepareViews,
        ),
        // Only `PrepareViews` (view targets, lights, clusters); the name is kept so
        // earlier recorded measurements stay comparable.
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
    fence_prepare_stages(render);
}

/// Timings of the stages inside `Prepare`.
fn fence_prepare_stages(render: &mut bevy::app::SubApp) {
    use bevy::render::{Render, RenderSystems};
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

impl Performance {
    pub fn record_contribution(&self, name: &'static str, value: serde_json::Value) {
        if let Some(stats) = self.1.lock().expect("GPU profiling state").as_mut() {
            stats.contributions.insert(name, value);
        }
    }
    pub fn record_reduction(&self, world: bool, processed: u64, full: u64) {
        if let Some(stats) = self.1.lock().expect("GPU profiling state").as_mut() {
            stats.reduction(world, processed, full);
        }
    }
    pub fn record_stage(&self, name: &'static str, milliseconds: f64) {
        self.record_stages([(name, milliseconds)]);
    }
    fn record_stages(&self, stages: impl IntoIterator<Item = (&'static str, f64)>) {
        let mut all = self.0.lock().expect("CPU timing");
        for (name, milliseconds) in stages {
            push_sample(&mut all, name, milliseconds);
        }
    }
}

impl Performance {
    /// Drops the stage samples collected so far (menu, loading and warm-up frames).
    pub(crate) fn restart_stages(&self) {
        self.0.lock().expect("CPU timing").clear();
    }
    /// Samples kept so far for one stage.
    pub(crate) fn recorded(&self, name: &'static str) -> usize {
        self.0
            .lock()
            .expect("CPU timing")
            .get(name)
            .map_or(0, |samples| samples.values.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_ms_skips_first_30_calls_and_records_from_31() {
        let perf = Performance::default();
        for i in 1..=35 {
            perf.record_ms("test_stage", f64::from(i));
        }
        assert_eq!(perf.recorded("test_stage"), 5);
        let report = perf.report();
        assert_eq!(report["test_stage"]["calls"], 35);
        assert_eq!(report["test_stage"]["samples"], 5);
        assert_eq!(report["test_stage"]["mean_ms"], 33.0);
        assert_eq!(report["test_stage"]["max_ms"], 35.0);
    }
}
