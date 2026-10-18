//! Shared source textures and reusable offscreen render targets. No OBS dependencies.
use anyhow::{Context, Result, bail};
use auroracast_core::{Canvas, Id, Project};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
    key: [f32; 4],
    key_options: [f32; 2],
    /// Position from the layer's centre in pixels, for the frame outline.
    local: [f32; 2],
    /// Frame: half width, half height, corner radius and edge fade in pixels; x below 0 is off.
    frame: [f32; 4],
}

const NO_FRAME: [f32; 4] = [-1., 0., 0., 0.];
/// Between two scenes with the Move transition: layers glide from where they were.
struct Motion {
    from: Id,
    started: std::time::Instant,
    duration: std::time::Duration,
}
/// Smooth start and finish.
fn ease(p: f32) -> f32 {
    if p < 0.5 { 4. * p * p * p } else { 1. - (-2. * p + 2.).powi(3) / 2. }
}
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
/// The layers to draw `p` of the way from `old` to `new`. A layer showing the same source in
/// both scenes moves between its two places; the rest fade out (under) or in.
fn moving_layers(old: &[auroracast_core::Layer], new: &[auroracast_core::Layer], p: f32) -> Vec<auroracast_core::Layer> {
    let t = ease(p);
    let mut used = vec![false; old.len()];
    let mut arriving = Vec::with_capacity(new.len());
    for layer in new.iter().filter(|l| l.visible) {
        let mut l = layer.clone();
        match old.iter().enumerate().position(|(i, o)| !used[i] && o.visible && o.source == layer.source) {
            Some(i) => {
                used[i] = true;
                let o = &old[i];
                for k in 0..4 {
                    l.rect[k] = mix(o.rect[k], layer.rect[k], t);
                    l.crop[k] = mix(o.crop[k], layer.crop[k], t);
                }
                l.opacity = mix(o.opacity, layer.opacity, t);
                l.frame.radius = mix(o.frame.radius, layer.frame.radius, t);
                l.frame.feather = mix(o.frame.feather, layer.frame.feather, t);
                if o.frame.shape != layer.frame.shape && t < 0.5 {
                    l.frame.shape = o.frame.shape;
                }
            }
            None => l.opacity *= t,
        }
        arriving.push(l);
    }
    let mut out: Vec<_> = old
        .iter()
        .enumerate()
        .filter(|(i, o)| !used[*i] && o.visible)
        .map(|(_, o)| auroracast_core::Layer { opacity: o.opacity * (1. - t), ..o.clone() })
        .collect();
    out.extend(arriving);
    out
}
struct SharedSource {
    texture: wgpu::Texture,
    binding: wgpu::BindGroup,
    width: u32,
    height: u32,
}
struct TransitionFrame {
    texture: wgpu::Texture,
    binding: wgpu::BindGroup,
    started: std::time::Instant,
    duration: std::time::Duration,
    kind: auroracast_core::Transition,
}
struct Target {
    scene: Option<Id>,
    transition: Option<TransitionFrame>,
    motion: Option<Motion>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    display: wgpu::TextureView,
    staging: Option<wgpu::Buffer>,
    stride: u32,
    vertices: Vec<Vertex>,
    vertex_buffer: Option<wgpu::Buffer>,
}

pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    sources: HashMap<Id, SharedSource>,
    targets: HashMap<(Id, u32, u32), Target>,
    pub adapter_name: String,
    pub software_adapter: bool,
    pub uploads: u64,
    pub vertex_allocations: u64,
    pub vertex_uploads: u64,
    vertices: Vec<Vertex>,
    draws: Vec<(Option<Id>, std::ops::Range<u32>)>,
}

