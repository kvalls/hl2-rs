//! Actual offscreen adapter test; the returned R8 bytes include uncleared neighbours and padding.
use super::{Batch, raster::Raster};
use crate::shadows::texture::texel_alpha;
use bevy::{
    app::{App, ScheduleRunnerPlugin},
    prelude::{DefaultPlugins, PluginGroup, Res, Resource, WindowPlugin, default},
    render::{
        RenderApp, RenderStartup,
        render_resource::{
            BufferDescriptor, BufferUsages, Extent3d, MapMode, PollType, TexelCopyBufferInfo,
            TexelCopyBufferLayout, Texture, TextureDescriptor, TextureDimension, TextureFormat,
            TextureUsages,
        },
        renderer::{RenderDevice, RenderQueue},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use glam::{Quat, UVec2, Vec2, Vec3};
use hl2_simulation::shadows::{
    frame::{Frame, Placement},
    silhouette,
};
use modkit_core::{Surface, Vertex};
use source_assets::vtf;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Resource, Clone, Default)]
struct Done(Arc<AtomicBool>);

#[test]
#[ignore = "requires a working graphics adapter"]
fn gpu_shadow_atlas_matches_cpu_on_motion_alpha_and_cell_reuse() {
    let done = Done::default();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .disable::<WinitPlugin>(),
    )
    .add_plugins(ScheduleRunnerPlugin::run_once());
    app.sub_app_mut(RenderApp)
        .insert_resource(done.clone())
        .add_systems(RenderStartup, compare);
    app.run();
    assert!(
        done.0.load(Ordering::Acquire),
        "render startup must execute the readback"
    );
}

fn shape(material: &str) -> Surface {
    let vertices = [
        (-5.87, -6.11, -0.7, 0.2),
        (5.43, -4.18, 1.8, 0.1),
        (-3.79, 5.83, -0.2, 1.7),
    ]
    .map(|(x, y, u, v)| Vertex {
        position: Vec3::new(x, y, 0.),
        uv: Vec2::new(u, v),
        normal: Vec3::Z,
        skin: None,
        light_uv: Vec2::ZERO,
        color: [255; 4],
    })
    .to_vec();
    Surface {
        material: material.into(),
        background: false,
        vertices,
        indices: vec![0, 1, 2, 2, 1, 0],
        lightmap: None,
        flex_source: None,
    }
}

