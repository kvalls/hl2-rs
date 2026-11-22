//! BSP leaf/PVS lookup for conservative world and monitor visibility.
use crate::{bytes, i16le, i32le, index, records, u16le, vec3};
use anyhow::{bail, Context, Result};
use glam::Vec3;
use std::collections::BTreeSet;

pub fn cluster(lumps: &[Vec<u8>], leaf_version: u32, point: Vec3) -> Result<i16> {
    i16le(leaf(lumps, leaf_version, point)?, 4)
}

/// Test all leaves overlapped by an axis-aligned entity bound, rather than only its origin.
/// Area/areaportal connectivity is a separate engine query and is not evaluated here.
pub fn bounds_in_pvs(
    lumps: &[Vec<u8>],
    leaf_version: u32,
    eye: Vec3,
    mins: Vec3,
    maxs: Vec3,
) -> Result<bool> {
    if !mins.is_finite() || !maxs.is_finite() || mins.cmpgt(maxs).any() {
        bail!("invalid BSP visibility bounds");
    }
    let visible = pvs_bits(
        lumps.get(4).context("BSP missing visibility")?,
        cluster(lumps, leaf_version, eye)?,
    )?;
    let nodes = lumps.get(5).context("BSP missing nodes")?;
    let planes = lumps.get(1).context("BSP missing planes")?;
    let leaves = lumps.get(10).context("BSP missing leaves")?;
    let leaf_size = if leaf_version == 0 { 56 } else { 32 };
    let root = i32le(
        bytes(lumps.get(14).context("BSP missing models")?, 0, 48)?,
        36,
    )?;
    walk_bounds(
        Walk {
            root,
            mins,
            maxs,
            visible: &visible,
            check_cycles: true,
        },
        &mut Vec::new(),
        |node| {
            let record = bytes(nodes, index(node, nodes.len() / 32)? * 32, 32)?;
            let plane = bytes(
                planes,
                index(i32le(record, 0)?, planes.len() / 20)? * 20,
                20,
            )?;
            Ok(Node {
                normal: vec3(plane, 0)?,
                distance: crate::f32le(plane, 12)?,
                children: [i32le(record, 4)?, i32le(record, 8)?],
            })
        },
        |leaf| {
            i16le(
                bytes(
                    leaves,
                    index(leaf, leaves.len() / leaf_size)? * leaf_size,
                    leaf_size,
                )?,
                4,
            )
        },
    )
}

fn validate_bounds(mins: Vec3, maxs: Vec3) -> Result<(Vec3, Vec3)> {
    if !mins.is_finite() || !maxs.is_finite() || mins.cmpgt(maxs).any() {
        bail!("invalid BSP visibility bounds")
    }
    let center = (mins + maxs) * 0.5;
    let extent = (maxs - mins) * 0.5;
    if !center.is_finite() || !extent.is_finite() {
        bail!("BSP visibility bounds overflow")
    }
    Ok((center, extent))
}

#[derive(Clone, Copy)]
struct Node {
    normal: Vec3,
    distance: f32,
    children: [i32; 2],
}
/// Push the children of `node` that the box `center ± extent` overlaps.
fn push_overlapped(node: Node, center: Vec3, extent: Vec3, pending: &mut Vec<i32>) {
    let distance = node.normal.dot(center) - node.distance;
    let radius = node.normal.abs().dot(extent);
    if distance + radius >= 0. {
        pending.push(node.children[0]);
    }
    if distance - radius < 0. {
        pending.push(node.children[1]);
    }
}

struct Walk<'a> {
    root: i32,
    mins: Vec3,
    maxs: Vec3,
    visible: &'a PvsRow,
    check_cycles: bool,
}

/// Whether any leaf overlapped by the bounds is in `visible`; stops at the first one.
fn walk_bounds(
    walk: Walk,
    pending: &mut Vec<i32>,
    mut node: impl FnMut(i32) -> Result<Node>,
    mut leaf: impl FnMut(i32) -> Result<i16>,
) -> Result<bool> {
    let (center, extent) = validate_bounds(walk.mins, walk.maxs)?;
    pending.clear();
    pending.push(walk.root);
    let mut visited = walk.check_cycles.then(BTreeSet::new);
    while let Some(id) = pending.pop() {
        if id < 0 {
            if walk
                .visible
                .contains(leaf(id.checked_neg().context("leaf overflow")? - 1)?)
            {
                return Ok(true);
            }
            continue;
        }
        if visited.as_mut().is_some_and(|visited| !visited.insert(id)) {
            bail!("cyclic BSP visibility tree")
        }
        push_overlapped(node(id)?, center, extent, pending);
    }
    Ok(false)
}

