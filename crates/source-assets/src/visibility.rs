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
    let visible = pvs(
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
        root,
        mins,
        maxs,
        &visible,
        true,
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

#[derive(Clone, Copy)]
struct Node {
    normal: Vec3,
    distance: f32,
    children: [i32; 2],
}
fn walk_bounds(
    root: i32,
    mins: Vec3,
    maxs: Vec3,
    visible: &BTreeSet<i16>,
    check_cycles: bool,
    mut node: impl FnMut(i32) -> Result<Node>,
    mut leaf: impl FnMut(i32) -> Result<i16>,
) -> Result<bool> {
    if !mins.is_finite() || !maxs.is_finite() || mins.cmpgt(maxs).any() {
        bail!("invalid BSP visibility bounds")
    }
    let center = (mins + maxs) * 0.5;
    let extent = (maxs - mins) * 0.5;
    if !center.is_finite() || !extent.is_finite() {
        bail!("BSP visibility bounds overflow")
    }
    let mut pending = vec![root];
    let mut visited = check_cycles.then(BTreeSet::new);
    while let Some(id) = pending.pop() {
        if id < 0 {
            if visible.contains(&leaf(id.checked_neg().context("leaf overflow")? - 1)?) {
                return Ok(true);
            }
            continue;
        }
        if visited.as_mut().is_some_and(|visited| !visited.insert(id)) {
            bail!("cyclic BSP visibility tree")
        }
        let node = node(id)?;
        let distance = node.normal.dot(center) - node.distance;
        let radius = node.normal.abs().dot(extent);
        if distance + radius >= 0. {
            pending.push(node.children[0]);
        }
        if distance - radius < 0. {
            pending.push(node.children[1]);
        }
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
    /// No valid PVS row means the renderer must conservatively keep geometry.
    pub fn row(&self, cluster: i16) -> Result<Option<BTreeSet<i16>>> {
        if cluster < 0 || self.visibility.is_empty() {
            return Ok(None);
        }
        Ok(Some(pvs(&self.visibility, cluster)?))
    }
    pub fn bounds_visible(&self, mins: Vec3, maxs: Vec3, visible: &BTreeSet<i16>) -> Result<bool> {
        walk_bounds(
            self.root,
            mins,
            maxs,
            visible,
            false,
            |id| Ok(self.nodes[id as usize]),
            |id| Ok(self.clusters[id as usize]),
        )
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

pub fn pvs(data: &[u8], cluster: i16) -> Result<BTreeSet<i16>> {
    if cluster < 0 || data.is_empty() {
        return Ok(BTreeSet::new());
    }
    let count = usize::try_from(i32le(data, 0)?)?;
    if count > 32768 {
        bail!("PVS cluster limit");
    }
    let id = index(cluster as i32, count)?;
    let offset = i32le(data, 4 + id * 8)?;
    if offset < 0 {
        return Ok((0..count).map(|i| i as i16).collect());
    }
    let mut i = offset as usize;
    let row = decompress(data, &mut i, count.div_ceil(8))?;
    Ok((0..count)
        .filter(|&c| row[c / 8] & (1 << (c % 8)) != 0)
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
            .bounds_visible(Vec3::X, Vec3::splat(2.), &row)
            .unwrap());
        assert!(!cached
            .bounds_visible(Vec3::splat(-2.), -Vec3::X, &row)
            .unwrap());
        assert!(cached
            .bounds_visible(Vec3::splat(-1.), Vec3::splat(1.), &row)
            .unwrap());
        assert!(cached.bounds_visible(Vec3::ONE, Vec3::ZERO, &row).is_err());
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
            let eye = world.spawn().0;
            let cluster = index.cluster(eye).unwrap();
            let row = index.row(cluster).unwrap().unwrap();
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
                assert_eq!(
                    index
                        .bounds_visible(center - radius, center + radius, &row)
                        .unwrap(),
                    bsp.bounds_in_pvs(eye, center - radius, center + radius)
                        .unwrap(),
                    "{map} bound {i}"
                );
            }
        }
    }
}
