//! BSP 19/20 world brush faces, displacement terrain, entity KeyValues, and solid brush planes.
use crate::{bytes, f32le, i16le, i32le, index, keyvalues, records, u16le, u32le, vec3};
use anyhow::{bail, Context, Result};
use glam::{Vec2, Vec3};
use modkit_core::{Brush, Plane, Surface, Vertex, World};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};

pub struct Bsp {
    pub version: u32,
    pub revision: u32,
    lumps: Vec<Vec<u8>>,
    pub lump_versions: Vec<u32>,
    pub static_props: Vec<modkit_core::ModelInstance>,
    lightmaps: Vec<modkit_core::Lightmap>,
    face_lighting: Vec<Option<crate::lighting::FaceLight>>,
    lightmap_styles: Vec<modkit_core::StyledLightmap>,
    /// Whether the HDR lighting copies are wanted (see [`crate::lighting::choose_hdr`]).
    hdr: bool,
}

/// Independent sky eligibility bits in the camera's containing BSP leaf.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct SkyVisibility {
    pub sky_3d: bool,
    pub sky_2d: bool,
}
impl SkyVisibility {
    /// A 3D sky also needs the distant 2D background behind its miniature geometry.
    pub fn background_visible(self) -> bool {
        self.sky_3d || self.sky_2d
    }
}
/// A lump as stored: raw, or LZMA-compressed (`LZMA` header).
fn decode_lump(raw: &[u8], i: usize) -> Result<Vec<u8>> {
    if !raw.starts_with(b"LZMA") {
        return Ok(raw.to_vec());
    }
    let expected = u32le(raw, 4)? as usize;
    let compressed = u32le(raw, 8)? as usize;
    if expected > 128 * 1024 * 1024 {
        bail!("BSP lump {i} exceeds 128 MiB limit");
    }
    let mut stream = bytes(raw, 12, 5)?.to_vec();
    stream.extend_from_slice(&(expected as u64).to_le_bytes());
    stream.extend_from_slice(bytes(raw, 17, compressed)?);
    let mut output = Vec::new();
    lzma_rs::lzma_decompress_with_options(
        &mut Cursor::new(stream),
        &mut output,
        &lzma_rs::decompress::Options {
            memlimit: Some(128 * 1024 * 1024),
            ..Default::default()
        },
    )
    .with_context(|| format!("LZMA lump {i}"))?;
    if output.len() != expected {
        bail!("BSP lump {i} decompressed size mismatch");
    }
    Ok(output)
}

mod changelevel;
pub use changelevel::read_changelevel_targets;

/// Reads and decodes lump 0 (the BSP entity text lump) from a BSP file without reading the rest of the map.
pub(super) fn read_entity_lump(path: &std::path::Path) -> Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).with_context(|| path.display().to_string())?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header)?;
    if &header[..4] != b"VBSP" {
        bail!("not a Valve BSP: {}", path.display());
    }
    let (offset, len) = (u32le(&header, 8)? as u64, u32le(&header, 12)? as usize);
    if len > 64 * 1024 * 1024 {
        bail!("BSP entity lump too large: {}", path.display());
    }
    let mut raw = vec![0; len];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut raw)?;
    decode_lump(&raw, 0)
}

/// The entities of a BSP file, reading only its header and entity lump (lump 0), not the map.
pub fn read_entities(path: &std::path::Path) -> Result<Vec<modkit_core::Entity>> {
    keyvalues::entities(&read_entity_lump(path)?)
}

