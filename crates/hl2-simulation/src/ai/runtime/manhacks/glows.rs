//! Manhack light attachments: resolve model instances once for the eligible actors.
use super::{manhack, AiClass, AiRuntime, Scene, Vec3, World};
use modkit_core::ModelInstance;

impl AiRuntime {
    /// The living manhacks' eye and light at the current pose. A stunned eye strobes.
    pub fn manhack_glows<'a>(
        &'a self,
        world: &'a World,
        scene: &'a Scene,
    ) -> impl Iterator<Item = (Vec3, bool)> + 'a {
        let mut eligible = self.actors.iter().filter_map(|(&id, actor)| {
            let AiClass::Manhack(m) = &actor.class else {
                return None;
            };
            let state = scene.states.get(id)?;
            (state.visible
                && !state.killed
                && !state.corpse
                && (!m.stunned || manhack::strobe_lit(scene.time, id)))
            .then_some((id, m.stunned))
        });
        let first = eligible.next();
        let second = eligible.next();
        // A singleton needs only the original lookup, without a temporary allocation.
        let solo = first.filter(|_| second.is_none()).map(|(id, stunned)| {
            (
                id,
                stunned,
                world.model_instances.iter().find(|i| i.entity == Some(id)),
            )
        });
        let mut actors: Vec<(usize, bool, Option<&ModelInstance>)> = if second.is_some() {
            first
                .into_iter()
                .chain(second)
                .chain(eligible)
                .map(|(id, stunned)| (id, stunned, None))
                .collect()
        } else {
            Vec::new()
        };
        let mut remaining = actors.len();
        if remaining != 0 {
            // Actor IDs are sorted by their owning BTreeMap. Keep the first instance,
            // exactly as attachment_frames does, without persistent-index invalidation.
            for instance in &world.model_instances {
                let Some(id) = instance.entity else {
                    continue;
                };
                if let Ok(i) = actors.binary_search_by_key(&id, |&(id, _, _)| id) {
                    if actors[i].2.is_none() {
                        actors[i].2 = Some(instance);
                        remaining -= 1;
                        if remaining == 0 {
                            break;
                        }
                    }
                }
            }
        }
        solo.into_iter()
            .chain(actors)
            .flat_map(move |(id, stunned, instance)| {
                instance
                    .map(|instance| {
                        scene.attachment_frames_for_instance(
                            world,
                            id,
                            &manhack::GLOW_ATTACHMENTS,
                            instance,
                        )
                    })
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(move |frame| (frame.w_axis.truncate(), stunned))
            })
    }
}

#[cfg(test)]
mod tests;
