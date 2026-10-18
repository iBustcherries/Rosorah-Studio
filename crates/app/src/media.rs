use anyhow::{Context, Result, bail};
use auroracast_core::{Canvas, Project, SourceKind};
use auroracast_gpu::{Compositor, test_pattern};
use std::{
    io::Write,
    path::Path,
    process::{Child, Command, Stdio},
};

/// Tracks what each compositor already holds, independently of UI/project
/// revisions. A transform, mixer edit or second preview never re-decodes an image.
#[derive(Default)]
pub struct Sources {
    uploaded: std::collections::BTreeMap<auroracast_core::Id, SourceStamp>,
}
#[derive(PartialEq)]
struct SourceStamp {
    kind: SourceKind,
    file: Option<(std::time::SystemTime, u64)>,
    frame: u32,
}
impl Sources {
    pub fn upload(
        &mut self,
        gpu: &mut Compositor,
        project: &Project,
        frame: u32,
        only_animated: bool,
    ) -> Result<()> {
        let used: std::collections::BTreeSet<_> = project
            .canvases
            .iter()
            .filter_map(|c| project.scene(c.scene))
            .flat_map(|s| s.layers.iter().filter(|l| l.visible).map(|l| l.source))
            .collect();
        for source in &project.sources {
            if !used.contains(&source.id)
                || (only_animated && !matches!(source.kind, SourceKind::TestPattern))
            {
                continue;
            }
            if matches!(
                source.kind,
                SourceKind::Browser { .. }
                    | SourceKind::ScreenCapture
                    | SourceKind::VideoCamera(_)
                    | SourceKind::VideoFile(_)
                    | SourceKind::Widget(_)
            ) {
                gpu.ensure_empty_source(source.id)?;
                self.uploaded.remove(&source.id);
                continue;
            }
            // Nested scenes are drawn by the compositor; there's nothing to load.
            if matches!(source.kind, SourceKind::Scene(_)) {
                continue;
            }
            // A slideshow loads one picture at a time, only when it's time to change.
            if let SourceKind::Slideshow(show) = &source.kind {
                let files = auroracast_core::folder_files(&show.folder, &auroracast_core::PICTURE_EXTENSIONS);
                if files.is_empty() {
                    gpu.ensure_empty_source(source.id)?;
                    continue;
                }
                let step = (crate::media::seconds_running() / show.seconds.max(0.5) as f64) as u64;
                let round = step / files.len() as u64;
                let index = auroracast_core::play_order(files.len(), round, show.shuffle)[(step % files.len() as u64) as usize];
                let stamp = SourceStamp { kind: source.kind.clone(), file: None, frame: index as u32 + 1 };
                if self.uploaded.get(&source.id) != Some(&stamp) {
                    let picture = image::ImageReader::open(&files[index])?.with_guessed_format()?.decode().with_context(|| format!("Decode {}", files[index]))?;
                    let picture = if picture.width() > 4096 || picture.height() > 4096 { picture.thumbnail(4096, 4096) } else { picture }.into_rgba8();
                    gpu.upload(source.id, picture.width(), picture.height(), picture.as_raw())?;
                    self.uploaded.insert(source.id, stamp);
                }
                continue;
            }
            let stamp = SourceStamp {
                kind: source.kind.clone(),
                file: if let SourceKind::Image { path } = &source.kind {
                    let metadata =
                        std::fs::metadata(path).with_context(|| format!("Read {path}"))?;
                    Some((metadata.modified()?, metadata.len()))
                } else {
                    None
                },
                frame: if matches!(source.kind, SourceKind::TestPattern) {
                    frame
                } else {
                    0
                },
            };
            if self.uploaded.get(&source.id) == Some(&stamp) {
                continue;
            }
            match &source.kind {
                SourceKind::TestPattern => {
                    gpu.upload(source.id, 640, 360, &test_pattern(640, 360, frame))?
                }
                SourceKind::Color(color) => gpu.upload(source.id, 1, 1, color)?,
                SourceKind::Image { path } => {
                    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
                    let mut limits = image::Limits::default();
                    limits.max_image_width = Some(4096);
                    limits.max_image_height = Some(4096);
                    reader.limits(limits);
                    let image = reader
                        .decode()
                        .with_context(|| format!("Decode {path}"))?
                        .into_rgba8();
                    gpu.upload(source.id, image.width(), image.height(), image.as_raw())?;
                }
                _ => bail!("{} is not implemented in this build", source.kind.label()),
            }
            // Failed reads/uploads remain retryable.
            self.uploaded.insert(source.id, stamp);
        }
        self.uploaded.retain(|id, _| project.source(*id).is_some());
        gpu.retain_sources(project);
        Ok(())
    }
}

