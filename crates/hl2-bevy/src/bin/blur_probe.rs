//! Opt-in frozen-input fragment probe. Inputs and reports belong in ignored artifacts.
use bevy::{
    app::ScheduleRunnerPlugin,
    prelude::{App, DefaultPlugins, PluginGroup, Res, Resource, WindowPlugin, default},
    render::{
        RenderApp, RenderStartup,
        diagnostic::{DiagnosticsRecorder, RecordDiagnostics},
        render_resource::{
            BindGroup, BindGroupEntries, BufferDescriptor, BufferUsages, ColorTargetState,
            ColorWrites, CommandEncoderDescriptor, Extent3d, FilterMode, MapMode, MultisampleState,
            Operations, PipelineCompilationOptions, PollType, RawFragmentState,
            RawRenderPipelineDescriptor, RawVertexState, RenderPassColorAttachment,
            RenderPassDescriptor, RenderPipeline, SamplerDescriptor, ShaderModuleDescriptor,
            ShaderSource, TexelCopyBufferInfo, TexelCopyBufferLayout, Texture, TextureDescriptor,
            TextureDimension, TextureFormat, TextureUsages,
        },
        renderer::{RenderAdapterInfo, RenderDevice, RenderQueue},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use std::{borrow::Cow, path::PathBuf, time::Instant};

#[derive(Resource, serde::Deserialize)]
struct Config {
    width: u32,
    height: u32,
    input: PathBuf,
    shaders: Vec<PathBuf>,
    output: PathBuf,
}
struct Variant {
    pipeline: RenderPipeline,
    group: BindGroup,
}
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("configuration JSON required");
    let config: Config = serde_json::from_slice(&std::fs::read(path).expect("configuration"))
        .expect("valid configuration");
    assert!(config.width > 0 && config.height > 0 && !config.shaders.is_empty());
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
        .insert_resource(config)
        .add_systems(RenderStartup, run);
    app.run();
}
fn texture(device: &RenderDevice, config: &Config) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some("frozen_blur_probe"),
        size: Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING
            | TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::COPY_DST
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
fn variants(device: &RenderDevice, config: &Config, input: &Texture) -> Vec<Variant> {
    let view = input.create_view(&default());
    let sampler = device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..default()
    });
    config
        .shaders
        .iter()
        .map(|path| {
            let code = std::fs::read_to_string(path).expect("probe shader");
            let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("frozen_blur_probe"),
                source: ShaderSource::Wgsl(Cow::Owned(code)),
            });
            let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("frozen_blur_probe"),
                layout: None,
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            let group = device.create_bind_group(
                Some("frozen_input"),
                &pipeline.get_bind_group_layout(0).into(),
                &BindGroupEntries::sequential((&view, &sampler)),
            );
            Variant { pipeline, group }
        })
        .collect()
}
fn batch(
    device: &RenderDevice,
    queue: &RenderQueue,
    output: &Texture,
    variant: &Variant,
    count: u32,
    recorder: &mut DiagnosticsRecorder,
) -> serde_json::Value {
    let view = output.create_view(&default());
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    recorder.begin_frame();
    let span = recorder.time_span(&mut encoder, "frozen_batch");
    for _ in 0..count {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("frozen_camera_gather"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: bevy::render::render_resource::LoadOp::Clear(default()),
                    store: bevy::render::render_resource::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&variant.pipeline);
        pass.set_bind_group(0, &*variant.group, &[]);
        pass.draw(0..3, 0..1);
    }
    span.end(&mut encoder);
    recorder.resolve(&mut encoder);
    let commands = encoder.finish();
    let start = Instant::now();
    let submission = queue.submit([commands]);
    let (send, receive) = std::sync::mpsc::channel();
    recorder.finish_frame(device, move |value| {
        send.send(format!("{value:?}")).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("GPU completion");
    let completion = start.elapsed().as_secs_f64() * 1000. / f64::from(count);
    recorder.begin_frame();
    let diagnostics = receive
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("timestamp callback");
    serde_json::json!({"completion_ms_per_pass":completion,"passes":count,"diagnostics":diagnostics})
}
fn capture(device: &RenderDevice, queue: &RenderQueue, output: &Texture, path: PathBuf) {
    let stride = (output.width() * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("probe_readback"),
        size: u64::from(stride) * u64::from(output.height()),
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        output.size(),
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(MapMode::Read, move |result| {
        send.send(result).expect("map callback");
    });
    device
        .poll(PollType::wait_indefinitely())
        .expect("readback completion");
    receive.recv().expect("map result").expect("map succeeded");
    let mapped = buffer.slice(..).get_mapped_range();
    let bytes: Vec<u8> = mapped
        .chunks(stride as usize)
        .flat_map(|row| row[..(output.width() * 4) as usize].iter().copied())
        .collect();
    std::fs::write(path, bytes).expect("capture");
}
fn run(
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    config: Res<Config>,
    adapter: Res<RenderAdapterInfo>,
) {
    let mut recorder = DiagnosticsRecorder::new(&adapter, &device, &queue);
    let input = texture(&device, &config);
    let bytes = std::fs::read(&config.input).expect("RGBA8 input");
    assert_eq!(bytes.len(), (config.width * config.height * 4) as usize);
    queue.write_texture(
        input.as_image_copy(),
        &bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(config.width * 4),
            rows_per_image: None,
        },
        input.size(),
    );
    let output = texture(&device, &config);
    let variants = variants(&device, &config, &input);
    std::fs::create_dir_all(&config.output).expect("output directory");
    let mut measurements = vec![Vec::new(); variants.len()];
    let mut priming = Vec::new();
    for (index, variant) in variants.iter().enumerate() {
        priming.push(batch(&device, &queue, &output, variant, 32, &mut recorder));
        capture(
            &device,
            &queue,
            &output,
            config.output.join(format!("{index}.rgba")),
        );
    }
    for iteration in 0..12 {
        for offset in 0..variants.len() {
            let index = if iteration % 2 == 0 {
                offset
            } else {
                variants.len() - 1 - offset
            };
            measurements[index].push(batch(
                &device,
                &queue,
                &output,
                &variants[index],
                64,
                &mut recorder,
            ));
        }
    }
    let result = serde_json::json!({"measurements":measurements,
        "priming":priming,"device_features":format!("{:?}",device.features()),
        "width":config.width,"height":config.height,"shaders":config.shaders,
        "caveat":"GPU-completed batch wall time includes submission/wait overhead; not isolated timestamp duration."});
    std::fs::write(
        config.output.join("timing.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    println!("{result}");
    std::process::exit(0);
}