impl Bsp {
    /// Creates an empty BSP with 64 empty lumps and version 20, representing no geometry.
    pub fn empty() -> Self {
        Self {
            lightmaps: Vec::new(),
            face_lighting: Vec::new(),
            lightmap_styles: Vec::new(),
            static_props: Vec::new(),
            version: 20,
            revision: 0,
            lumps: vec![Vec::new(); 64],
            lump_versions: vec![0; 64],
            hdr: true,
        }
    }
    pub fn parse(data: &[u8]) -> Result<Self> {
        Self::parse_with(data, true)
    }
    /// Parses a map for an HDR (`hdr`) or an LDR frame: the lightmaps, faces, ambient samples and
    /// world lights come from the matching lump copies.
    pub fn parse_with(data: &[u8], hdr: bool) -> Result<Self> {
        if bytes(data, 0, 4)? != b"VBSP" {
            bail!("not a Valve BSP");
        }
        bytes(data, 0, 1036)?;
        let version = u32le(data, 4)?;
        if version != 19 && version != 20 {
            bail!("unsupported BSP version {version}; reader supports HL2 BSP 19/20");
        }
        let mut lumps = Vec::new();
        let mut lump_versions = Vec::new();
        for i in 0..64 {
            let o = 8 + i * 16;
            let offset = u32le(data, o)? as usize;
            let len = u32le(data, o + 4)? as usize;
            lump_versions.push(u32le(data, o + 8)?);
            lumps.push(decode_lump(bytes(data, offset, len)?, i)?);
        }
        let (lightmaps, face_lighting) = crate::lighting::build(&lumps, hdr)?;
        let lightmap_styles = crate::lighting::styled_faces(&lumps, hdr, &face_lighting)?;
        Ok(Self {
            lightmaps,
            face_lighting,
            lightmap_styles,
            static_props: crate::models::static_props(data, &lumps[35])?,
            version,
            revision: u32le(data, 1032)?,
            lumps,
            lump_versions,
            hdr,
        })
    }
    pub fn lump(&self, id: usize) -> &[u8] {
        &self.lumps[id]
    }
    /// Leaf ambient samples, world lights and the leaf tree for model lighting.
    pub fn model_lighting(&self) -> Result<modkit_core::lighting::LightingData> {
        let hdr = crate::lighting::choose_hdr(&self.lumps, self.hdr);
        self.model_lighting_with(hdr)
    }
    pub fn model_lighting_with(&self, hdr: bool) -> Result<modkit_core::lighting::LightingData> {
        let mut data = crate::model_lighting::read_with(&self.lumps, &self.lump_versions, hdr)?;
        data.dynamic_minlight = self.dynamic_minlight(hdr);
        Ok(data)
    }
    /// LUMP_CUBEMAPS (42): where each sample was built, which also names its texture.
    pub fn cubemap_origins(&self) -> Vec<[i32; 3]> {
        let lump = self.lump(42);
        let samples = lump.as_chunks::<16>().0.iter();
        samples
            .map(|s| {
                std::array::from_fn(|i| {
                    i32::from_le_bytes([s[i * 4], s[i * 4 + 1], s[i * 4 + 2], s[i * 4 + 3]])
                })
            })
            .collect()
    }
    /// HDR lighting, LUMP_MAP_FLAGS (59) and the BSP version select the dynamic-light minlight
    /// floor (`modkit_core::lighting::dynamic_minlight_floor`).
    fn dynamic_minlight(&self, hdr: bool) -> f32 {
        let flags = self
            .lumps
            .get(59)
            .and_then(|l| l.get(..4))
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        modkit_core::lighting::dynamic_minlight_floor(self.version, flags, hdr)
    }
    pub fn visibility_index(&self) -> Result<crate::visibility::VisibilityIndex> {
        crate::visibility::VisibilityIndex::new(&self.lumps, self.lump_versions[10])
    }
    pub fn bounds_in_pvs(&self, eye: Vec3, mins: Vec3, maxs: Vec3) -> Result<bool> {
        crate::visibility::bounds_in_pvs(&self.lumps, self.lump_versions[10], eye, mins, maxs)
    }
    /// Query separate LEAF_FLAGS_SKY (3D) and LEAF_FLAGS_SKY2D eligibility.
    /// Missing/malformed node/leaf tables or a non-finite point return an error.
    pub fn sky_visibility(&self, point: Vec3) -> Result<SkyVisibility> {
        let leaf = crate::visibility::leaf(&self.lumps, self.lump_versions[10], point)?;
        // dleaf_t / dleaf_version_0_t pack area:9 and flags:7 into the word at +6.
        let flags = u16le(leaf, 6)? >> 9;
        Ok(SkyVisibility {
            sky_3d: flags & 0x01 != 0,
            sky_2d: flags & 0x04 != 0,
        })
    }
    /// Whether the current leaf permits miniature 3D sky scenery.
    /// A leaf with only 2D sky must not enable background models.
    pub fn sky_3d_visible(&self, point: Vec3) -> Result<bool> {
        Ok(self.sky_visibility(point)?.sky_3d)
    }
    pub fn world(&self, name: &str) -> Result<World> {
        let entities = keyvalues::entities(self.lump(0))?;
        let clusters = self.background_clusters(&entities)?;
        let tables = BspTables::parse(self)?;
        let mut world = self.model_world(name, 0, &tables, &clusters, entities)?;
        if let Some(e) = world.entities.iter().find(|e| e.class() == "sky_camera") {
            world.background_camera = Some(modkit_core::BackgroundCamera {
                origin: e.origin(),
                scale: e
                    .get("scale")
                    .and_then(|s| s.parse::<f32>().ok())
                    .filter(|s| s.is_finite() && *s > 0.)
                    .unwrap_or(16.),
            });
        }
        for instance in &mut world.model_instances {
            instance.background = clusters.contains(&crate::visibility::cluster(
                &self.lumps,
                self.lump_versions[10],
                instance.origin,
            )?);
        }
        for (id, e) in world.entities.iter().enumerate() {
            if clusters.contains(&crate::visibility::cluster(
                &self.lumps,
                self.lump_versions[10],
                e.origin(),
            )?) {
                world.background_entities.push(id);
            }
        }
        for (id, &record) in tables.models.iter().enumerate().skip(1) {
            let model = self.model_world(name, id, &tables, &clusters, Vec::new())?;
            world.brush_models.push(modkit_core::BrushModel {
                id,
                surfaces: model.surfaces,
                brushes: model.brushes,
                mins: vec3(record, 0)?,
                maxs: vec3(record, 12)?,
            });
        }
        match crate::physcollide::brush_model_solids(self.lump(29)) {
            Ok(solids) => world.brush_model_physics = solids,
            Err(e) => world.warnings.push(format!("PHYSCOLLIDE lump: {e:#}")),
        }
        match self.model_lighting_with(tables.use_hdr) {
            Ok(lighting) => world.lighting = Some(std::sync::Arc::new(lighting)),
            Err(e) => world.warnings.push(format!("model lighting data: {e:#}")),
        }
        Ok(world)
    }
    fn background_clusters(&self, entities: &[modkit_core::Entity]) -> Result<BTreeSet<i16>> {
        let Some(e) = entities.iter().find(|e| e.class() == "sky_camera") else {
            return Ok(BTreeSet::new());
        };
        crate::visibility::pvs(
            self.lump(4),
            crate::visibility::cluster(&self.lumps, self.lump_versions[10], e.origin())?,
        )
    }
    /// The atlas pages and styled faces, carried by the world model (model 0) only.
    fn lightmap_fields(
        &self,
        model_index: usize,
    ) -> (Vec<modkit_core::Lightmap>, Vec<modkit_core::StyledLightmap>) {
        if model_index == 0 {
            (self.lightmaps.clone(), self.lightmap_styles.clone())
        } else {
            Default::default()
        }
    }
    fn model_world(
        &self,
        name: &str,
        model_index: usize,
        tables: &BspTables<'_>,
        background_clusters: &BTreeSet<i16>,
        entities: Vec<modkit_core::Entity>,
    ) -> Result<World> {
        let (lightmaps, lightmap_styles) = self.lightmap_fields(model_index);
        let mut world = World {
            name: name.into(),
            lightmaps,
            lightmap_styles,
            entities,
            model_instances: if model_index == 0 {
                self.static_props.clone()
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        let model = tables
            .models
            .get(model_index)
            .context("BSP missing model")?;
        let background_faces = if model_index == 0 {
            crate::visibility::faces(&self.lumps, self.lump_versions[10], background_clusters)?
        } else {
            BTreeSet::new()
        };
        let overlays = if model_index == 0 {
            match crate::overlays::read(self.lump(45)) {
                Ok(overlays) => overlays,
                Err(e) => {
                    world.warnings.push(format!("info_overlay records: {e:#}"));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        let overlay_faces: BTreeSet<usize> = overlays
            .iter()
            .flat_map(|o| o.faces.iter().filter_map(|&f| usize::try_from(f).ok()))
            .collect();
        let built = self.build_model_surfaces(
            tables,
            model,
            &background_faces,
            &overlay_faces,
            &mut world.no_shadow_materials,
        )?;
        world.surfaces = built.batches.into_values().collect();
        world.terrain = built.terrain;
        build_overlays(
            tables,
            &overlays,
            &built.overlay_targets,
            &mut world.overlays,
            &mut world.warnings,
        );
        collect_model_brushes(
            tables,
            model,
            model_index == 0,
            &mut world.brushes,
            &mut world.brush_side_materials,
        )?;
        if model_index == 0 {
            world.warnings.push(format!("{} displacement surfaces have triangle collision; neighbor seams still need validation", built.displacement_count));
            world.warnings.push(
                "Source campaign fidelity remains incomplete; see README and entity diagnostics"
                    .into(),
            );
        }
        Ok(world)
    }

    fn build_model_surfaces(
        &self,
        tables: &BspTables<'_>,
        model: &[u8],
        background_faces: &BTreeSet<usize>,
        overlay_faces: &BTreeSet<usize>,
        no_shadow_materials: &mut BTreeSet<String>,
    ) -> Result<SurfaceBuildOut> {
        let first = i32le(model, 40)?;
        let count = i32le(model, 44)?;
        if first < 0
            || count < 0
            || (first as usize)
                .checked_add(count as usize)
                .is_none_or(|end| end > tables.faces.len())
        {
            bail!("world face range outside faces");
        }
        let mut out = SurfaceBuildOut::default();
        for (local_face, f) in tables.faces[first as usize..(first + count) as usize]
            .iter()
            .enumerate()
        {
            let face_index = first as usize + local_face;
            self.append_model_face(
                tables,
                f,
                face_index,
                (
                    background_faces.contains(&face_index),
                    overlay_faces.contains(&face_index),
                ),
                no_shadow_materials,
                &mut out,
            )?;
        }
        Ok(out)
    }

    fn append_model_face(
        &self,
        tables: &BspTables<'_>,
        f: &[u8],
        face_index: usize,
        (is_background, is_overlay): (bool, bool),
        no_shadow_materials: &mut BTreeSet<String>,
        out: &mut SurfaceBuildOut,
    ) -> Result<()> {
        let face_light = self.face_lighting[face_index];
        let ti = i16le(f, 10)?;
        if ti < 0 {
            return Ok(());
        }
        let info = tables.texinfo[index(ti as i32, tables.texinfo.len())?];
        let flags = u32le(info, 64)?;
        // Sky and nodraw faces are not world surfaces. No Source shaders are executed.
        if flags & (0x2 | 0x4 | 0x80) != 0 {
            return Ok(());
        }
        let td = tables.texdata[index(i32le(info, 68)?, tables.texdata.len())?];
        let material = tables.texinfo_material(ti)?;
        // Compile tools are invisible, except toolsblack (an UnlitGeneric black texture),
        // e.g. the frame around trainstation_02's Combine slate.
        if material.starts_with("tools/") && !material.starts_with("tools/toolsblack") {
            return Ok(());
        }
        // SURF_NOSHADOWS (vbsp sets it on water): no projected shadow lands here.
        if flags & 0x1000 != 0 && !no_shadow_materials.contains(&material) {
            no_shadow_materials.insert(material.clone());
        }
        let Some((mut polygon, normal, face_normal)) = face_polygon(tables, f)? else {
            return Ok(());
        };
        let disp = i16le(f, 12)?;
        if disp < 0 && is_overlay {
            out.overlay_targets.insert(
                face_index,
                OverlayTarget {
                    polygon: polygon.clone(),
                    normal: face_normal,
                    light: LightMapping {
                        s: vec3(info, 32)?,
                        s_offset: f32le(info, 44)? - i32le(f, 28)? as f32,
                        t: vec3(info, 48)?,
                        t_offset: f32le(info, 60)? - i32le(f, 32)? as f32,
                        face: face_light,
                    },
                    background: is_background,
                },
            );
        }
        let batch = out
            .batches
            .entry((material.clone(), face_light.map(|l| l.page), is_background))
            .or_insert_with(|| Surface {
                flex_source: None,
                background: is_background,
                material,
                vertices: Vec::new(),
                indices: Vec::new(),
                lightmap: face_light.map(|l| l.page),
            });
        let tex_size = (i32le(td, 16)?.max(1) as f32, i32le(td, 20)?.max(1) as f32);
        let ctx = FaceContext {
            f,
            info,
            tex_size,
            normal,
            face_normal,
            face_light,
        };
        if disp >= 0 {
            append_displacement_surface(tables, batch, &mut out.terrain, &mut polygon, ctx, disp)?;
            out.displacement_count += 1;
        } else {
            append_brush_face_surface(tables, batch, &polygon, ctx)?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct SurfaceBuildOut {
    batches: BTreeMap<(String, Option<usize>, bool), Surface>,
    terrain: Vec<Surface>,
    overlay_targets: BTreeMap<usize, OverlayTarget>,
    displacement_count: usize,
}

/// Shared parsed BSP lumps reused across the world model and all brush models in `Bsp::world`.
struct BspTables<'a> {
    positions: Vec<Vec3>,
    edges: Vec<[usize; 2]>,
    surfedges: Vec<i32>,
    texinfo: Vec<&'a [u8]>,
    texdata: Vec<&'a [u8]>,
    strings: &'a [u8],
    table: &'a [u8],
    models: Vec<&'a [u8]>,
    faces: Vec<&'a [u8]>,
    lighting: &'a [u8],
    disps: Vec<&'a [u8]>,
    dispverts: Vec<&'a [u8]>,
    use_hdr: bool,
    planes: Vec<Plane>,
    sides: Vec<&'a [u8]>,
    brushes: Vec<&'a [u8]>,
    nodes: Vec<&'a [u8]>,
    leaves: Vec<&'a [u8]>,
    leafbrushes: Vec<u16>,
}

impl<'a> BspTables<'a> {
    fn parse(bsp: &'a Bsp) -> Result<Self> {
        let use_hdr = crate::lighting::choose_hdr(&bsp.lumps, bsp.hdr);
        let leaf_size = if bsp.lump_versions[10] == 0 { 56 } else { 32 };
        Ok(Self {
            positions: records(bsp.lump(3), 12)?
                .map(|v| vec3(v, 0))
                .collect::<Result<Vec<_>>>()?,
            edges: records(bsp.lump(12), 4)?
                .map(|v| Ok([u16le(v, 0)? as usize, u16le(v, 2)? as usize]))
                .collect::<Result<Vec<_>>>()?,
            surfedges: records(bsp.lump(13), 4)?
                .map(|v| i32le(v, 0))
                .collect::<Result<Vec<_>>>()?,
            texinfo: records(bsp.lump(6), 72)?.collect(),
            texdata: records(bsp.lump(2), 32)?.collect(),
            strings: bsp.lump(43),
            table: bsp.lump(44),
            models: records(bsp.lump(14), 48)?.collect(),
            faces: records(bsp.lump(if use_hdr { 58 } else { 7 }), 56)?.collect(),
            lighting: bsp.lump(if use_hdr { 53 } else { 8 }),
            disps: records(bsp.lump(26), 176)?.collect(),
            dispverts: records(bsp.lump(33), 20)?.collect(),
            use_hdr,
            planes: records(bsp.lump(1), 20)?
                .map(|p| {
                    Ok(Plane {
                        normal: vec3(p, 0)?,
                        distance: f32le(p, 12)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            sides: records(bsp.lump(19), 8)?.collect(),
            brushes: records(bsp.lump(18), 12)?.collect(),
            nodes: records(bsp.lump(5), 32)?.collect(),
            leaves: records(bsp.lump(10), leaf_size)?.collect(),
            leafbrushes: records(bsp.lump(17), 2)?
                .map(|r| u16le(r, 0))
                .collect::<Result<Vec<_>>>()?,
        })
    }

    fn texinfo_material(&self, ti: i16) -> Result<String> {
        texinfo_material(&self.texinfo, &self.texdata, self.strings, self.table, ti)
    }
}

struct FaceContext<'a> {
    f: &'a [u8],
    info: &'a [u8],
    tex_size: (f32, f32),
    normal: Vec3,
    face_normal: Vec3,
    face_light: Option<crate::lighting::FaceLight>,
}

fn face_polygon(tables: &BspTables<'_>, f: &[u8]) -> Result<Option<(Vec<Vec3>, Vec3, Vec3)>> {
    let firstedge = i32le(f, 4)?;
    let numedges = i16le(f, 8)?;
    if firstedge < 0
        || numedges < 0
        || firstedge as usize + numedges as usize > tables.surfedges.len()
    {
        bail!("face edge range outside surfedges");
    }
    let mut polygon = Vec::new();
    for &edge in &tables.surfedges[firstedge as usize..firstedge as usize + numedges as usize] {
        let edge_id = edge.checked_abs().context("edge index overflow")?;
        let e = tables.edges[index(edge_id, tables.edges.len())?];
        let vertex = e[usize::from(edge < 0)];
        polygon.push(
            *tables
                .positions
                .get(vertex)
                .context("edge vertex outside vertices")?,
        );
    }
    if polygon.len() < 3 {
        return Ok(None);
    }
    let normal = (polygon[1] - polygon[0])
        .cross(polygon[2] - polygon[0])
        .normalize_or_zero();
    // Shading normal (envmap reflections): the face plane, oriented toward the side the
    // clockwise Source winding faces. In owned maps that is the stored plane for both
    // `side` values (d1_trainstation_02: 6597 of 6601 brush faces), so the winding decides
    // and `side` only breaks ties for degenerate windings. Displacements reuse their
    // base face's normal (an approximation).
    let plane = tables
        .planes
        .get(usize::from(u16le(f, 0)?))
        .context("face plane outside planes")?
        .normal;
    // Summed fan area: robust where the first three vertices are nearly collinear.
    let area = (1..polygon.len() - 1).fold(Vec3::ZERO, |sum, i| {
        sum + (polygon[i] - polygon[0]).cross(polygon[i + 1] - polygon[0])
    });
    let winding = plane.dot(area);
    let face_normal = if winding > 0. || (winding == 0. && bytes(f, 2, 1)?[0] != 0) {
        -plane
    } else {
        plane
    };
    Ok(Some((polygon, normal, face_normal)))
}

fn face_uv(info: &[u8], (width, height): (f32, f32), p: Vec3) -> Result<Vec2> {
    let s = vec3(info, 0)?;
    let so = f32le(info, 12)?;
    let t = vec3(info, 16)?;
    let to = f32le(info, 28)?;
    Ok(Vec2::new((s.dot(p) + so) / width, (t.dot(p) + to) / height))
}

fn face_color(f: &[u8], info: &[u8], lighting: &[u8], normal: Vec3, p: Vec3) -> Result<[u8; 4]> {
    let lightofs = i32le(f, 20)?;
    if lightofs < 0 || lighting.is_empty() {
        let v = (170. + normal.z.abs() * 55.) as u8;
        return Ok([v, v, v, 255]);
    }
    let x = (vec3(info, 32)?.dot(p) + f32le(info, 44)? - i32le(f, 28)? as f32)
        .round()
        .clamp(0., i32le(f, 36)?.max(0) as f32) as usize;
    let y = (vec3(info, 48)?.dot(p) + f32le(info, 60)? - i32le(f, 32)? as f32)
        .round()
        .clamp(0., i32le(f, 40)?.max(0) as f32) as usize;
    let stride = usize::try_from(i32le(f, 36)?.max(0))? + 1;
    let sample = bytes(lighting, lightofs as usize + 4 * (y * stride + x), 4)?;
    let scale = 2f32.powi(sample[3] as i8 as i32);
    let mut rgb = [0u8; 4];
    for i in 0..3 {
        rgb[i] =
            ((sample[i] as f32 * scale / 255.).max(0.04).powf(1. / 2.2) * 255.).min(255.) as u8;
    }
    rgb[3] = 255;
    Ok(rgb)
}

fn face_light_uv(
    f: &[u8],
    info: &[u8],
    face_light: Option<crate::lighting::FaceLight>,
    p: Vec3,
) -> Result<Vec2> {
    if let Some(l) = face_light {
        let x = (vec3(info, 32)?.dot(p) + f32le(info, 44)? - i32le(f, 28)? as f32)
            .clamp(0., (l.width - 1) as f32);
        let y = (vec3(info, 48)?.dot(p) + f32le(info, 60)? - i32le(f, 32)? as f32)
            .clamp(0., (l.height - 1) as f32);
        Ok(Vec2::new(
            (l.x as f32 + x + 0.5) / 1024.,
            (l.y as f32 + y + 0.5) / 1024.,
        ))
    } else {
        Ok(Vec2::ZERO)
    }
}

fn append_displacement_surface(
    tables: &BspTables<'_>,
    batch: &mut Surface,
    terrain: &mut Vec<Surface>,
    polygon: &mut [Vec3],
    ctx: FaceContext<'_>,
    disp: i16,
) -> Result<()> {
    if polygon.len() != 4 {
        bail!("displacement base is not a quad");
    }
    let d = tables.disps[index(disp as i32, tables.disps.len())?];
    let power = i32le(d, 20)?;
    if !(2..=4).contains(&power) {
        bail!("invalid displacement power {power}");
    }
    let start = vec3(d, 0)?;
    let corner = (0..4)
        .min_by(|&a, &b| {
            polygon[a]
                .distance_squared(start)
                .total_cmp(&polygon[b].distance_squared(start))
        })
        .unwrap();
    polygon.rotate_left(corner);
    let cells = 1usize << power;
    let side = cells + 1;
    let base = batch.vertices.len() as u32;
    let dvstart = i32le(d, 12)?;
    for y in 0..side {
        for x in 0..side {
            let fy = y as f32 / cells as f32;
            let fx = x as f32 / cells as f32;
            let p = polygon[0]
                .lerp(polygon[1], fy)
                .lerp(polygon[3].lerp(polygon[2], fy), fx);
            let dv =
                tables.dispverts[index(dvstart + (y * side + x) as i32, tables.dispverts.len())?];
            let pos = p + vec3(dv, 0)? * f32le(dv, 12)?;
            let [r, g, b, _] = match ctx.face_light {
                Some(_) => [255; 4],
                None => face_color(ctx.f, ctx.info, tables.lighting, ctx.normal, p)?,
            };
            batch.vertices.push(Vertex {
                normal: ctx.face_normal,
                position: pos,
                uv: face_uv(ctx.info, ctx.tex_size, p)?,
                color: [r, g, b, disp_alpha(f32le(dv, 16)?)],
                skin: None,
                light_uv: ctx.face_light.map_or(Vec2::ZERO, |l| l.disp_uv(fx, fy)),
            });
        }
    }
    for cell in (0..cells).flat_map(|y| (0..cells).map(move |x| y * side + x)) {
        let triangles = disp_cell_triangles(cell as u32, side as u32);
        batch.indices.extend(triangles.iter().map(|i| base + i));
    }
    let count = cells * cells * 6;
    terrain.push(Surface {
        flex_source: None,
        background: batch.background,
        lightmap: batch.lightmap,
        material: batch.material.clone(),
        vertices: batch.vertices[base as usize..].to_vec(),
        indices: batch.indices[batch.indices.len() - count..]
            .iter()
            .map(|i| i - base)
            .collect(),
    });
    Ok(())
}

fn append_brush_face_surface(
    tables: &BspTables<'_>,
    batch: &mut Surface,
    polygon: &[Vec3],
    ctx: FaceContext<'_>,
) -> Result<()> {
    let base = batch.vertices.len() as u32;
    for p in polygon.iter().copied() {
        batch.vertices.push(Vertex {
            normal: ctx.face_normal,
            position: p,
            uv: face_uv(ctx.info, ctx.tex_size, p)?,
            color: if ctx.face_light.is_some() {
                [255; 4]
            } else {
                face_color(ctx.f, ctx.info, tables.lighting, ctx.normal, p)?
            },
            skin: None,
            light_uv: face_light_uv(ctx.f, ctx.info, ctx.face_light, p)?,
        });
    }
    for i in 1..polygon.len() - 1 {
        batch
            .indices
            .extend_from_slice(&[base, base + i as u32, base + i as u32 + 1]);
    }
    Ok(())
}

fn build_overlays(
    tables: &BspTables<'_>,
    overlays: &[crate::overlays::Overlay],
    overlay_targets: &BTreeMap<usize, OverlayTarget>,
    out_overlays: &mut Vec<Surface>,
    warnings: &mut Vec<String>,
) {
    for overlay in overlays {
        let material = match tables.texinfo_material(overlay.texinfo) {
            Ok(material) => material,
            Err(e) => {
                warnings.push(format!("info_overlay {}: {e:#}", overlay.id));
                continue;
            }
        };
        let mut pieces: BTreeMap<(Option<usize>, bool), Surface> = BTreeMap::new();
        for target in overlay
            .faces
            .iter()
            .filter_map(|&f| overlay_targets.get(&usize::try_from(f).ok()?))
        {
            for mut polygon in overlay.fragments(&target.polygon, target.normal) {
                // Counter-clockwise around the face normal (the host's front face; world
                // faces get the same orientation from their winding normalization).
                let area = (1..polygon.len() - 1).fold(Vec3::ZERO, |sum, i| {
                    sum + (polygon[i].position - polygon[0].position)
                        .cross(polygon[i + 1].position - polygon[0].position)
                });
                if area.dot(target.normal) < 0. {
                    polygon.reverse();
                }
                let page = target.light.face.map(|l| l.page);
                let surface = pieces
                    .entry((page, target.background))
                    .or_insert_with(|| Surface {
                        flex_source: None,
                        background: target.background,
                        material: material.clone(),
                        vertices: Vec::new(),
                        indices: Vec::new(),
                        lightmap: page,
                    });
                let base = surface.vertices.len() as u32;
                surface.vertices.extend(polygon.iter().map(|v| Vertex {
                    normal: target.normal,
                    position: v.position,
                    uv: v.uv,
                    color: [255; 4],
                    skin: None,
                    light_uv: target.light.uv(v.position),
                }));
                for i in 1..polygon.len() as u32 - 1 {
                    surface.indices.extend([base, base + i, base + i + 1]);
                }
            }
        }
        out_overlays.extend(pieces.into_values());
    }
}

fn collect_model_brushes(
    tables: &BspTables<'_>,
    model: &[u8],
    is_world_model: bool,
    out_brushes: &mut Vec<Brush>,
    out_side_materials: &mut Vec<Vec<String>>,
) -> Result<()> {
    let mut stack = vec![i32le(model, 36)?];
    let mut visited = BTreeSet::new();
    let mut world_brushes = BTreeSet::new();
    while let Some(node) = stack.pop() {
        if !visited.insert(node) {
            continue;
        }
        if node >= 0 {
            let n = tables.nodes[index(node, tables.nodes.len())?];
            stack.push(i32le(n, 4)?);
            stack.push(i32le(n, 8)?);
        } else {
            let leaf = tables.leaves[index(
                node.checked_neg().context("leaf index overflow")? - 1,
                tables.leaves.len(),
            )?];
            let start = u16le(leaf, 24)? as usize;
            let count = u16le(leaf, 26)? as usize;
            for &brush in tables
                .leafbrushes
                .get(start..start + count)
                .context("leaf brush range")?
            {
                world_brushes.insert(brush as usize);
            }
        }
    }
    for id in world_brushes {
        let b = tables.brushes.get(id).context("brush index")?;
        let contents = u32le(b, 8)?;
        // World brushes that block anything (MASK_PLAYERSOLID / MASK_NPCSOLID /
        // MASK_SOLID): solid, window (0x2), grate (0x8: chain-link and scissor gates)
        // and player/monster clips; water, ladders and the like stay out.
        if is_world_model && contents & (1 | 0x2 | 0x8 | 0x10000 | 0x20000) == 0 {
            continue;
        }
        let start = usize::try_from(i32le(b, 0)?)?;
        let count = usize::try_from(i32le(b, 4)?)?;
        let mut ps = Vec::new();
        let mut side_materials = Vec::new();
        for side in tables
            .sides
            .get(start..start + count)
            .context("brush side range")?
        {
            ps.push(
                *tables
                    .planes
                    .get(u16le(side, 0)? as usize)
                    .context("brush plane index")?,
            );
            // dbrushside_t texinfo (offset 2); -1 (TEXINFO_NODE) has no material.
            let ti = i16::from_le_bytes([side[2], side[3]]);
            side_materials.push(if ti < 0 {
                String::new()
            } else {
                tables.texinfo_material(ti).unwrap_or_default()
            });
        }
        out_side_materials.push(side_materials);
        out_brushes.push(Brush {
            planes: ps,
            contents,
        });
    }
    Ok(())
}

/// A world brush face that info_overlays project onto.
struct OverlayTarget {
    polygon: Vec<Vec3>,
    normal: Vec3,
    light: LightMapping,
    background: bool,
}

/// The face's lightmap texture vectors relative to its lightmap mins, and its atlas rect.
struct LightMapping {
    s: Vec3,
    s_offset: f32,
    t: Vec3,
    t_offset: f32,
    face: Option<crate::lighting::FaceLight>,
}
impl LightMapping {
    /// Same mapping as world faces: luxel coordinates clamped to the face's rect, centered.
    fn uv(&self, p: Vec3) -> Vec2 {
        let Some(l) = self.face else {
            return Vec2::ZERO;
        };
        let x = (self.s.dot(p) + self.s_offset).clamp(0., (l.width - 1) as f32);
        let y = (self.t.dot(p) + self.t_offset).clamp(0., (l.height - 1) as f32);
        Vec2::new(
            (l.x as f32 + x + 0.5) / 1024.,
            (l.y as f32 + y + 0.5) / 1024.,
        )
    }
}

/// Material name of a texinfo (texdata string table entry), lower-case.
/// The two triangles of the displacement cell whose first vertex is `index` in a grid `side`
/// vertices wide. The diagonal alternates in a checkerboard by the index's parity, as the
/// engine, the lighting compiler and collision split it (Source SDK `public/builddisp.cpp`
/// `GetTriangleIndicesForDispBBox`); a single diagonal reshapes the terrain between vertices.
fn disp_cell_triangles(index: u32, side: u32) -> [u32; 6] {
    let (a, b) = (index, index + side);
    if index % 2 == 1 {
        [a, b, a + 1, a + 1, b, b + 1]
    } else {
        [a, b, b + 1, a, b + 1, a + 1]
    }
}

/// A dispvert's blend alpha (authored 0..255) as a vertex colour alpha: how much of
/// `$basetexture2` shows there.
fn disp_alpha(alpha: f32) -> u8 {
    if alpha.is_finite() {
        alpha.round().clamp(0., 255.) as u8
    } else {
        0
    }
}

fn texinfo_material(
    texinfo: &[&[u8]],
    texdata: &[&[u8]],
    strings: &[u8],
    table: &[u8],
    ti: i16,
) -> Result<String> {
    let info = texinfo[index(i32::from(ti), texinfo.len())?];
    let td = texdata[index(i32le(info, 68)?, texdata.len())?];
    let offset = u32le(table, index(i32le(td, 12)?, table.len() / 4)? * 4)? as usize;
    let tail = strings
        .get(offset..)
        .context("texture string outside lump")?;
    let end = tail
        .iter()
        .position(|b| *b == 0)
        .context("unterminated texture string")?;
    Ok(std::str::from_utf8(&tail[..end])?.to_lowercase())
}

#[cfg(test)]
mod brush_set_tests;
#[cfg(test)]
mod displacement_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bad_headers_are_rejected() {
        assert!(Bsp::parse(b"VBSP").is_err());
        let mut b = vec![0; 1036];
        b[..4].copy_from_slice(b"VBSP");
        b[4..8].copy_from_slice(&21u32.to_le_bytes());
        assert!(Bsp::parse(&b).is_err());
    }
    #[test]
    fn lump_bounds_are_checked() {
        let mut b = vec![0; 1036];
        b[..4].copy_from_slice(b"VBSP");
        b[4..8].copy_from_slice(&20u32.to_le_bytes());
        b[8..12].copy_from_slice(&1035u32.to_le_bytes());
        b[12..16].copy_from_slice(&20u32.to_le_bytes());
        assert!(Bsp::parse(&b).is_err());
    }
    fn sky_test_bsp(leaf_version: u32) -> Bsp {
        let mut lumps = vec![Vec::new(); 64];
        lumps[1] = vec![0; 20];
        lumps[1][..4].copy_from_slice(&1f32.to_le_bytes());
        lumps[5] = vec![0; 32];
        lumps[5][4..8].copy_from_slice(&(-1i32).to_le_bytes());
        lumps[5][8..12].copy_from_slice(&(-2i32).to_le_bytes());
        let size = if leaf_version == 0 { 56 } else { 32 };
        lumps[10] = vec![0; 2 * size];
        // Distinguish packed area bits from 3D/2D sky flags on opposite sides of x=0.
        lumps[10][6..8].copy_from_slice(&(511u16 | (1 << 9)).to_le_bytes());
        lumps[10][size + 6..size + 8].copy_from_slice(&(4u16 << 9).to_le_bytes());
        lumps[14] = vec![0; 48];
        let mut lump_versions = vec![0; 64];
        lump_versions[10] = leaf_version;
        Bsp {
            version: 20,
            revision: 0,
            lumps,
            lump_versions,
            static_props: Vec::new(),
            lightmaps: Vec::new(),
            face_lighting: Vec::new(),
            lightmap_styles: Vec::new(),
            hdr: true,
        }
    }
    #[test]
    fn sky_visibility_tracks_current_leaf_and_separates_2d_sky() {
        for version in [0, 1] {
            let bsp = sky_test_bsp(version);
            assert!(bsp.sky_3d_visible(Vec3::X).unwrap());
            assert!(bsp.sky_3d_visible(Vec3::ZERO).unwrap());
            assert!(!bsp.sky_3d_visible(-Vec3::X).unwrap());
            assert_eq!(
                bsp.sky_visibility(Vec3::X).unwrap(),
                SkyVisibility {
                    sky_3d: true,
                    sky_2d: false
                }
            );
            assert_eq!(
                bsp.sky_visibility(-Vec3::X).unwrap(),
                SkyVisibility {
                    sky_3d: false,
                    sky_2d: true
                }
            );
        }
    }
    #[test]
    fn sky_visibility_separates_neither_2d_3d_and_both_flags() {
        for version in [0, 1] {
            let mut bsp = sky_test_bsp(version);
            for (flags, sky_3d, sky_2d) in [
                (0, false, false),
                (1, true, false),
                (4, false, true),
                (5, true, true),
            ] {
                // All area bits and unrelated sky-rad bits must not alter eligibility.
                let packed = 511u16 | ((flags | 2) << 9);
                bsp.lumps[10][6..8].copy_from_slice(&packed.to_le_bytes());
                let visibility = bsp.sky_visibility(Vec3::X).unwrap();
                assert_eq!(visibility, SkyVisibility { sky_3d, sky_2d });
                assert_eq!(visibility.background_visible(), sky_3d || sky_2d);
                assert_eq!(bsp.sky_3d_visible(Vec3::X).unwrap(), sky_3d);
            }
        }
    }
    #[test]
    fn sky_visibility_rejects_missing_leaves_cycles_and_invalid_points() {
        let mut bsp = sky_test_bsp(1);
        assert!(bsp.sky_3d_visible(Vec3::splat(f32::NAN)).is_err());
        assert!(bsp.sky_3d_visible(Vec3::splat(f32::INFINITY)).is_err());
        assert!(bsp.sky_visibility(Vec3::splat(f32::NAN)).is_err());
        assert!(bsp.sky_visibility(Vec3::splat(f32::INFINITY)).is_err());
        bsp.lumps[5][4..8].copy_from_slice(&0i32.to_le_bytes());
        assert!(bsp.sky_3d_visible(Vec3::X).is_err());
        bsp.lumps[5][4..8].copy_from_slice(&(-1i32).to_le_bytes());
        bsp.lumps[10].clear();
        assert!(bsp.sky_3d_visible(Vec3::X).is_err());
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_trainstation_brush_normals_face_the_rendered_side() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        let bytes = vfs.read("maps/d1_trainstation_02.bsp").unwrap().unwrap();
        let world = Bsp::parse(&bytes)
            .unwrap()
            .world("d1_trainstation_02")
            .unwrap();
        let (mut front, mut total) = (0, 0);
        for s in &world.surfaces {
            for t in s.indices.as_chunks::<3>().0 {
                let [a, b, c] = t.map(|i| s.vertices[i as usize].position);
                let n = s.vertices[t[0] as usize].normal;
                assert!((n.length() - 1.).abs() < 1e-3);
                let cross = (b - a).cross(c - a);
                // Brush triangles lie in their plane (displacement cells do not).
                let d = cross.normalize_or_zero().dot(n);
                if cross.length() > 1e-2 && d.abs() > 0.99 {
                    total += 1;
                    // Clockwise Source windings: the cross product points behind the face.
                    front += usize::from(d < 0.);
                }
            }
        }
        assert!(front * 100 > total * 99, "{front}/{total}");
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_trainstation_overlays_become_lightmapped_fragments() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        let bytes = vfs.read("maps/d1_trainstation_02.bsp").unwrap().unwrap();
        let bsp = Bsp::parse(&bytes).unwrap();
        let world = bsp.world("d1_trainstation_02").unwrap();
        let triangles: usize = world.overlays.iter().map(|s| s.indices.len() / 3).sum();
        assert!(triangles > 0);
        let pages = world.lightmaps.len();
        for s in &world.overlays {
            assert!(s.lightmap.is_none_or(|p| p < pages));
            assert!(s
                .vertices
                .iter()
                .all(|v| v.position.is_finite() && v.uv.is_finite()));
        }
        let materials: BTreeSet<_> = world.overlays.iter().map(|s| s.material.as_str()).collect();
        eprintln!(
            "{} overlay surfaces, {triangles} triangles, materials {materials:?}",
            world.overlays.len()
        );
    }
    #[test]
    fn empty_bsp_has_empty_lumps() {
        let bsp = Bsp::empty();
        assert_eq!(bsp.version, 20);
        assert_eq!(bsp.lumps.len(), 64);
        assert!(bsp.lumps.iter().all(|l| l.is_empty()));
    }
}