pub fn snapshot(project: &Project, output: &Path) -> Result<()> {
    project.validate()?;
    if project.sources.iter().any(|s| {
        matches!(
            s.kind,
            SourceKind::ScreenCapture | SourceKind::VideoCamera(_)
        )
    }) {
        bail!(
            "Live capture recording/export is not implemented yet. Use a synthetic-only project for this test."
        );
    }
    std::fs::create_dir_all(output)?;
    let mut gpu = pollster::block_on(Compositor::new())?;
    let mut sources = Sources::default();
    sources.upload(&mut gpu, project, 0, false)?;
    for (i, canvas) in project.canvases.iter().enumerate() {
        let path = output.join(format!("canvas-{}.png", i + 1));
        if path.exists() {
            bail!("Output already exists: {}", path.display());
        }
        let rgba = gpu.render(project, canvas, canvas.width, canvas.height)?;
        image::save_buffer(
            &path,
            &rgba,
            canvas.width,
            canvas.height,
            image::ColorType::Rgba8,
        )?;
        println!(
            "Saved {} ({}x{})",
            path.display(),
            canvas.width,
            canvas.height
        );
    }
    println!(
        "Adapter: {} | source uploads: {}",
        gpu.adapter_name, gpu.uploads
    );
    Ok(())
}

