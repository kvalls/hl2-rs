//! Bounded MDL facial flex reader: descriptors, controllers, rules and mesh vertex deltas.
//! Rule evaluation follows SDK CStudioHdr::RunFlexRules; vertex weighting follows the
//! retail StudioRender flex path (reviewed privately). Delayed (smoothed) weights,
//! wrinkle maps and eyelid-specific controllers beyond the rule ops are not applied.
use crate::{bytes, f32le, i32le, u16le};
use anyhow::{bail, Context, Result};
use glam::Vec3;

#[derive(Clone, Debug, PartialEq)]
pub struct FlexController {
    pub kind: String,
    pub name: String,
    pub min: f32,
    pub max: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlexOp {
    pub op: i32,
    /// Index or float bits, depending on `op`.
    pub data: i32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FlexRule {
    pub flex: usize,
    pub ops: Vec<FlexOp>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VertAnim {
    /// Mesh-local vertex index.
    pub index: u16,
    pub speed: u8,
    pub side: u8,
    pub delta: Vec3,
    pub normal: Vec3,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MeshFlex {
    pub descriptor: usize,
    pub targets: [f32; 4],
    pub pair: usize,
    pub vertices: Vec<VertAnim>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MeshFlexes {
    pub bodypart: usize,
    pub model: usize,
    pub mesh: usize,
    pub flexes: Vec<MeshFlex>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlexModel {
    pub descriptors: Vec<String>,
    pub controllers: Vec<FlexController>,
    pub rules: Vec<FlexRule>,
    pub meshes: Vec<MeshFlexes>,
}
/// A model's flexes together with its eyeballs (whose FACS eyelids write descriptors).
#[derive(Clone, Debug, Default)]
pub struct FaceModel {
    pub flex: FlexModel,
    pub eyes: Vec<crate::eyes::Eyeball>,
}
impl FaceModel {
    /// Descriptor weights with each eye's bone-local basis, or its rest basis when the
    /// presentation has not supplied one.
    pub fn descriptor_weights(
        &self,
        values: &std::collections::BTreeMap<String, f32>,
        basis: impl Fn(&crate::eyes::Eyeball) -> Option<(Vec3, Vec3)>,
    ) -> Vec<f32> {
        let eyes: Vec<_> = self
            .eyes
            .iter()
            .map(|e| (e, basis(e).unwrap_or_else(|| crate::eyes::rest_basis(e))))
            .collect();
        self.flex.descriptor_weights(values, &eyes)
    }
}
const FLEXES_CONVERTED: u32 = 0x4000;
fn count(data: &[u8], at: usize, limit: usize) -> Result<usize> {
    let n = usize::try_from(i32le(data, at)?)?;
    if n > limit {
        bail!("studio flex table exceeds limit at {at}");
    }
    Ok(n)
}
fn relative(data: &[u8], base: usize, at: usize) -> Result<usize> {
    Ok(usize::try_from(base as i64 + i32le(data, at)? as i64)?)
}
fn string(data: &[u8], at: usize) -> Result<String> {
    let tail = data.get(at..).context("string outside MDL")?;
    let end = tail
        .iter()
        .take(256)
        .position(|b| *b == 0)
        .context("unterminated MDL string")?;
    Ok(std::str::from_utf8(&tail[..end])?.into())
}
fn half(data: &[u8], at: usize) -> Result<f32> {
    Ok(half::f16::from_bits(u16le(data, at)?).to_f32())
}
pub fn read_flexes(data: &[u8]) -> Result<FlexModel> {
    if bytes(data, 0, 4)? != b"IDST" {
        bail!("not a studio model");
    }
    if u32::from_le_bytes(bytes(data, 152, 4)?.try_into()?) & FLEXES_CONVERTED != 0 {
        bail!("pre-converted fixed-point flexes are not supported");
    }
    let mut model = FlexModel::default();
    let n = count(data, 260, 1024)?;
    let base = usize::try_from(i32le(data, 264)?)?;
    bytes(data, base, n * 4)?;
    for i in 0..n {
        let at = base + i * 4;
        model
            .descriptors
            .push(string(data, relative(data, at, at)?)?);
    }
    let n = count(data, 268, 1024)?;
    let base = usize::try_from(i32le(data, 272)?)?;
    bytes(data, base, n * 20)?;
    for i in 0..n {
        let at = base + i * 20;
        model.controllers.push(FlexController {
            kind: string(data, relative(data, at, at)?)?,
            name: string(data, relative(data, at, at + 4)?)?,
            min: f32le(data, at + 12)?,
            max: f32le(data, at + 16)?,
        });
    }
    let n = count(data, 276, 4096)?;
    let base = usize::try_from(i32le(data, 280)?)?;
    bytes(data, base, n * 12)?;
    for i in 0..n {
        let at = base + i * 12;
        let flex = usize::try_from(i32le(data, at)?)?;
        let ops = count(data, at + 4, 1024)?;
        let start = relative(data, at, at + 8)?;
        bytes(data, start, ops * 8)?;
        let ops = (0..ops)
            .map(|j| {
                Ok(FlexOp {
                    op: i32le(data, start + j * 8)?,
                    data: i32le(data, start + j * 8 + 4)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        model.rules.push(FlexRule { flex, ops });
    }
    let bodies = count(data, 232, 256)?;
    let start = usize::try_from(i32le(data, 236)?)?;
    bytes(data, start, bodies * 16)?;
    for body in 0..bodies {
        let b = start + body * 16;
        let models = count(data, b + 4, 256)?;
        let models_at = relative(data, b, b + 12)?;
        bytes(data, models_at, models * 148)?;
        for m in 0..models {
            let mat = models_at + m * 148;
            let meshes = count(data, mat + 72, 4096)?;
            let mesh_start = relative(data, mat, mat + 76)?;
            bytes(data, mesh_start, meshes * 116)?;
            for mesh in 0..meshes {
                let at = mesh_start + mesh * 116;
                let vertex_count = count(data, at + 8, 1 << 20)?;
                let n = count(data, at + 16, 4096)?;
                if n == 0 {
                    continue;
                }
                let flex_start = relative(data, at, at + 20)?;
                bytes(data, flex_start, n * 60)?;
                let mut flexes = Vec::with_capacity(n);
                for f in 0..n {
                    let fat = flex_start + f * 60;
                    let descriptor = usize::try_from(i32le(data, fat)?)?;
                    let pair = usize::try_from(i32le(data, fat + 28)?)?;
                    if descriptor >= model.descriptors.len()
                        || pair >= model.descriptors.len().max(1)
                    {
                        bail!("mesh flex descriptor outside table");
                    }
                    let verts = count(data, fat + 20, 1 << 20)?;
                    let vat = relative(data, fat, fat + 24)?;
                    let stride = match bytes(data, fat + 32, 1)?[0] {
                        0 => 16,
                        1 => 18,
                        other => bail!("unknown vertex animation type {other}"),
                    };
                    bytes(data, vat, verts * stride)?;
                    let vertices = (0..verts)
                        .map(|v| {
                            let at = vat + v * stride;
                            let index = u16le(data, at)?;
                            if usize::from(index) >= vertex_count {
                                bail!("flex vertex outside mesh");
                            }
                            Ok(VertAnim {
                                index,
                                speed: bytes(data, at + 2, 1)?[0],
                                side: bytes(data, at + 3, 1)?[0],
                                delta: Vec3::new(
                                    half(data, at + 4)?,
                                    half(data, at + 6)?,
                                    half(data, at + 8)?,
                                ),
                                normal: Vec3::new(
                                    half(data, at + 10)?,
                                    half(data, at + 12)?,
                                    half(data, at + 14)?,
                                ),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    flexes.push(MeshFlex {
                        descriptor,
                        targets: [
                            f32le(data, fat + 4)?,
                            f32le(data, fat + 8)?,
                            f32le(data, fat + 12)?,
                            f32le(data, fat + 16)?,
                        ],
                        pair,
                        vertices,
                    });
                }
                model.meshes.push(MeshFlexes {
                    bodypart: body,
                    model: m,
                    mesh,
                    flexes,
                });
            }
        }
    }
    Ok(model)
}
fn remap_clamped(value: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    if a == b {
        return if value >= b { d } else { c };
    }
    let t = ((value - a) / (b - a)).clamp(0., 1.);
    c + (d - c) * t
}
impl FlexModel {
    /// Descriptor weights for named controller values (lowercase names, controller ranges):
    /// RunFlexRules, then retail FACS eyelids for each eyeball with its bone-local
    /// forward/up basis.
    pub fn descriptor_weights(
        &self,
        values: &std::collections::BTreeMap<String, f32>,
        eyes: &[(&crate::eyes::Eyeball, (Vec3, Vec3))],
    ) -> Vec<f32> {
        let src: Vec<f32> = self
            .controllers
            .iter()
            .map(|c| values.get(&c.name.to_lowercase()).copied().unwrap_or(0.))
            .collect();
        let mut weights = self.run_rules(&src);
        for (eye, basis) in eyes {
            crate::eyes::apply_eyelids(eye, &mut weights, *basis);
        }
        weights
    }
    /// Bind-space position deltas keyed by (bodypart, mesh, mesh-local vertex) for the
    /// first model of each bodypart, with retail vertex weighting (no delayed weights).
    pub fn vertex_deltas(&self, descriptors: &[f32]) -> std::collections::HashMap<[u16; 3], Vec3> {
        let mut deltas = std::collections::HashMap::new();
        for mesh in self.meshes.iter().filter(|m| m.model == 0) {
            for flex in &mesh.flexes {
                let Some(weights) = flex.weights(descriptors) else {
                    continue;
                };
                for v in &flex.vertices {
                    let key = [mesh.bodypart as u16, mesh.mesh as u16, v.index];
                    *deltas.entry(key).or_insert(Vec3::ZERO) += v.delta * v.weight(weights);
                }
            }
        }
        deltas
    }
    /// CStudioHdr::RunFlexRules: controller values (in each controller's own range, model
    /// order) to descriptor weights. Invalid ops are skipped as the SDK CHECK macros do.
    pub fn run_rules(&self, src: &[f32]) -> Vec<f32> {
        let mut dest = vec![0.; self.descriptors.len()];
        let value =
            |i: i32| -> Option<f32> { usize::try_from(i).ok().and_then(|i| src.get(i).copied()) };
        let controller = |i: i32| {
            usize::try_from(i)
                .ok()
                .and_then(|i| self.controllers.get(i))
        };
        for rule in &self.rules {
            if rule.flex >= dest.len() {
                continue;
            }
            let mut stack = [0f32; 32];
            let mut k = 0usize;
            for op in &rule.ops {
                match op.op {
                    4..=7 | 13 | 14 if k >= 2 => {
                        let (a, b) = (stack[k - 2], stack[k - 1]);
                        stack[k - 2] = match op.op {
                            4 => a + b,
                            5 => a - b,
                            6 => a * b,
                            7 => {
                                if b > 0.0001 {
                                    a / b
                                } else {
                                    0.
                                }
                            }
                            13 => a.max(b),
                            _ => a.min(b),
                        };
                        k -= 1;
                    }
                    8 if k >= 1 => stack[k - 1] = -stack[k - 1],
                    1 if k < 32 => {
                        stack[k] = f32::from_bits(op.data as u32);
                        k += 1;
                    }
                    2 if k < 32 => {
                        if let Some(v) = value(op.data) {
                            stack[k] = v;
                            k += 1;
                        }
                    }
                    3 if k < 32 => {
                        if let Some(v) = usize::try_from(op.data).ok().and_then(|i| dest.get(i)) {
                            stack[k] = *v;
                            k += 1;
                        }
                    }
                    15 | 16 if k < 32 => {
                        if let Some(v) = value(op.data) {
                            stack[k] = if op.op == 15 {
                                remap_clamped(v, -1., 0., 1., 0.)
                            } else {
                                remap_clamped(v, 0., 1., 0., 1.)
                            };
                            k += 1;
                        }
                    }
                    18 => {
                        let m = op.data.max(0) as usize;
                        if m <= k && m > 0 {
                            let km = k - m;
                            for i in km + 1..k {
                                stack[km] *= stack[i];
                            }
                            k = km + 1;
                        }
                    }
                    19 => {
                        let m = op.data.max(0) as usize;
                        if k > m && m > 0 {
                            let km = k - m;
                            let dv: f32 = stack[km..k].iter().product();
                            stack[km - 1] *= 1. - dv;
                            k -= m;
                        }
                    }
                    17 if k >= 5 => {
                        let selected = stack[k - 1] as i32;
                        if let (Some(mut v), Some(scale)) = (value(selected), value(op.data)) {
                            let (x, y, z, w) =
                                (stack[k - 5], stack[k - 4], stack[k - 3], stack[k - 2]);
                            v = if v <= x || v >= w {
                                0.
                            } else if v < y {
                                remap_clamped(v, x, y, 0., 1.)
                            } else if v > z {
                                remap_clamped(v, z, w, 1., 0.)
                            } else {
                                1.
                            };
                            stack[k - 5] = v * scale;
                            k -= 4;
                        }
                    }
                    20 | 21 if k >= 3 => {
                        let (Some(close_v), Some(close)) =
                            (controller(op.data), controller(stack[k - 1] as i32))
                        else {
                            continue;
                        };
                        let close_v = remap_clamped(
                            value(op.data).unwrap_or(0.),
                            close_v.min,
                            close_v.max,
                            0.,
                            1.,
                        );
                        let close = remap_clamped(
                            value(stack[k - 1] as i32).unwrap_or(0.),
                            close.min,
                            close.max,
                            0.,
                            1.,
                        );
                        let up_down = stack[k - 3] as i32;
                        let eye = controller(up_down).map_or(0., |c| {
                            remap_clamped(value(up_down).unwrap_or(0.), c.min, c.max, -1., 1.)
                        });
                        stack[k - 3] = if op.op == 20 {
                            if eye > 0. {
                                (1. - eye) * (1. - close_v) * close
                            } else {
                                (1. - close_v) * close
                            }
                        } else if eye < 0. {
                            (1. + eye) * close_v * close
                        } else {
                            close_v * close
                        };
                        k -= 2;
                    }
                    _ => {}
                }
            }
            dest[rule.flex] = stack[0];
        }
        dest
    }
}
impl MeshFlex {
    fn ramp(&self, v: f32) -> f32 {
        let [t0, t1, t2, t3] = self.targets;
        if !(t0 < v && v < t3) {
            0.
        } else if v < t1 {
            (v - t0) / (t1 - t0)
        } else if v > t2 {
            (t3 - v) / (t3 - t2)
        } else {
            1.
        }
    }
    /// Retail R_StudioFlexVerts weights for (descriptor, pair) without delayed smoothing:
    /// returns `None` when the flex has no influence.
    pub fn weights(&self, descriptors: &[f32]) -> Option<(f32, f32)> {
        let w1 = self.ramp(descriptors.get(self.descriptor).copied().unwrap_or(0.));
        let w3 = if self.pair != 0 {
            self.ramp(descriptors.get(self.pair).copied().unwrap_or(0.))
        } else {
            w1
        };
        (w1.abs() > 0.001 || w3.abs() > 0.001).then_some((w1, w3))
    }
}
impl VertAnim {
    /// Side blends descriptor (0) and pair (255); delayed weights equal current ones here.
    pub fn weight(&self, (w1, w3): (f32, f32)) -> f32 {
        let side = f32::from(self.side) / 255.;
        w1 * (1. - side) + w3 * side
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn op(op: i32, data: i32) -> FlexOp {
        FlexOp { op, data }
    }
    fn model(rules: Vec<FlexRule>) -> FlexModel {
        FlexModel {
            descriptors: vec!["a".into(), "b".into()],
            controllers: vec![
                FlexController {
                    kind: "default".into(),
                    name: "c0".into(),
                    min: 0.,
                    max: 1.,
                },
                FlexController {
                    kind: "default".into(),
                    name: "c1".into(),
                    min: -1.,
                    max: 1.,
                },
            ],
            rules,
            meshes: Vec::new(),
        }
    }
    #[test]
    fn rules_evaluate_sdk_stack_ops() {
        let rules = vec![
            // a = c0 * 2 + c1
            FlexRule {
                flex: 0,
                ops: vec![
                    op(2, 0),
                    op(1, 2f32.to_bits() as i32),
                    op(6, 0),
                    op(2, 1),
                    op(4, 0),
                ],
            },
            // b = 2way_1(c1) combined with fetch2(a) over 2 values
            FlexRule {
                flex: 1,
                ops: vec![op(16, 1), op(3, 0), op(18, 2)],
            },
        ];
        let w = model(rules).run_rules(&[0.25, 0.5]);
        assert!((w[0] - 1.).abs() < 1e-6);
        assert!((w[1] - 0.5).abs() < 1e-6);
        // Division by a small value yields zero; an underflowing op is skipped.
        let w = model(vec![FlexRule {
            flex: 0,
            ops: vec![op(1, 1f32.to_bits() as i32), op(1, 0), op(7, 0), op(4, 0)],
        }])
        .run_rules(&[0., 0.]);
        assert_eq!(w[0], 0.);
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_barney_and_kleiner_flexes_parse_and_evaluate() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for path in ["models/barney.mdl", "models/kleiner.mdl"] {
            let data = vfs.read(path).unwrap().unwrap();
            let flexes = read_flexes(&data).unwrap();
            let verts: usize = flexes
                .meshes
                .iter()
                .flat_map(|m| &m.flexes)
                .map(|f| f.vertices.len())
                .sum();
            eprintln!(
                "FLEXES {path}: {} descriptors, {} controllers, {} rules, {} flexed meshes, {verts} vertex deltas",
                flexes.descriptors.len(),
                flexes.controllers.len(),
                flexes.rules.len(),
                flexes.meshes.len()
            );
            assert!(!flexes.controllers.is_empty() && !flexes.rules.is_empty() && verts > 0);
            // Every flexed studio mesh has a rendered surface whose vertices cover its deltas.
            let surfaces = crate::models::read_model(&vfs, path, 0).unwrap();
            for mesh in &flexes.meshes {
                let source = surfaces
                    .iter()
                    .filter_map(|s| s.flex_source.as_ref())
                    .find(|f| {
                        (f.bodypart, f.model, f.mesh) == (mesh.bodypart, mesh.model, mesh.mesh)
                    });
                if mesh.model != 0 {
                    continue;
                }
                let source = source.unwrap_or_else(|| {
                    panic!(
                        "no surface for flexed mesh {mesh:?}",
                        mesh = (mesh.bodypart, mesh.mesh)
                    )
                });
                let ids: std::collections::BTreeSet<u16> =
                    source.vertex_ids.iter().copied().collect();
                let covered = mesh
                    .flexes
                    .iter()
                    .flat_map(|f| &f.vertices)
                    .filter(|v| ids.contains(&v.index))
                    .count();
                let total: usize = mesh.flexes.iter().map(|f| f.vertices.len()).sum();
                assert!(
                    covered * 10 >= total * 9,
                    "{path}: {covered}/{total} flex vertices rendered"
                );
            }
            assert!(flexes
                .controllers
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case("jaw_drop")));
            let neutral = flexes.run_rules(&vec![0.; flexes.controllers.len()]);
            assert!(neutral.iter().all(|w| w.is_finite()));
            // A neutral face looking along its authored eye direction has open lids: the
            // FACS eyelid descriptors land on their neutral targets (no lid deformation).
            let face = FaceModel {
                flex: flexes.clone(),
                eyes: crate::eyes::load(&vfs, path).unwrap(),
            };
            assert!(face.eyes.iter().all(|e| e.lids.is_some()), "{path}");
            let rest = face.descriptor_weights(&Default::default(), |_| None);
            assert!(
                face.flex
                    .vertex_deltas(&rest)
                    .values()
                    .all(|d| d.length() < 0.05),
                "{path}: neutral face deforms"
            );
            // Without FACS eyelids the same descriptors would half-close the upper lids.
            assert!(!flexes.vertex_deltas(&neutral).is_empty());
            let jaw = flexes
                .controllers
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case("jaw_drop"))
                .unwrap();
            let mut open = vec![0.; flexes.controllers.len()];
            open[jaw] = flexes.controllers[jaw].max;
            let weights = flexes.run_rules(&open);
            let active = flexes
                .meshes
                .iter()
                .flat_map(|m| &m.flexes)
                .filter(|f| f.weights(&weights).is_some())
                .count();
            assert!(active > 0, "jaw_drop activates no mesh flex in {path}");
            assert!(flexes
                .meshes
                .iter()
                .flat_map(|m| &m.flexes)
                .flat_map(|f| &f.vertices)
                .all(|v| v.delta.is_finite() && v.normal.is_finite()));
        }
    }
    #[test]
    fn mesh_flex_ramp_and_side_blend() {
        let flex = MeshFlex {
            descriptor: 0,
            targets: [0., 0.5, 1., 1.5],
            pair: 1,
            vertices: Vec::new(),
        };
        assert_eq!(flex.weights(&[0., 0.]), None);
        let (w1, w3) = flex.weights(&[0.25, 1.25]).unwrap();
        assert!((w1 - 0.5).abs() < 1e-6 && (w3 - 0.5).abs() < 1e-6);
        assert_eq!(flex.weights(&[0.75, 0.]).unwrap(), (1., 0.));
        let v = VertAnim {
            index: 0,
            speed: 255,
            side: 255,
            delta: Vec3::X,
            normal: Vec3::ZERO,
        };
        assert_eq!(v.weight((1., 0.)), 0.);
        let left = VertAnim { side: 0, ..v };
        assert_eq!(left.weight((1., 0.)), 1.);
    }
}
