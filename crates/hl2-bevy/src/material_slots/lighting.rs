//! Model lighting in the shared rows buffer: one slot of [`ROWS`] rows per lit entity (and one for
//! the viewmodel), shared by all of its materials. A material's first lighting is written into
//! the material as before; when its lighting changes again (the model moves through the light
//! cache, an entity light reaches it, the viewmodel follows the view), the material changes once
//! to name its entity's slot and later changes only rewrite the slot's rows. Static models never
//! take a slot. Out of slots, a material keeps the asset path.
use super::MaterialSlots;
use crate::rendering::{ModelLighting, SourceMaterial};
use bevy::{
    math::{Mat4, Vec4},
    platform::collections::HashMap,
    prelude::{AssetId, Assets, Entity, Handle},
};

/// Lit entities the buffer holds.
pub(super) const SLOTS: usize = 1024;
/// A [`ModelLighting`] as rows: params, the basis columns, the six ambient faces, then the four
/// lights' positions, colours, directions, attenuations and spot terms (as in `source.wesl`).
pub(super) const ROWS: usize = 31;

/// A lit draw's owner: its entity (`None` for the viewmodel) and whether it is half-Lambert.
pub type LightKey = (Option<usize>, bool);

#[derive(Default)]
pub(super) struct LightSlots {
    by_key: HashMap<LightKey, u32>,
    by_material: HashMap<AssetId<SourceMaterial>, u32>,
    by_draw: HashMap<Entity, u32>,
    /// Each slot's owner and how many materials or tagged draws draw from it.
    users: HashMap<u32, (LightKey, u32)>,
    free: Vec<u32>,
    next: u32,
}
impl LightSlots {
    /// A removed material stops drawing from its slot; the last user frees it.
    pub(super) fn release(&mut self, id: AssetId<SourceMaterial>) {
        if let Some(slot) = self.by_material.remove(&id) {
            self.release_slot(slot);
        }
    }
    /// A despawned lit draw stops drawing from its slot; the last user frees it.
    pub(super) fn release_draw(&mut self, draw: Entity) {
        if let Some(slot) = self.by_draw.remove(&draw) {
            self.release_slot(slot);
        }
    }
    fn release_slot(&mut self, slot: u32) {
        if let Some((key, count)) = self.users.get_mut(&slot) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                let key = *key;
                self.users.remove(&slot);
                self.by_key.remove(&key);
                self.free.push(slot);
            }
        }
    }
    /// Whether `key` already has a lighting slot or a slot is still available for it.
    pub(super) fn has_slot_for(&self, key: LightKey) -> bool {
        self.by_key.contains_key(&key) || !self.free.is_empty() || (self.next as usize) < SLOTS
    }
    /// Drops all lighting-slot assignments when installing a new map.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
    fn slot_for_with_status(&mut self, key: LightKey) -> Option<(u32, bool)> {
        if let Some(&slot) = self.by_key.get(&key) {
            return Some((slot, false));
        }
        let slot = self.free.pop().or_else(|| {
            (self.next < SLOTS as u32).then(|| {
                self.next += 1;
                self.next - 1
            })
        })?;
        self.by_key.insert(key, slot);
        self.users.insert(slot, (key, 0));
        Some((slot, true))
    }
    fn slot_for(&mut self, key: LightKey) -> Option<u32> {
        self.slot_for_with_status(key).map(|(slot, _)| slot)
    }
}

/// The first row of lighting slot `slot` in [`super::LIGHT_ROWS`].
fn start(slot: u32) -> usize {
    slot as usize * ROWS
}

fn rows_of(l: &ModelLighting) -> [Vec4; ROWS] {
    let mut rows = [Vec4::ZERO; ROWS];
    rows[0] = l.params;
    rows[1..5].copy_from_slice(&l.basis.to_cols_array_2d().map(Vec4::from_array));
    rows[5..11].copy_from_slice(&l.ambient);
    for (i, group) in [l.position, l.color, l.direction, l.attenuation, l.spot]
        .iter()
        .enumerate()
    {
        rows[11 + i * 4..15 + i * 4].copy_from_slice(group);
    }
    rows
}

