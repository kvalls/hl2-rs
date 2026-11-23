//! Material values that change every frame (a texture-scroll transform, a proxy's tint) kept in
//! one shared storage buffer instead of the material asset. Any change to a `SourceMaterial`
//! rebuilds its whole bind group in the render world; a material with a slot is changed once (to
//! name its slot) and afterwards only its rows here change, uploaded together once per frame.
//! The values are still computed on the CPU from the scene clock, so pause and scene time behave
//! as before. When the slots run out, a material keeps the asset path.
//! Model lighting ([`lighting`]) shares the buffer: one slot per lit entity, after the material slots.
mod lighting;
use crate::rendering::SourceMaterial;
use bevy::{
    asset::uuid_handle,
    math::{Mat3, Vec3, Vec4},
    platform::collections::HashMap,
    prelude::{AssetEvent, AssetId, Assets, Handle, MessageReader, ResMut, Resource},
    render::storage::ShaderBuffer,
};

pub const MATERIAL_ROWS: Handle<ShaderBuffer> =
    uuid_handle!("1e7cec58-7516-4d6b-879e-0784ed801107");
pub const LIGHT_ROWS: Handle<ShaderBuffer> = uuid_handle!("8b4c9e21-3f7a-4d12-9c6e-5a1b8f042d93");
/// Animated materials the buffer holds. Its size never changes: a resized buffer is a new GPU
/// buffer the materials' bind groups would not see.
const SLOTS: usize = 256;
/// `tint` (rgb; alpha stays in the material), then the three columns of `secondary_uv`.
const ROWS: usize = 4;
const MATERIAL_TOTAL_ROWS: usize = SLOTS * ROWS;
const LIGHT_TOTAL_ROWS: usize = lighting::SLOTS * lighting::ROWS;

/// What a material takes from its slot (`SourceMaterial::dynamic`: x slot, y tint flag).
#[derive(Clone, Copy, PartialEq)]
pub enum Animated {
    /// The texture transform only; the tint stays in the material.
    Transform(Mat3),
    /// The tint's colour and the texture transform.
    TintAndTransform(Vec3, Mat3),
}

