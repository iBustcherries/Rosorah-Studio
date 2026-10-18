//! Shared source textures and reusable offscreen render targets. Rosorah's own compositor:
//! every layer's settings go into one table per frame, so effects cost nothing when unused.
use anyhow::{Context, Result, bail};
use auroracast_core::{BlendMode, Canvas, FrameShape, Id, Layer, LayerTransition, Project, SourceKind, Transition};
use bytemuck::{Pod, Zeroable};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    /// Position from the layer's centre in pixels (before rotation), for outlines and masks.
    local: [f32; 2],
    /// Which entry of the layer table this corner belongs to.
    layer: u32,
    _pad: u32,
}
/// One layer's settings as the shader reads them (see composite.wgsl for each field).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
struct LayerParams {
    basic: [f32; 4],
    key: [f32; 4],
    key_options: [f32; 4],
    frame: [f32; 4],
    half: [f32; 4],
    colour: [f32; 4],
    colour2: [f32; 4],
    luma: [f32; 4],
    scroll: [f32; 4],
    crop: [f32; 4],
    tint: [f32; 4],
    wipe: [f32; 4],
}
const NO_FRAME: [f32; 4] = [-1., 0., 0., 0.];
impl LayerParams {
    /// Drawn as it is: no key, frame, colour work or pattern.
    fn plain(opacity: f32, half: [f32; 2]) -> Self {
        Self {
            basic: [opacity, 0., 0., 0.],
            key: [0., 0., 0., -1.],
            key_options: [0.001, 0., 0., 0.],
            frame: NO_FRAME,
            half: [half[0], half[1], 0., 0.],
            colour: [0., 0., 1., 0.],
            colour2: [1., 0., 0., 0.],
            luma: [0., 0., 1., 0.05],
            scroll: [0., 0., 0., 0.],
            crop: [0., 0., 1., 1.],
            tint: [1., 1., 1., 1.],
            wipe: [0.; 4],
        }
    }
}
/// What a draw takes its picture from.
#[derive(Clone, Copy, PartialEq)]
enum Picture {
    Source(Id),
    /// The old scene, during a transition.
    Snapshot,
    /// Plain white, for solid colour areas (tinted).
    White,
}
struct Draw {
    picture: Picture,
    /// Look-up table and mask pictures, by file; None uses the empty set.
    extras: Option<(String, String)>,
    blend: BlendMode,
    vertices: std::ops::Range<u32>,
}
/// Between two scenes with the Move transition: layers glide from where they were.
struct Motion {
    from: Id,
    started: Instant,
    duration: Duration,
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
fn moving_layers(old: &[Layer], new: &[Layer], p: f32) -> Vec<Layer> {
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
                l.look.rotation = mix(o.look.rotation, layer.look.rotation, t);
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
        .map(|(_, o)| Layer { opacity: o.opacity * (1. - t), ..o.clone() })
        .collect();
    out.extend(arriving);
    out
}
/// A layer part-way through its show or hide animation: `k` is 0 (gone) … 1 (fully there).
fn appearing(layer: &mut Layer, how: LayerTransition, k: f32) {
    let k = ease(k.clamp(0., 1.));
    let [x, y, w, h] = layer.rect;
    match how {
        LayerTransition::None => {}
        LayerTransition::Fade => layer.opacity *= k,
        LayerTransition::Zoom => {
            layer.rect = [x + w * (1. - k) / 2., y + h * (1. - k) / 2., w * k, h * k];
            layer.opacity *= k;
        }
        LayerTransition::SlideLeft => layer.rect[0] = mix(-w, x, k),
        LayerTransition::SlideRight => layer.rect[0] = mix(1., x, k),
        LayerTransition::SlideUp => layer.rect[1] = mix(-h, y, k),
        LayerTransition::SlideDown => layer.rect[1] = mix(1., y, k),
    }
}
struct SharedSource {
    texture: wgpu::Texture,
    binding: wgpu::BindGroup,
    width: u32,
    height: u32,
}
/// Past frames of a source shown late (render delay), oldest first.
struct DelayRing {
    frames: VecDeque<(Instant, SharedSource)>,
    spare: Vec<SharedSource>,
}
struct TransitionFrame {
    /// Held so the old scene's picture lives as long as the transition draws it.
    _texture: wgpu::Texture,
    binding: wgpu::BindGroup,
    started: Instant,
    duration: Duration,
    kind: Transition,
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
    params: Vec<LayerParams>,
    param_buffer: Option<wgpu::Buffer>,
    param_binding: Option<wgpu::BindGroup>,
    /// Each layer's visibility last frame and when it last changed, for show/hide animations.
    shown: HashMap<(Id, usize), (bool, Instant)>,
    /// When this target was last drawn; nested-scene and thumbnail targets are dropped when idle.
    used: Instant,
}
/// Pipelines for each blend mode.
struct Pipelines {
    normal: wgpu::RenderPipeline,
    add: wgpu::RenderPipeline,
    multiply: wgpu::RenderPipeline,
    screen: wgpu::RenderPipeline,
    lighten: wgpu::RenderPipeline,
    darken: wgpu::RenderPipeline,
}
impl Pipelines {
    fn get(&self, blend: BlendMode) -> &wgpu::RenderPipeline {
        match blend {
            BlendMode::Normal => &self.normal,
            BlendMode::Add => &self.add,
            BlendMode::Multiply => &self.multiply,
            BlendMode::Screen => &self.screen,
            BlendMode::Lighten => &self.lighten,
            BlendMode::Darken => &self.darken,
        }
    }
}
/// The output mode the shader uses for each blend mode (0 straight, 1 premultiplied, 2 toward white).
fn output_mode(blend: BlendMode) -> f32 {
    match blend {
        BlendMode::Normal => 0.,
        BlendMode::Darken => 2.,
        _ => 1.,
    }
}

pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Pipelines,
    layout: wgpu::BindGroupLayout,
    params_layout: wgpu::BindGroupLayout,
    extras_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    sources: HashMap<Id, SharedSource>,
    targets: HashMap<(Id, u32, u32), Target>,
    white: SharedSource,
    /// The look-up table and mask pictures in use, loaded once per file.
    extras: HashMap<(String, String), wgpu::BindGroup>,
    no_extras: wgpu::BindGroup,
    /// Sources shown late, and how late (render delay).
    delays: HashMap<Id, Duration>,
    rings: HashMap<Id, DelayRing>,
    /// Nested scenes drawn this frame, as pictures layers can show.
    nested: HashMap<Id, (wgpu::BindGroup, u32, u32)>,
    /// Whether the last frame had something moving by itself (a scrolling layer).
    continuous: bool,
    pub adapter_name: String,
    pub software_adapter: bool,
    pub uploads: u64,
    pub vertex_allocations: u64,
    pub vertex_uploads: u64,
    vertices: Vec<Vertex>,
    params: Vec<LayerParams>,
    draws: Vec<Draw>,
}

