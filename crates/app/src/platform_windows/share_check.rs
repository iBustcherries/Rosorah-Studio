//! `rosorah-studio --gpu-share-check`: hands pictures to the compositor on the graphics card the
//! way window capture does, once with Direct3D 12 and once with Vulkan, and checks what's drawn.
use anyhow::{Context, Result, bail};
use auroracast_core::Project;
use auroracast_gpu::Compositor;

/// Runs the check and shows the result in a message box (it's also printed).
pub fn run() -> Result<()> {
    let mut lines = vec!["Game capture on the graphics card".to_string(), String::new()];
    let mut passed = false;
    for (name, backends) in [("Direct3D 12", eframe::wgpu::Backends::DX12), ("Vulkan", eframe::wgpu::Backends::VULKAN)] {
        match check(backends) {
            Ok(adapter) => {
                passed = true;
                lines.push(format!("{name}: works ({adapter})"));
            }
            Err(e) => lines.push(format!("{name}: doesn't work: {e:#}")),
        }
    }
    let text = lines.join("\n");
    println!("{text}");
    show(&text, passed);
    if passed { Ok(()) } else { bail!("Game capture on the graphics card doesn't work here") }
}

/// Six frames of different colours, so every shared texture is used and handed back at least once.
fn check(backends: eframe::wgpu::Backends) -> Result<String> {
    let mut gpu = pollster::block_on(Compositor::with_backends(backends)).context("no graphics card with this API")?;
    let luid = auroracast_gpu::external::adapter().context("the compositor can't open shared pictures")?;
    let (width, height) = (64, 36);
    let mut capture = rosorah_windows::window::ShareCheck::new(luid, width, height)?;
    // Only the full-screen layer of the first source, which gets the shared pictures.
    let mut project = Project::default();
    let id = project.sources[0].id;
    for scene in &mut project.scenes {
        scene.layers.retain(|l| l.source == id);
    }
    for colour in [[200, 30, 60, 255], [20, 220, 50, 255], [30, 60, 210, 255], [250, 250, 10, 255], [10, 10, 10, 255], [128, 64, 192, 255]] {
        let frame = capture.frame(colour)?.context("the compositor didn't hand a picture back")?;
        crate::capture::upload_shared(&mut gpu, id, frame.shared.as_ref().context("the picture wasn't shared")?)?;
        drop(frame);
        let pixels = gpu.render(&project, &project.canvases[0], width, height)?;
        let centre = ((height / 2 * width + width / 2) * 4) as usize;
        for at in [0, centre] {
            if !pixels[at..at + 4].iter().zip(colour).all(|(a, b)| a.abs_diff(b) <= 2) {
                bail!("drew {:?} instead of {:?}", &pixels[at..at + 4], colour);
            }
        }
    }
    Ok(gpu.adapter_name.clone())
}

fn show(text: &str, passed: bool) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, title: *const u16, flags: u32) -> i32;
    }
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Rosorah Studio".encode_utf16().chain(Some(0)).collect();
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), if passed { 0x40 } else { 0x10 }) };
}