impl Compositor {
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: if cfg!(target_os="windows"){wgpu::Backends::DX12 | wgpu::Backends::VULKAN}else{wgpu::Backends::VULKAN},
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .context("No Vulkan adapter; install your GPU's Vulkan driver")?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Rosorah Studio compositor"),
                ..Default::default()
            })
            .await?;
        Ok(Self::from_device(device, queue, info))
    }

    /// Reuse the UI device so the canvas can be sampled directly by egui.
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, info: wgpu::AdapterInfo) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shared image"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Linear clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("composite.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32x4, 4 => Float32x2, 5 => Float32x2, 6 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Canvas compositing"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        Self {
            device,
            queue,
            pipeline,
            layout,
            sampler,
            sources: HashMap::new(),
            targets: HashMap::new(),
            adapter_name: info.name,
            software_adapter: info.device_type == wgpu::DeviceType::Cpu,
            uploads: 0,
            vertex_allocations: 0,
            vertex_uploads: 0,
            vertices: Vec::new(),
            draws: Vec::new(),
        }
    }

    pub fn ensure_empty_source(&mut self, id: Id) -> Result<()> {
        if !self.sources.contains_key(&id) {
            self.upload(id, 1, 1, &[0, 0, 0, 0])?;
        }
        Ok(())
    }
    pub fn upload(&mut self, id: Id, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
        if width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || rgba.len() != width as usize * height as usize * 4
        {
            bail!("Invalid source frame dimensions or length");
        }
        if self
            .sources
            .get(&id)
            .is_none_or(|s| s.width != width || s.height != height)
        {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Shared source texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.sources.insert(
                id,
                SharedSource {
                    texture,
                    binding,
                    width,
                    height,
                },
            );
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.sources[&id].texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.uploads += 1;
        Ok(())
    }

    pub fn retain_sources(&mut self, project: &Project) {
        self.sources.retain(|id, _| project.source(*id).is_some());
    }

    pub fn animating(&self) -> bool {
        self.targets.values().any(|t| {
            t.transition
                .as_ref()
                .is_some_and(|f| f.started.elapsed() < f.duration)
                || t.motion.as_ref().is_some_and(|m| m.started.elapsed() < m.duration)
        })
    }
    /// GPU-only canvas presentation: no map, CPU copy, or blocking device poll.
    pub fn render_gpu(
        &mut self,
        project: &Project,
        canvas: &Canvas,
        width: u32,
        height: u32,
    ) -> Result<wgpu::TextureView> {
        if !(16..=4096).contains(&width) || !(16..=4096).contains(&height) {
            bail!("Invalid render dimensions");
        }
        let scene = project
            .scene(canvas.scene)
            .context("Canvas references a missing scene")?;
        // Bound cache; targets are scratch surfaces, not separate source captures.
        if !self.targets.contains_key(&(canvas.id, width, height)) {
            // A resized canvas replaces its old scratch surface immediately.
            // Retaining eight full-size surfaces made dragging panels costly.
            self.targets.retain(|(id, _, _), _| *id != canvas.id);
            if self.targets.len() >= 8 {
                self.targets.clear();
            }
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Canvas target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
            });
            let view = texture.create_view(&Default::default());
            let stride = (width * 4).div_ceil(256) * 256;
            // Egui expects gamma-encoded samples. Reinterpret the sRGB target
            // as Unorm for display; the render attachment remains sRGB.
            let display = texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(wgpu::TextureFormat::Rgba8Unorm),
                ..Default::default()
            });
            self.targets.insert(
                (canvas.id, width, height),
                Target {
                    scene: None,
                    transition: None,
                    motion: None,
                    texture,
                    view,
                    display,
                    staging: None,
                    stride,
                    vertices: Vec::new(),
                    vertex_buffer: None,
                },
            );
        }
        let target = self.targets.get_mut(&(canvas.id, width, height)).unwrap();
        let mut commands = self.device.create_command_encoder(&Default::default());
        if target.scene.is_some_and(|s| s != canvas.scene)
            && project.preferences.transition.blends()
        {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Transition snapshot"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            commands.copy_texture_to_texture(
                target.texture.as_image_copy(),
                texture.as_image_copy(),
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&Default::default());
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Transition image"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            target.transition = Some(TransitionFrame {
                texture,
                binding,
                started: std::time::Instant::now(),
                duration: std::time::Duration::from_millis(
                    project.preferences.transition_ms.clamp(100, 2000) as u64,
                ),
                kind: project.preferences.transition,
            });
        } else if let Some(from) = target.scene.filter(|s| *s != canvas.scene) {
            target.transition = None;
            // Move needs no snapshot: the old scene's layers are interpolated as they are drawn.
            target.motion = (project.preferences.transition == auroracast_core::Transition::Move).then(|| Motion {
                from,
                started: std::time::Instant::now(),
                duration: std::time::Duration::from_millis(project.preferences.transition_ms.clamp(100, 2000) as u64),
            });
        }
        target.scene = Some(canvas.scene);
        if target
            .transition
            .as_ref()
            .is_some_and(|t| t.started.elapsed() >= t.duration)
        {
            target.transition = None;
        }
        if target.motion.as_ref().is_some_and(|m| m.started.elapsed() >= m.duration) {
            target.motion = None;
        }
        let moving = target.motion.as_ref().and_then(|m| {
            let old = project.scene(m.from)?;
            Some(moving_layers(&old.layers, &scene.layers, m.started.elapsed().as_secs_f32() / m.duration.as_secs_f32()))
        });
        let layers = moving.as_deref().unwrap_or(&scene.layers);

        let vertices = &mut self.vertices;
        let draws = &mut self.draws;
        vertices.clear();
        draws.clear();
        for layer in layers.iter().filter(|l| l.visible) {
            if project.source(layer.source).is_some_and(|s|matches!(&s.kind,auroracast_core::SourceKind::VideoFile(v) if !v.canvases.is_empty() && !v.canvases.contains(&canvas.id))){continue;}
            if !self.sources.contains_key(&layer.source) {
                bail!("Source {} has no uploaded frame", layer.source);
            }
            let [mut x, mut y, mut w, mut h] = layer.rect;
            let [u0,v0,right,bottom]=layer.crop;let u1=1.-right;let v1=1.-bottom;
            if project.source(layer.source).is_some_and(|s|matches!(s.kind,auroracast_core::SourceKind::Widget(_))){let source=&self.sources[&layer.source];let ratio=source.width as f32/source.height as f32*(u1-u0)/(v1-v0)*canvas.height as f32/canvas.width as f32;let fw=w.min(h*ratio);let fh=fw/ratio;x+=(w-fw)/2.;y+=(h-fh)/2.;w=fw;h=fh;}
            let x0 = x * 2. - 1.;
            let x1 = (x + w) * 2. - 1.;
            let y0 = 1. - y * 2.;
            let y1 = 1. - (y + h) * 2.;
            // Frame outline in output pixels; plain rectangles skip the outline work entirely.
            let (hw, hh) = (w * width as f32 / 2., h * height as f32 / 2.);
            let short = hw.min(hh);
            let frame = if layer.frame.plain() {
                NO_FRAME
            } else {
                let feather = layer.frame.feather.clamp(0., 0.5) * short * 2.;
                match layer.frame.shape {
                    auroracast_core::FrameShape::Circle => [short, short, short, feather],
                    auroracast_core::FrameShape::Rounded => [hw, hh, layer.frame.radius.clamp(0., 0.5) * short * 2., feather],
                    auroracast_core::FrameShape::Square => [hw, hh, 0., feather],
                }
            };
            let key = project.preferences.chroma_keys.get(&layer.source).cloned().unwrap_or_default().shader();
            let start = vertices.len() as u32;
            for (position, uv, local) in [
                ([x0, y0], [u0,v0], [-hw, -hh]),
                ([x0, y1], [u0,v1], [-hw, hh]),
                ([x1, y1], [u1,v1], [hw, hh]),
                ([x0, y0], [u0,v0], [-hw, -hh]),
                ([x1, y1], [u1,v1], [hw, hh]),
                ([x1, y0], [u1,v0], [hw, -hh]),
            ] {
                vertices.push(Vertex {
                    local,
                    frame,
                    position,
                    uv,
                    opacity: layer.opacity,
                    key: key.0,
                    key_options: key.1,
                });
            }
            draws.push((Some(layer.source), start..start + 6));
        }
        if let Some(t) = &target.transition {
            let _keep_alive = &t.texture;
            let p = (t.started.elapsed().as_secs_f32() / t.duration.as_secs_f32()).clamp(0., 1.);
            let wipe = t.kind == auroracast_core::Transition::Wipe;
            let x = if wipe { p } else { 0. };
            let opacity = if wipe { 1. } else { 1. - p };
            let start = vertices.len() as u32;
            for (position, uv) in [
                ([x * 2. - 1., 1.], [x, 0.]),
                ([x * 2. - 1., -1.], [x, 1.]),
                ([1., -1.], [1., 1.]),
                ([x * 2. - 1., 1.], [x, 0.]),
                ([1., -1.], [1., 1.]),
                ([1., 1.], [1., 0.]),
            ] {
                vertices.push(Vertex {
                    position,
                    uv,
                    opacity,
                    key: [0., 0., 0., -1.],
                    key_options: [0.001, 0.],
                    local: [0., 0.],
                    frame: NO_FRAME,
                });
            }
            draws.push((None, start..start + 6));
        }
        // Stinger/screen overlay: on top of everything, covering the canvas without stretching.
        if let Some(o) = project.preferences.overlays.get(&canvas.id)
            && o.opacity > 0.
            && let Some(source) = self.sources.get(&o.source)
            && source.width > 1
        {
            let texture = source.width as f32 / source.height.max(1) as f32;
            let target_ratio = width as f32 / height.max(1) as f32;
            let (mut u0, mut v0, mut u1, mut v1) = (0., 0., 1., 1.);
            if texture > target_ratio {
                let keep = target_ratio / texture;
                u0 = (1. - keep) / 2.;
                u1 = 1. - u0;
            } else {
                let keep = texture / target_ratio;
                v0 = (1. - keep) / 2.;
                v1 = 1. - v0;
            }
            let start = vertices.len() as u32;
            for (position, uv) in [
                ([-1., 1.], [u0, v0]),
                ([-1., -1.], [u0, v1]),
                ([1., -1.], [u1, v1]),
                ([-1., 1.], [u0, v0]),
                ([1., -1.], [u1, v1]),
                ([1., 1.], [u1, v0]),
            ] {
                vertices.push(Vertex {
                    position,
                    uv,
                    opacity: o.opacity.clamp(0., 1.),
                    key: [0., 0., 0., -1.],
                    key_options: [0.001, 0.],
                    local: [0., 0.],
                    frame: NO_FRAME,
                });
            }
            draws.push((Some(o.source), start..start + 6));
        }
        // Keep one vertex buffer per target. Static geometry needs no GPU
        // writes; animated transitions and edits update the existing buffer.
        if !vertices.is_empty() && target.vertices != *vertices {
            let bytes = bytemuck::cast_slice(vertices);
            if target
                .vertex_buffer
                .as_ref()
                .is_none_or(|b| b.size() < bytes.len() as u64)
            {
                target.vertex_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Reusable scene vertices"),
                    size: (bytes.len() as u64).next_power_of_two(),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.vertex_allocations += 1;
            }
            self.queue
                .write_buffer(target.vertex_buffer.as_ref().unwrap(), 0, bytes);
            self.vertex_uploads += 1;
        }
        target.vertices.clone_from(vertices);
        {
            let mut pass = commands.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render canvas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.005,
                            g: 0.008,
                            b: 0.015,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            if !draws.is_empty() {
                pass.set_vertex_buffer(0, target.vertex_buffer.as_ref().unwrap().slice(..));
            }
            for (source, range) in draws.iter() {
                let binding = if let Some(source) = source {
                    &self.sources[source].binding
                } else {
                    &target.transition.as_ref().unwrap().binding
                };
                pass.set_bind_group(0, binding, &[]);
                pass.draw(range.clone(), 0..1);
            }
        }
        self.queue.submit(Some(commands.finish()));
        Ok(target.display.clone())
    }

    /// CPU pixels for export and the background recorder, never the UI preview.
    pub fn render(
        &mut self,
        project: &Project,
        canvas: &Canvas,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        self.render_gpu(project, canvas, width, height)?;
        let target = self.targets.get_mut(&(canvas.id, width, height)).unwrap();
        let staging = target.staging.get_or_insert_with(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Export readback"),
                size: target.stride as u64 * height as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        let mut commands = self.device.create_command_encoder(&Default::default());
        commands.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(commands.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(2)),
        })?;
        rx.recv().context("Readback callback missing")??;
        let mapped = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in mapped.chunks(target.stride as usize) {
            rgba.extend_from_slice(&row[..width as usize * 4]);
        }
        drop(mapped);
        staging.unmap();
        Ok(rgba)
    }
}

