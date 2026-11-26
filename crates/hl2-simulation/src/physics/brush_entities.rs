//! Which brush entities get colliders.

use super::{angles, brush_shape, pose, Physics, MASK_SOLID, PLAYER_BRUSH_MASK};
use crate::brush_solidity::solid;
use glam::Vec3;
use modkit_core::Trace;
use modkit_core::World;
use rapier3d::prelude::{ActiveHooks, ColliderBuilder};

mod pushers;

impl Physics {
    /// One collider per brush of each solid brush entity, at its spawn pose. A brush with no
    /// solid contents (clip only: the DETAIL|MONSTERCLIP func_brush around
    /// d1_trainstation_01's turnstile) is a clip brush like the world's: the player's
    /// movement applies MASK_PLAYERSOLID (no CONTENTS_MONSTERCLIP) and physics objects pass;
    /// NPC traces keep its contents. A func_physbox whose motion is enabled is one
    /// dynamic body instead (physbox.rs).
    pub(super) fn add_brush_entity_colliders(&mut self, world: &World) {
        for (id, e) in world.entities.iter().enumerate() {
            let model = e
                .get("model")
                .and_then(|m| m.strip_prefix('*'))
                .and_then(|s| s.parse::<usize>().ok())
                .and_then(|model| world.brush_models.iter().find(|m| m.id == model));
            let Some(model) = model.filter(|_| solid(e)) else {
                continue;
            };
            let spawn = modkit_core::parse_vec3(e.get("angles").unwrap_or("0 0 0"));
            let position = pose(e.origin(), angles(spawn.unwrap_or(Vec3::ZERO)));
            if self.add_physbox(world, id, e, model, position) {
                continue;
            }
            for b in &model.brushes {
                let Some(shape) = brush_shape(b) else {
                    continue;
                };
                let clip = b.contents & MASK_SOLID == 0;
                let builder = ColliderBuilder::new(shape)
                    .position(position)
                    .user_data(id as u128 + 1)
                    .friction(0.8);
                let c = self.colliders.insert(if clip {
                    builder.active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS)
                } else {
                    builder
                });
                if clip {
                    self.world_sides.clip_brushes.insert(c);
                    self.world_brush_contents.insert(c, b.contents);
                    if b.contents & PLAYER_BRUSH_MASK != 0 {
                        self.entity_player_clips.push(c);
                    }
                }
                self.entity_colliders.entry(id).or_default().push(c);
                self.npc_collider_contents.insert(c, b.contents);
            }
        }
    }

    /// Sweep the player hull against brush entities' player clips (CONTENTS_PLAYERCLIP
    /// without solid contents, MASK_PLAYERSOLID): d1_trainstation_01's func_tracktrain
    /// carries DETAIL|PLAYERCLIP blocks over its seats (the seat props are solid 0).
    /// Static world clips are traced from the BSP's planes; these move with their
    /// entity, so their planes come from the collider's current pose.
    pub(super) fn trace_entity_player_clips(
        &self,
        mut cache: std::sync::MutexGuard<'_, super::player_convex::ConvexCache>,
        sweep: (Vec3, Vec3, Vec3, Vec3),
        result: &mut Trace,
    ) {
        let (start, end, mins, maxs) = sweep;
        let low = start.min(end) + mins - Vec3::ONE;
        let high = start.max(end) + maxs + Vec3::ONE;
        for &handle in &self.entity_player_clips {
            let Some(collider) = self.colliders.get(handle).filter(|c| c.is_enabled()) else {
                continue;
            };
            let aabb = collider.compute_aabb();
            let (lo, hi) = (
                aabb.mins / super::SCALE,
                aabb.maxs / super::SCALE,
            );
            if lo.x > high.x || lo.y > high.y || lo.z > high.z {
                continue;
            }
            if hi.x < low.x || hi.y < low.y || hi.z < low.z {
                continue;
            }
            let Some(brush) = cache.brush(handle, collider) else {
                continue;
            };
            let hit =
                modkit_core::trace_brushes(std::slice::from_ref(brush), start, end, mins, maxs, 1);
            result.start_solid |= hit.start_solid;
            result.all_solid |= hit.all_solid;
            if hit.fraction < result.fraction {
                result.fraction = hit.fraction;
                result.normal = hit.normal;
            }
        }
    }
}
