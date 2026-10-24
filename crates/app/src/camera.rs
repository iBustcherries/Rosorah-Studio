//! V4L2 webcams and DroidCam's documented HTTP MJPEG stream.
use anyhow::{Context, Result, ensure};
use auroracast_core::CameraSettings;
use gst::prelude::*;
use rosorah_capture::{Backend, Frame, Options, Producer};
#[cfg(not(windows))]
use std::path::Path;
use std::{
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[derive(Clone)]
pub struct Device {
    pub name: String,
    pub path: String,
}
#[cfg(not(windows))]
pub fn devices() -> Vec<Device> {
    let mut devices = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") {
        for entry in entries.flatten() {
            let node = format!("/dev/{}", entry.file_name().to_string_lossy());
            // Exclude metadata-only V4L2 nodes when capability probing is permitted.
            if let Ok(file) = std::fs::File::open(&node) {
                use std::os::fd::AsRawFd;
                let mut caps = [0u32; 26];
                // Linux VIDIOC_QUERYCAP: fixed 104-byte v4l2_capability UAPI.
                if unsafe { libc::ioctl(file.as_raw_fd(), 0x80685600u64, caps.as_mut_ptr()) } == 0 {
                    let flags = if caps[21] & 0x80000000 != 0 {
                        caps[22]
                    } else {
                        caps[21]
                    };
                    let driver = String::from_utf8_lossy(unsafe {
                        std::slice::from_raw_parts(caps.as_ptr().cast::<u8>(), 16)
                    })
                    .trim_end_matches('\0')
                    .to_string();
                    // exclusive_caps loopback nodes advertise OUTPUT until a producer
                    // opens them. Keep these selectable instead of hiding DroidCam.
                    if !selectable_device(flags, &driver) {
                        continue;
                    }
                }
            }
            let name =
                std::fs::read_to_string(entry.path().join("name")).unwrap_or_else(|_| node.clone());
            // Prefer persistent by-id symlinks, keeping identically named devices distinct.
            let persistent = std::fs::read_dir("/dev/v4l/by-id")
                .ok()
                .and_then(|entries| {
                    entries.flatten().find_map(|e| {
                        (std::fs::canonicalize(e.path()).ok().as_deref() == Some(Path::new(&node)))
                            .then(|| e.path().to_string_lossy().into_owned())
                    })
                })
                .unwrap_or(node);
            devices.push(Device {
                name: name.trim().into(),
                path: persistent,
            });
        }
    }
    devices.sort_by(|a, b| a.path.cmp(&b.path));
    devices
}
#[cfg(any(not(windows), test))]
fn selectable_device(flags: u32, driver: &str) -> bool {
    flags & (0x1 | 0x1000) != 0 || (flags & (0x2 | 0x2000) != 0 && loopback_driver(driver))
}
#[cfg(any(not(windows), test))]
fn loopback_driver(driver: &str) -> bool {
    ["v4l2 loopback", "v4l2loopback", "v4l2loopback_dc"]
        .contains(&driver.to_ascii_lowercase().as_str())
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct DeviceMode {
    width: u32,
    height: u32,
    fourcc: u32,
    fps: Option<(u32, u32)>,
}
// Read-only Linux UAPI probes. Never change the producer's format or frame rate.
// This build targets x86_64, where v4l2_format has an 8-byte-aligned union.
#[cfg(not(windows))]
fn loopback_mode(device: &str) -> Result<Option<DeviceMode>> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(device)
        .with_context(|| format!("Cannot inspect camera {device}"))?;
    let fd = file.as_raw_fd();
    let mut caps = [0u32; 26];
    if unsafe { libc::ioctl(fd, 0x80685600u64, caps.as_mut_ptr()) } < 0 {
        return Err(std::io::Error::last_os_error()).context("Cannot read camera capabilities");
    }
    let bytes: Vec<u8> = caps[..4].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let driver = String::from_utf8_lossy(&bytes);
    if !loopback_driver(driver.trim_end_matches('\0')) {
        return Ok(None);
    }
    let flags = if caps[21] & 0x80000000 != 0 {
        caps[22]
    } else {
        caps[21]
    };
    ensure!(
        flags & 1 != 0,
        "Virtual camera is not sending video yet. Connect the phone in DroidCam and leave its desktop client running; Rosorah will retry automatically."
    );
    let mut format = [0u64; 26];
    format[0] = 1; // V4L2_BUF_TYPE_VIDEO_CAPTURE
    if unsafe { libc::ioctl(fd, 0xc0d05604u64, format.as_mut_ptr()) } < 0 {
        return Err(std::io::Error::last_os_error()).context("Cannot read virtual camera format");
    }
    let bytes: Vec<u8> = format.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let word = |offset| u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let (width, height, fourcc) = (word(8), word(12), word(16));
    ensure!(
        (16..=4096).contains(&width) && (16..=4096).contains(&height),
        "Virtual camera output {width} × {height} is outside the supported 16–4096 pixel range"
    );
    let mut parm = [0u32; 51];
    parm[0] = 1;
    let fps = if unsafe { libc::ioctl(fd, 0xc0cc5615u64, parm.as_mut_ptr()) } == 0
        && parm[3] > 0
        && parm[4] > 0
    {
        Some((parm[4], parm[3]))
    } else {
        None
    };
    Ok(Some(DeviceMode {
        width,
        height,
        fourcc,
        fps,
    }))
}
fn device_caps(settings: &CameraSettings, current: Option<&DeviceMode>) -> Result<gst::Caps> {
    let fields = if settings.auto_mode {
        if let Some(mode) = current {
            let mut fields = format!("width={},height={}", mode.width, mode.height);
            if let Some((num, den)) = mode.fps {
                fields.push_str(&format!(",framerate={num}/{den}"));
            }
            fields
        } else {
            "width=(int)[16,4096],height=(int)[16,4096]".into()
        }
    } else {
        format!(
            "width={},height={},framerate={}/1",
            settings.width, settings.height, settings.fps
        )
    };
    Ok(format!("video/x-raw,{fields};image/jpeg,{fields}").parse()?)
}
fn input_description(sample: &gst::Sample, settings: &CameraSettings) -> Result<String> {
    let caps = sample.caps().context("Camera frame has no format")?;
    let info = gst_video::VideoInfo::from_caps(caps)?;
    let rate = info.fps();
    let fps = if rate.numer() <= 0 || rate.denom() <= 0 {
        "FPS not reported".into()
    } else if rate.numer() % rate.denom() == 0 {
        format!("{} FPS", rate.numer() / rate.denom())
    } else {
        format!("{:.2} FPS", rate.numer() as f64 / rate.denom() as f64)
    };
    let mode = if !settings.network && settings.auto_mode {
        "Auto detected"
    } else {
        "Input"
    };
    Ok(format!(
        "Live · {mode}: {} × {} · {fps}{}",
        info.width(),
        info.height(),
        if settings.crop != [0; 4] {
            " · crop applied"
        } else {
            ""
        }
    ))
}
pub fn droidcam_url(settings: &CameraSettings) -> Result<String> {
    let host = settings.host.trim();
    ensure!(
        !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)),
        "Enter the phone's Wi-Fi IP or hostname, without http:// or a path"
    );
    ensure!(
        settings.port != 0,
        "DroidCam port must be between 1 and 65535"
    );
    Ok(format!(
        "http://{host}:{}/video{}",
        settings.port,
        if settings.legacy_url {
            String::new()
        } else {
            format!("/{}x{}", settings.width, settings.height)
        }
    ))
}
pub struct CameraBackend(pub CameraSettings);
struct Playing(gst::Pipeline);
impl Drop for Playing {
    fn drop(&mut self) {
        let _ = self.0.set_state(gst::State::Null);
    }
}
impl Backend for CameraBackend {
    fn run(
        self: Box<Self>,
        output: Producer,
        cancel: watch::Receiver<bool>,
        _: Options,
    ) -> Result<()> {
        self.0.validate()?;
        ensure!(
            self.0.network || !self.0.device.is_empty(),
            "Select a camera first"
        );
        gst::init()?;
        while !*cancel.borrow() {
            let observed = if !self.0.network && self.0.auto_mode {
                loopback_mode(&self.0.device)?
            } else {
                None
            };
            output.waiting("Connecting camera…");
            let (pipeline, sink) = pipeline(&self.0, None)?;
            let _guard = Playing(pipeline.clone());
            pipeline.set_state(gst::State::Playing).context(
                "Camera could not start; check device permissions or whether it is in use",
            )?;
            let bus = pipeline.bus().context("Camera has no message bus")?;
            let mut last = Instant::now();
            let mut next_frame = Instant::now();
            let mut next_probe = Instant::now() + Duration::from_secs(1);
            let mut last_caps = None;
            let mut description = String::new();
            while !*cancel.borrow() {
                if let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_mseconds(100)) {
                    last = Instant::now();
                    if !self.0.network || Instant::now() >= next_frame {
                        if sample.caps() != last_caps.as_deref() {
                            description = input_description(&sample, &self.0)?;
                            last_caps = sample.caps().map(ToOwned::to_owned);
                        }
                        output.publish_with_detail(
                            transform(crate::capture::decode(&sample)?, &self.0)?,
                            Some(&description),
                        )?;
                        next_frame = (next_frame + Duration::from_secs_f64(1. / self.0.fps as f64))
                            .max(Instant::now());
                    }
                }
                while let Some(message) = bus.pop() {
                    match message.view() {
                        gst::MessageView::Error(e) => {
                            let origin = e
                                .src()
                                .and_then(|s| s.downcast_ref::<gst::Element>())
                                .and_then(|e| e.factory())
                                .map(|f| f.name().to_string())
                                .unwrap_or_else(|| "pipeline".into());
                            let input = if self.0.network {
                                "DroidCam Wi-Fi"
                            } else {
                                "Webcam / USB"
                            };
                            let hint = if self.0.network {
                                "Check the phone IP/port and try the legacy /video option. For a USB-connected Linux client, select Webcam / USB instead."
                            } else if self.0.auto_mode {
                                "Automatic mode could not read the camera. Ensure the desktop client is sending video and the correct device is selected."
                            } else {
                                "Ensure the desktop client is sending to its virtual camera, then match its resolution/FPS and refresh cameras."
                            };
                            anyhow::bail!(
                                "{input}: {} ({origin}). {hint} Details: {}",
                                e.error(),
                                e.debug().as_deref().unwrap_or("No additional details")
                            );
                        }
                        gst::MessageView::Eos(_) => {
                            anyhow::bail!("Camera disconnected; reconnecting automatically")
                        }
                        _ => (),
                    }
                }
                if observed.is_some() && Instant::now() >= next_probe {
                    next_probe = Instant::now() + Duration::from_secs(1);
                    if loopback_mode(&self.0.device)? != observed {
                        output.waiting("Camera output changed; detecting its new mode…");
                        break; // Drop the old pipeline before negotiating a new one.
                    }
                }
                ensure!(
                    last.elapsed() < Duration::from_secs(10),
                    "No camera frames for 10 seconds; check the device or phone connection"
                );
            }
        }
        Ok(())
    }
}
fn pipeline(
    settings: &CameraSettings,
    test: Option<gst::Element>,
) -> Result<(gst::Pipeline, gst_app::AppSink)> {
    let current = if test.is_none() && !settings.network && settings.auto_mode {
        loopback_mode(&settings.device)?
    } else {
        None
    };
    let pipeline = gst::Pipeline::default();
    let source = if let Some(source) = test {
        source
    } else if settings.network {
        gst::ElementFactory::make("souphttpsrc")
            .property("location", droidcam_url(settings)?)
            .property("is-live", true)
            .property("timeout", 5u32)
            .build()
            .context("DroidCam Wi-Fi needs gstreamer1-plugins-good (souphttpsrc)")?
    } else {
        ensure!(!settings.device.is_empty(), "Select a camera first");
        camera_source(&settings.device)?
    };
    let decode = gst::ElementFactory::make("decodebin").build()?;
    let convert = gst::ElementFactory::make("videoconvert").build()?;
    let scale = gst::ElementFactory::make("videoscale").build()?;
    let sink = gst_app::AppSink::builder()
        .caps(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .field("width", gst::IntRange::<i32>::new(1, 4096))
                .field("height", gst::IntRange::<i32>::new(1, 4096))
                .build(),
        )
        .max_buffers(1)
        .drop(true)
        .sync(false)
        .build();
    pipeline.add_many([&source, &decode, &convert, &scale, sink.upcast_ref()])?;
    if settings.network {
        source.link(&decode)?;
        // HTTP endpoints may return multipart MJPEG, a raw JPEG sequence, or
        // another supported video container. Let decodebin inspect the stream.
        // Multipart is infinite: publish its sole stream without waiting for EOS.
        decode
            .clone()
            .downcast::<gst::Bin>()
            .map_err(|_| anyhow::anyhow!("Invalid video decoder"))?
            .connect_deep_element_added(|_, _, element| {
                if element
                    .factory()
                    .is_some_and(|f| f.name() == "multipartdemux")
                {
                    element.set_property("single-stream", true);
                }
            });
    } else {
        let caps = device_caps(settings, current.as_ref())?;
        let filter = gst::ElementFactory::make("capsfilter")
            .property("caps", caps)
            .build()?;
        pipeline.add(&filter)?;
        gst::Element::link_many([&source, &filter, &decode])?;
    }
    gst::Element::link_many([&convert, &scale, sink.upcast_ref()])?;
    let convert_weak = convert.downgrade();
    decode.connect_pad_added(move |_, pad| {
        if let Some(convert) = convert_weak.upgrade() {
            let sink = convert.static_pad("sink").unwrap();
            let video = pad
                .current_caps()
                .and_then(|c| c.structure(0).map(|s| s.name().starts_with("video/")))
                .unwrap_or(false);
            if video && !sink.is_linked() {
                let _ = pad.link(&sink);
            }
        }
    });
    Ok((pipeline, sink))
}
pub fn transform(frame: Frame, settings: &CameraSettings) -> Result<Frame> {
    let [left, top, right, bottom] = settings.crop;
    ensure!(
        left.saturating_add(right) < frame.width && top.saturating_add(bottom) < frame.height,
        "Crop removes the entire camera image; reduce crop margins"
    );
    if settings.crop == [0; 4] && !settings.flip {
        return Ok(frame);
    }
    let width = frame.width - left - right;
    let height = frame.height - top - bottom;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in top..top + height {
        let start = ((y * frame.width + left) * 4) as usize;
        let row = &frame.rgba[start..start + (width * 4) as usize];
        if settings.flip {
            for pixel in row.as_chunks::<4>().0.iter().rev() {
                rgba.extend_from_slice(pixel);
            }
        } else {
            rgba.extend_from_slice(row);
        }
    }
    Ok(Frame { shared: None,
        width,
        height,
        rgba,
    })
}