/// A .cube colour look-up table: its size and RGBA texels (red varying fastest).
fn read_cube(path: &str) -> Result<(u32, Vec<u8>)> {
    let text = std::fs::read_to_string(path).with_context(|| format!("Couldn't read {path}"))?;
    let mut size = 0u32;
    let mut texels = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some(n) = line.strip_prefix("LUT_3D_SIZE") {
            size = n.trim().parse().context("Bad LUT_3D_SIZE")?;
            continue;
        }
        if line.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let values: Vec<f32> = line.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        if values.len() == 3 {
            texels.extend(values.iter().map(|v| (v.clamp(0., 1.) * 255.).round() as u8).chain([255]));
        }
    }
    ensure_lut(size, &texels)?;
    Ok((size, texels))
}
fn ensure_lut(size: u32, texels: &[u8]) -> Result<()> {
    if !(2..=65).contains(&size) || texels.len() != (size * size * size * 4) as usize {
        bail!("Not a 3D .cube look-up table");
    }
    Ok(())
}

impl Compositor {
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: if cfg!(target_os = "windows") { wgpu::Backends::DX12 | wgpu::Backends::VULKAN } else { wgpu::Backends::VULKAN },
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })
            .await
            .context("No Vulkan adapter; install your GPU's Vulkan driver")?;
        let info = adapter.get_info();
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor { label: Some("Rosorah Studio compositor"), ..Default::default() }).await?;
        Ok(Self::from_device(device, queue, info))
    }

    /// Reuse the UI device so the canvas can be sampled directly by egui.
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, info: wgpu::AdapterInfo) -> Self {
        let texture_entry = |binding, dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dimension, multisampled: false },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shared image"),
            entries: &[texture_entry(0, wgpu::TextureViewDimension::D2), sampler_entry(1)],
        });
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Layer table"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let extras_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Look-up table and mask"),
            entries: &[texture_entry(0, wgpu::TextureViewDimension::D3), texture_entry(1, wgpu::TextureViewDimension::D2), sampler_entry(2)],
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
            bind_group_layouts: &[&layout, &params_layout, &extras_layout],
            push_constant_ranges: &[],
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Uint32];
        let pipeline = |label, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8UnormSrgb, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
        let alpha = C { src_factor: F::One, dst_factor: F::OneMinusSrcAlpha, operation: O::Add };
        let with = |color: C| wgpu::BlendState { color, alpha };
        let pipelines = Pipelines {
            normal: pipeline("Normal", wgpu::BlendState::ALPHA_BLENDING),
            add: pipeline("Add", with(C { src_factor: F::One, dst_factor: F::One, operation: O::Add })),
            multiply: pipeline("Multiply", with(C { src_factor: F::Dst, dst_factor: F::OneMinusSrcAlpha, operation: O::Add })),
            screen: pipeline("Screen", with(C { src_factor: F::One, dst_factor: F::OneMinusSrc, operation: O::Add })),
            lighten: pipeline("Lighten", with(C { src_factor: F::One, dst_factor: F::One, operation: O::Max })),
            darken: pipeline("Darken", with(C { src_factor: F::One, dst_factor: F::One, operation: O::Min })),
        };
        let white = Self::make_source(&device, &queue, &layout, &sampler, 1, 1, &[255, 255, 255, 255]);
        let no_extras = Self::make_extras(&device, &queue, &extras_layout, &sampler, (2, &identity_lut(2)), (1, 1, &[255, 255, 255, 255]));
        Self {
            device,
            queue,
            pipelines,
            layout,
            params_layout,
            extras_layout,
            sampler,
            sources: HashMap::new(),
            targets: HashMap::new(),
            white,
            extras: HashMap::new(),
            no_extras,
            delays: HashMap::new(),
            rings: HashMap::new(),
            nested: HashMap::new(),
            continuous: false,
            adapter_name: info.name,
            software_adapter: info.device_type == wgpu::DeviceType::Cpu,
            uploads: 0,
            vertex_allocations: 0,
            vertex_uploads: 0,
            vertices: Vec::new(),
            params: Vec::new(),
            draws: Vec::new(),
        }
    }
    fn make_source(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, width: u32, height: u32, rgba: &[u8]) -> SharedSource {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shared source texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        let source = SharedSource { texture, binding, width, height };
        Self::write(queue, &source, rgba);
        source
    }
    fn write(queue: &wgpu::Queue, source: &SharedSource, rgba: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &source.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(source.width * 4), rows_per_image: Some(source.height) },
            wgpu::Extent3d { width: source.width, height: source.height, depth_or_array_layers: 1 },
        );
    }
    /// A bind group with a look-up table and a mask picture.
    fn make_extras(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, lut: (u32, &[u8]), mask: (u32, u32, &[u8])) -> wgpu::BindGroup {
        let (size, texels) = lut;
        let lut_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Look-up table"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: size },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &lut_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            texels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 4), rows_per_image: Some(size) },
            wgpu::Extent3d { width: size, height: size, depth_or_array_layers: size },
        );
        let (mw, mh, mask_pixels) = mask;
        let mask_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Mask"),
            size: wgpu::Extent3d { width: mw, height: mh, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &mask_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            mask_pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(mw * 4), rows_per_image: Some(mh) },
            wgpu::Extent3d { width: mw, height: mh, depth_or_array_layers: 1 },
        );
        let lut_view = lut_texture.create_view(&Default::default());
        let mask_view = mask_texture.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Look-up table and mask"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&lut_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&mask_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }
    /// Loads (once) the look-up table and mask a layer uses. Files that can't be read are left out.
    fn extras_for(&mut self, lut: &str, mask: &str) -> (bool, bool) {
        let key = (lut.to_owned(), mask.to_owned());
        let lut_data = (!lut.is_empty()).then(|| read_cube(lut).ok()).flatten();
        let mask_data = (!mask.is_empty()).then(|| image::open(mask).ok().map(|i| i.thumbnail(1024, 1024).to_rgba8())).flatten();
        let ok = (lut_data.is_some(), mask_data.is_some());
        if !self.extras.contains_key(&key) && (ok.0 || ok.1) {
            let lut_pair = lut_data.unwrap_or_else(|| (2, identity_lut(2)));
            let mask_image = mask_data.unwrap_or_else(|| image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4])));
            let group = Self::make_extras(&self.device, &self.queue, &self.extras_layout, &self.sampler, (lut_pair.0, &lut_pair.1), (mask_image.width(), mask_image.height(), mask_image.as_raw()));
            // A small cache: different files replace the oldest entries.
            if self.extras.len() >= 16 {
                self.extras.clear();
            }
            self.extras.insert(key, group);
        }
        ok
    }

    pub fn ensure_empty_source(&mut self, id: Id) -> Result<()> {
        if !self.sources.contains_key(&id) {
            self.upload(id, 1, 1, &[0, 0, 0, 0])?;
        }
        Ok(())
    }
    pub fn upload(&mut self, id: Id, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
        if width == 0 || height == 0 || width > 4096 || height > 4096 || rgba.len() != width as usize * height as usize * 4 {
            bail!("Invalid source frame dimensions or length");
        }
        // A delayed source keeps its recent frames; what's drawn is the one from `delay` ago.
        if let Some(delay) = self.delays.get(&id).copied() {
            let ring = self.rings.entry(id).or_insert_with(|| DelayRing { frames: VecDeque::new(), spare: Vec::new() });
            // Frames older than needed are recycled; at most 64 are kept (about 2 s at 30 fps).
            while ring.frames.len() > 1 && (ring.frames.len() >= 64 || ring.frames[1].0.elapsed() > delay) {
                let (_, old) = ring.frames.pop_front().unwrap();
                ring.spare.push(old);
            }
            let reused = ring.spare.iter().position(|s| s.width == width && s.height == height).map(|i| ring.spare.swap_remove(i));
            ring.spare.retain(|s| s.width == width && s.height == height);
            let frame = match reused {
                Some(frame) => {
                    Self::write(&self.queue, &frame, rgba);
                    frame
                }
                None => Self::make_source(&self.device, &self.queue, &self.layout, &self.sampler, width, height, rgba),
            };
            ring.frames.push_back((Instant::now(), frame));
            self.uploads += 1;
            return Ok(());
        }
        if self.sources.get(&id).is_none_or(|s| s.width != width || s.height != height) {
            let source = Self::make_source(&self.device, &self.queue, &self.layout, &self.sampler, width, height, rgba);
            self.sources.insert(id, source);
        } else {
            Self::write(&self.queue, &self.sources[&id], rgba);
        }
        self.uploads += 1;
        Ok(())
    }
    /// The picture to draw for a source now: its delayed frame, a nested scene, or its latest frame.
    fn picture(&self, id: Id) -> Option<(&wgpu::BindGroup, u32, u32)> {
        if let Some((group, w, h)) = self.nested.get(&id) {
            return Some((group, *w, *h));
        }
        if let (Some(delay), Some(ring)) = (self.delays.get(&id), self.rings.get(&id)) {
            let frame = ring.frames.iter().rev().find(|(at, _)| at.elapsed() >= *delay).or(ring.frames.front());
            if let Some((_, f)) = frame {
                return Some((&f.binding, f.width, f.height));
            }
        }
        self.sources.get(&id).map(|s| (&s.binding, s.width, s.height))
    }

    pub fn retain_sources(&mut self, project: &Project) {
        self.sources.retain(|id, _| project.source(*id).is_some());
        self.rings.retain(|id, _| project.source(*id).is_some());
    }

    pub fn animating(&self) -> bool {
        self.targets.values().any(|t| {
            t.transition.as_ref().is_some_and(|f| f.started.elapsed() < f.duration)
                || t.motion.as_ref().is_some_and(|m| m.started.elapsed() < m.duration)
                || t.shown.values().any(|(_, at)| at.elapsed() < Duration::from_secs(2))
        }) || !self.delays.is_empty()
            || self.continuous
    }

    /// Draws `scene` into a target of its own (Studio Mode's preview, Multiview thumbnails).
    /// `key` tells the targets apart; it never starts a transition.
    pub fn render_scene(&mut self, project: &Project, key: Id, scene: Id, width: u32, height: u32) -> Result<wgpu::TextureView> {
        let template = project.canvases.first().context("No canvas")?;
        let canvas = Canvas { id: key, scene, width, height, ..template.clone() };
        self.render_gpu(project, &canvas, width, height)
    }

    /// GPU-only canvas presentation: no map, CPU copy, or blocking device poll.
    pub fn render_gpu(&mut self, project: &Project, canvas: &Canvas, width: u32, height: u32) -> Result<wgpu::TextureView> {
        self.render_depth(project, canvas, width, height, 0)
    }
    fn render_depth(&mut self, project: &Project, canvas: &Canvas, width: u32, height: u32, depth: u32) -> Result<wgpu::TextureView> {
        if !(16..=4096).contains(&width) || !(16..=4096).contains(&height) {
            bail!("Invalid render dimensions");
        }
        let scene = project.scene(canvas.scene).context("Canvas references a missing scene")?;
        if depth == 0 {
            // Which sources are shown late; their frames start collecting from now.
            self.delays.clear();
            for layer in project.scenes.iter().flat_map(|s| &s.layers).filter(|l| l.visible && l.look.delay_ms > 0) {
                let delay = Duration::from_millis(layer.look.delay_ms.min(2000) as u64);
                let entry = self.delays.entry(layer.source).or_default();
                *entry = (*entry).max(delay);
            }
            self.rings.retain(|id, _| self.delays.contains_key(id));
            self.nested.clear();
            self.continuous = scene.layers.iter().any(|l| l.visible && l.look.scroll != [0., 0.]);
            // Scratch targets nobody has drawn for a while (closed thumbnails, old previews) are freed.
            self.targets.retain(|_, t| t.used.elapsed() < Duration::from_secs(5));
        }
        // Bound cache; targets are scratch surfaces, not separate source captures.
        if !self.targets.contains_key(&(canvas.id, width, height)) {
            // A resized canvas replaces its old scratch surface immediately.
            self.targets.retain(|(id, _, _), _| *id != canvas.id);
            // At most 24 scratch pictures; the ones unused the longest go first.
            while self.targets.len() >= 24 {
                let oldest = self.targets.iter().min_by_key(|(_, t)| t.used).map(|(k, _)| *k).unwrap();
                self.targets.remove(&oldest);
            }
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Canvas target"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
            });
            let view = texture.create_view(&Default::default());
            let stride = (width * 4).div_ceil(256) * 256;
            // Egui expects gamma-encoded samples. Reinterpret the sRGB target as Unorm for display.
            let display = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(wgpu::TextureFormat::Rgba8Unorm), ..Default::default() });
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
                    params: Vec::new(),
                    param_buffer: None,
                    param_binding: None,
                    shown: HashMap::new(),
                    used: Instant::now(),
                },
            );
        }
        let mut commands = self.device.create_command_encoder(&Default::default());
        // The transition into this scene: its own override, or the usual one.
        let (kind, ms) = project
            .preferences
            .scene_transitions
            .get(&canvas.scene)
            .map_or((project.preferences.transition, project.preferences.transition_ms), |t| (t.kind, t.ms));
        let duration = Duration::from_millis(ms.clamp(100, 3000) as u64);
        {
            let target = self.targets.get_mut(&(canvas.id, width, height)).unwrap();
            target.used = Instant::now();
            if target.scene.is_some_and(|s| s != canvas.scene) && kind.blends() && depth == 0 {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Transition snapshot"),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                commands.copy_texture_to_texture(target.texture.as_image_copy(), texture.as_image_copy(), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
                let view = texture.create_view(&Default::default());
                let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Transition image"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    ],
                });
                target.transition = Some(TransitionFrame { _texture: texture, binding, started: Instant::now(), duration, kind });
            } else if let Some(from) = target.scene.filter(|s| *s != canvas.scene) {
                target.transition = None;
                // Move needs no snapshot: the old scene's layers are interpolated as they are drawn.
                target.motion = (kind == Transition::Move && depth == 0).then(|| Motion { from, started: Instant::now(), duration });
            }
            if target.scene != Some(canvas.scene) {
                target.shown.clear();
            }
            target.scene = Some(canvas.scene);
            if target.transition.as_ref().is_some_and(|t| t.started.elapsed() >= t.duration) {
                target.transition = None;
            }
            if target.motion.as_ref().is_some_and(|m| m.started.elapsed() >= m.duration) {
                target.motion = None;
            }
        }
        // The layers to draw: moving between scenes, or this scene's, with show/hide animations.
        let mut layers: Vec<Layer> = {
            let target = &self.targets[&(canvas.id, width, height)];
            match target.motion.as_ref().and_then(|m| Some((project.scene(m.from)?, m))) {
                Some((old, m)) => moving_layers(&old.layers, &scene.layers, m.started.elapsed().as_secs_f32() / m.duration.as_secs_f32()),
                None => Vec::new(),
            }
        };
        if layers.is_empty() {
            let target = self.targets.get_mut(&(canvas.id, width, height)).unwrap();
            for (i, layer) in scene.layers.iter().enumerate() {
                let entry = target.shown.entry((scene.id, i)).or_insert((layer.visible, Instant::now() - Duration::from_secs(60)));
                if entry.0 != layer.visible {
                    *entry = (layer.visible, Instant::now());
                }
                let how = if layer.visible { layer.look.show } else { layer.look.hide };
                let span = Duration::from_millis(layer.look.transition_ms.clamp(50, 3000) as u64);
                let animating = how != LayerTransition::None && entry.1.elapsed() < span;
                if layer.visible || animating {
                    let mut l = layer.clone();
                    l.visible = true;
                    if animating {
                        let k = entry.1.elapsed().as_secs_f32() / span.as_secs_f32();
                        appearing(&mut l, how, if layer.visible { k } else { 1. - k });
                    }
                    layers.push(l);
                }
            }
        }
        // Nested scenes are drawn first, into pictures of their own (three levels at most, no loops).
        if depth < 3 {
            for layer in layers.iter().filter(|l| l.visible) {
                if let Some(SourceKind::Scene(inner)) = project.source(layer.source).map(|s| &s.kind)
                    && *inner != canvas.scene
                    && !self.nested.contains_key(&layer.source)
                    && project.scene(*inner).is_some()
                {
                    let nested = Canvas { id: layer.source ^ 0x5ce0_0000_0000_0000, scene: *inner, width: canvas.width, height: canvas.height, ..canvas.clone() };
                    let (w, h) = (width, height);
                    let view = self.render_depth(project, &nested, w, h, depth + 1)?;
                    let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Nested scene"),
                        layout: &self.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                        ],
                    });
                    self.nested.insert(layer.source, (group, w, h));
                }
            }
        }
        let (transition, progress) = {
            let target = &self.targets[&(canvas.id, width, height)];
            match &target.transition {
                Some(t) => (Some(t.kind), (t.started.elapsed().as_secs_f32() / t.duration.as_secs_f32()).clamp(0., 1.)),
                None => (None, 0.),
            }
        };
        // Slide and Swipe move the whole new scene in from the right.
        let shift = match transition {
            Some(Transition::Slide | Transition::Swipe) => 1. - ease(progress),
            _ => 0.,
        };
        self.vertices.clear();
        self.params.clear();
        self.draws.clear();
        let quad = |vertices: &mut Vec<Vertex>, corners: [[f32; 2]; 4], uv: [f32; 4], half: [f32; 2], layer: u32| -> std::ops::Range<u32> {
            let start = vertices.len() as u32;
            let [u0, v0, u1, v1] = uv;
            let local = [[-half[0], -half[1]], [-half[0], half[1]], [half[0], half[1]], [half[0], -half[1]]];
            let uvs = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
            for i in [0, 1, 2, 0, 2, 3] {
                vertices.push(Vertex { position: corners[i], uv: uvs[i], local: local[i], layer, _pad: 0 });
            }
            start..start + 6
        };
        let full = [[-1., 1.], [-1., -1.], [1., -1.], [1., 1.]];
        let half_canvas = [width as f32 / 2., height as f32 / 2.];
        // Under Slide and Swipe the old scene is drawn first, and the new scene's area is filled
        // with the canvas colour so the old one can't show through its gaps.
        if let Some(kind @ (Transition::Slide | Transition::Swipe)) = transition {
            let out = if kind == Transition::Slide { -ease(progress) * 2. } else { 0. };
            let index = self.params.len() as u32;
            self.params.push(LayerParams::plain(1., half_canvas));
            let corners = full.map(|[x, y]| [x + out, y]);
            let range = quad(&mut self.vertices, corners, [0., 0., 1., 1.], half_canvas, index);
            self.draws.push(Draw { picture: Picture::Snapshot, extras: None, blend: BlendMode::Normal, vertices: range });
            let index = self.params.len() as u32;
            let mut fill = LayerParams::plain(1., half_canvas);
            fill.tint = [0.005, 0.008, 0.015, 1.];
            self.params.push(fill);
            let corners = full.map(|[x, y]| [x + shift * 2., y]);
            let range = quad(&mut self.vertices, corners, [0., 0., 1., 1.], half_canvas, index);
            self.draws.push(Draw { picture: Picture::White, extras: None, blend: BlendMode::Normal, vertices: range });
        }
        let fade_to_colour_first_half = transition == Some(Transition::FadeToColor) && progress < 0.5;
        if !fade_to_colour_first_half {
            for layer in layers.iter().filter(|l| l.visible) {
                if project.source(layer.source).is_some_and(|s| matches!(&s.kind, SourceKind::VideoFile(v) if !v.canvases.is_empty() && !v.canvases.contains(&canvas.id))) {
                    continue;
                }
                let Some((_, sw, sh)) = self.picture(layer.source) else {
                    if project.source(layer.source).is_some_and(|s| matches!(s.kind, SourceKind::Scene(_))) {
                        continue;
                    }
                    bail!("Source {} has no uploaded frame", layer.source);
                };
                let [mut x, mut y, mut w, mut h] = layer.rect;
                let [u0, v0, right, bottom] = layer.crop;
                let (u1, v1) = (1. - right, 1. - bottom);
                if project.source(layer.source).is_some_and(|s| matches!(s.kind, SourceKind::Widget(_))) {
                    let ratio = sw as f32 / sh as f32 * (u1 - u0) / (v1 - v0) * canvas.height as f32 / canvas.width as f32;
                    let fw = w.min(h * ratio);
                    let fh = fw / ratio;
                    x += (w - fw) / 2.;
                    y += (h - fh) / 2.;
                    w = fw;
                    h = fh;
                }
                let look = &layer.look;
                // Layer size and centre in output pixels; rotation turns the corners around the centre.
                let (hw, hh) = (w * width as f32 / 2., h * height as f32 / 2.);
                let (cx, cy) = ((x + w / 2.) * width as f32 + shift * width as f32, (y + h / 2.) * height as f32);
                let (sin, cos) = look.rotation.to_radians().sin_cos();
                let corner = |lx: f32, ly: f32| {
                    let (rx, ry) = (lx * cos - ly * sin, lx * sin + ly * cos);
                    [(cx + rx) / width as f32 * 2. - 1., 1. - (cy + ry) / height as f32 * 2.]
                };
                let corners = [corner(-hw, -hh), corner(-hw, hh), corner(hw, hh), corner(hw, -hh)];
                let (crop_u0, crop_v0, crop_u1, crop_v1) = (u0, v0, u1, v1);
                // Flipping reads the shown part of the source back to front.
                let (u0, u1) = if look.flip_horizontal { (u1, u0) } else { (u0, u1) };
                let (v0, v1) = if look.flip_vertical { (v1, v0) } else { (v0, v1) };
                let short = hw.min(hh);
                let frame = if layer.frame.plain() {
                    NO_FRAME
                } else {
                    let feather = layer.frame.feather.clamp(0., 0.5) * short * 2.;
                    match layer.frame.shape {
                        FrameShape::Circle => [short, short, short, feather],
                        FrameShape::Rounded => [hw, hh, layer.frame.radius.clamp(0., 0.5) * short * 2., feather],
                        FrameShape::Square => [hw, hh, 0., feather],
                    }
                };
                let key = project.preferences.chroma_keys.get(&layer.source).cloned().unwrap_or_default().shader();
                let (has_lut, has_mask) = if look.lut.is_empty() && look.mask.is_empty() { (false, false) } else { self.extras_for(&look.lut, &look.mask) };
                let time = (Instant::now() - *START).as_secs_f32();
                let mut params = LayerParams::plain(layer.opacity, [hw, hh]);
                params.basic = [layer.opacity, output_mode(look.blend), if has_mask { 1. } else { 0. }, if look.mask_invert { 1. } else { 0. }];
                params.key = key.0;
                params.key_options = [key.1[0], key.1[1], 0., 0.];
                params.frame = frame;
                params.colour = [look.brightness, look.contrast, look.saturation, look.hue.to_radians()];
                params.colour2 = [look.gamma, if has_lut { look.lut_amount.clamp(0., 1.) } else { 0. }, look.sharpen.clamp(0., 1.), if look.colour_adjusted() { 1. } else { 0. }];
                params.luma = [if look.luma_key { 1. } else { 0. }, look.luma_min, look.luma_max, look.luma_softness];
                params.scroll = [(look.scroll[0] * time).fract(), (look.scroll[1] * time).fract(), 1. / sw as f32, 1. / sh as f32];
                params.crop = [crop_u0, crop_v0, crop_u1, crop_v1];
                let index = self.params.len() as u32;
                self.params.push(params);
                let range = quad(&mut self.vertices, corners, [u0, v0, u1, v1], [hw, hh], index);
                let extras = (has_lut || has_mask).then(|| (look.lut.clone(), look.mask.clone()));
                self.draws.push(Draw { picture: Picture::Source(layer.source), extras, blend: look.blend, vertices: range });
            }
        }
        // The old scene on top: fading, wiping, revealing through a pattern, or fading to a colour.
        match transition {
            Some(Transition::Fade | Transition::Wipe) => {
                let wipe = transition == Some(Transition::Wipe);
                let x = if wipe { progress } else { 0. };
                let index = self.params.len() as u32;
                self.params.push(LayerParams::plain(if wipe { 1. } else { 1. - progress }, half_canvas));
                let corners = [[x * 2. - 1., 1.], [x * 2. - 1., -1.], [1., -1.], [1., 1.]];
                let range = quad(&mut self.vertices, corners, [x, 0., 1., 1.], half_canvas, index);
                self.draws.push(Draw { picture: Picture::Snapshot, extras: None, blend: BlendMode::Normal, vertices: range });
            }
            Some(Transition::PatternWipe) => {
                let style = &project.preferences.transition_style;
                let index = self.params.len() as u32;
                let mut params = LayerParams::plain(1., half_canvas);
                params.wipe = [style.pattern.clamp(1, 7) as f32, progress, style.softness.clamp(0., 0.5), 0.];
                self.params.push(params);
                let range = quad(&mut self.vertices, full, [0., 0., 1., 1.], half_canvas, index);
                self.draws.push(Draw { picture: Picture::Snapshot, extras: None, blend: BlendMode::Normal, vertices: range });
            }
            Some(Transition::FadeToColor) => {
                if fade_to_colour_first_half {
                    let index = self.params.len() as u32;
                    self.params.push(LayerParams::plain(1., half_canvas));
                    let range = quad(&mut self.vertices, full, [0., 0., 1., 1.], half_canvas, index);
                    self.draws.push(Draw { picture: Picture::Snapshot, extras: None, blend: BlendMode::Normal, vertices: range });
                }
                let [r, g, b] = project.preferences.transition_style.color.map(|c| (c as f32 / 255.).powf(2.2));
                let strength = if progress < 0.5 { progress * 2. } else { 2. - progress * 2. };
                let index = self.params.len() as u32;
                let mut fill = LayerParams::plain(1., half_canvas);
                fill.tint = [r, g, b, strength];
                self.params.push(fill);
                let range = quad(&mut self.vertices, full, [0., 0., 1., 1.], half_canvas, index);
                self.draws.push(Draw { picture: Picture::White, extras: None, blend: BlendMode::Normal, vertices: range });
            }
            _ => {}
        }
        // Stinger/screen overlay: on top of everything, covering the canvas without stretching.
        if let Some(o) = project.preferences.overlays.get(&canvas.id)
            && o.opacity > 0.
            && let Some((_, sw, sh)) = self.picture(o.source)
            && sw > 1
        {
            let texture = sw as f32 / sh.max(1) as f32;
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
            let index = self.params.len() as u32;
            self.params.push(LayerParams::plain(o.opacity.clamp(0., 1.), half_canvas));
            let range = quad(&mut self.vertices, full, [u0, v0, u1, v1], half_canvas, index);
            self.draws.push(Draw { picture: Picture::Source(o.source), extras: None, blend: BlendMode::Normal, vertices: range });
        }
        let target = self.targets.get_mut(&(canvas.id, width, height)).unwrap();
        // One vertex buffer and one layer table per target, grown when needed and reused.
        if !self.vertices.is_empty() && target.vertices != self.vertices {
            let bytes = bytemuck::cast_slice(&self.vertices);
            if target.vertex_buffer.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
                target.vertex_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Reusable scene vertices"),
                    size: (bytes.len() as u64).next_power_of_two(),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.vertex_allocations += 1;
            }
            self.queue.write_buffer(target.vertex_buffer.as_ref().unwrap(), 0, bytes);
            self.vertex_uploads += 1;
        }
        target.vertices.clone_from(&self.vertices);
        if !self.params.is_empty() && target.params != self.params {
            let bytes = bytemuck::cast_slice(&self.params);
            if target.param_buffer.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Layer table"),
                    size: (bytes.len() as u64).next_power_of_two().max(256),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                target.param_binding = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Layer table"),
                    layout: &self.params_layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
                }));
                target.param_buffer = Some(buffer);
            }
            self.queue.write_buffer(target.param_buffer.as_ref().unwrap(), 0, bytes);
            target.params.clone_from(&self.params);
        }
        {
            let mut pass = commands.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render canvas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.005, g: 0.008, b: 0.015, a: 1. }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if !self.draws.is_empty() {
                pass.set_vertex_buffer(0, target.vertex_buffer.as_ref().unwrap().slice(..));
                pass.set_bind_group(1, target.param_binding.as_ref().unwrap(), &[]);
            }
            let mut current = None;
            for draw in &self.draws {
                if current != Some(draw.blend) {
                    pass.set_pipeline(self.pipelines.get(draw.blend));
                    current = Some(draw.blend);
                }
                let binding = match draw.picture {
                    Picture::Source(id) => match self.nested.get(&id) {
                        Some((group, _, _)) => group,
                        None => match (self.delays.get(&id), self.rings.get(&id)) {
                            (Some(delay), Some(ring)) if !ring.frames.is_empty() => {
                                let frame = ring.frames.iter().rev().find(|(at, _)| at.elapsed() >= *delay).or(ring.frames.front());
                                &frame.unwrap().1.binding
                            }
                            _ => &self.sources[&id].binding,
                        },
                    },
                    Picture::Snapshot => &target.transition.as_ref().unwrap().binding,
                    Picture::White => &self.white.binding,
                };
                pass.set_bind_group(0, binding, &[]);
                let extras = draw.extras.as_ref().and_then(|k| self.extras.get(k)).unwrap_or(&self.no_extras);
                pass.set_bind_group(2, extras, &[]);
                pass.draw(draw.vertices.clone(), 0..1);
            }
        }
        self.queue.submit(Some(commands.finish()));
        Ok(target.display.clone())
    }

    /// CPU pixels for export and the background recorder, never the UI preview.
    pub fn render(&mut self, project: &Project, canvas: &Canvas, width: u32, height: u32) -> Result<Vec<u8>> {
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
            wgpu::TexelCopyTextureInfo { texture: &target.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: staging, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(target.stride), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(commands.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(2)) })?;
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
/// The moment the compositor first ran, so scrolling layers move smoothly over time.
static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
/// A look-up table that changes nothing.
fn identity_lut(size: u32) -> Vec<u8> {
    let mut texels = Vec::with_capacity((size * size * size * 4) as usize);
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                let v = |c: u32| (c * 255 / (size - 1)) as u8;
                texels.extend([v(r), v(g), v(b), 255]);
            }
        }
    }
    texels
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
        p.scenes[0].layers = vec![Layer { crop: [0.; 4], group: 0, frame: Default::default(), look: Default::default(), source: 1, rect: [0., 0., 1., 1.], visible: true, locked: false, opacity: 1. }];
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
        p.scenes[0].layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(), look: Default::default(),
            source: 1,
            rect: [0., 0., 1., 1.],
            visible: true,
            locked: false,
            opacity: 1.,
        }];
        p.scenes[1].layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(), look: Default::default(),
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
        p.scenes[0].layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(), look: Default::default(),
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
        p.scenes[0].layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(), look: Default::default(),
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
        Layer { crop: [0.; 4], group: 0, frame: Default::default(), look: Default::default(), source, rect: [0., 0., 1., 1.], visible: true, locked: false, opacity: 1. }
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