/// One synthetic input frame, uploaded once and referenced by both canvases.
pub fn test_pattern(width: u32, height: u32, frame: u32) -> Vec<u8> {
    let mut rgba = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            let grid = x % 80 < 2 || y % 80 < 2;
            let moving = (x + frame * 5) % width < width / 12;
            rgba[i..i + 4].copy_from_slice(&if grid {
                [45, 57, 90, 255]
            } else if moving {
                [118, 87, 226, 255]
            } else {
                [
                    18 + (x * 40 / width) as u8,
                    25 + (y * 60 / height) as u8,
                    55 + (x * 50 / width) as u8,
                    255,
                ]
            });
        }
    }
    rgba
}

#[cfg(test)]
mod transition_tests {
    use super::*;
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn stinger_overlay_covers_switch_and_only_its_canvas() -> Result<()> {
        use auroracast_core::{Layer, Overlay, SourceKind, Transition};
        let mut p = Project::default();
        p.sources[0].kind = SourceKind::Color([255, 0, 0, 255]);
        p.scenes[0].layers = vec![Layer { crop: [0.; 4], group: 0, frame: Default::default(), source: 1, rect: [0., 0., 1., 1.], visible: true, locked: false, opacity: 1. }];
        p.scenes[1].layers = p.scenes[0].layers.clone();
        p.canvases[1].scene = p.scenes[0].id;
        let mut g = pollster::block_on(Compositor::new())?;
        g.upload(1, 1, 1, &[255, 0, 0, 255])?;
        g.upload(9, 2, 2, &[0, 0, 255, 255].repeat(4))?;
        g.render(&p, &p.canvases[0], 64, 32)?;
        // A stinger switch must not start the snapshot blend; the overlay hides it instead.
        p.preferences.transition = Transition::Stinger;
        p.canvases[0].scene = p.scenes[1].id;
        p.preferences.overlays.insert(p.canvases[0].id, Overlay { source: 9, opacity: 1. });
        let covered = g.render(&p, &p.canvases[0], 64, 32)?;
        assert!(g.targets[&(p.canvases[0].id, 64, 32)].transition.is_none());
        assert_eq!(&covered[..4], &[0, 0, 255, 255], "stinger covers the canvas");
        assert_eq!(&g.render(&p, &p.canvases[1], 32, 64)?[..4], &[255, 0, 0, 255], "other canvas untouched");
        p.preferences.overlays.insert(p.canvases[0].id, Overlay { source: 9, opacity: 0.5 });
        let half = g.render(&p, &p.canvases[0], 64, 32)?;
        assert!(half[0] > 90 && half[2] > 90, "half-faded screen blends {:?}", &half[..4]);
        p.preferences.overlays.clear();
        assert_eq!(&g.render(&p, &p.canvases[0], 64, 32)?[..4], &[255, 0, 0, 255]);
        Ok(())
    }
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn fade_wipe_and_canvas_isolation() -> Result<()> {
        use auroracast_core::{Layer, SourceKind, Transition};
        let mut p = Project::default();
        p.sources[0].kind = SourceKind::Color([255, 0, 0, 255]);
        p.sources[1].kind = SourceKind::Color([0, 255, 0, 255]);
        p.scenes[0].layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(),
            source: 1,
            rect: [0., 0., 1., 1.],
            visible: true,
            locked: false,
            opacity: 1.,
        }];
        p.scenes[1].layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(),
            source: 2,
            rect: [0., 0., 1., 1.],
            visible: true,
            locked: false,
            opacity: 1.,
        }];
        p.canvases[1].scene = p.scenes[0].id;
        let mut g = pollster::block_on(Compositor::new())?;
        g.upload(1, 1, 1, &[255, 0, 0, 255])?;
        g.upload(2, 1, 1, &[0, 255, 0, 255])?;
        g.render(&p, &p.canvases[0], 64, 32)?;
        g.render(&p, &p.canvases[1], 32, 64)?;
        p.preferences.transition = Transition::Fade;
        p.preferences.transition_ms = 2000;
        p.canvases[0].scene = p.scenes[1].id;
        g.render_gpu(&p, &p.canvases[0], 64, 32)?;
        let key = (p.canvases[0].id, 64, 32);
        g.targets
            .get_mut(&key)
            .unwrap()
            .transition
            .as_mut()
            .unwrap()
            .started = std::time::Instant::now() - std::time::Duration::from_millis(1000);
        let half = g.render(&p, &p.canvases[0], 64, 32)?;
        assert!(
            half[0] > 150 && half[1] > 150 && half[2] < 5,
            "Fade pixels {:?}",
            &half[..4]
        );
        let other = g.render(&p, &p.canvases[1], 32, 64)?;
        assert_eq!(&other[..4], &[255, 0, 0, 255]);
        g.targets.get_mut(&key).unwrap().transition = None;
        g.render(&p, &p.canvases[0], 64, 32)?;
        p.preferences.transition = Transition::Wipe;
        p.canvases[0].scene = p.scenes[0].id;
        g.render_gpu(&p, &p.canvases[0], 64, 32)?;
        g.targets
            .get_mut(&key)
            .unwrap()
            .transition
            .as_mut()
            .unwrap()
            .started = std::time::Instant::now() - std::time::Duration::from_millis(1000);
        let wipe = g.render(&p, &p.canvases[0], 64, 32)?;
        assert_eq!(&wipe[..4], &[255, 0, 0, 255]);
        assert_eq!(&wipe[63 * 4..64 * 4], &[0, 255, 0, 255]);
        Ok(())
    }
}

