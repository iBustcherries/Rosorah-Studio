mod system_monitor;
mod accounts;
mod activity;
mod audio;
mod audio_fx;
mod automations;
mod automation_preview;
mod backups;
mod profiles;
mod alert_guide;
mod alert_samples;
mod announce;
mod app_audio;
mod appearance;
mod fonts;
mod rewards_ui;
mod browser_chat;
mod browser_dock;
mod browser_source;
mod camera;
mod camera_layout;
mod canvas_edit;
mod capture;
mod capture_log;
mod chat_render;
#[cfg(not(windows))]
mod desktop;
#[cfg(windows)]
#[path = "platform_windows/desktop.rs"]
mod desktop;
mod donations;
mod donations_live;
mod edit_history;
#[cfg(not(windows))]
mod game_audio;
mod hotkeys;
#[cfg(windows)]
#[path = "platform_windows/game_audio.rs"]
mod game_audio;
mod kofi;
#[cfg(not(windows))]
mod launch_hook;
#[cfg(windows)]
#[path = "platform_windows/launch_hook.rs"]
mod launch_hook;
mod material;
mod media;
mod music_service;
#[cfg(not(windows))]
mod music;
#[cfg(windows)]
#[path = "platform_windows/music.rs"]
mod music;
mod utilities;
mod external_import;
mod recording;
mod scripts;
mod remote;
#[cfg(not(windows))]
mod renderer_capture;
#[cfg(windows)]
#[path = "platform_windows/renderer_capture.rs"]
mod renderer_capture;
mod snapping;
mod sports;
mod streaming;
mod updates;
mod studio;
mod video;
mod virtual_camera;
mod widgets;
mod window_memory;
#[path = "dock_workspace.rs"]
mod workspace;

use anyhow::{Context, Result, bail};
use auroracast_core::Project;
use std::path::PathBuf;

fn option(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Rosorah Studio 0.32.0-preview.9 live streaming\n\n  rosorah-studio                       Open native studio\n  rosorah-studio --project FILE        Open a project\n  rosorah-studio --self-check          Validate GPU composition\n  rosorah-studio --snapshot --output DIR\n  rosorah-studio --render-demo --output DIR [--seconds 3] [--project FILE]\n\nRender-demo records synthetic video, not your screen. Requires ffmpeg with libx264.\nRecord and Go live are at the top right (FFmpeg required). Auto prefers a tested VAAPI/NVENC GPU encoder with software fallback. Existing output files are never overwritten. Sign in to Twitch, YouTube and Kick in Settings > Accounts."
        );
        return Ok(());
    }
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--self-check" | "--snapshot" | "--render-demo" => index += 1,
            "--project" | "--output" | "--seconds" | "--screenshot" | "--page" => {
                if args.get(index + 1).is_none_or(|v| v.starts_with("--")) {
                    bail!("{} requires a value", args[index]);
                }
                index += 2;
            }
            flag => bail!("Unknown argument {flag}; use --help"),
        }
    }
    let modes = ["--self-check", "--snapshot", "--render-demo"];
    if modes
        .iter()
        .filter(|m| args.iter().any(|a| a == **m))
        .count()
        > 1
    {
        bail!("Choose one engine command at a time");
    }
    if args.iter().any(|a| a == "--self-check") {
        return media::self_check();
    }
    let project_path = option(&args, "--project").map(PathBuf::from);
    if args
        .iter()
        .any(|a| a == "--render-demo" || a == "--snapshot")
    {
        let project = match &project_path {
            Some(path) => Project::load(path)?,
            None => Project::default(),
        };
        let output = PathBuf::from(option(&args, "--output").context("Specify --output DIR")?);
        if args.iter().any(|a| a == "--snapshot") {
            return media::snapshot(&project, &output);
        }
        let seconds: u32 = option(&args, "--seconds")
            .unwrap_or("3".into())
            .parse()
            .context("Seconds must be an integer")?;
        if !(1..=30).contains(&seconds) {
            bail!("Demo duration must be 1-30 seconds");
        }
        return media::render_demo(&project, &output, seconds);
    }
    let project_path = project_path.unwrap_or_else(|| profiles::path(&profiles::current()));
    let (project, notice) = profiles::open(&project_path)?;
    let screenshot = option(&args, "--screenshot").map(PathBuf::from);
    let page = option(&args, "--page").unwrap_or("studio".into());
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Rosorah Studio")
        .with_inner_size([1440., 900.])
        .with_min_inner_size([1100., 700.])
        .with_app_id("io.rosorah.Studio")
        .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/app-256.png")).unwrap_or_default());
    // Open the way it was left (screenshots always use the standard window).
    let viewport = if screenshot.is_none() { window_memory::apply(viewport, &window_memory::load()) } else { viewport };
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            desired_maximum_frame_latency: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };
    let _launch_hook = launch_hook::Guard::start();
    eframe::run_native(
        "Rosorah Studio",
        options,
        Box::new(move |cc| {
            let mut studio = studio::Studio::new(cc, project, project_path, screenshot, page);
            if let Some(notice) = notice {
                studio.notice(notice);
            }
            Ok(Box::new(studio))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Studio failed to open: {e}"))
}
