//! A map's unique studio models, read on all cores before its instances are placed.
use super::{read_model_levels, ModelLevels};
use crate::vpk::Vfs;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A model as instances ask for it: lower-case path, skin and body.
pub(super) type ModelKey = (String, usize, i32);

/// Each key's levels, or why it could not be read, in `keys` order. Workers take the next key
/// as they finish one, so a few large models do not hold up the rest.
pub(super) fn read_all(
    vfs: &Vfs,
    keys: &[ModelKey],
    root_lod: usize,
) -> Vec<Result<ModelLevels, String>> {
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(keys.len())
        .max(1);
    let mut read: Vec<(usize, Result<ModelLevels, String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some((path, skin, body)) = keys.get(index) else {
                            break done;
                        };
                        done.push((index, read_one(vfs, path, (*skin, *body), root_lod)));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    read.sort_by_key(|(index, _)| *index);
    read.into_iter().map(|(_, result)| result).collect()
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