#[cfg(test)]
mod look_tests {
    use super::*;
    use auroracast_core::{Layer, Look, SceneTransition, SourceKind};
    const N: u32 = 32;
    fn layer(source: Id, rect: [f32; 4]) -> Layer {
        Layer { crop: [0.; 4], group: 0, frame: Default::default(), look: Default::default(), source, rect, visible: true, locked: false, opacity: 1. }
    }
    fn px(f: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * N + x) * 4) as usize;
        [f[i], f[i + 1], f[i + 2], f[i + 3]]
    }
    /// A project with one full-canvas layer showing a 2×1 picture: red on the left, blue on the right.
    fn setup() -> Result<(Project, Compositor, Id)> {
        let mut p = Project::default();
        p.preferences.transition = Transition::Cut;
        let id = p.sources[0].id;
        p.scenes[0].layers = vec![layer(id, [0., 0., 1., 1.])];
        p.canvases[0].scene = p.scenes[0].id;
        let mut g = pollster::block_on(Compositor::new())?;
        g.upload(id, 2, 1, &[255, 0, 0, 255, 0, 0, 255, 255])?;
        Ok((p, g, id))
    }
    fn render(g: &mut Compositor, p: &Project) -> Result<Vec<u8>> {
        g.render(p, &p.canvases[0], N, N)
    }
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn flip_and_rotation_turn_the_picture() -> Result<()> {
        let (mut p, mut g, _) = setup()?;
        let normal = render(&mut g, &p)?;
        assert!(px(&normal, 2, 16)[0] > 200 && px(&normal, 29, 16)[2] > 200, "red left, blue right");
        p.scenes[0].layers[0].look.flip_horizontal = true;
        let flipped = render(&mut g, &p)?;
        assert!(px(&flipped, 2, 16)[2] > 200 && px(&flipped, 29, 16)[0] > 200, "flipped");
        p.scenes[0].layers[0].look = Look { rotation: 180., ..Default::default() };
        let turned = render(&mut g, &p)?;
        assert!(px(&turned, 2, 16)[2] > 200 && px(&turned, 29, 16)[0] > 200, "turned around");
        Ok(())
    }
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn colour_correction_luma_key_and_blend() -> Result<()> {
        let (mut p, mut g, _) = setup()?;
        p.scenes[0].layers[0].look.saturation = 0.;
        let grey = px(&render(&mut g, &p)?, 2, 16);
        assert!((grey[0] as i32 - grey[1] as i32).abs() < 6 && (grey[1] as i32 - grey[2] as i32).abs() < 6, "no colour left: {grey:?}");
        // Luma key keeping only bright parts removes both (red and blue are fairly dark).
        p.scenes[0].layers[0].look = Look { luma_key: true, luma_min: 0.8, ..Default::default() };
        let keyed = px(&render(&mut g, &p)?, 2, 16);
        assert!(keyed[0] < 40, "dark red removed: {keyed:?}");
        // Multiply over the canvas colour (near black) leaves it near black; Darken keeps the darker.
        for blend in [BlendMode::Multiply, BlendMode::Darken] {
            p.scenes[0].layers[0].look = Look { blend, ..Default::default() };
            let out = px(&render(&mut g, &p)?, 2, 16);
            assert!(out[0] < 40, "{blend:?}: {out:?}");
        }
        p.scenes[0].layers[0].look = Look { blend: BlendMode::Screen, ..Default::default() };
        assert!(px(&render(&mut g, &p)?, 2, 16)[0] > 200, "screen brightens");
        Ok(())
    }
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn lut_and_mask_files_apply() -> Result<()> {
        let (mut p, mut g, _) = setup()?;
        let dir = std::env::temp_dir().join(format!("rosorah-look-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // A LUT that swaps red and blue.
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 { for gr in 0..2 { for r in 0..2 { cube += &format!("{b} {gr} {r}\n"); } } }
        std::fs::write(dir.join("swap.cube"), cube)?;
        p.scenes[0].layers[0].look = Look { lut: dir.join("swap.cube").display().to_string(), ..Default::default() };
        let graded = px(&render(&mut g, &p)?, 2, 16);
        assert!(graded[2] > 200 && graded[0] < 60, "red became blue: {graded:?}");
        // A mask that's white on the left and black on the right hides the right half.
        let mask = image::RgbaImage::from_fn(2, 1, |x, _| if x == 0 { image::Rgba([255; 4]) } else { image::Rgba([0, 0, 0, 255]) });
        mask.save(dir.join("mask.png"))?;
        p.scenes[0].layers[0].look = Look { mask: dir.join("mask.png").display().to_string(), ..Default::default() };
        let masked = render(&mut g, &p)?;
        assert!(px(&masked, 2, 16)[0] > 200 && px(&masked, 29, 16)[2] < 40, "right half hidden");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn transitions_slide_pattern_and_colour() -> Result<()> {
        let (mut p, mut g, id) = setup()?;
        let other = p.scenes[1].id;
        p.scenes[1].layers = vec![layer(id, [0., 0., 1., 1.])];
        p.scenes[1].layers[0].look.flip_horizontal = true;
        for kind in [Transition::Slide, Transition::Swipe, Transition::PatternWipe, Transition::FadeToColor] {
            p.canvases[0].scene = p.scenes[0].id;
            p.preferences.transition = Transition::Cut;
            render(&mut g, &p)?;
            p.preferences.transition = kind;
            p.preferences.transition_ms = 400;
            p.canvases[0].scene = other;
            render(&mut g, &p)?;
            assert!(g.animating(), "{kind:?} animates");
            std::thread::sleep(Duration::from_millis(450));
            let done = render(&mut g, &p)?;
            assert!(px(&done, 2, 16)[2] > 200, "{kind:?} ends on the new scene");
        }
        // A per-scene override wins over the usual transition.
        p.preferences.scene_transitions.insert(p.scenes[0].id, SceneTransition { kind: Transition::Cut, ms: 300 });
        p.preferences.transition = Transition::Fade;
        p.canvases[0].scene = p.scenes[0].id;
        let instant = render(&mut g, &p)?;
        assert!(px(&instant, 2, 16)[0] > 200, "cut straight to scene one");
        Ok(())
    }
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn nested_scene_and_show_animation() -> Result<()> {
        let (mut p, mut g, id) = setup()?;
        // Scene two holds the picture; scene one shows scene two in its right half.
        p.scenes[1].layers = vec![layer(id, [0., 0., 1., 1.])];
        let nested = p.id();
        p.sources.push(auroracast_core::Source { id: nested, name: "Nested".into(), kind: SourceKind::Scene(p.scenes[1].id) });
        p.scenes[0].layers = vec![layer(nested, [0.5, 0., 0.5, 1.])];
        let out = render(&mut g, &p)?;
        assert!(px(&out, 18, 16)[0] > 200 && px(&out, 30, 16)[2] > 200, "nested scene drawn in the right half");
        assert!(px(&out, 4, 16)[0] < 40, "left half empty");
        // Hiding with a fade keeps drawing it for a moment.
        p.scenes[0].layers[0].look.hide = LayerTransition::Fade;
        p.scenes[0].layers[0].look.transition_ms = 500;
        p.scenes[0].layers[0].visible = false;
        let fading = render(&mut g, &p)?;
        assert!(px(&fading, 18, 16)[0] > 60, "still fading out");
        Ok(())
    }
}