/// Validated immutable BSP visibility tree. PVS rows are decoded once per view cluster
/// by the host; querying current render bounds performs no map parsing or file I/O.
pub struct VisibilityIndex {
    nodes: Vec<Node>,
    clusters: Vec<i16>,
    root: i32,
    visibility: Vec<u8>,
}
impl VisibilityIndex {
    pub fn new(lumps: &[Vec<u8>], leaf_version: u32) -> Result<Self> {
        let leaf_size = match leaf_version {
            0 => 56,
            1 => 32,
            _ => bail!("unsupported BSP leaf version {leaf_version}"),
        };
        let node_data = lumps.get(5).context("BSP missing nodes")?;
        let plane_data = lumps.get(1).context("BSP missing planes")?;
        let leaf_data = lumps.get(10).context("BSP missing leaves")?;
        let planes: Vec<_> = records(plane_data, 20)?.collect();
        let records: Vec<_> = records(node_data, 32)?.collect();
        if records.len() > 1_048_576 {
            bail!("visibility node budget")
        }
        let nodes: Vec<_> = records
            .iter()
            .map(|r| {
                let plane = planes[index(i32le(r, 0)?, planes.len())?];
                Ok(Node {
                    normal: vec3(plane, 0)?,
                    distance: crate::f32le(plane, 12)?,
                    children: [i32le(r, 4)?, i32le(r, 8)?],
                })
            })
            .collect::<Result<_>>()?;
        let clusters = crate::records(leaf_data, leaf_size)?
            .map(|r| i16le(r, 4))
            .collect::<Result<Vec<_>>>()?;
        let root = i32le(
            bytes(lumps.get(14).context("BSP missing models")?, 0, 48)?,
            36,
        )?;
        let validate = |id: i32| -> Result<()> {
            if id < 0 {
                index(
                    id.checked_neg().context("leaf overflow")? - 1,
                    clusters.len(),
                )?;
            } else {
                index(id, nodes.len())?;
            }
            Ok(())
        };
        validate(root)?;
        for node in &nodes {
            for child in node.children {
                validate(child)?;
            }
        }
        // Validate a reachable tree once; reject cycles and shared non-tree nodes.
        let mut colors = vec![0u8; nodes.len()];
        let mut stack = vec![(root, false)];
        while let Some((id, exit)) = stack.pop() {
            if id < 0 {
                continue;
            }
            let i = id as usize;
            if exit {
                colors[i] = 2;
                continue;
            }
            if colors[i] == 1 {
                bail!("cyclic BSP visibility tree")
            }
            if colors[i] == 2 {
                bail!("shared BSP visibility node")
            }
            colors[i] = 1;
            stack.push((id, true));
            for child in nodes[i].children {
                stack.push((child, false));
            }
        }
        Ok(Self {
            nodes,
            clusters,
            root,
            visibility: lumps.get(4).context("BSP missing visibility")?.clone(),
        })
    }
    pub fn cluster(&self, point: Vec3) -> Result<i16> {
        if !point.is_finite() {
            bail!("non-finite BSP lookup point")
        }
        let mut id = self.root;
        while id >= 0 {
            let node = self.nodes[id as usize];
            id = node.children[usize::from(node.normal.dot(point) < node.distance)];
        }
        Ok(self.clusters[(-id - 1) as usize])
    }
    /// The decoded PVS row of `cluster` as a bitset; `None` (no valid row) when the map lacks
    /// PVS or `cluster < 0`, so the renderer must conservatively keep geometry.
    pub fn row(&self, cluster: i16) -> Result<Option<PvsRow>> {
        if cluster < 0 || self.visibility.is_empty() {
            return Ok(None);
        }
        Ok(Some(pvs_bits(&self.visibility, cluster)?))
    }
    /// Whether any leaf overlapped by `mins..=maxs` is in `visible`, reusing `stack`.
    pub fn bounds_visible(
        &self,
        mins: Vec3,
        maxs: Vec3,
        visible: &PvsRow,
        stack: &mut Vec<i32>,
    ) -> Result<bool> {
        walk_bounds(
            Walk {
                root: self.root,
                mins,
                maxs,
                visible,
                check_cycles: false,
            },
            stack,
            |id| Ok(self.nodes[id as usize]),
            |id| Ok(self.clusters[id as usize]),
        )
    }
    /// Collect sorted deduplicated non-negative leaf clusters touched by `mins..=maxs`,
    /// reusing `stack` and `out` across calls.
    pub fn bounds_clusters(
        &self,
        mins: Vec3,
        maxs: Vec3,
        stack: &mut Vec<i32>,
        out: &mut Vec<i16>,
    ) -> Result<()> {
        let (center, extent) = validate_bounds(mins, maxs)?;
        stack.clear();
        out.clear();
        stack.push(self.root);
        while let Some(id) = stack.pop() {
            if id < 0 {
                let leaf = id.checked_neg().context("leaf overflow")? - 1;
                let cluster = self.clusters[leaf as usize];
                if cluster >= 0 {
                    out.push(cluster);
                }
                continue;
            }
            push_overlapped(self.nodes[id as usize], center, extent, stack);
        }
        out.sort_unstable();
        out.dedup();
        Ok(())
    }
}

