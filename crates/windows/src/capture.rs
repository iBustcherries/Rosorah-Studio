//! DXGI desktop duplication. The capture engine owns the worker and one-frame mailbox.
//! Window capture and game hooks are separate backends, never silently full-screen fallbacks.
use crate::pixels::{self, Rotation};
use anyhow::{Context, Result, bail, ensure};
use rosorah_capture::{Backend, Options, Producer, Target};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use windows::{
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    },
    core::Interface,
};

#[link(name="kernel32")]
unsafe extern "system" {
    fn QueryPerformanceCounter(value: *mut i64) -> i32;
    fn QueryPerformanceFrequency(value: *mut i64) -> i32;
}
fn desktop_age_ms(present: i64) -> Option<f64> {
    if present <= 0 { return None; }
    let (mut now, mut frequency) = (0, 0);
    unsafe {
        if QueryPerformanceCounter(&mut now) == 0 || QueryPerformanceFrequency(&mut frequency) == 0 || frequency <= 0 { return None; }
    }
    Some(now.saturating_sub(present).max(0) as f64 * 1000. / frequency as f64)
}

#[derive(Clone, Debug)]
pub struct Monitor {
    pub id: String,
    pub width: u32,
    pub height: u32,
}
fn output_name(desc: &DXGI_OUTPUT_DESC) -> String {
    String::from_utf16_lossy(
        &desc.DeviceName[..desc
            .DeviceName
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(desc.DeviceName.len())],
    )
}
fn outputs() -> Result<Vec<(IDXGIAdapter1, IDXGIOutput, DXGI_OUTPUT_DESC)>> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut result = Vec::new();
        for a in 0.. {
            let adapter = match factory.EnumAdapters1(a) {
                Ok(a) => a,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(e.into()),
            };
            for o in 0.. {
                let output = match adapter.EnumOutputs(o) {
                    Ok(o) => o,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return Err(e.into()),
                };
                let desc = output.GetDesc()?;
                if desc.AttachedToDesktop.as_bool() {
                    result.push((adapter.clone(), output, desc));
                }
            }
        }
        Ok(result)
    }
}
pub fn monitors() -> Result<Vec<Monitor>> {
    Ok(outputs()?
        .into_iter()
        .map(|(_, _, d)| Monitor {
            id: output_name(&d),
            width: (d.DesktopCoordinates.right - d.DesktopCoordinates.left).max(0) as u32,
            height: (d.DesktopCoordinates.bottom - d.DesktopCoordinates.top).max(0) as u32,
        })
        .collect())
}
struct Duplication {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    staging: Option<ID3D11Texture2D>,
    size: (u32, u32),
    held: bool,
    pending_since: Option<Instant>,
    has_image: bool,
    desktop_present: i64,
    readback_ms: f64,
    conversion_ms: f64,
    rotation: Rotation,
}
impl Duplication {
    fn open(id: &str) -> Result<Self> {
        unsafe {
            let (adapter, output, desc) = outputs()?
                .into_iter()
                .find(|(_, _, d)| output_name(d) == id)
                .context("Selected monitor is disconnected")?;
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("Cannot initialize Windows capture device")?;
            let device = device.context("Windows returned no capture device")?;
            let context = context.context("Windows returned no capture context")?;
            let duplication = output
                .cast::<IDXGIOutput1>()?
                .DuplicateOutput(&device)
                .context("Cannot duplicate this display; unlock the desktop and try again")?;
            let rotation = match desc.Rotation {
                DXGI_MODE_ROTATION_ROTATE90 => Rotation::Clockwise90,
                DXGI_MODE_ROTATION_ROTATE180 => Rotation::Clockwise180,
                DXGI_MODE_ROTATION_ROTATE270 => Rotation::Clockwise270,
                _ => Rotation::Identity,
            };
            Ok(Self {
                device,
                context,
                duplication,
                staging: None,
                size: (0, 0),
                held: false,
                pending_since: None,
                has_image: false,
                desktop_present: 0,
                readback_ms: 0.0,
                conversion_ms: 0.0,
                rotation,
            })
        }
    }
    fn next(
        &mut self,
        resolution: u32,
        cancel: &watch::Receiver<bool>,
    ) -> Result<Option<rosorah_capture::Frame>> {
        unsafe {
            // Keep ownership during conversion and frame pacing. Releasing just
            // before acquisition avoids redundant desktop copies by DXGI.
            if self.pending_since.is_none() && self.held {
                self.held = false;
                self.duplication.ReleaseFrame()?;
            }
            if self.pending_since.is_none() {
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut resource = None;
                match self
                    .duplication
                    .AcquireNextFrame(16, &mut info, &mut resource)
                {
                    Ok(()) => {}
                    Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                    Err(e) => return Err(e.into()),
                }
                self.held = true;
                // Pointer-only updates do not change the bitmap. Cursor composition
                // is disabled on this backend, so avoid another full desktop copy.
                if self.has_image && info.LastPresentTime == 0 {
                    return Ok(None);
                }
                self.desktop_present = info.LastPresentTime;
                let texture: ID3D11Texture2D = resource
                    .context("Windows returned no desktop texture")?
                    .cast()?;
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                ensure!(
                    desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM,
                    "Unsupported desktop pixel format"
                );
                ensure!(
                    desc.Width > 0
                        && desc.Height > 0
                        && desc.Width <= 16384
                        && desc.Height <= 16384,
                    "Desktop dimensions out of bounds"
                );
                if self.staging.is_none() || self.size != (desc.Width, desc.Height) {
                    desc.Usage = D3D11_USAGE_STAGING;
                    desc.BindFlags = 0;
                    desc.MiscFlags = 0;
                    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                    let mut staging = None;
                    self.device
                        .CreateTexture2D(&desc, None, Some(&mut staging))?;
                    self.staging =
                        Some(staging.context("Cannot allocate capture staging texture")?);
                    self.size = (desc.Width, desc.Height);
                }
                let staging = self.staging.as_ref().unwrap();
                self.context.CopyResource(staging, &texture);
                self.pending_since = Some(Instant::now());
                self.context.Flush();
            }
            let staging = self
                .staging
                .as_ref()
                .context("Missing pending staging texture")?;
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            // Never let a busy GPU trap this worker in a blocking Map. One
            // readback is in flight; a missed poll deadline retains that copy
            // without submitting another. Commands remain ordered on this context.
            let deadline = Instant::now() + Duration::from_millis(16);
            loop {
                if *cancel.borrow() || cancel.has_changed().is_err() {
                    return Ok(None);
                }
                match self.context.Map(
                    staging,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                ) {
                    Ok(()) => break,
                    Err(e) if e.code() == DXGI_ERROR_WAS_STILL_DRAWING => {
                        if Instant::now() >= deadline {
                            return Ok(None);
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            struct Unmap<'a>(&'a ID3D11DeviceContext, &'a ID3D11Texture2D);
            impl Drop for Unmap<'_> {
                fn drop(&mut self) {
                    unsafe {
                        self.0.Unmap(self.1, 0);
                    }
                }
            }
            let _unmap = Unmap(&self.context, staging);
            let age = self
                .pending_since
                .take()
                .map(|t| t.elapsed())
                .unwrap_or_default();
            self.readback_ms = age.as_secs_f64() * 1000.0;
            ensure!(
                !mapped.pData.is_null(),
                "Windows returned no mapped capture data"
            );
            let size = (mapped.RowPitch as usize)
                .checked_mul(self.size.1 as usize)
                .context("Mapped texture overflow")?;
            ensure!(size <= 1024 * 1024 * 1024, "Mapped texture too large");
            let bytes = std::slice::from_raw_parts(mapped.pData.cast::<u8>(), size);
            let conversion_start = Instant::now();
            let frame = pixels::convert(
                bytes,
                self.size.0,
                self.size.1,
                mapped.RowPitch as usize,
                self.rotation,
                resolution,
            )?;
            self.conversion_ms = conversion_start.elapsed().as_secs_f64() * 1000.0;
            self.has_image = true;
            Ok(Some(frame))
        }
    }
}
impl Drop for Duplication {
    fn drop(&mut self) {
        if self.held {
            unsafe {
                let _ = self.duplication.ReleaseFrame();
            }
        }
    }
}
/// Explicit monitor selection. Construct only after the user chooses a monitor.
pub struct ScreenBackend {
    pub monitor_id: String,
}
impl Backend for ScreenBackend {
    fn run(
        self: Box<Self>,
        output: Producer,
        cancel: watch::Receiver<bool>,
        options: Options,
    ) -> Result<()> {
        ensure!(
            options.target == Target::Screen,
            "This Windows backend captures a selected monitor only"
        );
        ensure!(
            !options.cursor,
            "Separate Windows pointer rendering is not implemented yet; disable Capture cursor for this diagnostic"
        );
        ensure!(
            matches!(options.fps, 30 | 60),
            "Capture rate must be 30 or 60 FPS"
        );
        output.connecting();
        let mut capture = Duplication::open(&self.monitor_id)?;
        // AcquireNextFrame waits for the desktop to change, so frames are taken as they're
        // drawn; at most twice the capture rate keeps the copy work bounded on fast monitors.
        let gap = Duration::from_secs_f64(0.5 / options.fps as f64);
        let mut last = Instant::now() - gap;
        while !*cancel.borrow() && cancel.has_changed().is_ok() {
            if let Some(wait) = gap.checked_sub(last.elapsed()) {
                std::thread::sleep(wait);
            }
            match capture.next(options.resolution, &cancel) {
                Ok(Some(frame)) => {
                    last = Instant::now();
                    output.publish_with_detail(
                    frame,
                    Some(&format!(
                        "Live · Windows DXGI · readback {:.1} ms · conversion {:.1} ms · desktop-to-publish {}",
                        capture.readback_ms, capture.conversion_ms,
                        desktop_age_ms(capture.desktop_present).map(|v| format!("{v:.1} ms")).unwrap_or_else(|| "unavailable".into())
                    )),
                    )?
                }
                Ok(None) => {}
                Err(e)
                    if e.downcast_ref::<windows::core::Error>().is_some_and(|e| {
                        matches!(
                            e.code(),
                            DXGI_ERROR_ACCESS_LOST
                                | DXGI_ERROR_DEVICE_REMOVED
                                | DXGI_ERROR_DEVICE_RESET
                        )
                    }) =>
                {
                    output.waiting("Display changed; reconnecting Windows capture…");
                    drop(capture);
                    let deadline = Instant::now() + Duration::from_secs(15);
                    capture = loop {
                        if *cancel.borrow() || cancel.has_changed().is_err() {
                            return Ok(());
                        }
                        match Duplication::open(&self.monitor_id) {
                            Ok(c) => break c,
                            Err(e) if Instant::now() >= deadline => return Err(e),
                            Err(_) => std::thread::sleep(Duration::from_millis(100)),
                        }
                    };
                }
                Err(e) => bail!("Windows capture failed: {e:#}"),
            }
        }
        Ok(())
    }
}

pub(crate) fn monitor_handle(id:&str)->Result<windows::Win32::Graphics::Gdi::HMONITOR>{outputs()?.into_iter().find(|(_,_,d)|output_name(d)==id).map(|(_,_,d)|d.Monitor).context("Selected monitor disconnected")}
