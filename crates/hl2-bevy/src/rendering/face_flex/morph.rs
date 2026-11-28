//! GPU facial flex morph target construction and per-frame weight updates.
use crate::source_to_bevy;
use bevy::{
    math::Vec3,
    mesh::{
        Mesh,
        morph::{MAX_MORPH_WEIGHTS, MeshMorphWeights, MorphAttributes},
    },
};
use source_assets::flexes::{FaceModel, MeshFlex};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MorphChannel {
    pub(crate) descriptor: usize,
    pub(crate) targets: [f32; 4],
}

#[derive(Clone, Debug)]
pub(crate) struct MorphChannels {
    pub(crate) channels: Vec<MorphChannel>,
}

impl MorphChannels {
    pub(crate) fn len(&self) -> usize {
        self.channels.len()
    }

    pub(crate) fn weights_component(&self) -> MeshMorphWeights {
        MeshMorphWeights::Value {
            weights: vec![0.; self.channels.len()],
        }
    }

    pub(crate) fn write_weights(&self, descriptors: &[f32], weights: &mut Vec<f32>) {
        if weights.len() != self.channels.len() {
            weights.resize(self.channels.len(), 0.);
        }
        for (slot, channel) in weights.iter_mut().zip(&self.channels) {
            let value = descriptors.get(channel.descriptor).copied().unwrap_or(0.);
            *slot = MeshFlex::ramp_value(value, channel.targets);
        }
    }
}

/// Discover active facial flex channels for a flexed draw mesh and upload its Bevy morph targets.
/// Returns `None` when the mesh receives no non-zero flex deltas or exceeds `MAX_MORPH_WEIGHTS`.
pub(crate) fn build_morph_targets(
    mesh: &mut Mesh,
    face: &FaceModel,
    flex: &[Option<[u16; 3]>],
    scale: f32,
) -> Option<MorphChannels> {
    let vertex_count = mesh.count_vertices();
    if vertex_count == 0 || flex.iter().all(Option::is_none) {
        return None;
    }
    let mut channels = Vec::new();
    let mut first = vec![None::<([u32; 4], usize)>; face.flex.descriptors.len()];
    let mut extra = Vec::<(usize, [u32; 4], usize)>::new();
    let mut deltas = Vec::<(usize, usize, Vec3)>::new();
    for_each_flex_delta(
        face,
        flex,
        scale,
        vertex_count,
        |descriptor, targets, v_idx, delta| {
            let bits = targets.map(f32::to_bits);
            let ch_idx = if let Some(slot) = first.get_mut(descriptor) {
                match slot {
                    Some((b, idx)) if *b == bits => *idx,
                    Some(_) => {
                        push_extra_channel(&mut extra, &mut channels, descriptor, targets, bits)
                    }
                    None => {
                        let idx = channels.len();
                        channels.push(MorphChannel {
                            descriptor,
                            targets,
                        });
                        *slot = Some((bits, idx));
                        idx
                    }
                }
            } else {
                push_extra_channel(&mut extra, &mut channels, descriptor, targets, bits)
            };
            deltas.push((ch_idx, v_idx, delta));
        },
    );
    if channels.is_empty() || channels.len() > MAX_MORPH_WEIGHTS {
        return None;
    }
    let mut targets = vec![MorphAttributes::default(); channels.len() * vertex_count];
    for (ch_idx, v_idx, delta) in deltas {
        targets[ch_idx * vertex_count + v_idx].position += delta;
    }
    mesh.set_morph_targets(targets);
    Some(MorphChannels { channels })
}

fn push_extra_channel(
    extra: &mut Vec<(usize, [u32; 4], usize)>,
    channels: &mut Vec<MorphChannel>,
    descriptor: usize,
    targets: [f32; 4],
    bits: [u32; 4],
) -> usize {
    if let Some(&(_, _, idx)) = extra
        .iter()
        .find(|&&(d, b, _)| d == descriptor && b == bits)
    {
        return idx;
    }
    let idx = channels.len();
    channels.push(MorphChannel {
        descriptor,
        targets,
    });
    extra.push((descriptor, bits, idx));
    idx
}

struct ChannelTable {
    ranges: Vec<(u32, u32)>,
    entries: Vec<(usize, [u32; 4], usize)>,
}

