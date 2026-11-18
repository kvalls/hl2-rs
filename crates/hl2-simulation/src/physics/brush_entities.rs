//! Which brush entities get colliders.

use super::{angles, brush_shape, pose, Physics, MASK_SOLID};
use glam::Vec3;
use modkit_core::{Entity, World};
use rapier3d::prelude::{ActiveHooks, ColliderBuilder};

/// A solid brush entity: one of the solid classes, not made non-solid by its keys
/// (`solid 0`, func_brush `Solidity 1`, a func_tracktrain's SF_TRACKTRAIN_PASSABLE).
pub(super) fn solid(e: &Entity) -> bool {
    let class = matches!(
        e.class(),
        "func_brush"
            | "func_wall"
            | "func_wall_toggle"
            | "func_door"
            | "func_door_rotating"
            | "func_movelinear"
            | "func_rotating"
            | "func_breakable"
            | "func_physbox"
            | "func_detail"
            | "func_tracktrain"
    );
    let passable_train = e.class() == "func_tracktrain"
        && e.get("spawnflags")
            .and_then(|f| f.trim().parse::<u32>().ok())
            .unwrap_or(0)
            & crate::trains::PASSABLE
            != 0;
    class && !passable_train && e.get("solid") != Some("0") && e.get("Solidity") != Some("1")
}

impl Physics {
    /// One collider per brush of each solid brush entity, at its spawn pose. A brush with no
    /// solid contents (clip only: the DETAIL|MONSTERCLIP func_brush around
    /// d1_trainstation_01's turnstile) is a clip brush like the world's: the player's
    /// movement applies MASK_PLAYERSOLID (no CONTENTS_MONSTERCLIP) and physics objects pass;
    /// NPC traces keep its contents.
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
                }
                self.entity_colliders.entry(id).or_default().push(c);
                self.npc_collider_contents.insert(c, b.contents);
            }
        }
    }
}
