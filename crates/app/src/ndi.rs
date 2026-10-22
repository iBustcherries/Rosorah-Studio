//! NDI: video and sound over the local network, through GStreamer's NDI plugin. The plugin needs
//! the free NDI runtime, which the user installs (NDI Tools); without it NDI explains itself.
use anyhow::{Context, Result, bail};
use auroracast_core::Id;
use gst::prelude::*;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

pub const GET_NDI: &str = "https://ndi.video/tools/";
/// What to tell someone whose computer can't do NDI yet.
pub fn missing() -> &'static str {
    if cfg!(windows) {
        "NDI needs the free NDI Runtime. Install NDI Tools from ndi.video, then restart Rosorah."
    } else {
        "NDI needs the free NDI runtime (libndi). Install the NDI SDK for Linux from ndi.video, then restart Rosorah."
    }
}

/// Makes the NDI elements available, loading the plugin by hand when GStreamer skipped it (it
/// remembers a plugin that once failed to load, e.g. before the NDI runtime was installed).
pub fn ready() -> Result<()> {
    static TRIED: Mutex<bool> = Mutex::new(false);
    gst::init()?;
    if gst::ElementFactory::find("ndisrc").is_some() {
        return Ok(());
    }
    let mut tried = TRIED.lock().unwrap();
    if !*tried {
        *tried = true;
        #[cfg(windows)]
        runtime_from_registry();
        for path in plugin_files() {
            if path.is_file() && gst::Plugin::load_file(&path).is_ok() {
                break;
            }
        }
    }
    if gst::ElementFactory::find("ndisrc").is_none() {
        if !plugin_files().iter().any(|p| p.is_file()) && !cfg!(windows) {
            bail!("This Rosorah build doesn't include GStreamer's NDI plugin (packaging/ndi/build-ndi.sh builds it).");
        }
        bail!(missing());
    }
    Ok(())
}
/// Where the NDI plugin may be: beside GStreamer's own plugins, or shipped with Rosorah in
/// /opt/rosorah-studio/gstreamer (Linux).
fn plugin_files() -> Vec<std::path::PathBuf> {
    let name = if cfg!(windows) { "gstndi.dll" } else { "libgstndi.so" };
    let mut dirs = Vec::new();
    if let Some(dir) = gst::Registry::get().find_plugin("coreelements").and_then(|p| p.filename()).and_then(|f| f.parent().map(|d| d.to_path_buf())) {
        dirs.push(dir);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join("gstreamer"));
    }
    dirs.into_iter().map(|d| d.join(name)).collect()
}
/// The NDI installer sets NDI_RUNTIME_DIR_V6 for new programs only; a Rosorah already running
/// when NDI Tools was installed reads it from the registry instead.
#[cfg(windows)]
fn runtime_from_registry() {
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(key: isize, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut std::ffi::c_void, bytes: *mut u32) -> i32;
    }
    use std::os::windows::ffi::OsStringExt;
    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x2;
    let key: Vec<u16> = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment".encode_utf16().chain(Some(0)).collect();
    for var in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
        if std::env::var_os(var).is_some() {
            return;
        }
        let name: Vec<u16> = var.encode_utf16().chain(Some(0)).collect();
        let mut data = [0u16; 1024];
        let mut bytes = (data.len() * 2) as u32;
        let status = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), data.as_mut_ptr().cast(), &mut bytes) };
        let len = data.iter().position(|&c| c == 0).unwrap_or(data.len());
        if status == 0 && len > 0 {
            // Set before any other thread reads the environment for NDI.
            unsafe { std::env::set_var(var, std::ffi::OsString::from_wide(&data[..len])) };
            return;
        }
    }
}

