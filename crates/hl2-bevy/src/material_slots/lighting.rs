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
    prelude::{AssetId, Assets, Handle},
};

/// Lit entities the buffer holds.
pub(super) const SLOTS: usize = 512;
/// A [`ModelLighting`] as rows: params, the basis columns, the six ambient faces, then the four
/// lights' positions, colours, directions, attenuations and spot terms (as in `source.wgsl`).
pub(super) const ROWS: usize = 31;

/// A lit draw's owner: its entity (`None` for the viewmodel) and whether it is half-Lambert.
pub type LightKey = (Option<usize>, bool);

#[derive(Default)]
pub(super) struct LightSlots {
    by_key: HashMap<LightKey, u32>,
    by_material: HashMap<AssetId<SourceMaterial>, u32>,
    /// Each slot's owner and how many materials draw from it.
    users: HashMap<u32, (LightKey, u32)>,
    free: Vec<u32>,
    next: u32,
}
impl LightSlots {
    /// A removed material stops drawing from its slot; the last one frees it.
    pub(super) fn release(&mut self, id: AssetId<SourceMaterial>) {
        let Some(slot) = self.by_material.remove(&id) else {
            return;
        };
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
    fn slot_for(&mut self, key: LightKey) -> Option<u32> {
        if let Some(&slot) = self.by_key.get(&key) {
            return Some(slot);
        }
        let slot = self.free.pop().or_else(|| {
            (self.next < SLOTS as u32).then(|| {
                self.next += 1;
                self.next - 1
            })
        })?;
        self.by_key.insert(key, slot);
        self.users.insert(slot, (key, 0));
        Some(slot)
    }
}

/// The first row of lighting slot `slot` in the buffer.
fn start(slot: u32) -> usize {
    super::SLOTS * super::ROWS + slot as usize * ROWS
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
            Some(&slot) => from_rows(&self.rows[start(slot)..][..ROWS]),
            None => material.lighting,
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
            Some(slot) => self.set_rows(start(slot), &rows_of(&lighting)),
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
}

#[cfg(test)]
mod shader_layout {
    /// `source.wgsl` reads lighting slots at the same offset and stride as these rows.
    #[test]
    fn the_shader_reads_lighting_rows_where_they_are_written() {
        let shader = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/source.wgsl"
        ));
        let base = super::super::SLOTS * super::super::ROWS;
        assert!(shader.contains(&format!("const LIGHT_BASE: u32 = {base}u;")));
        assert!(shader.contains(&format!("u32(material.dynamic.z) * {}u", super::ROWS)));
    }
}