impl ChannelTable {
    fn new(channels: &[MorphChannel]) -> Self {
        let mut entries: Vec<(usize, [u32; 4], usize)> = channels
            .iter()
            .enumerate()
            .map(|(i, c)| (c.descriptor, c.targets.map(f32::to_bits), i))
            .collect();
        entries.sort_unstable_by_key(|&(desc, _, idx)| (desc, idx));
        let max_desc = entries.last().map_or(0, |&(d, _, _)| d);
        let ranges = if !entries.is_empty() && max_desc <= 4096 {
            let mut ranges = vec![(0u32, 0u32); max_desc + 1];
            let mut start = 0usize;
            while start < entries.len() {
                let desc = entries[start].0;
                let mut end = start + 1;
                while end < entries.len() && entries[end].0 == desc {
                    end += 1;
                }
                ranges[desc] = (start as u32, (end - start) as u32);
                start = end;
            }
            ranges
        } else {
            Vec::new()
        };
        Self { ranges, entries }
    }

    fn get(&self, descriptor: usize, ramp_bits: [u32; 4]) -> Option<usize> {
        let slice = if !self.ranges.is_empty() {
            let &(start, len) = self.ranges.get(descriptor)?;
            &self.entries[start as usize..(start + len) as usize]
        } else {
            let start = self.entries.partition_point(|&(d, _, _)| d < descriptor);
            let end = start + self.entries[start..].partition_point(|&(d, _, _)| d == descriptor);
            &self.entries[start..end]
        };
        slice
            .iter()
            .find(|&&(_, bits, _)| bits == ramp_bits)
            .map(|&(_, _, idx)| idx)
    }
}

/// Populate morph targets for a coarser (or collapsed) LOD mesh using LOD0's channel list.
pub(crate) fn apply_morph_channels(
    mesh: &mut Mesh,
    channels: &MorphChannels,
    face: Option<&FaceModel>,
    flex: &[Option<[u16; 3]>],
    scale: f32,
) {
    let vertex_count = mesh.count_vertices();
    let mut targets = vec![MorphAttributes::default(); channels.len() * vertex_count];
    if vertex_count > 0
        && let Some(face) = face
        && flex.iter().any(Option::is_some)
    {
        let channel_index = ChannelTable::new(&channels.channels);
        for_each_flex_delta(
            face,
            flex,
            scale,
            vertex_count,
            |descriptor, ramp, v_idx, delta| {
                if let Some(ch_idx) = channel_index.get(descriptor, ramp.map(f32::to_bits)) {
                    targets[ch_idx * vertex_count + v_idx].position += delta;
                }
            },
        );
    }
    mesh.set_morph_targets(targets);
}

fn vertices_by_key(flex: &[Option<[u16; 3]>]) -> Vec<([u16; 3], u32)> {
    let mut entries = Vec::with_capacity(flex.len());
    for (vertex, entry) in flex.iter().enumerate() {
        if let Some(key) = entry {
            entries.push((*key, vertex as u32));
        }
    }
    entries.sort_unstable_by_key(|&(key, vertex)| (key, vertex));
    entries
}

fn for_each_flex_delta(
    face: &FaceModel,
    flex: &[Option<[u16; 3]>],
    scale: f32,
    vertex_count: usize,
    mut emit: impl FnMut(usize, [f32; 4], usize, Vec3),
) {
    let by_key = vertices_by_key(flex);
    if by_key.is_empty() {
        return;
    }
    for mesh_flexes in face.flex.meshes.iter().filter(|mesh| mesh.model == 0) {
        let (Ok(bp), Ok(m)) = (
            u16::try_from(mesh_flexes.bodypart),
            u16::try_from(mesh_flexes.mesh),
        ) else {
            continue;
        };
        let pair_start = by_key.partition_point(|&(k, _)| (k[0], k[1]) < (bp, m));
        let pair_end =
            pair_start + by_key[pair_start..].partition_point(|&(k, _)| (k[0], k[1]) == (bp, m));
        let mesh_by_key = &by_key[pair_start..pair_end];
        if mesh_by_key.is_empty() {
            continue;
        }
        for mesh_flex in &mesh_flexes.flexes {
            for vert in &mesh_flex.vertices {
                let start = mesh_by_key.partition_point(|&(k, _)| k[2] < vert.index);
                if start >= mesh_by_key.len() || mesh_by_key[start].0[2] != vert.index {
                    continue;
                }
                let end =
                    start + mesh_by_key[start..].partition_point(|&(k, _)| k[2] == vert.index);
                let mesh_vertices = &mesh_by_key[start..end];
                let delta = source_to_bevy(Vec3::from_array((vert.delta * scale).to_array()));
                if delta == Vec3::ZERO {
                    continue;
                }
                if mesh_flex.pair == 0 {
                    emit_for_vertices(
                        mesh_vertices,
                        vertex_count,
                        mesh_flex.descriptor,
                        mesh_flex.targets,
                        delta,
                        &mut emit,
                    );
                } else {
                    let side = f32::from(vert.side) / 255.0;
                    emit_for_vertices(
                        mesh_vertices,
                        vertex_count,
                        mesh_flex.descriptor,
                        mesh_flex.targets,
                        delta * (1.0 - side),
                        &mut emit,
                    );
                    emit_for_vertices(
                        mesh_vertices,
                        vertex_count,
                        mesh_flex.pair,
                        mesh_flex.targets,
                        delta * side,
                        &mut emit,
                    );
                }
            }
        }
    }
}