/// NDI senders seen on the network. Looking starts on the first call and keeps going, so the
/// list fills in over a few seconds.
pub fn senders() -> Result<Vec<String>> {
    static MONITOR: Mutex<Option<gst::DeviceMonitor>> = Mutex::new(None);
    ready()?;
    let mut monitor = MONITOR.lock().unwrap();
    if monitor.is_none() {
        let m = gst::DeviceMonitor::new();
        m.add_filter(Some("Source/Network"), None);
        m.start().context("Couldn't look for NDI senders")?;
        *monitor = Some(m);
    }
    let mut names: Vec<String> = monitor.as_ref().unwrap().devices().iter().filter(|d| d.has_classes("Source/Network")).map(|d| d.display_name().to_string()).collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// One NDI sender's picture as a source; its sound goes to the mixer like a media file's.
pub struct NdiBackend {
    pub id: Id,
    pub sender: String,
    pub audio: crate::video::AudioBus,
}
struct Playing(gst::Pipeline);
impl Drop for Playing {
    fn drop(&mut self) {
        let _ = self.0.set_state(gst::State::Null);
    }
}
impl rosorah_capture::Backend for NdiBackend {
    fn run(self: Box<Self>, output: rosorah_capture::Producer, cancel: tokio::sync::watch::Receiver<bool>, _: rosorah_capture::Options) -> Result<()> {
        anyhow::ensure!(!self.sender.is_empty(), "Choose an NDI sender in Layer Properties");
        ready()?;
        output.waiting(&format!("Connecting to “{}”…", self.sender));
        let pipeline = gst::Pipeline::default();
        let source = gst::ElementFactory::make("ndisrc").property("ndi-name", &self.sender).property("receiver-ndi-name", "Rosorah Studio").build()?;
        let demux = gst::ElementFactory::make("ndisrcdemux").build()?;
        pipeline.add_many([&source, &demux])?;
        source.link(&demux)?;
        let _guard = Playing(pipeline.clone());
        let video_sink = gst_app::AppSink::builder()
            .caps(&gst::Caps::builder("video/x-raw").field("format", "RGBA").build())
            .max_buffers(1)
            .drop(true)
            .sync(false)
            .build();
        let frames = output.clone();
        let detail = format!("NDI · {}", self.sender);
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw = seen.clone();
        video_sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |s| {
                    let sample = s.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let frame = crate::capture::decode(&sample).map_err(|_| gst::FlowError::Error)?;
                    frames.publish_with_detail(frame, Some(&detail)).map_err(|_| gst::FlowError::Error)?;
                    saw.store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        let audio_sink = gst_app::AppSink::builder()
            .caps(&gst::Caps::builder("audio/x-raw").field("format", "F32LE").field("rate", 48_000).field("channels", 2).field("layout", "interleaved").build())
            .sync(false)
            .build();
        let (bus, id) = (self.audio.clone(), self.id);
        audio_sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |s| {
                    let sample = s.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(map) = sample.buffer().and_then(|b| b.map_readable().ok()) {
                        bus.push(id, &map);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        // The demuxer adds a video and an audio pad once the sender's streams arrive.
        let video_chain = ["queue", "videoconvert"].map(|f| gst::ElementFactory::make(f).build());
        let audio_chain = ["queue", "audioconvert", "audioresample"].map(|f| gst::ElementFactory::make(f).build());
        let video_chain: Vec<gst::Element> = video_chain.into_iter().collect::<Result<_, _>>()?;
        let audio_chain: Vec<gst::Element> = audio_chain.into_iter().collect::<Result<_, _>>()?;
        for (chain, sink) in [(&video_chain, video_sink.upcast_ref::<gst::Element>()), (&audio_chain, audio_sink.upcast_ref())] {
            pipeline.add_many(chain.iter())?;
            pipeline.add(sink)?;
            gst::Element::link_many(chain.iter().chain([sink]))?;
        }
        let (video_in, audio_in) = (video_chain[0].static_pad("sink").unwrap(), audio_chain[0].static_pad("sink").unwrap());
        demux.connect_pad_added(move |_, pad| {
            let target = if pad.name().starts_with("video") { &video_in } else { &audio_in };
            if !target.is_linked() {
                let _ = pad.link(target);
            }
        });
        pipeline.set_state(gst::State::Playing).context("NDI couldn't start")?;
        let messages = pipeline.bus().context("NDI has no message bus")?;
        let started = Instant::now();
        let mut told = false;
        while !*cancel.borrow() {
            if let Some(message) = messages.timed_pop(gst::ClockTime::from_mseconds(100)) {
                match message.view() {
                    gst::MessageView::Error(e) => bail!("NDI “{}”: {} ({})", self.sender, e.error(), e.debug().as_deref().unwrap_or("no details")),
                    gst::MessageView::Eos(_) => bail!("“{}” stopped sending; reconnecting", self.sender),
                    _ => (),
                }
            }
            if !told && started.elapsed() > Duration::from_secs(1) && !seen.load(std::sync::atomic::Ordering::Relaxed) {
                told = true;
                output.waiting(&format!("Waiting for “{}” on the network", self.sender));
            }
        }
        Ok(())
    }
}

/// Sends Rosorah's picture and sound to the network as an NDI sender.
pub struct Sender {
    pipeline: gst::Pipeline,
    video: gst_app::AppSrc,
    audio: gst_app::AppSrc,
    audio_frames: u64,
}
impl Sender {
    pub fn new(name: &str, width: u32, height: u32, fps: u32) -> Result<Self> {
        ready()?;
        anyhow::ensure!(!name.is_empty(), "Give the NDI output a name");
        let pipeline = gst::Pipeline::default();
        let video = gst_app::AppSrc::builder()
            .caps(&gst::Caps::builder("video/x-raw").field("format", "RGBA").field("width", width as i32).field("height", height as i32).field("framerate", gst::Fraction::new(fps as i32, 1)).build())
            .format(gst::Format::Time)
            .is_live(true)
            .build();
        let audio = gst_app::AppSrc::builder()
            .caps(&gst::Caps::builder("audio/x-raw").field("format", "F32LE").field("rate", 48_000).field("channels", 2).field("layout", "interleaved").build())
            .format(gst::Format::Time)
            .is_live(true)
            .build();
        let video_convert = gst::ElementFactory::make("videoconvert").build()?;
        let audio_convert = gst::ElementFactory::make("audioconvert").build()?;
        let combiner = gst::ElementFactory::make("ndisinkcombiner").build()?;
        let sink = gst::ElementFactory::make("ndisink").property("ndi-name", name).build()?;
        pipeline.add_many([video.upcast_ref(), &video_convert, audio.upcast_ref(), &audio_convert, &combiner, &sink])?;
        gst::Element::link_many([video.upcast_ref(), &video_convert])?;
        video_convert.link_pads(None, &combiner, Some("video"))?;
        audio.link(&audio_convert)?;
        let audio_pad = combiner.request_pad_simple("audio").context("NDI output has no sound input")?;
        audio_convert.static_pad("src").unwrap().link(&audio_pad)?;
        combiner.link(&sink)?;
        pipeline.set_state(gst::State::Playing).context("NDI output couldn't start")?;
        Ok(Self { pipeline, video, audio, audio_frames: 0 })
    }
    /// One RGBA picture, `at` after the output started.
    pub fn picture(&self, rgba: &[u8], at: Duration) -> Result<()> {
        let mut buffer = gst::Buffer::from_slice(rgba.to_vec());
        buffer.get_mut().unwrap().set_pts(gst::ClockTime::from_nseconds(at.as_nanos() as u64));
        self.video.push_buffer(buffer).map_err(|e| anyhow::anyhow!("NDI output stopped: {e:?}"))?;
        self.check()
    }
    /// Stereo 48 kHz samples, following on from the last ones.
    pub fn sound(&mut self, pcm: &[f32]) -> Result<()> {
        let bytes: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut buffer = gst::Buffer::from_slice(bytes);
        let frames = pcm.len() as u64 / 2;
        {
            let b = buffer.get_mut().unwrap();
            b.set_pts(gst::ClockTime::from_nseconds(self.audio_frames * 1_000_000_000 / 48_000));
            b.set_duration(gst::ClockTime::from_nseconds(frames * 1_000_000_000 / 48_000));
        }
        self.audio_frames += frames;
        self.audio.push_buffer(buffer).map_err(|e| anyhow::anyhow!("NDI output stopped: {e:?}"))?;
        Ok(())
    }
    fn check(&self) -> Result<()> {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(message) = bus.pop() {
                if let gst::MessageView::Error(e) = message.view() {
                    bail!("NDI output: {} ({})", e.error(), e.debug().as_deref().unwrap_or("no details"));
                }
            }
        }
        Ok(())
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    /// Needs the NDI runtime; ROSORAH_NDI_PLUGIN points at a libgstndi.so when GStreamer has none.
    #[test]
    #[ignore]
    fn real_ndi_output_is_found_and_received_with_picture_and_sound() -> Result<()> {
        gst::init()?;
        if let Some(plugin) = std::env::var_os("ROSORAH_NDI_PLUGIN") {
            gst::Plugin::load_file(plugin)?;
        }
        let mut sender = Sender::new("Rosorah NDI test", 320, 240, 30)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let sending = std::thread::spawn(move || -> Result<()> {
            let red: Vec<u8> = [255, 0, 0, 255].repeat(320 * 240);
            let start = Instant::now();
            while !stopping.load(Ordering::Relaxed) {
                sender.picture(&red, start.elapsed())?;
                sender.sound(&[0.5; 3200])?;
                std::thread::sleep(Duration::from_millis(33));
            }
            Ok(())
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        let name = loop {
            if let Some(name) = senders()?.into_iter().find(|n| n.ends_with("(Rosorah NDI test)")) {
                break name;
            }
            anyhow::ensure!(Instant::now() < deadline, "Rosorah's NDI output never appeared in the sender list");
            std::thread::sleep(Duration::from_millis(200));
        };
        let audio = crate::video::AudioBus::default();
        let mut engine = rosorah_capture::Engine::default();
        engine.start(1, Box::new(NdiBackend { id: 1, sender: name, audio: audio.clone() }), rosorah_capture::Options::default())?;
        let (mut picture, mut sound) = (false, false);
        while !(picture && sound) && Instant::now() < deadline + Duration::from_secs(10) {
            if let Some((_, Some(frame))) = engine.reader().latest().into_iter().find(|(id, _)| *id == 1) {
                assert_eq!((frame.width, frame.height), (320, 240));
                // NDI may carry it as YUV, so red comes back within a few steps of pure red.
                let [r, g, b, a] = frame.rgba[..4] else { unreachable!() };
                assert!(r > 240 && g < 16 && b < 16 && a == 255, "{:?}", &frame.rgba[..4]);
                picture = true;
            }
            sound |= audio.read().get(&1).is_some_and(|pcm| pcm.iter().any(|s| (s - 0.5).abs() < 0.01));
            std::thread::sleep(Duration::from_millis(50));
        }
        engine.stop(1);
        stop.store(true, Ordering::Relaxed);
        sending.join().unwrap()?;
        anyhow::ensure!(picture, "No picture arrived over NDI");
        anyhow::ensure!(sound, "No sound arrived over NDI");
        Ok(())
    }
}
