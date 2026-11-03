//! Optional profiling sink. No collection or timing work when the host omits it.
use super::BlurMemory;
use std::{sync::Arc, time::Instant};

pub trait Reporter: Send + Sync + 'static {
    fn memory(&self, report: BlurMemory);
    fn reduction(&self, world: bool, processed: u64, full: u64);
    fn stage(&self, name: &'static str, milliseconds: f64);
}

#[derive(bevy::prelude::Resource, Clone)]
pub struct Diagnostics(pub Arc<dyn Reporter>);
impl Diagnostics {
    pub fn record_blur_memory(&self, report: BlurMemory) {
        self.0.memory(report);
    }
    pub fn record_reduction(&self, world: bool, processed: u64, full: u64) {
        self.0.reduction(world, processed, full);
    }
    pub fn scope(&self, name: &'static str) -> Scope<'_> {
        Scope {
            sink: self,
            name,
            start: Instant::now(),
        }
    }
}
pub struct Scope<'a> {
    sink: &'a Diagnostics,
    name: &'static str,
    start: Instant,
}
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.sink
            .0
            .stage(self.name, self.start.elapsed().as_secs_f64() * 1000.);
    }
}