#[derive(Resource)]
pub struct MaterialSlots {
    by_material: HashMap<AssetId<SourceMaterial>, u32>,
    free: Vec<u32>,
    next: u32,
    rows: Vec<Vec4>,
    /// Dirty row range `[lo, hi)` since the last material-buffer upload.
    dirty: Option<(usize, usize)>,
    light_rows: Vec<Vec4>,
    /// Dirty row range `[lo, hi)` since the last lighting-buffer upload.
    light_dirty: Option<(usize, usize)>,
    lights: lighting::LightSlots,
}
impl Default for MaterialSlots {
    fn default() -> Self {
        Self {
            by_material: HashMap::default(),
            free: Vec::new(),
            next: 0,
            rows: vec![Vec4::ZERO; MATERIAL_TOTAL_ROWS],
            dirty: None,
            light_rows: vec![Vec4::ZERO; LIGHT_TOTAL_ROWS],
            light_dirty: None,
            lights: lighting::LightSlots::default(),
        }
    }
}
impl MaterialSlots {
    /// Materials holding a slot.
    pub fn len(&self) -> usize {
        self.by_material.len()
    }
    /// Sets this frame's animated values of `handle`'s material.
    pub fn write(
        &mut self,
        materials: &mut Assets<SourceMaterial>,
        handle: &Handle<SourceMaterial>,
        values: Animated,
    ) {
        let tint = matches!(values, Animated::TintAndTransform(..));
        let (color, transform) = match values {
            Animated::Transform(uv) => (None, uv),
            Animated::TintAndTransform(color, uv) => (Some(color), uv),
        };
        let differs = |m: &SourceMaterial| {
            m.secondary_uv != transform || color.is_some_and(|c| m.tint.truncate() != c)
        };
        let slot = match self.by_material.get(&handle.id()) {
            Some(&slot) => Some(slot),
            // A material whose values never move keeps the asset path and no slot.
            None if !materials.get(handle).is_some_and(differs) => return,
            None => self.assign(materials, handle.id(), tint),
        };
        let Some(slot) = slot else {
            // Out of slots: the asset path, changed only when a value differs.
            if materials.get(handle).is_some_and(differs)
                && let Some(mut m) = materials.get_mut(handle)
            {
                m.secondary_uv = transform;
                if let Some(color) = color {
                    m.tint = color.extend(m.tint.w);
                }
            }
            return;
        };
        let rows = [
            color.unwrap_or(Vec3::ONE).extend(0.),
            transform.x_axis.extend(0.),
            transform.y_axis.extend(0.),
            transform.z_axis.extend(0.),
        ];
        self.set_rows(slot as usize * ROWS, &rows);
    }
    /// The values a material draws with: its slot's when it has one, else the asset's.
    pub fn values(&self, material: &SourceMaterial, id: AssetId<SourceMaterial>) -> (Vec4, Mat3) {
        let Some(&slot) = self.by_material.get(&id) else {
            return (material.tint, material.secondary_uv);
        };
        let row = &self.rows[slot as usize * ROWS..][..ROWS];
        let tint = if material.dynamic.y > 0.5 {
            row[0].truncate().extend(material.tint.w)
        } else {
            material.tint
        };
        let transform = Mat3::from_cols(row[1].truncate(), row[2].truncate(), row[3].truncate());
        (tint, transform)
    }
    /// Writes `rows` at `start`, marking `[start, start + rows.len())` dirty when any differs.
    fn set_rows(&mut self, start: usize, rows: &[Vec4]) {
        let target = &mut self.rows[start..start + rows.len()];
        if target != rows {
            target.copy_from_slice(rows);
            mark_dirty(&mut self.dirty, start, start + rows.len());
        }
    }
    /// Writes lighting `rows` at `start`, marking `[start, start + rows.len())` dirty when any differs.
    fn set_light_rows(&mut self, start: usize, rows: &[Vec4]) {
        let target = &mut self.light_rows[start..start + rows.len()];
        if target != rows {
            target.copy_from_slice(rows);
            mark_dirty(&mut self.light_dirty, start, start + rows.len());
        }
    }
    fn assign(
        &mut self,
        materials: &mut Assets<SourceMaterial>,
        id: AssetId<SourceMaterial>,
        tint: bool,
    ) -> Option<u32> {
        materials.get(id)?;
        let slot = self.free.pop().or_else(|| {
            (self.next < SLOTS as u32).then(|| {
                self.next += 1;
                self.next - 1
            })
        })?;
        let mut material = materials.get_mut(id)?;
        material.dynamic.x = slot as f32;
        material.dynamic.y = f32::from(u8::from(tint));
        self.by_material.insert(id, slot);
        Some(slot)
    }
}

fn mark_dirty(range: &mut Option<(usize, usize)>, start: usize, end: usize) {
    *range = Some(match *range {
        Some((lo, hi)) => (lo.min(start), hi.max(end)),
        None => (start, end),
    });
}

/// Registers the slots, their buffer and the upload after this frame's writers.
pub fn plugin(app: &mut bevy::app::App) {
    use bevy::prelude::{IntoScheduleConfigs, PostUpdate, Startup, TransformSystems};
    app.init_resource::<MaterialSlots>()
        .add_systems(Startup, install)
        .add_systems(
            PostUpdate,
            upload
                .after(crate::monitors::present_materials)
                .after(crate::material_proxies::present)
                .after(crate::rendering::present_lighting)
                .before(TransformSystems::Propagate),
        );
}

/// Creates the buffers before any material reads them.
pub fn install(mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let mut materials = ShaderBuffer::default();
    materials.set_data(vec![Vec4::ZERO; MATERIAL_TOTAL_ROWS]);
    let _ = buffers.insert(MATERIAL_ROWS.id(), materials);
    let mut lights = ShaderBuffer::default();
    lights.set_data(vec![Vec4::ZERO; LIGHT_TOTAL_ROWS]);
    let _ = buffers.insert(LIGHT_ROWS.id(), lights);
}