/// Unpacked PVS cluster visibility bitset for one view cluster (`O(1)` lookup).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PvsRow {
    bits: Vec<u8>,
    count: usize,
    all: bool,
}

impl PvsRow {
    /// Empty row matching no clusters.
    pub fn empty() -> Self {
        Self {
            bits: Vec::new(),
            count: 0,
            all: false,
        }
    }

    /// Full row matching every cluster in `0..count`.
    pub fn all(count: usize) -> Self {
        Self {
            bits: Vec::new(),
            count,
            all: true,
        }
    }

    /// Whether `cluster` is potentially visible in this row.
    pub fn contains(&self, cluster: i16) -> bool {
        let Ok(c) = usize::try_from(cluster) else {
            return false;
        };
        c < self.count && (self.all || (self.bits[c / 8] & (1 << (c % 8))) != 0)
    }
}

/// Find the containing world leaf without allocating a copy of the node/leaf tables.
pub(crate) fn leaf(lumps: &[Vec<u8>], leaf_version: u32, point: Vec3) -> Result<&[u8]> {
    if !point.is_finite() {
        bail!("non-finite BSP lookup point");
    }
    let leaf_size = match leaf_version {
        0 => 56,
        1 => 32,
        _ => bail!("unsupported BSP leaf version {leaf_version}"),
    };
    let nodes = lumps.get(5).context("BSP missing nodes")?;
    let planes = lumps.get(1).context("BSP missing planes")?;
    let leaves = lumps.get(10).context("BSP missing leaves")?;
    // Validate complete records before traversing. Empty tables are allowed only when unused.
    let _ = records(nodes, 32)?;
    let _ = records(planes, 20)?;
    let _ = records(leaves, leaf_size)?;
    let mut node = i32le(
        bytes(lumps.get(14).context("BSP missing models")?, 0, 48)?,
        36,
    )?;
    for _ in 0..=nodes.len() / 32 {
        if node < 0 {
            let id = index(
                node.checked_neg().context("leaf overflow")? - 1,
                leaves.len() / leaf_size,
            )?;
            return bytes(leaves, id * leaf_size, leaf_size);
        }
        let record = bytes(nodes, index(node, nodes.len() / 32)? * 32, 32)?;
        let plane = bytes(
            planes,
            index(i32le(record, 0)?, planes.len() / 20)? * 20,
            20,
        )?;
        let back = vec3(plane, 0)?.dot(point) < crate::f32le(plane, 12)?;
        node = i32le(record, if back { 8 } else { 4 })?;
    }
    bail!("cyclic BSP node tree")
}

pub fn pvs_bits(data: &[u8], cluster: i16) -> Result<PvsRow> {
    if cluster < 0 || data.is_empty() {
        return Ok(PvsRow::empty());
    }
    let count = usize::try_from(i32le(data, 0)?)?;
    if count > 32768 {
        bail!("PVS cluster limit");
    }
    let id = index(cluster as i32, count)?;
    let offset = i32le(data, 4 + id * 8)?;
    if offset < 0 {
        return Ok(PvsRow::all(count));
    }
    let mut i = offset as usize;
    let bits = decompress(data, &mut i, count.div_ceil(8))?;
    Ok(PvsRow {
        bits,
        count,
        all: false,
    })
}

