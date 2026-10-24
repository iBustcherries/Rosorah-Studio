//! Turning finished canvases into NV12 on the graphics card and copying them out without waiting.
//! The CPU only touches the final picture, which is 3/8 the size of the RGBA canvas, and the next
//! frame can be drawn while the last one is still on its way out.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

/// One NV12 picture to make from a canvas: its size, and where the canvas goes inside it
/// (left, top, width, height).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Nv12 {
    pub width: u32,
    pub height: u32,
    pub fit: [u32; 4],
}
impl Nv12 {
    /// The canvas at its own size; None when its width isn't a multiple of 4 or its height is odd.
    pub fn same(width: u32, height: u32) -> Option<Self> {
        Self::fitted(width, height, width, height)
    }
    /// The canvas scaled to fit `width` x `height`, centred, with black bars.
    pub fn fitted(source_width: u32, source_height: u32, width: u32, height: u32) -> Option<Self> {
        if width < 4 || height < 2 || !width.is_multiple_of(4) || !height.is_multiple_of(2) || source_width == 0 || source_height == 0 {
            return None;
        }
        let scale = (width as f64 / source_width as f64).min(height as f64 / source_height as f64);
        let fit_width = (((source_width as f64 * scale).round() as u32) & !1).clamp(2, width);
        let fit_height = (((source_height as f64 * scale).round() as u32) & !1).clamp(2, height);
        Some(Self { width, height, fit: [((width - fit_width) / 2) & !1, ((height - fit_height) / 2) & !1, fit_width, fit_height] })
    }
    pub fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * 3 / 2
    }
}

struct Converter {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}
/// A frame on its way out of the graphics card.
struct Flight {
    tag: u64,
    parts: Vec<wgpu::Buffer>,
    mapped: Arc<AtomicUsize>,
    failed: Arc<AtomicBool>,
    submission: wgpu::SubmissionIndex,
}
#[derive(Default)]
pub(crate) struct State {
    converter: Option<Converter>,
    /// The converted picture and its settings, per picture and canvas size (reused every frame;
    /// the queue keeps each frame's use of them in order).
    outputs: HashMap<(Nv12, u32, u32), (wgpu::Buffer, wgpu::Buffer)>,
    /// Read-back buffers not in use, by size.
    free: HashMap<u64, Vec<wgpu::Buffer>>,
    flights: VecDeque<Flight>,
}

impl Compositor {
    fn converter(&mut self) -> &Converter {
        let device = &self.device;
        self.nv12.converter.get_or_insert_with(|| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("NV12"), source: wgpu::ShaderSource::Wgsl(include_str!("nv12.wgsl").into()) });
            let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("NV12"),
                entries: &[
                    entry(0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                    entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                    entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
                    entry(3, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("NV12"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("NV12"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("NV12 scaling"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            Converter { pipeline, layout, sampler }
        })
    }

    /// Draws the canvas and starts copying it out as each of `pictures`; returns without waiting.
    /// The pictures come back from `finished_nv12` with the same `tag`.
    pub fn begin_nv12(&mut self, project: &Project, canvas: &Canvas, width: u32, height: u32, pictures: &[Nv12], tag: u64) -> Result<()> {
        self.render_gpu(project, canvas, width, height)?;
        self.converter();
        let converter = self.nv12.converter.as_ref().unwrap();
        let target = self.targets.get(&(canvas.id, width, height)).context("Canvas target missing")?;
        let mut commands = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("NV12") });
        let mut parts = Vec::with_capacity(pictures.len());
        for picture in pictures {
            let size = picture.bytes() as u64;
            let (storage, params) = self.nv12.outputs.entry((*picture, width, height)).or_insert_with(|| {
                let storage = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("NV12 picture"), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
                let params = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("NV12 settings"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let [left, top, fit_width, fit_height] = picture.fit;
                self.queue.write_buffer(&params, 0, bytemuck::cast_slice(&[picture.width, picture.height, left, top, fit_width, fit_height, width, height]));
                (storage, params)
            });
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("NV12"),
                layout: &converter.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&target.display) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&converter.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: storage.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: params.as_entire_binding() },
                ],
            });
            {
                let mut pass = commands.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("NV12"), timestamp_writes: None });
                pass.set_pipeline(&converter.pipeline);
                pass.set_bind_group(0, &binding, &[]);
                pass.dispatch_workgroups((picture.width / 4).div_ceil(8), (picture.height / 2).div_ceil(8), 1);
            }
            let staging = self.nv12.free.get_mut(&size).and_then(Vec::pop).unwrap_or_else(|| {
                self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("NV12 read-back"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
            });
            commands.copy_buffer_to_buffer(storage, 0, &staging, 0, size);
            parts.push(staging);
        }
        let submission = self.queue.submit(Some(commands.finish()));
        let (mapped, failed) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        for part in &parts {
            let (mapped, failed) = (mapped.clone(), failed.clone());
            part.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                if result.is_err() {
                    failed.store(true, Ordering::Relaxed);
                }
                mapped.fetch_add(1, Ordering::Relaxed);
            });
        }
        self.nv12.flights.push_back(Flight { tag, parts, mapped, failed, submission });
        Ok(())
    }

    /// Frames whose copy has finished, oldest first, each with one picture per requested `Nv12`.
    /// With `wait`, blocks until at least the oldest frame in flight is back.
    pub fn finished_nv12(&mut self, wait: bool) -> Result<Vec<(u64, Vec<Vec<u8>>)>> {
        match self.nv12.flights.front() {
            None => return Ok(Vec::new()),
            Some(oldest) if wait => {
                self.device.poll(wgpu::PollType::Wait { submission_index: Some(oldest.submission.clone()), timeout: Some(Duration::from_secs(2)) })?;
            }
            Some(_) => {
                self.device.poll(wgpu::PollType::Poll)?;
            }
        }
        let mut done = Vec::new();
        while let Some(flight) = self.nv12.flights.front()
            && flight.mapped.load(Ordering::Relaxed) == flight.parts.len()
        {
            let flight = self.nv12.flights.pop_front().unwrap();
            if flight.failed.load(Ordering::Relaxed) {
                bail!("Couldn't copy the picture out of the graphics card");
            }
            let mut pictures = Vec::with_capacity(flight.parts.len());
            for part in flight.parts {
                pictures.push(part.slice(..).get_mapped_range().to_vec());
                part.unmap();
                self.nv12.free.entry(part.size()).or_default().push(part);
            }
            done.push((flight.tag, pictures));
        }
        Ok(done)
    }

    /// How many frames are still on their way out.
    pub fn nv12_in_flight(&self) -> usize {
        self.nv12.flights.len()
    }

    /// Draws and converts one frame and waits for it (for tests).
    #[cfg(test)]
    pub fn render_nv12(&mut self, project: &Project, canvas: &Canvas, width: u32, height: u32, pictures: &[Nv12]) -> Result<Vec<Vec<u8>>> {
        let tag = u64::MAX;
        self.begin_nv12(project, canvas, width, height, pictures, tag)?;
        loop {
            for (done, pictures) in self.finished_nv12(true)? {
                if done == tag {
                    return Ok(pictures);
                }
            }
        }
    }
}
