//! Which brush entities get colliders.

use super::{angles, brush_shape, pose, Physics, MASK_SOLID, PLAYER_BRUSH_MASK};
use glam::Vec3;
use modkit_core::Trace;
use modkit_core::{Entity, World};
use rapier3d::prelude::{ActiveHooks, ColliderBuilder};

/// A solid brush entity: one of the solid classes, not made non-solid by its keys
/// (`solid 0`, func_brush `Solidity 1`, a func_tracktrain's SF_TRACKTRAIN_PASSABLE).
pub(super) fn solid(e: &Entity) -> bool {
    // func_lod (SDK func_lod.cpp CFunc_LOD): SOLID_BSP, and its "Solid" key is inverted:
    // any non-zero value adds FSOLID_NOT_SOLID (d1_trainstation_01's lod_turnstyle_frame,
    // Solid 0, is the turnstile's solid frame).
    if e.class() == "func_lod" {
        let value = e.get("Solid").map_or("0", str::trim);
        let digits = value.trim_start_matches(['-', '+']);
        let end = digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(digits.len());
        return digits[..end].trim_start_matches('0').is_empty();
    }
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
                aabb.mins.coords / super::SCALE,
                aabb.maxs.coords / super::SCALE,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn func_lod_is_solid_unless_its_inverted_solid_key_is_set() {
        let lod = |solid: Option<&str>| {
            let mut pairs = vec![("classname", "func_lod")];
            pairs.extend(solid.map(|v| ("Solid", v)));
            super::solid(&entity(&pairs))
        };
        assert!(lod(Some("0")));
        assert!(lod(None));
        assert!(!lod(Some("1")));
        assert!(!lod(Some("2 ")));
        // The generic keys still apply to the other classes.
        assert!(!super::solid(&entity(&[
            ("classname", "func_brush"),
            ("solid", "0")
        ])));
        assert!(super::solid(&entity(&[("classname", "func_wall")])));
    }
}