#[cfg(not(windows))]
fn camera_source(device: &str) -> Result<gst::Element> { gst::ElementFactory::make("v4l2src").property("device", device).build().context("Camera capture needs GStreamer v4l2src") }
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_flip_and_invalid_margins() {
        let settings = CameraSettings {
            crop: [1, 0, 0, 1],
            flip: true,
            ..Default::default()
        };
        let f = Frame { shared: None,
            width: 3,
            height: 2,
            rgba: (0u8..6).flat_map(|v| [v, 0, 0, 255]).collect(),
        };
        let out = transform(f.clone(), &settings).unwrap();
        assert_eq!((out.width, out.height), (2, 1));
        assert_eq!(out.rgba, [2, 0, 0, 255, 1, 0, 0, 255]);
        assert!(
            transform(
                f,
                &CameraSettings {
                    crop: [3, 0, 0, 0],
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn phone_url_validation() {
        let mut s = CameraSettings {
            host: "192.168.1.5".into(),
            ..Default::default()
        };
        assert_eq!(
            droidcam_url(&s).unwrap(),
            "http://192.168.1.5:4747/video/1280x720"
        );
        s.legacy_url = true;
        assert!(droidcam_url(&s).unwrap().ends_with("/video"));
        for host in ["", "localhost/path", "user@host", "host\n", "http://host"] {
            s.host = host.into();
            if host != "host\n" {
                assert!(droidcam_url(&s).is_err());
            }
        }
    }
    #[test]
    fn real_camera_pipeline_negotiates_frames() -> Result<()> {
        gst::init()?;
        let settings = CameraSettings {
            width: 320,
            height: 240,
            fps: 30,
            auto_mode: false,
            ..Default::default()
        };
        let source = gst::ElementFactory::make("videotestsrc")
            .property("num-buffers", 3i32)
            .build()?;
        let (p, sink) = pipeline(&settings, Some(source))?;
        let _guard = Playing(p.clone());
        p.set_state(gst::State::Playing)?;
        let sample = sink
            .try_pull_sample(gst::ClockTime::from_seconds(5))
            .context("No camera test frame")?;
        let frame = crate::capture::decode(&sample)?;
        assert_eq!((frame.width, frame.height), (320, 240));
        Ok(())
    }
    #[test]
    fn real_http_mjpeg_uses_shared_reader_and_stops_cleanly() -> Result<()> {
        http_camera_roundtrip(true)
    }
    #[test]
    fn real_http_raw_jpeg_uses_format_detection() -> Result<()> {
        http_camera_roundtrip(false)
    }
    #[test]
    fn loopback_output_devices_remain_selectable() {
        assert!(selectable_device(2, "v4l2 loopback"));
        assert!(selectable_device(2, "v4l2loopback"));
        assert!(selectable_device(1, "uvcvideo"));
        assert!(!selectable_device(0x800000, "uvcvideo"));
        assert!(!selectable_device(2, "unrelated output"));
    }
    #[test]
    fn automatic_yuyv_input_follows_caps_changes_and_reports_rate() -> Result<()> {
        gst::init()?;
        // Match the user's raw DroidCam format, while saved manual settings
        // intentionally disagree. The camera advertises YUYV as GStreamer YUY2.
        let settings = CameraSettings {
            width: 640,
            height: 480,
            fps: 15,
            ..Default::default()
        };
        let src = gst_app::AppSrc::builder()
            .is_live(true)
            .format(gst::Format::Time)
            .build();
        let (p, sink) = pipeline(&settings, Some(src.clone().upcast()))?;
        let _guard = Playing(p.clone());
        let caps = |w, h, n, d| {
            gst::Caps::builder("video/x-raw")
                .field("format", "YUY2")
                .field("width", w)
                .field("height", h)
                .field("framerate", gst::Fraction::new(n, d))
                .build()
        };
        src.set_caps(Some(&caps(1280i32, 720i32, 30, 1)));
        p.set_state(gst::State::Playing)?;
        for (w, h, n, d, rate) in [
            (1280, 720, 30, 1, "30 FPS"),
            (1920, 1080, 60, 1, "60 FPS"),
            (640, 480, 30000, 1001, "29.97 FPS"),
        ] {
            src.set_caps(Some(&caps(w, h, n, d)));
            let pixels: Vec<u8> = (0..w * h / 2).flat_map(|_| [100, 128, 100, 128]).collect();
            src.push_buffer(gst::Buffer::from_mut_slice(pixels))?;
            let sample = sink
                .try_pull_sample(gst::ClockTime::from_seconds(5))
                .context("Auto camera did not produce frame")?;
            let frame = crate::capture::decode(&sample)?;
            assert_eq!((frame.width, frame.height), (w as u32, h as u32));
            assert!(frame.rgba[0] > 60 && frame.rgba[0] < 150);
            let description = input_description(&sample, &settings)?;
            assert!(description.contains(&format!("{w} × {h}")), "{description}");
            assert!(description.contains(rate), "{description}");
            assert!(description.contains("Auto detected"));
        }
        Ok(())
    }
    #[test]
    fn device_mode_caps_preserve_fractional_fps_and_manual_override() -> Result<()> {
        gst::init()?;
        let mode = DeviceMode {
            width: 1920,
            height: 1080,
            fourcc: 0,
            fps: Some((60000, 1001)),
        };
        let mut settings = CameraSettings::default();
        let caps = device_caps(&settings, Some(&mode))?;
        let s = caps.structure(0).unwrap();
        assert_eq!(s.get::<i32>("width")?, 1920);
        assert_eq!(
            s.get::<gst::Fraction>("framerate")?,
            gst::Fraction::new(60000, 1001)
        );
        settings.auto_mode = false;
        let caps = device_caps(&settings, Some(&mode))?;
        assert_eq!(caps.structure(0).unwrap().get::<i32>("width")?, 1280);
        assert_eq!(
            caps.structure(0)
                .unwrap()
                .get::<gst::Fraction>("framerate")?,
            gst::Fraction::new(60, 1)
        );
        Ok(())
    }
    fn http_camera_roundtrip(multipart: bool) -> Result<()> {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
        };
        gst::init()?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let mut jpeg = Vec::new();
        let rgb: Vec<u8> = (0..32 * 24)
            .flat_map(|i| {
                if i % 32 < 16 {
                    [240, 20, 20]
                } else {
                    [20, 220, 30]
                }
            })
            .collect();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode(
            &rgb,
            32,
            24,
            image::ExtendedColorType::Rgb8,
        )?;
        let server = std::thread::spawn(move || -> Result<()> {
            let deadline = Instant::now() + Duration::from_secs(12);
            while !done.load(Ordering::Relaxed) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                        let mut request = [0; 2048];
                        let n = stream.read(&mut request)?;
                        ensure!(
                            String::from_utf8_lossy(&request[..n])
                                .starts_with("GET /video/1280x720 "),
                            "Unexpected phone URL"
                        );
                        if multipart {
                            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nConnection: close\r\n\r\n")?;
                        } else {
                            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nConnection: close\r\n\r\n")?;
                        }
                        while !done.load(Ordering::Relaxed) && Instant::now() < deadline {
                            if (if multipart {write!(stream,"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",jpeg.len())} else {Ok(())})
                            .and_then(|_| stream.write_all(&jpeg))
                            .and_then(|_| stream.write_all(b"\r\n"))
                            .is_err()
                            {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(30));
                        }
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(())
        });
        let mut engine = rosorah_capture::Engine::default();
        let reader = engine.reader();
        engine.start(
            40,
            Box::new(CameraBackend(CameraSettings {
                network: true,
                host: "127.0.0.1".into(),
                port,
                crop: [2, 2, 2, 2],
                flip: true,
                ..Default::default()
            })),
            Options::default(),
        )?;
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut frame = None;
        while Instant::now() < deadline {
            frame = reader
                .latest()
                .into_iter()
                .find_map(|(id, f)| (id == 40).then_some(f).flatten());
            if frame.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let snapshot = engine.snapshot(40);
        engine.stop(40);
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap()?;
        let frame = frame.with_context(|| format!("No HTTP camera frame: {snapshot:?}"))?;
        assert_eq!((frame.width, frame.height), (28, 20));
        assert!(
            frame.rgba[1] > frame.rgba[0],
            "Flipped image should start with green half"
        );
        assert!(
            reader.latest().iter().all(|(_, f)| f.is_none()),
            "Disconnect must clear the recorder's frame"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while engine.running(40) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!engine.running(40));
        Ok(())
    }
}
#[cfg(windows)]
include!("platform_windows/camera.rs");
