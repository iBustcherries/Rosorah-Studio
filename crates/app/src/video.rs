//! One decoder per media source, shared by preview and every output.
use anyhow::{Context, Result, bail};
use auroracast_core::{Id, VideoSettings};
use gst::prelude::*;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
#[derive(Clone, Default)]
pub struct AudioBus(Arc<Mutex<BTreeMap<Id, VecDeque<f32>>>>);
impl AudioBus {
    pub fn clear(&self, id: Id) {
        self.0.lock().unwrap().remove(&id);
    }
    pub fn push(&self, id: Id, bytes: &[u8]) {
        let mut all = self.0.lock().unwrap();
        let q = all.entry(id).or_default();
        q.extend(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .map(|v| if v.is_finite() { v } else { 0. }),
        );
        while q.len() > 9600 {
            q.pop_front();
        }
    }
    pub fn read(&self) -> BTreeMap<Id, Vec<f32>> {
        self.0
            .lock()
            .unwrap()
            .iter_mut()
            .map(|(id, q)| {
                let mut pcm = vec![0.; 960];
                for s in &mut pcm {
                    *s = q.pop_front().unwrap_or(0.);
                }
                (*id, pcm)
            })
            .collect()
    }
}
pub struct VideoBackend {
    pub id: Id,
    pub settings: VideoSettings,
    pub audio: AudioBus,
}
struct Playing(gst::Element);
impl Drop for Playing {
    fn drop(&mut self) {
        let _ = self.0.set_state(gst::State::Null);
    }
}
impl rosorah_capture::Backend for VideoBackend {
    fn run(
        self: Box<Self>,
        output: rosorah_capture::Producer,
        cancel: tokio::sync::watch::Receiver<bool>,
        _: rosorah_capture::Options,
    ) -> Result<()> {
        // Still images remain visible for the alert duration without decoding repeatedly.
        let ext=std::path::Path::new(&self.settings.path).extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();
        if ["png","jpg","jpeg"].contains(&ext.as_str()) {
            let image=image::open(&self.settings.path).context("Could not open alert image")?.thumbnail(4096,4096).into_rgba8();
            output.publish(rosorah_capture::Frame{width:image.width(),height:image.height(),rgba:image.into_raw()})?;
            let until=std::time::Instant::now()+std::time::Duration::from_secs(self.settings.duration_secs.max(1) as u64);
            while !*cancel.borrow()&&std::time::Instant::now()<until{std::thread::sleep(std::time::Duration::from_millis(20));}
            return Ok(());
        }
        gst::init()?;
        // VA-API's VP8/VP9 alpha decoders (seen on AMD/Mesa) return fully transparent frames,
        // which makes transparent stingers invisible. The software alpha decoder is correct.
        for name in ["vavp9alphadecodebin", "vavp8alphadecodebin"] {
            if let Some(feature) = gst::Registry::get().lookup_feature(name) {
                feature.set_rank(gst::Rank::NONE);
            }
        }
        // A playlist plays a folder's videos in turn; otherwise the one file.
        let playlist = if self.settings.folder.is_empty() { Vec::new() } else { auroracast_core::folder_files(&self.settings.folder, &auroracast_core::VIDEO_EXTENSIONS) };
        let mut round = 0u64;
        let mut position = 0usize;
        let mut order = auroracast_core::play_order(playlist.len(), round, self.settings.shuffle);
        let file_uri = |file: &str| -> Result<String> {
            let path = std::fs::canonicalize(file).with_context(|| format!("Video file not found: {file}"))?;
            anyhow::ensure!(path.is_file(), "Choose a video file");
            Ok(gst::glib::filename_to_uri(path, None)?.to_string())
        };
        let uri = match playlist.first() {
            Some(_) => file_uri(&playlist[order[0]])?,
            None => file_uri(&self.settings.path)?,
        };
        let sink = gst_app::AppSink::builder()
            .caps(
                &gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .field("width", gst::IntRange::<i32>::new(1, 4096))
                    .field("height", gst::IntRange::<i32>::new(1, 4096))
                    .build(),
            )
            .sync(true)
            .max_buffers(2)
            .drop(true)
            .build();
        let frames = output.clone();
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |s| {
                    let sample = s.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let frame =
                        crate::capture::decode(&sample).map_err(|_| gst::FlowError::Error)?;
                    frames.publish(frame).map_err(|_| gst::FlowError::Error)?;
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        let audio_sink = gst_app::AppSink::builder()
            .caps(
                &gst::Caps::builder("audio/x-raw")
                    .field("format", "F32LE")
                    .field("layout", "interleaved")
                    .field("rate", 48000i32)
                    .field("channels", 2i32)
                    .build(),
            )
            .sync(true)
            .max_buffers(4)
            .drop(true)
            .build();
        let bus = self.audio.clone();
        let id = self.id;
        let sound = self.settings.audio;
        let gain = self.settings.gain.clamp(0., 2.);
        audio_sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |s| {
                    let sample = s.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if sound
                        && let Some(b) = sample.buffer()
                        && let Ok(m) = b.map_readable()
                    {
                        let scaled: Vec<u8> = m
                            .as_slice()
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .flat_map(|b| (f32::from_le_bytes(*b) * gain).to_le_bytes())
                            .collect();
                        bus.push(id, &scaled);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        let player = gst::ElementFactory::make("playbin")
            .property("uri", uri.as_str())
            .property("video-sink", &sink)
            .property("audio-sink", &audio_sink)
            .build()
            .context("Video playback needs GStreamer playback plugins")?;
        let _guard = Playing(player.clone());
        output.connecting();
        player.set_state(gst::State::Playing)?;
        let messages = player.bus().context("Video player has no bus")?;
        let started = std::time::Instant::now();
        while !*cancel.borrow() {
            if (self.settings.automation_only || self.settings.trigger.is_some())
                && self.settings.duration_secs > 0
                && started.elapsed().as_secs() >= u64::from(self.settings.duration_secs)
            {
                break;
            }
            if let Some(message) = messages.timed_pop(gst::ClockTime::from_mseconds(20)) {
                match message.view() {
                    gst::MessageView::Error(e) => {
                        self.audio.clear(id);
                        bail!(
                            "Video: {}. Install the GStreamer codec for this file or try another format.",
                            e.error()
                        );
                    }
                    gst::MessageView::Eos(..) if !playlist.is_empty() => {
                        // Next video; after the last one, start over (reshuffled) or stop.
                        self.audio.clear(id);
                        position += 1;
                        if position >= playlist.len() {
                            if !self.settings.looped {
                                break;
                            }
                            position = 0;
                            round += 1;
                            order = auroracast_core::play_order(playlist.len(), round, self.settings.shuffle);
                        }
                        player.set_state(gst::State::Ready)?;
                        player.set_property("uri", file_uri(&playlist[order[position]])?);
                        player.set_state(gst::State::Playing)?;
                    }
                    gst::MessageView::Eos(..) => {
                        self.audio.clear(id);
                        if self.settings.looped
                            && self.settings.trigger.is_none()
                            && !self.settings.automation_only
                        {
                            player.seek_simple(
                                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                                gst::ClockTime::ZERO,
                            )?;
                        } else {
                            break;
                        }
                    }
                    _ => (),
                }
            }
        }
        self.audio.clear(id);
        Ok(())
    }
}
#[derive(Default)]
pub struct Playback {
    pub settings: BTreeMap<Id, VideoSettings>,
    pub queued: BTreeMap<Id, u8>,
    pub restart: std::collections::BTreeSet<Id>,
    pub bus: AudioBus,
}
impl Playback {
    pub fn trigger(&mut self, id: Id) {
        let q = self.queued.entry(id).or_default();
        *q = q.saturating_add(1).min(16);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn bounded_audio_and_clear() {
        let b = AudioBus::default();
        let bytes: Vec<_> = (0..12000).flat_map(|_| 0.25f32.to_le_bytes()).collect();
        b.push(5, &bytes);
        assert_eq!(b.0.lock().unwrap()[&5].len(), 9600);
        assert_eq!(b.read()[&5], vec![0.25; 960]);
        b.clear(5);
        assert!(b.read().is_empty());
    }
    #[test]
    fn video_playback_loop_and_cancel() -> Result<()> {
        use rosorah_capture::*;
        let path = std::env::temp_dir().join(format!("rosorah-video-{}.webm", std::process::id()));
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=160x90:r=30:d=0.3",
                "-c:v",
                "libvpx",
            ])
            .arg(&path)
            .status()?;
        anyhow::ensure!(status.success(), "Create video fixture");
        let mut engine = Engine::default();
        engine.start(
            40,
            Box::new(VideoBackend {
                id: 40,
                settings: VideoSettings {
                    path: path.display().to_string(),
                    ..Default::default()
                },
                audio: AudioBus::default(),
            }),
            Options::default(),
        )?;
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < end && engine.snapshot(40).is_none_or(|s| s.received < 15)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        let s = engine.snapshot(40).unwrap();
        anyhow::ensure!(s.received >= 15, "Loop did not play: {}", s.detail);
        assert_eq!(s.dimensions, Some([160, 90]));
        engine.stop(40);
        assert!(engine.reader().latest()[0].1.is_none());
        std::fs::remove_file(path)?;
        Ok(())
    }
}
#[cfg(test)]mod sound_tests{use super::*;
 /// Plays a real video file (ROSORAH_VIDEO) and reports frames, transparency and sound.
 #[test]#[ignore="Plays ROSORAH_VIDEO on request"]fn real_video_frames_alpha_and_sound()->Result<()>{
 let path=std::env::var("ROSORAH_VIDEO")?;let bus=AudioBus::default();let mut engine=rosorah_capture::Engine::default();
 engine.start(78,Box::new(VideoBackend{id:78,settings:VideoSettings{path,audio:true,automation_only:true,looped:false,..Default::default()},audio:bus.clone()}),rosorah_capture::Options::default())?;
 let until=std::time::Instant::now()+std::time::Duration::from_secs(4);let(mut frames,mut transparent,mut heard,mut size)=(0,false,false,(0,0));let mut last=0usize;
 while std::time::Instant::now()<until{if let Some((_,Some(f)))=engine.reader().latest().into_iter().next(){let key=f.rgba.len()^f.rgba.iter().step_by(4099).map(|b|*b as usize).sum::<usize>();if key!=last{frames+=1;last=key;}size=(f.width,f.height);transparent|=f.rgba.chunks_exact(4).any(|p|p[3]<250);}heard|=bus.read().values().flatten().any(|v|v.abs()>0.01);std::thread::sleep(std::time::Duration::from_millis(15));}
 println!("frames {frames} · size {size:?} · transparent pixels {transparent} · sound {heard} · status {:?}",engine.snapshot(78));engine.stop(78);Ok(())
 }
 #[test]fn audio_only_alert_produces_pcm_and_stops()->Result<()>{
 let path=std::env::temp_dir().join(format!("rosorah-sound-{}.wav",std::process::id()));
 let status=std::process::Command::new("ffmpeg").args(["-v","error","-y","-f","lavfi","-i","sine=frequency=440:duration=2","-ar","48000","-ac","2"]).arg(&path).status()?;anyhow::ensure!(status.success(),"Create sound fixture");
 let bus=AudioBus::default();let mut engine=rosorah_capture::Engine::default();engine.start(77,Box::new(VideoBackend{id:77,settings:VideoSettings{path:path.to_string_lossy().into(),audio:true,automation_only:true,looped:false,duration_secs:2,..Default::default()},audio:bus.clone()}),rosorah_capture::Options::default())?;
 let until=std::time::Instant::now()+std::time::Duration::from_secs(5);let mut heard=false;while std::time::Instant::now()<until{if bus.read().values().flatten().any(|v|v.abs()>0.01){heard=true;break;}std::thread::sleep(std::time::Duration::from_millis(10));}engine.stop(77);let _=std::fs::remove_file(path);anyhow::ensure!(heard,"Sound-only alert produced no audio: {:?}",engine.snapshot(77));Ok(())
 }
}