fn from_rows(rows: &[Vec4]) -> ModelLighting {
    let four = |at: usize| [rows[at], rows[at + 1], rows[at + 2], rows[at + 3]];
    ModelLighting {
        params: rows[0],
        basis: Mat4::from_cols(rows[1], rows[2], rows[3], rows[4]),
        ambient: [rows[5], rows[6], rows[7], rows[8], rows[9], rows[10]],
        position: four(11),
        color: four(15),
        direction: four(19),
        attenuation: four(23),
        spot: four(27),
    }
}

impl MaterialSlots {
    /// The lighting a material draws with: its slot's when it has one, else its own.
    pub fn lighting(
        &self,
        material: &SourceMaterial,
        id: AssetId<SourceMaterial>,
    ) -> ModelLighting {
        match self.lights.by_material.get(&id) {
            Some(&slot) => from_rows(&self.light_rows[start(slot)..][..ROWS]),
            None => material.lighting,
        }
    }
    /// The lighting a draw with `MeshTag::new(tag)` draws with: its tag's slot when `tag > 0`, else the material's.
    pub fn draw_lighting(
        &self,
        tag: u32,
        material: &SourceMaterial,
        id: AssetId<SourceMaterial>,
    ) -> ModelLighting {
        if tag > 0 && ((tag - 1) as usize) < SLOTS {
            from_rows(&self.light_rows[start(tag - 1)..][..ROWS])
        } else {
            self.lighting(material, id)
        }
    }
    /// Whether a draw of `key` can take a shared per-entity lighting slot without mutating its
    /// material asset.
    pub fn has_slot_for(&self, key: LightKey) -> bool {
        self.lights.has_slot_for(key)
    }
    /// Drops per-entity lighting slot assignments before spawning a newly loaded map.
    pub fn clear_lights(&mut self) {
        self.lights.clear();
    }
    /// Allocates or reuses a per-entity lighting slot for a tagged draw `draw` of `key`,
    /// returning its `MeshTag` value (`slot + 1`). Newly created slots start with fallback lighting
    /// so tagged draws never sample uninitialized rows before `present_lighting` writes them.
    pub fn assign_draw_tag(&mut self, draw: Entity, key: LightKey) -> Option<u32> {
        if let Some(&slot) = self.lights.by_draw.get(&draw) {
            return Some(slot + 1);
        }
        let (slot, fresh) = self.lights.slot_for_with_status(key)?;
        self.lights.by_draw.insert(draw, slot);
        if let Some((_, count)) = self.lights.users.get_mut(&slot) {
            *count += 1;
        }
        if fresh {
            self.set_light_rows(start(slot), &rows_of(&ModelLighting::fallback(key.1)));
        }
        Some(slot + 1)
    }
    /// Writes `lighting` into the slot named by `MeshTag::new(tag)` (`tag > 0`).
    pub fn write_tagged_lighting(&mut self, tag: u32, lighting: ModelLighting) {
        if tag > 0 && ((tag - 1) as usize) < SLOTS {
            self.set_light_rows(start(tag - 1), &rows_of(&lighting));
        }
    }
    /// Sets the lighting `handle`'s material draws with (the caller found it changed), shared
    /// by the materials of `key` once they have a slot.
    pub fn write_lighting(
        &mut self,
        materials: &mut Assets<SourceMaterial>,
        handle: &Handle<SourceMaterial>,
        key: LightKey,
        lighting: ModelLighting,
    ) {
        let slot = match self.lights.by_material.get(&handle.id()) {
            Some(&slot) => Some(slot),
            // Never lit yet (a lit uniform has params.x = 1): the first lighting stays in the
            // material, so models that never relight take no slot.
            None if materials
                .get(handle)
                .is_none_or(|m| m.lighting.params.x < 0.5) =>
            {
                None
            }
            None => self.assign_lighting(materials, handle.id(), key),
        };
        match slot {
            Some(slot) => self.set_light_rows(start(slot), &rows_of(&lighting)),
            None => {
                if let Some(mut material) = materials.get_mut(handle) {
                    material.lighting = lighting;
                }
            }
        }
    }
    fn assign_lighting(
        &mut self,
        materials: &mut Assets<SourceMaterial>,
        id: AssetId<SourceMaterial>,
        key: LightKey,
    ) -> Option<u32> {
        materials.get(id)?;
        let slot = self.lights.slot_for(key)?;
        let mut material = materials.get_mut(id)?;
        material.dynamic.z = slot as f32;
        self.lights.by_material.insert(id, slot);
        if let Some((_, count)) = self.lights.users.get_mut(&slot) {
            *count += 1;
        }
        Some(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::{LightKey, SLOTS, from_rows, rows_of};
    use crate::material_slots::{MaterialSlots, test_material};
    use crate::rendering::{ModelLighting, SourceMaterial};
    use bevy::{
        math::{Mat4, Vec3, Vec4},
        prelude::Assets,
    };

    fn lit(level: f32) -> ModelLighting {
        ModelLighting {
            params: Vec4::new(1., 2., 0., 0.),
            basis: Mat4::from_translation(Vec3::new(level, 2., 3.)),
            ambient: [Vec4::splat(level); 6],
            position: [Vec4::new(1., 2., 3., 0.); 4],
            color: [Vec4::new(level, 0.5, 0.25, 0.); 4],
            direction: [Vec4::Y; 4],
            attenuation: [Vec4::new(0., 1., 0., 0.); 4],
            spot: [Vec4::new(1., 0.5, 2., 0.); 4],
        }
    }

    #[test]
    fn rows_round_trip_every_lighting_field() {
        let l = lit(0.75);
        assert_eq!(from_rows(&rows_of(&l)), l);
    }

    #[test]
    fn an_entity_relit_moves_to_one_shared_slot_and_static_models_take_none() {
        let mut assets = Assets::<SourceMaterial>::default();
        let mut slots = MaterialSlots::default();
        let head = assets.add(test_material());
        let body = assets.add(test_material());
        let prop = assets.add(test_material());
        let key: LightKey = (Some(7), false);
        // First lighting: written into each material, no slot (a static prop stays so).
        for handle in [&head, &body, &prop] {
            slots.write_lighting(&mut assets, handle, key, lit(0.1));
            assert_eq!(assets.get(handle).unwrap().lighting, lit(0.1));
            assert_eq!(assets.get(handle).unwrap().dynamic.z, -1.);
        }
        // Relit: both of the entity's materials name one slot; the materials keep their old
        // lighting and draw the slot's.
        slots.write_lighting(&mut assets, &head, key, lit(0.2));
        slots.write_lighting(&mut assets, &body, key, lit(0.2));
        let (h, b) = (assets.get(&head).unwrap(), assets.get(&body).unwrap());
        assert_eq!(h.dynamic.z, 0.);
        assert_eq!(b.dynamic.z, 0.);
        assert_eq!(h.lighting, lit(0.1));
        assert_eq!(slots.lighting(h, head.id()), lit(0.2));
        // Later changes touch only the rows.
        slots.write_lighting(&mut assets, &head, key, lit(0.3));
        assert_eq!(assets.get(&head).unwrap().dynamic.z, 0.);
        assert_eq!(
            slots.lighting(assets.get(&body).unwrap(), body.id()),
            lit(0.3)
        );
        assert_eq!(
            slots.lighting(assets.get(&prop).unwrap(), prop.id()),
            lit(0.1)
        );
        // Removing both materials frees the slot for another entity.
        slots.lights.release(head.id());
        slots.lights.release(body.id());
        assert_eq!(slots.lights.slot_for((Some(8), true)), Some(0));
    }

    #[test]
    fn without_free_slots_a_relit_material_keeps_the_asset_path() {
        let mut assets = Assets::<SourceMaterial>::default();
        let mut slots = MaterialSlots::default();
        slots.lights.next = SLOTS as u32;
        let handle = assets.add(test_material());
        slots.write_lighting(&mut assets, &handle, (Some(1), false), lit(0.1));
        slots.write_lighting(&mut assets, &handle, (Some(1), false), lit(0.4));
        let m = assets.get(&handle).unwrap();
        assert_eq!((m.dynamic.z, m.lighting), (-1., lit(0.4)));
    }

    #[test]
    fn tagged_draws_share_one_material_and_release_their_slot_when_removed() {
        use bevy::prelude::Entity;
        let mut assets = Assets::<SourceMaterial>::default();
        let mut slots = MaterialSlots::default();
        let mut shared_material = test_material();
        shared_material.lighting = lit(0.1);
        let shared = assets.add(shared_material);
        let draw_a1 = Entity::from_bits(10);
        let draw_a2 = Entity::from_bits(11);
        let draw_b = Entity::from_bits(20);
        let key_a: LightKey = (Some(1), true);
        let key_b: LightKey = (Some(2), false);
        assert!(slots.has_slot_for(key_a));

        let tag_a1 = slots.assign_draw_tag(draw_a1, key_a).unwrap();
        // Fresh slots start with fallback lighting so frame-0 draws never sample zeros.
        let m = assets.get(&shared).unwrap();
        assert_eq!(m.dynamic.z, -1.);
        assert_eq!(
            slots.draw_lighting(tag_a1, m, shared.id()),
            ModelLighting::fallback(true)
        );
        slots.write_tagged_lighting(tag_a1, lit(0.3));

        // Another draw on the same entity reuses the tag without overwriting written lighting.
        let tag_a2 = slots.assign_draw_tag(draw_a2, key_a).unwrap();
        assert_eq!(tag_a2, tag_a1);
        assert_eq!(slots.draw_lighting(tag_a2, m, shared.id()), lit(0.3));

        // Another entity gets its own slot while sharing the same material asset.
        let tag_b = slots.assign_draw_tag(draw_b, key_b).unwrap();
        assert_ne!(tag_a1, tag_b);
        slots.write_tagged_lighting(tag_b, lit(0.7));
        assert_eq!(slots.draw_lighting(tag_a1, m, shared.id()), lit(0.3));
        assert_eq!(slots.draw_lighting(tag_b, m, shared.id()), lit(0.7));

        // Removing one draw of entity 1 keeps its slot live until the second draw is removed.
        slots.lights.release_draw(draw_a1);
        assert!(slots.lights.users.contains_key(&(tag_a1 - 1)));
        slots.lights.release_draw(draw_a2);
        assert_eq!(slots.lights.slot_for((Some(3), false)), Some(tag_a1 - 1));

        // Overflow reports no free slots, and clear_lights resets for a new map.
        slots.lights.free.clear();
        slots.lights.next = SLOTS as u32;
        assert!(!slots.has_slot_for((Some(4), false)));
        assert_eq!(
            slots.assign_draw_tag(Entity::from_bits(30), (Some(4), false)),
            None
        );

        slots.clear_lights();
        assert!(slots.has_slot_for((Some(4), false)));
        assert_eq!(
            slots.assign_draw_tag(Entity::from_bits(30), (Some(4), false)),
            Some(1)
        );
    }
}

#[cfg(test)]
mod shader_layout {
    /// `source.wesl` reads lighting slots at the same binding and stride as [`super::LIGHT_ROWS`].
    #[test]
    fn the_shader_reads_lighting_rows_where_they_are_written() {
        let shader = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/source.wesl"
        ));
        assert!(shader.contains("@binding(22) var<storage, read> light_rows: array<vec4<f32>>;"));
        assert!(shader.contains(&format!("light_rows[u32(slot) * {}u + i]", super::ROWS)));
        assert!(shader.contains("mesh_functions::get_tag(instance_index)"));
    }
}