#[cfg(test)]
mod efficiency_tests {
    use super::*;
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn reuses_geometry_and_releases_resized_targets() -> Result<()> {
        let mut p = Project::default();
        let mut g = pollster::block_on(Compositor::new())?;
        for source in &p.sources {
            g.upload(source.id, 1, 1, &[255, 0, 0, 255])?;
        }
        for _ in 0..120 {
            g.render_gpu(&p, &p.canvases[0], 64, 36)?;
            g.render_gpu(&p, &p.canvases[1], 36, 64)?;
        }
        assert_eq!(g.vertex_allocations, 2);
        assert_eq!(g.vertex_uploads, 2);
        p.scenes[0].layers[0].rect[2] = 0.5;
        let pixels = g.render(&p, &p.canvases[0], 64, 36)?;
        assert!(pixels[0] > 250);
        assert!(pixels[63 * 4] < 30, "transform must update pixels");
        assert_eq!(g.vertex_allocations, 2);
        assert_eq!(g.vertex_uploads, 3);
        p.scenes[0].layers.clear();
        let empty = g.render(&p, &p.canvases[0], 64, 36)?;
        assert!(empty[0] < 30);
        // Resizing one canvas must neither keep its old large targets nor evict
        // the other canvas's texture (which the UI may still be displaying).
        for width in 65..85 {
            g.render_gpu(&p, &p.canvases[0], width, 36)?;
            assert_eq!(g.targets.len(), 2);
            assert!(g.targets.contains_key(&(p.canvases[1].id, 36, 64)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod alert_canvas_tests {
    use super::*;
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn alert_media_canvas_routing() -> Result<()> {
        use auroracast_core::{Layer, SourceKind, VideoSettings};
        let mut p = Project::default();
        p.sources[0].kind = SourceKind::VideoFile(VideoSettings {
            canvases: vec![p.canvases[0].id],
            ..Default::default()
        });
        p.scenes[0].layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(),
            source: p.sources[0].id,
            rect: [0., 0., 1., 1.],
            visible: true,
            locked: false,
            opacity: 1.,
        }];
        p.canvases[1].scene = p.scenes[0].id;
        let mut g = pollster::block_on(Compositor::new())?;
        g.upload(p.sources[0].id, 1, 1, &[255, 0, 0, 255])?;
        let first = g.render(&p, &p.canvases[0], 32, 32)?;
        let second = g.render(&p, &p.canvases[1], 32, 32)?;
        assert_eq!(&first[..4], &[255, 0, 0, 255]);
        assert!(second[0] < 30);
        if let SourceKind::VideoFile(v) = &mut p.sources[0].kind {
            v.canvases.clear();
        }
        let both = g.render(&p, &p.canvases[1], 32, 32)?;
        assert_eq!(&both[..4], &[255, 0, 0, 255]);
        Ok(())
    }
}

#[cfg(test)]
mod chroma_tests {
    use super::*;
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn removes_key_preserves_foreground_and_updates_without_new_frame() -> Result<()> {
        use auroracast_core::{Layer, browser::ChromaKey};
        let mut p = Project::default();
        let id = p.sources[0].id;
        p.scenes[0].layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(),
            source: id,
            rect: [0., 0., 1., 1.],
            visible: true,
            locked: false,
            opacity: 1.,
        }];
        let mut key = ChromaKey::default();
        key.enabled = true;
        p.preferences.chroma_keys.insert(id, key);
        let mut g = pollster::block_on(Compositor::new())?;
        g.upload(id, 2, 1, &[0, 255, 0, 255, 255, 0, 0, 255])?;
        let pixels = g.render(&p, &p.canvases[0], 32, 32)?;
        assert!(pixels[0] < 30 && pixels[1] < 30, "green removed");
        assert!(
            pixels[31 * 4] > 240 && pixels[31 * 4 + 1] < 10,
            "red preserved"
        );
        p.preferences.chroma_keys.get_mut(&id).unwrap().enabled = false;
        let raw = g.render(&p, &p.canvases[0], 32, 32)?;
        assert!(raw[1] > 240, "toggle without a new source frame");
        assert_eq!(g.uploads, 1);
        Ok(())
    }
}

#[cfg(test)] mod crop_tests {
 use super::*;
 #[test] #[ignore="requires Vulkan; exercised in release validation"]
 fn crop_samples_source_uv_without_affecting_other_canvas()->Result<()> {
  let mut p=Project::default();p.preferences.transition=auroracast_core::Transition::Cut;
  p.sources[0].kind=auroracast_core::SourceKind::ScreenCapture;
  let mut layer=p.scenes[0].layers[0].clone();layer.source=p.sources[0].id;layer.rect=[0.,0.,1.,1.];layer.visible=true;layer.opacity=1.;layer.crop=[0.5,0.,0.,0.];
  p.scenes[0].layers=vec![layer.clone()];layer.crop=[0.;4];p.scenes[1].layers=vec![layer];p.canvases[0].scene=p.scenes[0].id;p.canvases[1].scene=p.scenes[1].id;
  let mut gpu=pollster::block_on(Compositor::new())?;let mut pixels=vec![];for _ in 0..8{for x in 0..8{pixels.extend_from_slice(if x<4{&[255,0,0,255]}else{&[0,255,0,255]});}}
  gpu.upload(p.sources[0].id,8,8,&pixels)?;let cropped=gpu.render(&p,&p.canvases[0],64,64)?;let full=gpu.render(&p,&p.canvases[1],64,64)?;
  let at=|f:&[u8],x:usize|f[(32*64+x)*4..(32*64+x)*4+4].to_vec();assert_eq!(at(&cropped,16),vec![0,255,0,255]);assert_eq!(at(&full,16),vec![255,0,0,255]);Ok(())
 }
}