struct Encoder {
    child: Child,
    closed: bool,
}
impl Encoder {
    fn new(canvas: &Canvas, path: &Path) -> Result<Self> {
        let size = format!("{}x{}", canvas.width, canvas.height);
        let fps = canvas.fps.to_string();
        let child = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-n",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
                &size,
                "-framerate",
                &fps,
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Install ffmpeg with libx264 to record the demo")?;
        Ok(Self {
            child,
            closed: false,
        })
    }
    fn write(&mut self, rgba: &[u8]) -> Result<()> {
        self.child
            .stdin
            .as_mut()
            .context("Encoder input closed")?
            .write_all(rgba)
            .context("Encoder rejected frame")
    }
    fn finish(&mut self) -> Result<()> {
        drop(self.child.stdin.take());
        let status = self.child.wait()?;
        self.closed = true;
        if !status.success() {
            bail!("ffmpeg exited with {status}");
        }
        Ok(())
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.closed {
            drop(self.child.stdin.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn render_demo(project: &Project, output: &Path, seconds: u32) -> Result<()> {
    project.validate()?;
    if project.sources.iter().any(|s| {
        matches!(
            s.kind,
            SourceKind::ScreenCapture | SourceKind::VideoCamera(_)
        )
    }) {
        bail!(
            "Live capture recording/export is not implemented yet. Use a synthetic-only project for this test."
        );
    }
    // One clock drives all canvases in this first encoder proof.
    let fps = project.canvases[0].fps;
    if project
        .canvases
        .iter()
        .any(|c| c.fps != fps || c.width % 2 != 0 || c.height % 2 != 0)
    {
        bail!("Demo canvases must share an FPS and have even dimensions");
    }
    std::fs::create_dir_all(output)?;
    let paths: Vec<_> = (0..project.canvases.len())
        .map(|i| output.join(format!("canvas-{}.mp4", i + 1)))
        .collect();
    if paths.iter().any(|p| p.exists()) {
        bail!("Output exists; choose an empty output folder");
    }
    let mut gpu = pollster::block_on(Compositor::new())?;
    let mut sources = Sources::default();
    println!(
        "Adapter: {}{}",
        gpu.adapter_name,
        if gpu.software_adapter {
            " (software Vulkan)"
        } else {
            ""
        }
    );
    sources.upload(&mut gpu, project, 0, false)?;
    let mut encoders = Vec::new();
    for (canvas, path) in project.canvases.iter().zip(&paths) {
        encoders.push(Encoder::new(canvas, path)?);
    }
    for frame in 0..seconds * fps {
        if frame > 0 {
            sources.upload(&mut gpu, project, frame, true)?;
        }
        for (canvas, encoder) in project.canvases.iter().zip(encoders.iter_mut()) {
            encoder.write(&gpu.render(project, canvas, canvas.width, canvas.height)?)?;
        }
    }
    for encoder in &mut encoders {
        encoder.finish()?;
    }
    for path in &paths {
        println!("Saved {}", path.display());
    }
    println!(
        "{} frames per canvas; {} source uploads, shared across {} canvases",
        seconds * fps,
        gpu.uploads,
        project.canvases.len()
    );
    Ok(())
}

pub fn self_check() -> Result<()> {
    let mut project = Project::default();
    project.sources[0].kind = SourceKind::Color([200, 30, 60, 255]);
    project.sources[1].kind = SourceKind::Color([20, 220, 50, 255]);
    project.sources.push(auroracast_core::Source {
        id: 20,
        name: "Unused image".into(),
        kind: SourceKind::Image {
            path: "/missing/unused-image.png".into(),
        },
    });
    let mut gpu = pollster::block_on(Compositor::new())?;
    let mut sources = Sources::default();
    sources.upload(&mut gpu, &project, 0, false)?;
    let a = gpu.render(&project, &project.canvases[0], 64, 36)?;
    let b = gpu.render(&project, &project.canvases[1], 36, 64)?;
    let close = |pixel: &[u8], expected: [u8; 4]| {
        pixel.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2)
    };
    if !close(&a[0..4], [200, 30, 60, 255]) || !close(&b[0..4], [200, 30, 60, 255]) {
        bail!("Shared source pixel mismatch");
    }
    let offset = ((5 * 64 + 54) * 4) as usize;
    if !close(&a[offset..offset + 4], [20, 220, 50, 255]) {
        bail!("Overlay placement mismatch");
    }
    // Layer ordering must affect the actual GPU composition used by outputs.
    project.scenes[0].layers[0].locked = false;
    let moved = project.scenes[0]
        .reorder_layer(0, auroracast_core::LayerOrder::Front)
        .context("Could not bring background to front")?;
    let reordered = gpu.render(&project, &project.canvases[0], 64, 36)?;
    if !close(&reordered[offset..offset + 4], [200, 30, 60, 255]) {
        bail!("Layer stacking order did not change the compositor output");
    }
    project.scenes[0].reorder_layer(moved, auroracast_core::LayerOrder::Back);
    project.scenes[0].layers[1].visible = false;
    let hidden = gpu.render(&project, &project.canvases[0], 64, 36)?;
    if !close(&hidden[offset..offset + 4], [200, 30, 60, 255]) {
        bail!("Source visibility mismatch");
    }
    // Validate partially transparent layers compose in linear light.
    project.scenes[0].layers[1].visible = true;
    project.scenes[0].layers[1].opacity = 0.5;
    let blended = gpu.render(&project, &project.canvases[0], 64, 36)?;
    if !close(&blended[offset..offset + 4], [147, 163, 55, 255]) {
        bail!(
            "Alpha composition mismatch: {:?}",
            &blended[offset..offset + 4]
        );
    }
    // Reopened capture sources are transparent until permission is requested.
    let capture_id = 99;
    gpu.ensure_empty_source(capture_id)?;
    let old_id = project.scenes[0].layers[0].source;
    project.scenes[0].layers[0].source = capture_id;
    let waiting = gpu.render(&project, &project.canvases[0], 64, 36)?;
    if !close(&waiting[0..4], [16, 22, 33, 255]) {
        bail!("Stopped capture is not transparent");
    }
    project.scenes[0].layers[0].source = old_id;
    gpu.upload(capture_id, 1, 1, &[200, 30, 60, 255])?;
    gpu.ensure_empty_source(capture_id)?; // Must not clear an already uploaded frame.
    let previous = project.scenes[0].layers[0].source;
    project.scenes[0].layers[0].source = capture_id;
    let capture = gpu.render(&project, &project.canvases[0], 64, 36)?;
    if !close(&capture[0..4], [200, 30, 60, 255]) {
        bail!("Capture texture was overwritten by placeholder");
    }
    project.scenes[0].layers[0].source = previous;
    if gpu.uploads != 5 {
        bail!("Source uploads were duplicated per canvas");
    }
    project.scenes[0].layers.clear();
    let empty = gpu.render(&project, &project.canvases[0], 64, 36)?;
    if !close(&empty[0..4], [16, 22, 33, 255]) {
        bail!("Empty scene did not clear correctly");
    }
    println!(
        "PASS: two canvas sizes, shared textures, layer placement and stacking, visibility, alpha, row padding, unused assets, empty scene\nAdapter: {}{}",
        gpu.adapter_name,
        if gpu.software_adapter {
            " (software Vulkan; hardware performance unverified)"
        } else {
            ""
        }
    );
    Ok(())
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    #[ignore = "requires Vulkan; exercised in release validation"]
    fn static_assets_reuse_invalidate_and_recover() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rosorah-assets-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let file = dir.join("image.png");
        image::save_buffer(&file, &[255, 0, 0, 255], 1, 1, image::ColorType::Rgba8)?;
        let mut p = Project::default();
        p.sources[0].kind = SourceKind::Image {
            path: file.display().to_string(),
        };
        p.sources[1].kind = SourceKind::Color([0, 255, 0, 255]);
        let mut g = pollster::block_on(Compositor::new())?;
        let mut cache = Sources::default();
        cache.upload(&mut g, &p, 0, false)?;
        let initial = g.uploads;
        for n in 0..120 {
            p.audio[0].gain_db = -(n as f32);
            p.scenes[0].layers[0].rect[2] = 0.5;
            cache.upload(&mut g, &p, 0, false)?;
        }
        assert_eq!(
            g.uploads, initial,
            "unrelated edits must not re-upload assets"
        );
        image::save_buffer(
            &file,
            &[0, 0, 255, 255].repeat(4),
            2,
            2,
            image::ColorType::Rgba8,
        )?;
        cache.upload(&mut g, &p, 0, false)?;
        assert_eq!(g.uploads, initial + 1);
        let pixel = g.render(&p, &p.canvases[0], 64, 36)?;
        assert!(
            pixel[2] > 250 && pixel[0] < 5,
            "replaced file must be visible"
        );
        std::fs::remove_file(&file)?;
        assert!(cache.upload(&mut g, &p, 0, false).is_err());
        image::save_buffer(&file, &[0, 255, 0, 255], 1, 1, image::ColorType::Rgba8)?;
        cache.upload(&mut g, &p, 0, false)?;
        let pixel = g.render(&p, &p.canvases[0], 64, 36)?;
        assert!(pixel[1] > 250, "failed reads must recover");
        p.sources[0].kind = SourceKind::Color([255, 0, 0, 255]);
        cache.upload(&mut g, &p, 0, false)?;
        assert!(g.render(&p, &p.canvases[0], 64, 36)?[0] > 250);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
    #[test]
    #[ignore = "performance comparison; requires Vulkan"]
    fn measure_repeated_image_edits() -> Result<()> {
        let file = std::env::temp_dir().join(format!("rosorah-perf-{}.png", std::process::id()));
        image::save_buffer(
            &file,
            &test_pattern(1920, 1080, 0),
            1920,
            1080,
            image::ColorType::Rgba8,
        )?;
        let mut p = Project::default();
        p.sources[0].kind = SourceKind::Image {
            path: file.display().to_string(),
        };
        p.sources[1].kind = SourceKind::Color([0, 255, 0, 255]);
        let mut g = pollster::block_on(Compositor::new())?;
        let mut cached = Sources::default();
        cached.upload(&mut g, &p, 0, false)?;
        g.render(&p, &p.canvases[0], 64, 36)?;
        for warm_cache in [false, true] {
            let initial = g.uploads;
            let start = std::time::Instant::now();
            for n in 0..60 {
                p.scenes[0].layers[0].rect[2] = 0.5 + n as f32 / 120.;
                if warm_cache {
                    cached.upload(&mut g, &p, 0, false)?;
                } else {
                    Sources::default().upload(&mut g, &p, 0, false)?;
                }
                g.render(&p, &p.canvases[0], 64, 36)?;
            }
            eprintln!(
                "IMAGE_EDIT_BENCH cached={warm_cache} elapsed_ms={} source_uploads={}",
                start.elapsed().as_millis(),
                g.uploads - initial
            );
        }
        std::fs::remove_file(file)?;
        Ok(())
    }
}
/// Seconds since Rosorah started, for things that change on a schedule (slideshows).
pub fn seconds_running() -> f64 {
    static START: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
    START.elapsed().as_secs_f64()
}