/// Writes `rows[range]` into `buffer.data`, reusing the existing byte buffer allocation.
pub(crate) fn write_rows_range(
    buffer: &mut ShaderBuffer,
    rows: &[Vec4],
    range: std::ops::Range<usize>,
) {
    let total_bytes = rows.len() * 16;
    let bytes = buffer.data.get_or_insert_with(|| vec![0; total_bytes]);
    let range = if bytes.len() != total_bytes {
        bytes.resize(total_bytes, 0);
        0..rows.len()
    } else {
        range.start.min(rows.len())..range.end.min(rows.len())
    };
    for (dst, row) in bytes[range.start * 16..range.end * 16]
        .as_chunks_mut::<16>()
        .0
        .iter_mut()
        .zip(&rows[range])
    {
        for (chunk, value) in dst.as_chunks_mut::<4>().0.iter_mut().zip(row.to_array()) {
            *chunk = value.to_le_bytes();
        }
    }
}

/// Writes `rows` into `buffer.data`, reusing the existing byte buffer allocation when present.
pub(crate) fn write_rows(buffer: &mut ShaderBuffer, rows: &[Vec4]) {
    write_rows_range(buffer, rows, 0..rows.len());
}

/// Uploads the changed row ranges when a slot changed this frame; frees removed materials and draws.
pub fn upload(
    mut slots: ResMut<MaterialSlots>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut events: MessageReader<AssetEvent<SourceMaterial>>,
    mut removed_lit: bevy::ecs::lifecycle::RemovedComponents<crate::rendering::ModelLit>,
) {
    for event in events.read() {
        if let AssetEvent::Removed { id } = event {
            if let Some(slot) = slots.by_material.remove(id) {
                slots.free.push(slot);
            }
            slots.lights.release(*id);
        }
    }
    for draw in removed_lit.read() {
        slots.lights.release_draw(draw);
    }
    if let Some((lo, hi)) = slots.dirty.take()
        && let Some(mut buffer) = buffers.get_mut(&MATERIAL_ROWS)
    {
        write_rows_range(&mut buffer, &slots.rows, lo..hi);
    }
    if let Some((lo, hi)) = slots.light_dirty.take()
        && let Some(mut buffer) = buffers.get_mut(&LIGHT_ROWS)
    {
        write_rows_range(&mut buffer, &slots.light_rows, lo..hi);
    }
}

/// A plain material for tests.
#[cfg(test)]
pub(crate) fn test_material() -> SourceMaterial {
    use bevy::prelude::AlphaMode;
    SourceMaterial {
        tint: Vec4::ONE,
        parameters: Vec4::ZERO,
        base: Handle::default(),
        lightmap: Handle::default(),
        iris: Handle::default(),
        secondary_uv: Mat3::IDENTITY,
        lighting: crate::rendering::ModelLighting::default(),
        envmap: Handle::default(),
        envmap_tint: Vec4::ZERO,
        envmap_contrast: Vec4::ZERO,
        envmap_saturation: Vec4::ZERO,
        envmap_mask: Handle::default(),
        self_illum: Vec4::ZERO,
        flashlight: Handle::default(),
        viewmodel_flashlight: crate::flashlight::ViewmodelLight::default(),
        dynamic: Vec4::new(-1., 0., -1., 0.),
        alpha: AlphaMode::Opaque,
        two_sided: false,
        eye_rows: crate::eyes::EYE_ROWS,
        material_rows: MATERIAL_ROWS,
        light_rows: LIGHT_ROWS,
    }
}

#[cfg(test)]
mod tests {
    use super::{Animated, MaterialSlots, SLOTS};
    use crate::rendering::SourceMaterial;
    use bevy::{
        math::{Mat3, Vec3, Vec4},
        prelude::{Assets, Handle},
    };

    fn material(assets: &mut Assets<SourceMaterial>) -> Handle<SourceMaterial> {
        assets.add(super::test_material())
    }