fn read(device: &RenderDevice, queue: &RenderQueue, atlas: &Texture) -> Vec<u8> {
    let stride = 256;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("test shadow readback"),
        size: u64::from(stride * atlas.height()),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&default());
    enc.copy_texture_to_buffer(
        atlas.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        atlas.size(),
    );
    queue.submit([enc.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    buffer
        .slice(..)
        .get_mapped_range()
        .chunks_exact(stride as usize)
        .flat_map(|r| r[..atlas.width() as usize].iter().copied())
        .collect()
}

// Hardware rasterization snaps vertices to subpixel coordinates and uses a half-open
// edge rule. The CPU uses floating-point inclusive edges. Only centres within 1/128
// of a texel of the actual outline may disagree by more than one R8 rounding step.
fn check_cell(gpu: &[u8], previous: &[u8], page: &[u8], at: u32, triangle: [Vec2; 3]) -> usize {
    let mut edges = 0;
    for (i, (&actual, &expected)) in gpu.iter().zip(page).enumerate() {
        let x = (i % 64) as u32;
        if !(at..at + 32).contains(&x) {
            assert_eq!(actual, previous[i], "unmodified neighbour at {i}");
        } else if actual.abs_diff(expected) > 1 {
            let centre = Vec2::new((x - at) as f32, (i / 64) as f32) + 0.5;
            let distance = (0..3)
                .map(|j| {
                    let a = triangle[j];
                    let edge = triangle[(j + 1) % 3] - a;
                    let t = ((centre - a).dot(edge) / edge.length_squared()).clamp(0., 1.);
                    centre.distance(a + edge * t)
                })
                .fold(f32::INFINITY, f32::min);
            assert!(
                distance < 1. / 128.,
                "texel {centre:?}, {distance} from outline: {actual} vs {expected}"
            );
            eprintln!(
                "outline texel {centre:?}, distance {distance:.6}, GPU {actual}, CPU {expected}"
            );
            edges += 1;
        }
    }
    edges
}

fn compare(device: Res<RenderDevice>, queue: Res<RenderQueue>, done: Res<Done>) {
    let size = Extent3d {
        width: 64,
        height: 32,
        depth_or_array_layers: 1,
    };
    let atlas = device.create_texture(&TextureDescriptor {
        label: Some("test atlas"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::R8Unorm,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut raster = Raster::new(&device);
    let view = atlas.create_view(&default());
    let frame = Frame::render_to_texture(
        (Vec3::splat(-8.), Vec3::splat(8.)),
        &Placement::new(Vec3::ZERO, Quat::IDENTITY),
        Vec3::NEG_Z,
        80.,
    );
    let picture = Arc::new(vtf::Image {
        width: 2,
        height: 2,
        rgba: [0u8, 64, 100, 255]
            .into_iter()
            .flat_map(|a| [255, 255, 255, a])
            .collect(),
        format: 0,
        translucent: true,
    });
    let alphas = BTreeMap::from([("cutout".to_owned(), (picture.clone(), 0.5))]);
    let mut page = vec![0; 64 * 32];
    let mut last = page.clone();
    let mut last_gpu = page.clone();
    for (at, material, delta) in [
        (0, "opaque", 0.),
        (32, "cutout", 0.),
        (0, "opaque", 8.),
        (32, "cutout", -2.3),
    ] {
        let mut batch = Batch::default();
        let surface = shape(material);
        let place = |v: &Vertex| v.position + Vec3::X * delta;
        batch.cell(
            32,
            UVec2::new(at, 0),
            &frame,
            (std::slice::from_ref(&surface), &place),
            &alphas,
        );
        let mut cpu = vec![0.; 32 * 32];
        for tri in surface.indices.as_chunks::<3>().0 {
            let points = tri.map(|i| place(&surface.vertices[i as usize]));
            let uv = tri.map(|i| surface.vertices[i as usize].uv);
            let alpha = |v| texel_alpha(&picture, v) * 0.5;
            silhouette::draw(
                &mut cpu,
                32,
                &frame,
                (points, uv),
                (material == "cutout").then_some(&alpha),
            );
        }
        for (y, row) in cpu.as_chunks::<32>().0.iter().enumerate() {
            let base = y * 64 + at as usize;
            for (x, c) in row.iter().enumerate() {
                page[base + x] = (c * 255.).round() as u8;
            }
        }
        raster.draw(&device, &queue, &atlas, &view, &batch);
        let got = read(&device, &queue, &atlas);
        let triangle =
            [0, 1, 2].map(|i| silhouette::project(&frame, 32, place(&surface.vertices[i])));
        let edges = check_cell(&got, &last_gpu, &page, at, triangle);
        eprintln!("{material} move={delta} edge differences={edges}");
        last_gpu = got;
        assert!(page != last && page.iter().filter(|&&a| a > 0).count() > 30);
        if material == "cutout" {
            assert!(cpu.iter().any(|&c| c > 0. && c < 0.9));
        }
        if delta != 0. {
            assert!(
                page.iter()
                    .zip(&last)
                    .any(|(&now, &before)| now == 0 && before > 0)
            );
        }
        last.copy_from_slice(&page);
    }
    measure_faint_layers(&device, &queue, &mut raster, &atlas, &frame);
    done.0.store(true, Ordering::Release);
}

// Characterize repeated rounding separately from the ordinary coverage comparisons.
// Sixteen layers of a quarter of one R8 step accumulate to four on the CPU.
fn measure_faint_layers(
    device: &RenderDevice,
    queue: &RenderQueue,
    raster: &mut Raster,
    atlas: &Texture,
    frame: &Frame,
) {
    let picture = Arc::new(vtf::Image {
        width: 1,
        height: 1,
        rgba: vec![255, 255, 255, 1],
        format: 0,
        translucent: true,
    });
    let alphas = BTreeMap::from([("faint".to_owned(), (picture, 0.25))]);
    let mut surface = shape("faint");
    surface.indices = [0, 1, 2].repeat(16);
    let mut batch = Batch::default();
    batch.cell(
        32,
        UVec2::ZERO,
        frame,
        (std::slice::from_ref(&surface), &|v| v.position),
        &alphas,
    );
    raster.draw(device, queue, atlas, &atlas.create_view(&default()), &batch);
    let page = read(device, queue, atlas);
    let corners = [0, 1, 2].map(|i| silhouette::project(frame, 32, surface.vertices[i].position));
    let centre = ((corners[0] + corners[1] + corners[2]) / 3.).as_uvec2();
    let value = page[(centre.y * 64 + centre.x) as usize];
    let mut cpu = vec![0.; 32 * 32];
    for _ in surface.indices.as_chunks::<3>().0 {
        silhouette::draw_projected(
            &mut cpu,
            32,
            (corners, [Vec2::ZERO; 3]),
            Some(&|_| 0.25 / 255.),
        );
    }
    let reference = (cpu[(centre.y * 32 + centre.x) as usize] * 255.).round() as u8;
    assert_eq!(reference, 4);
    assert!(value.abs_diff(reference) <= 8, "sixteen R8 rounding steps");
    eprintln!("16 faint layers at triangle centre: GPU {value}, CPU {reference}");
}