fn emit_for_vertices(
    mesh_vertices: &[([u16; 3], u32)],
    vertex_count: usize,
    descriptor: usize,
    targets: [f32; 4],
    delta: Vec3,
    emit: &mut impl FnMut(usize, [f32; 4], usize, Vec3),
) {
    if delta == Vec3::ZERO {
        return;
    }
    for &(_, v_idx) in mesh_vertices {
        let v_idx = v_idx as usize;
        if v_idx < vertex_count {
            emit(descriptor, targets, v_idx, delta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    use bevy::render::render_resource::PrimitiveTopology;
    use source_assets::flexes::{FlexModel, MeshFlexes, VertAnim};

    #[test]
    fn gpu_morph_targets_match_cpu_vertex_deltas_for_single_and_paired_flexes() {
        let scale = 0.75;
        let bind_source = [
            glam::Vec3::new(1.0, 2.0, 3.0),
            glam::Vec3::new(-4.0, 5.0, 1.5),
            glam::Vec3::new(0.5, -1.0, 2.5),
        ];
        let mut flex_model = FlexModel {
            descriptors: vec!["jaw".into(), "blink_l".into(), "blink_r".into()],
            controllers: Vec::new(),
            rules: Vec::new(),
            meshes: vec![MeshFlexes {
                bodypart: 0,
                model: 0,
                mesh: 0,
                flexes: vec![
                    MeshFlex {
                        descriptor: 0,
                        targets: [0.0, 0.25, 0.75, 1.0],
                        pair: 0,
                        vertices: vec![
                            VertAnim {
                                index: 10,
                                speed: 0,
                                side: 0,
                                delta: glam::Vec3::new(0.4, -0.2, 0.6),
                                normal: glam::Vec3::Z,
                            },
                            VertAnim {
                                index: 11,
                                speed: 0,
                                side: 200,
                                delta: glam::Vec3::new(-0.1, 0.3, -0.5),
                                normal: glam::Vec3::Z,
                            },
                        ],
                    },
                    MeshFlex {
                        descriptor: 1,
                        targets: [0.0, 0.5, 0.5, 1.0],
                        pair: 2,
                        vertices: vec![VertAnim {
                            index: 10,
                            speed: 0,
                            side: 64,
                            delta: glam::Vec3::new(0.0, 0.8, -0.4),
                            normal: glam::Vec3::Z,
                        }],
                    },
                ],
            }],
        };
        // Bodygroup alternatives reuse mesh-local indices. Only the displayed model contributes.
        let mut alternate = flex_model.meshes[0].clone();
        alternate.model = 1;
        flex_model.meshes.push(alternate);
        let slots = flex_model.vertex_slots();
        let flex_vertices = vec![Some([0, 0, 10]), Some([0, 0, 11]), Some([0, 0, 10])];
        let face = FaceModel {
            flex: flex_model,
            slots,
            eyes: Vec::new(),
        };
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            VertexAttributeValues::Float32x3(
                bind_source
                    .iter()
                    .map(|p| source_to_bevy(Vec3::from_array((*p * scale).to_array())).to_array())
                    .collect(),
            ),
        );
        let channels = build_morph_targets(&mut mesh, &face, &flex_vertices, scale).unwrap();
        assert_eq!(channels.len(), 3);
        let targets = mesh.morph_targets().expect("morph targets uploaded");
        let n = bind_source.len();
        assert_eq!(targets.len(), channels.len() * n);

        let mut cpu_deltas = Vec::new();
        let mut weights = Vec::new();
        for descriptors in [
            [0.15, 0.3, 0.9],
            [0.5, 0.5, 0.2],
            [0.0, 0.0, 0.0],
            [0.9, 1.0, 0.8],
        ] {
            face.flex
                .vertex_deltas(&face.slots, &descriptors, &mut cpu_deltas);
            channels.write_weights(&descriptors, &mut weights);
            for (v_idx, key) in flex_vertices.iter().enumerate() {
                let slot = face.slots.slot(key.unwrap()).unwrap() as usize;
                let cpu_pos = source_to_bevy(Vec3::from_array(
                    ((bind_source[v_idx] + cpu_deltas[slot]) * scale).to_array(),
                ));
                let mut gpu_pos =
                    source_to_bevy(Vec3::from_array((bind_source[v_idx] * scale).to_array()));
                for (ch_idx, w) in weights.iter().enumerate() {
                    gpu_pos += targets[ch_idx * n + v_idx].position * *w;
                }
                assert!(
                    (cpu_pos - gpu_pos).length() < 1e-5,
                    "mismatch for {descriptors:?} v{v_idx}: cpu={cpu_pos:?} gpu={gpu_pos:?}"
                );
            }
        }
    }

    #[test]
    fn apply_morph_channels_skips_absent_bodypart_mesh_pairs_and_matches_targets() {
        let scale = 1.0;
        let flex_model = FlexModel {
            descriptors: vec!["a".into(), "b".into()],
            controllers: Vec::new(),
            rules: Vec::new(),
            meshes: vec![
                MeshFlexes {
                    bodypart: 0,
                    model: 0,
                    mesh: 0,
                    flexes: vec![MeshFlex {
                        descriptor: 0,
                        targets: [0.0, 0.0, 1.0, 1.0],
                        pair: 0,
                        vertices: vec![VertAnim {
                            index: 5,
                            speed: 0,
                            side: 0,
                            delta: glam::Vec3::new(0.0, 2.0, 0.0),
                            normal: glam::Vec3::Z,
                        }],
                    }],
                },
                MeshFlexes {
                    bodypart: 1,
                    model: 0,
                    mesh: 2,
                    flexes: vec![MeshFlex {
                        descriptor: 1,
                        targets: [0.0, 0.0, 1.0, 1.0],
                        pair: 0,
                        vertices: vec![VertAnim {
                            index: 5,
                            speed: 0,
                            side: 0,
                            delta: glam::Vec3::new(4.0, 0.0, 0.0),
                            normal: glam::Vec3::Z,
                        }],
                    }],
                },
            ],
        };
        let face = FaceModel {
            slots: flex_model.vertex_slots(),
            flex: flex_model,
            eyes: Vec::new(),
        };
        let mut lod0 = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
        lod0.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 1]);
        let channels = build_morph_targets(&mut lod0, &face, &[Some([0, 0, 5])], scale).unwrap();
        assert_eq!(channels.len(), 1);
        let mut lod1 = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
        lod1.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 2]);
        apply_morph_channels(
            &mut lod1,
            &channels,
            Some(&face),
            &[None, Some([0, 0, 5])],
            scale,
        );
        let targets = lod1.morph_targets().expect("lod1 morph targets");
        assert_eq!(targets[0].position, Vec3::ZERO);
        assert_eq!(
            targets[1].position,
            source_to_bevy(Vec3::new(0.0, 2.0, 0.0))
        );
    }

    #[ignore]
    #[test]
    fn owned_hl2_face_models_fit_morph_channel_budget() {
        let Some(root) = source_assets::install::discover().ok() else {
            panic!("owned Half-Life 2 install not discovered");
        };
        let vfs = source_assets::vpk::Vfs::mount_game(&root, "hl2").expect("mount hl2 VFS");
        let mut tested = 0usize;
        for name in [
            "models/barney.mdl",
            "models/kleiner.mdl",
            "models/alyx.mdl",
            "models/breen.mdl",
            "models/gman.mdl",
            "models/mossman.mdl",
            "models/eli.mdl",
            "models/humans/group01/male_01.mdl",
            "models/humans/group01/female_01.mdl",
        ] {
            let Ok(Some(bytes)) = vfs.read(name) else {
                continue;
            };
            let flex = source_assets::flexes::read_flexes(&bytes).expect("parse flexes");
            if flex.meshes.is_empty() {
                continue;
            }
            let slots = flex.vertex_slots();
            let face = FaceModel {
                flex,
                slots,
                eyes: Vec::new(),
            };
            let unique_keys: std::collections::BTreeSet<[u16; 3]> = face
                .flex
                .meshes
                .iter()
                .flat_map(|m| {
                    m.flexes.iter().flat_map(move |f| {
                        f.vertices
                            .iter()
                            .map(move |v| [m.bodypart as u16, m.mesh as u16, v.index])
                    })
                })
                .collect();
            let flex_vertices: Vec<Option<[u16; 3]>> = unique_keys.into_iter().map(Some).collect();
            let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![[0.0f32; 3]; flex_vertices.len()],
            );
            let channels = build_morph_targets(&mut mesh, &face, &flex_vertices, 1.0)
                .unwrap_or_else(|| panic!("{name} failed morph target construction"));
            assert!(
                !channels.channels.is_empty() && channels.len() <= MAX_MORPH_WEIGHTS,
                "{name} channel count {} out of budget 1..={MAX_MORPH_WEIGHTS}",
                channels.len()
            );
            eprintln!("{name}: {} morph channels", channels.len());
            tested += 1;
        }
        assert!(
            tested >= 5,
            "expected at least 5 HL2 face models tested, got {tested}"
        );
    }
}
