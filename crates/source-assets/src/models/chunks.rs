//! Which propdata chunk models a map's breakable props can break into.
use modkit_core::World;

/// Chunk models of the propdata chunk sets that breakable props without .phy gibs use
/// (PropBreakableCreateAll's else-branch; ParsePropDataFile precaches every chunk model).
pub fn used_chunk_models(world: &World) -> Vec<String> {
    let mut models = Vec::new();
    for (key, data) in &world.model_prop_data {
        if data.health <= 0. || data.breakable_count <= 0 || world.model_breaks.contains_key(key) {
            continue;
        }
        let Some(name) = &data.breakable_model else {
            continue;
        };
        // GetRandomChunkModel: the first set whose name starts with the requested one.
        if let Some((_, set)) = world
            .breakable_chunks
            .iter()
            .find(|(set, _)| set.starts_with(name.as_str()))
        {
            models.extend(set.iter().cloned());
        }
    }
    models
}
