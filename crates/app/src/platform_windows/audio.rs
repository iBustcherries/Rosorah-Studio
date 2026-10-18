// WASAPI endpoints and application loopback; never fall back to desktop audio.
fn graph() -> Result<Value> {
    let mut rows = vec![
        serde_json::json!({"serial":"wasapi:microphone","target":"","label":"System default microphone","kind":"Audio/Source","id":1}),
        serde_json::json!({"serial":"wasapi:desktop","target":DEFAULT_MONITOR,"label":"Desktop audio — system default output","kind":"Audio/Sink","id":2}),
    ];
    for (i, d) in rosorah_windows::audio::endpoints()?.into_iter().enumerate() {
        rows.push(serde_json::json!({"serial":format!("wasapi:{}:{}",if d.capture{"input"}else{"output"},d.id),"target":format!("wasapi:{}:{}",if d.capture{"input"}else{"output"},d.id),"label":d.name,"kind":if d.capture{"Audio/Source"}else{"Audio/Sink"},"id":i as u64+100}));
    }
    let mut seen = std::collections::BTreeSet::new();
    for w in rosorah_windows::window::windows()? {
        if seen.insert(w.process.pid) {
            // The program's file name (without .exe) identifies the app across restarts.
            let app = std::path::Path::new(&w.process.name).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            rows.push(serde_json::json!({"serial":format!("wasapi:process:{}:{}",w.process.pid,w.process.start_time),"target":w.process.pid.to_string(),"label":format!("Application · {}",w.title),"kind":"Stream/Output/Audio","pid":w.process.pid,"id":w.process.start_time,"app":app,"title":w.title}));
        }
    }
    Ok(Value::Array(rows))
}
fn devices(g: &Value) -> Vec<Device> {
    g.as_array()
        .into_iter()
        .flatten()
        .map(|v| Device {
            serial: v["serial"].as_str().unwrap_or_default().into(),
            target: v["target"].as_str().unwrap_or_default().into(),
            label: v["label"].as_str().unwrap_or_default().into(),
            kind: v["kind"].as_str().unwrap_or_default().into(),
            id: v["id"].as_u64().unwrap_or(0),
            pid: v["pid"].as_u64().map(|p| p as u32),
            channels: vec!["FL".into(), "FR".into()],
            app: v["app"].as_str().unwrap_or_default().into(),
            title: v["title"].as_str().unwrap_or_default().into(),
            role: String::new(),
        })
        .collect()
}
struct Input {
    buffer: Arc<Mutex<Buffer>>,
    stop: Arc<AtomicBool>,
    reader: Option<thread::JoinHandle<()>>,
}
fn append_audio(buffer: &Arc<Mutex<Buffer>>, samples: Vec<f32>, discontinuity: bool) {
    let mut b = buffer.lock().unwrap();
    if discontinuity {
        b.samples.clear();
    }
    b.samples.extend(samples);
    let excess = b.samples.len().saturating_sub(SAMPLES * 10);
    b.samples.drain(..excess);
}
impl Input {
    fn start(d: &Device, _: Id, noise: auroracast_core::NoiseSuppression) -> Result<Self> {
        let kind = if d.serial == "wasapi:microphone" {
            rosorah_windows::audio::Input::Microphone
        } else if d.serial == "wasapi:desktop" {
            rosorah_windows::audio::Input::Desktop
        } else if d.serial.starts_with("wasapi:input:") || d.serial.starts_with("wasapi:output:") {
            rosorah_windows::audio::Input::Endpoint {
                id: d.serial.splitn(3,':').nth(2).unwrap_or_default().into(),
                loopback: d.kind == "Audio/Sink",
            }
        } else if let Some(pid) = d.pid {
            ensure!(
                rosorah_windows::window::process(pid).is_some_and(|p| p.start_time == d.id),
                "Application closed or restarted; refresh audio sources"
            );
            rosorah_windows::audio::Input::Process(pid)
        } else {
            bail!("Select a Windows audio device")
        };
        let buffer = Arc::new(Mutex::new(Buffer {
            samples: VecDeque::new(),
            ended: false,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        // GStreamer processes microphone suppression in bounded queues, without a child process.
        let filter = if noise != auroracast_core::NoiseSuppression::Off {
            use gst::prelude::*;
            gst::init()?;
            let caps = gst::Caps::builder("audio/x-raw")
                .field("format", "F32LE")
                .field("layout", "interleaved")
                .field("rate", 48000i32)
                .field("channels", 2i32)
                .build();
            let src = gst_app::AppSrc::builder()
                .caps(&caps)
                .is_live(true)
                .format(gst::Format::Time)
                .do_timestamp(true)
                .block(false)
                .max_bytes((SAMPLES * 4 * 6) as u64)
                .build();
            let convert = gst::ElementFactory::make("audioconvert").build()?;
            let dsp=gst::ElementFactory::make("webrtcdsp").property("echo-cancel",false).property("gain-control",false).property("noise-suppression",true).property_from_str("noise-suppression-level",if noise==auroracast_core::NoiseSuppression::Strong{"very-high"}else{"moderate"}).build().context("Microphone suppression needs GStreamer's WebRTC DSP plugin; reinstall media dependencies or select Off")?;
            let out = gst::ElementFactory::make("audioconvert").build()?;
            let sink = gst_app::AppSink::builder()
                .caps(&caps)
                .sync(false)
                .max_buffers(6)
                .drop(true)
                .build();
            let pipeline = gst::Pipeline::default();
            pipeline.add_many([src.upcast_ref(), &convert, &dsp, &out, sink.upcast_ref()])?;
            gst::Element::link_many([src.upcast_ref(), &convert, &dsp, &out, sink.upcast_ref()])?;
            pipeline.set_state(gst::State::Playing)?;
            Some((pipeline, src, sink))
        } else {
            None
        };
        let (b, s) = (buffer.clone(), stop.clone());
        let reader = thread::spawn(move || {
            use gst::prelude::*;
            let filtered_reader = filter.as_ref().map(|(pipeline, _, sink)| {
                let pipeline = pipeline.clone();
                let sink = sink.clone();
                let b = b.clone();
                let s = s.clone();
                thread::spawn(move || {
                    while !s.load(Ordering::Relaxed) {
                        if pipeline.bus().is_some_and(|bus| {
                            bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos])
                                .is_some()
                        }) {
                            crate::capture_log::event("Windows microphone suppression stopped; check the GStreamer WebRTC DSP plugin and device format");
                            s.store(true, Ordering::Relaxed);
                            break;
                        }
                        if let Some(sample) =
                            sink.try_pull_sample(gst::ClockTime::from_mseconds(20))
                        {
                            if let Some(data) = sample.buffer().and_then(|b| b.map_readable().ok())
                            {
                                append_audio(
                                    &b,
                                    data.as_slice()
                                        .chunks_exact(4)
                                        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                                        .map(|v| if v.is_finite() { v } else { 0. })
                                        .collect(),
                                    false,
                                );
                            }
                        }
                    }
                })
            });
            let result = rosorah_windows::audio::run(kind, &s, |packet| {
                if let Some((_, src, _)) = &filter {
                    // Never enqueue a large driver packet as an unbounded latency burst.
                    let skip=packet.samples.len().saturating_sub(SAMPLES*6);
                    for chunk in packet.samples[skip..].chunks(SAMPLES) {
                        if src.current_level_bytes() >= (SAMPLES*4*6)as u64 { break; }
                        let bytes:Vec<u8>=chunk.iter().flat_map(|v|v.to_le_bytes()).collect();
                        let mut buffer=gst::Buffer::from_mut_slice(bytes);
                        buffer.get_mut().unwrap().set_duration(gst::ClockTime::from_nseconds(chunk.len()as u64*1_000_000_000/96_000));
                        src.push_buffer(buffer)?;
                    }
                } else {
                    append_audio(&b, packet.samples, packet.discontinuity);
                }
                Ok(())
            });
            s.store(true, Ordering::Relaxed);
            if let Some((pipeline, _, _)) = filter {
                let _ = pipeline.set_state(gst::State::Null);
            }
            if let Some(t) = filtered_reader {
                let _ = t.join();
            }
            if let Err(e) = result {
                crate::capture_log::event(format!("Windows audio: {e}"));
            }
            b.lock().unwrap().ended = true;
        });
        Ok(Self {
            buffer,
            stop,
            reader: Some(reader),
        })
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.reader.take() {
            let _ = t.join();
        }
    }
}
struct Monitor {
    target: String,
    tx: std::sync::mpsc::SyncSender<Vec<f32>>,
    stop: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Monitor {
    fn ended(&mut self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }
    fn start(target: &str) -> Result<Self> {
        use gst::prelude::*;
        ensure!(
            target == DEFAULT_MONITOR
                || target == "wasapi:desktop"
                || target.starts_with("wasapi:output:"),
            "Select a Windows output device"
        );
        gst::init()?;
        let src = gst_app::AppSrc::builder()
            .caps(
                &gst::Caps::builder("audio/x-raw")
                    .field("format", "F32LE")
                    .field("layout", "interleaved")
                    .field("rate", 48000i32)
                    .field("channels", 2i32)
                    .build(),
            )
            .is_live(true)
            .format(gst::Format::Time)
            .do_timestamp(true)
            .block(false)
            .max_bytes((SAMPLES * 4 * 6) as u64)
            .build();
        let convert = gst::ElementFactory::make("audioconvert").build()?;
        let resample = gst::ElementFactory::make("audioresample").build()?;
        let sink = gst::ElementFactory::make("wasapisink")
            .build()
            .context("Windows audio monitoring needs the GStreamer WASAPI plugin")?;
        if let Some(device) = target.strip_prefix("wasapi:output:") {
            sink.set_property("device", device);
        }
        let pipeline = gst::Pipeline::default();
        pipeline.add_many([src.upcast_ref(), &convert, &resample, &sink])?;
        gst::Element::link_many([src.upcast_ref(), &convert, &resample, &sink])?;
        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(e.into());
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(3);
        let stop = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let (s, e) = (stop.clone(), ended.clone());
        let worker = thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                if pipeline.bus().is_some_and(|b| {
                    b.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos])
                        .is_some()
                }) {
                    break;
                }
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(pcm) => {
                        if src.current_level_bytes() >= (SAMPLES * 4 * 6) as u64 {
                            continue;
                        }
                        let bytes: Vec<u8> = pcm.iter().flat_map(|x| x.to_le_bytes()).collect();
                        if src.push_buffer(gst::Buffer::from_mut_slice(bytes)).is_err() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
                    Err(_) => break,
                }
            }
            let _ = pipeline.set_state(gst::State::Null);
            e.store(true, Ordering::Relaxed);
        });
        Ok(Self {
            target: target.into(),
            tx,
            stop,
            ended,
            worker: Some(worker),
        })
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