    #[test]
    fn a_slot_is_named_once_and_later_frames_leave_the_material_unchanged() {
        let mut assets = Assets::<SourceMaterial>::default();
        let mut slots = MaterialSlots::default();
        let scroll = material(&mut assets);
        let monitor = material(&mut assets);
        let uv = Mat3::from_cols(Vec3::X, Vec3::Y, Vec3::new(0.25, 0.5, 1.));
        slots.write(&mut assets, &scroll, Animated::Transform(uv));
        slots.write(
            &mut assets,
            &monitor,
            Animated::TintAndTransform(Vec3::new(0.5, 0.25, 1.), Mat3::IDENTITY),
        );
        let (a, b) = (assets.get(&scroll).unwrap(), assets.get(&monitor).unwrap());
        assert_eq!(a.dynamic, Vec4::new(0., 0., -1., 0.));
        assert_eq!(b.dynamic, Vec4::new(1., 1., -1., 0.));
        // The material keeps its own tint and transform; draws read the slot.
        assert_eq!(a.secondary_uv, Mat3::IDENTITY);
        assert_eq!(slots.values(a, scroll.id()), (a.tint, uv));
        assert_eq!(
            slots.values(b, monitor.id()).0,
            Vec3::new(0.5, 0.25, 1.).extend(b.tint.w)
        );
        // A later frame changes the rows only.
        let before = assets.get(&scroll).unwrap().dynamic;
        slots.write(&mut assets, &scroll, Animated::Transform(Mat3::IDENTITY));
        assert_eq!(assets.get(&scroll).unwrap().dynamic, before);
        assert_eq!(
            slots.values(assets.get(&scroll).unwrap(), scroll.id()).1,
            Mat3::IDENTITY
        );
    }

    #[test]
    fn without_free_slots_the_material_itself_carries_the_values() {
        let mut assets = Assets::<SourceMaterial>::default();
        let mut slots = MaterialSlots {
            next: SLOTS as u32,
            ..MaterialSlots::default()
        };
        let handle = material(&mut assets);
        let uv = Mat3::from_cols(Vec3::X, Vec3::Y, Vec3::new(0.5, 0., 1.));
        slots.write(
            &mut assets,
            &handle,
            Animated::TintAndTransform(Vec3::splat(0.5), uv),
        );
        let m = assets.get(&handle).unwrap();
        assert_eq!(m.dynamic.x, -1.);
        assert_eq!((m.secondary_uv, m.tint.truncate()), (uv, Vec3::splat(0.5)));
        assert_eq!(slots.len(), 0);
    }

    #[test]
    fn write_rows_reuses_the_existing_shader_buffer_allocation() {
        use bevy::render::render_resource::encase::StorageBuffer;
        use bevy::render::storage::ShaderBuffer;
        let mut buffer = ShaderBuffer::default();
        let rows = [
            Vec4::new(1., 2., 3., 4.),
            Vec4::new(5., 6., 7., 8.),
            Vec4::new(-1.5, 0.25, 42., -0.),
        ];
        super::write_rows(&mut buffer, &rows);
        let mut expected = Vec::new();
        StorageBuffer::new(&mut expected).write(&rows).unwrap();
        assert_eq!(buffer.data.as_deref(), Some(expected.as_slice()));

        let first_ptr = buffer.data.as_ref().unwrap().as_ptr();
        let updated = [Vec4::new(1., 2., 3., 4.), Vec4::splat(9.), Vec4::splat(10.)];
        super::write_rows_range(&mut buffer, &updated, 1..2);
        let second_ptr = buffer.data.as_ref().unwrap().as_ptr();
        assert_eq!(first_ptr, second_ptr);

        let mixed = [rows[0], updated[1], rows[2]];
        let mut expected_mixed = Vec::new();
        StorageBuffer::new(&mut expected_mixed)
            .write(&mixed)
            .unwrap();
        assert_eq!(buffer.data.as_deref(), Some(expected_mixed.as_slice()));
    }
}