pub fn pvs(data: &[u8], cluster: i16) -> Result<BTreeSet<i16>> {
    let row = pvs_bits(data, cluster)?;
    Ok((0..row.count)
        .filter(|&c| row.contains(c as i16))
        .map(|c| c as i16)
        .collect())
}
fn decompress(data: &[u8], cursor: &mut usize, length: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(length);
    while out.len() < length {
        let value = *data.get(*cursor).context("truncated PVS")?;
        *cursor += 1;
        if value != 0 {
            out.push(value);
        } else {
            let run = *data.get(*cursor).context("truncated PVS run")? as usize;
            *cursor += 1;
            if run == 0 || out.len() + run > length {
                bail!("invalid PVS zero run");
            }
            out.resize(out.len() + run, 0);
        }
    }
    Ok(out)
}
pub fn faces(
    lumps: &[Vec<u8>],
    leaf_version: u32,
    clusters: &BTreeSet<i16>,
) -> Result<BTreeSet<usize>> {
    let mut result = BTreeSet::new();
    for leaf in records(&lumps[10], if leaf_version == 0 { 56 } else { 32 })? {
        if !clusters.contains(&i16le(leaf, 4)?) {
            continue;
        }
        let first = u16le(leaf, 20)? as usize;
        let count = u16le(leaf, 22)? as usize;
        for face in records(bytes(&lumps[16], first * 2, count * 2)?, 2)? {
            result.insert(u16le(face, 0)? as usize);
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compressed_rows_preserve_cluster_bits_and_reject_bad_runs() {
        let mut cursor = 0;
        assert_eq!(
            decompress(&[5, 0, 2, 128], &mut cursor, 4).unwrap(),
            vec![5, 0, 0, 128]
        );
        assert!(decompress(&[0, 0], &mut 0, 1).is_err());
        assert!(decompress(&[0, 2], &mut 0, 1).is_err());
        assert!(decompress(&[0], &mut 0, 1).is_err());
    }
    #[test]
    fn bounds_visibility_crosses_leaf_planes_and_rejects_bad_bounds_and_cycles() {
        let mut lumps = vec![vec![]; 64];
        lumps[1] = vec![0; 20];
        lumps[1][0..4].copy_from_slice(&1f32.to_le_bytes());
        lumps[5] = vec![0; 32];
        lumps[5][4..8].copy_from_slice(&(-1i32).to_le_bytes());
        lumps[5][8..12].copy_from_slice(&(-2i32).to_le_bytes());
        lumps[10] = vec![0; 64];
        lumps[10][36..38].copy_from_slice(&1i16.to_le_bytes());
        lumps[14] = vec![0; 48];
        lumps[4] = vec![0; 22];
        lumps[4][0..4].copy_from_slice(&2i32.to_le_bytes());
        lumps[4][4..8].copy_from_slice(&20i32.to_le_bytes());
        lumps[4][12..16].copy_from_slice(&21i32.to_le_bytes());
        lumps[4][20] = 1;
        lumps[4][21] = 2;
        let eye = Vec3::X;
        let cached = VisibilityIndex::new(&lumps, 1).unwrap();
        assert_eq!(cached.cluster(eye).unwrap(), 0);
        assert_eq!(cached.cluster(-eye).unwrap(), 1);
        let row = cached.row(cached.cluster(eye).unwrap()).unwrap().unwrap();
        assert!(cached
            .bounds_visible(Vec3::X, Vec3::splat(2.), &row, &mut Vec::new())
            .unwrap());
        assert!(!cached
            .bounds_visible(Vec3::splat(-2.), -Vec3::X, &row, &mut Vec::new())
            .unwrap());
        assert!(cached
            .bounds_visible(Vec3::splat(-1.), Vec3::splat(1.), &row, &mut Vec::new())
            .unwrap());
        assert!(cached
            .bounds_visible(Vec3::ONE, Vec3::ZERO, &row, &mut Vec::new())
            .is_err());
        assert!(cached.row(-1).unwrap().is_none());
        assert!(bounds_in_pvs(&lumps, 1, eye, Vec3::X, Vec3::splat(2.)).unwrap());
        assert!(!bounds_in_pvs(&lumps, 1, eye, Vec3::splat(-2.), -Vec3::X).unwrap());
        assert!(bounds_in_pvs(&lumps, 1, eye, Vec3::splat(-1.), Vec3::splat(1.)).unwrap());
        assert!(bounds_in_pvs(&lumps, 1, eye, Vec3::splat(f32::NAN), Vec3::ZERO).is_err());
        assert!(bounds_in_pvs(&lumps, 1, eye, Vec3::ONE, Vec3::ZERO).is_err());
        lumps[5][4..8].copy_from_slice(&0i32.to_le_bytes());
        assert!(bounds_in_pvs(&lumps, 1, eye, Vec3::ZERO, Vec3::ONE).is_err());
        assert!(VisibilityIndex::new(&lumps, 1).is_err());
    }
    #[test]
    fn pvs_row_and_bounds_clusters_match_set_queries_and_reuse_scratch() {
        let mut lumps = vec![vec![]; 64];
        lumps[1] = vec![0; 20];
        lumps[1][0..4].copy_from_slice(&1f32.to_le_bytes());
        lumps[5] = vec![0; 32];
        lumps[5][4..8].copy_from_slice(&(-1i32).to_le_bytes());
        lumps[5][8..12].copy_from_slice(&(-2i32).to_le_bytes());
        lumps[10] = vec![0; 64];
        lumps[10][36..38].copy_from_slice(&1i16.to_le_bytes());
        lumps[14] = vec![0; 48];
        // Two clusters: cluster 0 sees {0}, cluster 1 sees {0, 1} via offset = -1 ("all").
        lumps[4] = vec![0; 21];
        lumps[4][0..4].copy_from_slice(&2i32.to_le_bytes());
        lumps[4][4..8].copy_from_slice(&20i32.to_le_bytes());
        lumps[4][12..16].copy_from_slice(&(-1i32).to_le_bytes());
        lumps[4][20] = 1;

        let index = VisibilityIndex::new(&lumps, 1).unwrap();
        for cluster in [0, 1] {
            let row = index.row(cluster).unwrap().unwrap();
            let set = pvs(&lumps[4], cluster).unwrap();
            for c in -2i16..=4 {
                assert_eq!(
                    row.contains(c),
                    set.contains(&c),
                    "row {cluster} cluster {c}"
                );
            }
        }
        assert!(index.row(-1).unwrap().is_none());

        let mut stack = Vec::new();
        let mut out = Vec::new();
        index
            .bounds_clusters(Vec3::X, Vec3::splat(2.), &mut stack, &mut out)
            .unwrap();
        assert_eq!(out, [0]);
        let stack_cap = stack.capacity();
        let out_ptr = out.as_ptr();
        index
            .bounds_clusters(Vec3::splat(-1.), Vec3::splat(1.), &mut stack, &mut out)
            .unwrap();
        assert_eq!(out, [0, 1]);
        assert_eq!(out.as_ptr(), out_ptr);
        assert!(stack.capacity() >= stack_cap);
        assert!(index
            .bounds_clusters(Vec3::ONE, Vec3::ZERO, &mut stack, &mut out)
            .is_err());
        assert!(index
            .bounds_clusters(Vec3::splat(f32::NAN), Vec3::ZERO, &mut stack, &mut out)
            .is_err());
    }
    #[test]
    #[ignore = "requires the user's installed HL2 content"]
    fn owned_station_cached_visibility_matches_raw_tree_queries() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for map in [
            "d1_trainstation_01",
            "d1_trainstation_02",
            "d1_trainstation_03",
        ] {
            let bsp =
                crate::bsp::Bsp::parse(&vfs.read(&format!("maps/{map}.bsp")).unwrap().unwrap())
                    .unwrap();
            let index = bsp.visibility_index().unwrap();
            let world = bsp.world(map).unwrap();
            let spawn = world.spawn().0;
            let (mut stack, mut clusters) = (Vec::new(), Vec::new());
            let mut checked_views = 0;
            // Several view clusters around the spawn; boxes reach across their leaf planes.
            for offset in [
                Vec3::ZERO,
                Vec3::X * 384.,
                -Vec3::Y * 512.,
                Vec3::new(-640., 256., 0.),
            ] {
                let eye = spawn + offset;
                let Some(row) = index.row(index.cluster(eye).unwrap()).unwrap() else {
                    continue;
                };
                checked_views += 1;
                for i in 0..400 {
                    let center = eye
                        + Vec3::new(
                            (i % 20) as f32 * 83. - 830.,
                            (i / 20) as f32 * 57. - 570.,
                            (i % 7) as f32 * 19. - 57.,
                        );
                    let radius = Vec3::new(
                        (i % 5) as f32 * 13.,
                        (i % 9) as f32 * 17.,
                        (i % 3) as f32 * 41.,
                    );
                    let (mins, maxs) = (center - radius, center + radius);
                    let raw = bsp.bounds_in_pvs(eye, mins, maxs).unwrap();
                    let walked = index.bounds_visible(mins, maxs, &row, &mut stack).unwrap();
                    index
                        .bounds_clusters(mins, maxs, &mut stack, &mut clusters)
                        .unwrap();
                    let listed = clusters.iter().any(|&c| row.contains(c));
                    assert_eq!(walked, raw, "{map} view {offset} bound {i}");
                    assert_eq!(listed, raw, "{map} view {offset} listed bound {i}");
                }
            }
            assert!(checked_views > 0, "{map} has no valid view cluster");
        }
    }
}
