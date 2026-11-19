//! A map's unique studio models, read on all cores before its instances are placed.
use super::{read_model_levels, ModelLevels};
use crate::vpk::Vfs;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A model as instances ask for it: lower-case path, skin and body.
pub(super) type ModelKey = (String, usize, i32);

/// Each key's levels, or why it could not be read, in `keys` order.
pub(super) fn read_all(
    vfs: &Vfs,
    keys: &[ModelKey],
    root_lod: usize,
) -> Vec<Result<ModelLevels, String>> {
    parallel_map(keys, |(path, skin, body)| {
        read_one(vfs, path, (*skin, *body), root_lod)
    })
}

/// `work` on every item, on all cores, results in `items` order. Workers take the next item as
/// they finish one, so a few large items do not hold up the rest; a panic in `work` resumes on
/// the calling thread.
pub(super) fn parallel_map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(items.len())
        .max(1);
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else { break };
                        done.push((index, work(item)));
                    }
                    done
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, result)| result).collect()
}

/// One model's levels; a reader panic becomes an error for that model alone.
fn read_one(
    vfs: &Vfs,
    path: &str,
    (skin, body): (usize, i32),
    root_lod: usize,
) -> Result<ModelLevels, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        read_model_levels(vfs, path, skin, body, root_lod)
    })) {
        Ok(Ok(levels)) => Ok(levels),
        Ok(Err(error)) => Err(format!("{error:#}")),
        Err(_) => Err("model reader panicked".into()),
    }
}