#[cfg(test)]
mod frame_key_move_tests {
    use super::*;
    use auroracast_core::{FrameShape, Layer, browser::ChromaKey};
    fn full(source: Id) -> Layer {
        Layer { crop: [0.; 4], group: 0, frame: Default::default(), source, rect: [0., 0., 1., 1.], visible: true, locked: false, opacity: 1. }
    }
    fn at(f: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * size + x) * 4;
        [f[i], f[i + 1], f[i + 2], f[i + 3]]
    }
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn green_screen_removes_green_and_keeps_skin_and_red() -> Result<()> {
        let mut p = Project::default();
        let id = p.sources[0].id;
        p.scenes[0].layers = vec![full(id)];
        p.preferences.chroma_keys.insert(id, ChromaKey { enabled: true, ..Default::default() });
        let mut g = pollster::block_on(Compositor::new())?;
        // Studio green, a skin tone, red.
        g.upload(id, 3, 1, &[40, 200, 60, 255, 224, 172, 140, 255, 220, 30, 30, 255])?;
        let px = g.render(&p, &p.canvases[0], 30, 30)?;
        // Removed means the canvas's dark navy background (about 16, 24, 33) shows through.
        let green = at(&px, 30, 4, 15);
        assert!(green[1] < 30 && green[0] < 25 && green[2] < 40, "green removed: {green:?}");
        let skin = at(&px, 30, 15, 15);
        assert!(skin[0] > 180 && skin[1] > 130, "skin kept: {skin:?}");
        let red = at(&px, 30, 25, 15);
        assert!(red[0] > 180, "red kept: {red:?}");
        Ok(())
    }
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn circle_frame_clears_corners_and_keeps_centre() -> Result<()> {
        let mut p = Project::default();
        let id = p.sources[0].id;
        let mut layer = full(id);
        layer.frame.shape = FrameShape::Circle;
        p.scenes[0].layers = vec![layer];
        let mut g = pollster::block_on(Compositor::new())?;
        g.upload(id, 1, 1, &[255, 255, 255, 255])?;
        let px = g.render(&p, &p.canvases[0], 64, 64)?;
        assert!(at(&px, 64, 32, 32)[0] > 240, "centre drawn");
        assert!(at(&px, 64, 2, 2)[0] < 20, "corner cleared");
        // A feathered edge is partly see-through between the two.
        p.scenes[0].layers[0].frame.feather = 0.3;
        let soft = g.render(&p, &p.canvases[0], 64, 64)?;
        let edge = at(&soft, 64, 32, 4)[0];
        assert!(edge > 10 && edge < 200, "edge fades: {edge}");
        Ok(())
    }
    #[test]
    fn move_glides_shared_layers_and_fades_the_rest() {
        let mut a = full(1);
        a.rect = [0., 0., 0.2, 0.2];
        let mut b = full(1);
        b.rect = [0.8, 0.8, 0.2, 0.2];
        let only_old = full(2);
        let only_new = full(3);
        let mid = moving_layers(&[a.clone(), only_old], &[b.clone(), only_new], 0.5);
        // Old-only first (underneath), then the new scene's layers in order.
        assert_eq!(mid.iter().map(|l| l.source).collect::<Vec<_>>(), vec![2, 1, 3]);
        assert!((mid[1].rect[0] - 0.4).abs() < 1e-5, "halfway: {:?}", mid[1].rect);
        assert!((mid[0].opacity - 0.5).abs() < 1e-5 && (mid[2].opacity - 0.5).abs() < 1e-5);
        let end = moving_layers(&[a], &[b.clone()], 1.);
        assert_eq!(end[0].rect, b.rect);
    }
}
