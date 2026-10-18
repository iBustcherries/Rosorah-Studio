//! Encoder selection uses real, cancellable encodes with the live recording
//! command, format and dimensions. An advertised encoder alone is not usable
//! hardware: the driver, device access and codec profile must work as well.
#[cfg(unix)]
use std::os::{fd::AsRawFd, unix::process::CommandExt};
use anyhow::{Context, Result, bail, ensure};
use auroracast_core::{Canvas, RecordingEncoder};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Acceleration {
    Software,
    Vaapi { device: PathBuf, low_power: bool },
    Nvenc,
    #[cfg(windows)]
    Amf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Codecs {
    video: &'static str,
    audio: &'static str,
    acceleration: Acceleration,
    stream_kbps: Option<u32>,
}
impl Codecs {
    pub(super) fn software(video: &'static str, audio: &'static str) -> Self {
        Self {
            video,
            audio,
            acceleration: Acceleration::Software,
            stream_kbps: None,
        }
    }
    fn hardware(&self) -> bool {
        self.acceleration != Acceleration::Software
    }
    pub(super) fn label(&self) -> String {
        let video = match self.video {
            "mpeg4" => "MPEG-4",
            "libvpx" => "VP8",
            _ => "H.264",
        };
        let backend = match &self.acceleration {
            Acceleration::Software => "Software".into(),
            Acceleration::Nvenc => "Hardware · NVIDIA NVENC".into(),
            #[cfg(windows)]
            Acceleration::Amf => "Hardware · AMD AMF".into(),
            Acceleration::Vaapi { device, low_power } => {
                let node = device.file_name().unwrap_or_default().to_string_lossy();
                let vendor = std::fs::read_to_string(
                    Path::new("/sys/class/drm")
                        .join(node.as_ref())
                        .join("device/vendor"),
                )
                .unwrap_or_default();
                let vendor = match vendor.trim() {
                    "0x1002" => "AMD ",
                    "0x8086" => "Intel ",
                    _ => "",
                };
                format!(
                    "Hardware · {vendor}VAAPI ({node}{})",
                    if *low_power { ", low power" } else { "" }
                )
            }
        };
        format!(
            "{backend} · {video} + {}",
            if self.audio == "aac" { "AAC" } else { "FLAC" }
        )
    }
    fn device_args(&self, cmd: &mut Command) {
        if let Acceleration::Vaapi { device, .. } = &self.acceleration {
            cmd.arg("-vaapi_device").arg(device);
        }
    }
    fn video_args(&self, cmd: &mut Command, fps: u32) {
        cmd.args(["-c:v", self.video, "-threads:v", "2"]);
        if let Some(kbps) = self.stream_kbps {
            let bitrate = format!("{kbps}k");
            cmd.args([
                "-b:v",
                &bitrate,
                "-minrate",
                &bitrate,
                "-maxrate",
                &bitrate,
                "-bufsize",
                &format!("{}k", kbps * 2),
                "-g",
                &(fps * 2).to_string(),
                "-bf",
                "0",
                "-profile:v",
                "high",
            ]);
            match &self.acceleration {
                Acceleration::Vaapi { low_power, .. } => {
                    cmd.args([
                        "-vf",
                        "format=nv12,hwupload",
                        "-pix_fmt",
                        "vaapi",
                        "-rc_mode",
                        "CBR",
                    ]);
                    if *low_power {
                        cmd.args(["-low_power", "1"]);
                    }
                }
                Acceleration::Nvenc => {
                    cmd.args([
                        "-pix_fmt",
                        "yuv420p",
                        "-preset",
                        "p4",
                        "-tune",
                        "ll",
                        "-rc",
                        "cbr",
                        "-rc-lookahead",
                        "0",
                        "-zerolatency",
                        "1",
                    ]);
                }
                #[cfg(windows)]
                Acceleration::Amf => { cmd.args(["-pix_fmt","nv12","-usage","lowlatency","-quality","balanced","-rc","cbr"]); }
                Acceleration::Software => {
                    cmd.args([
                        "-pix_fmt",
                        "yuv420p",
                        "-preset",
                        "veryfast",
                        "-tune",
                        "zerolatency",
                        "-sc_threshold",
                        "0",
                        "-x264-params",
                        "nal-hrd=cbr:force-cfr=1",
                    ]);
                }
            }
            cmd.args([
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
            ]);
            return;
        }
        match &self.acceleration {
            Acceleration::Vaapi { low_power, .. } => {
                cmd.args([
                    "-vf",
                    "format=nv12,hwupload",
                    "-pix_fmt",
                    "vaapi",
                    "-rc_mode",
                    "CQP",
                    "-qp",
                    "23",
                    "-bf",
                    "0",
                    "-g",
                ])
                .arg((fps * 2).to_string());
                if *low_power {
                    cmd.args(["-low_power", "1"]);
                }
            }
            Acceleration::Nvenc => {
                cmd.args([
                    "-pix_fmt",
                    "yuv420p",
                    "-preset",
                    "p1",
                    "-tune",
                    "ll",
                    "-rc",
                    "constqp",
                    "-qp",
                    "23",
                    "-bf",
                    "0",
                    "-rc-lookahead",
                    "0",
                    "-zerolatency",
                    "1",
                    "-g",
                ])
                .arg((fps * 2).to_string());
            }
            #[cfg(windows)]
            Acceleration::Amf => {
                cmd.args(["-pix_fmt","nv12","-usage","lowlatency","-quality","balanced","-rc","cqp","-qp_i","23","-qp_p","23","-bf","0","-g"]).arg((fps*2).to_string());
            }
            Acceleration::Software => {
                cmd.args(["-pix_fmt", "yuv420p"]);
                match self.video {
                    "libx264" => {
                        cmd.args(["-preset", "ultrafast", "-tune", "zerolatency", "-crf", "23"]);
                    }
                    "libvpx" => {
                        cmd.args([
                            "-deadline",
                            "realtime",
                            "-cpu-used",
                            "8",
                            "-lag-in-frames",
                            "0",
                            "-b:v",
                            "6M",
                        ]);
                    }
                    _ => {
                        cmd.args(["-q:v", "4", "-bf", "0"]);
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct Selection {
    pub codecs: Codecs,
    pub details: String,
}
fn has_encoder(text: &str, name: &str) -> bool {
    text.lines()
        .any(|line| line.split_whitespace().nth(1) == Some(name))
}
fn render_nodes(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name();
            name.to_str()?
                .strip_prefix("renderD")?
                .parse::<u32>()
                .ok()?;
            Some(entry.path())
        })
        .collect();
    paths.sort_by_key(|p| {
        p.file_name().unwrap().to_string_lossy()[7..]
            .parse::<u32>()
            .unwrap()
    });
    paths.truncate(8);
    paths
}
fn choose(
    text: &str,
    devices: &[PathBuf],
    mode: RecordingEncoder,
    mut check: impl FnMut(&Codecs) -> Result<()>,
    cancelled: impl Fn() -> bool,
    mut report: impl FnMut(&str),
) -> Result<Selection> {
    let audio = ["aac", "flac"]
        .into_iter()
        .find(|n| has_encoder(text, n))
        .context("FFmpeg needs an AAC or FLAC audio encoder")?;
    let mut candidates = Vec::new();
    let mut details = Vec::new();
    if mode == RecordingEncoder::Auto {
        if has_encoder(text, "h264_vaapi") {
            if devices.is_empty() {
                details.push("VAAPI: no DRM render devices found in /dev/dri.".into());
            }
            for device in devices {
                // Some Intel drivers expose only the low-power encode entrypoint.
                for low_power in [false, true] {
                    candidates.push(Codecs {
                        stream_kbps: None,
                        video: "h264_vaapi",
                        audio,
                        acceleration: Acceleration::Vaapi {
                            device: device.clone(),
                            low_power,
                        },
                    });
                }
            }
        } else {
            details.push("FFmpeg does not include h264_vaapi.".into());
        }
        #[cfg(windows)]
        if has_encoder(text, "h264_amf") {
            candidates.push(Codecs { stream_kbps:None, video:"h264_amf", audio, acceleration:Acceleration::Amf });
        }
        if has_encoder(text, "h264_nvenc") {
            candidates.push(Codecs {
                stream_kbps: None,
                video: "h264_nvenc",
                audio,
                acceleration: Acceleration::Nvenc,
            });
        } else {
            details.push("FFmpeg does not include h264_nvenc.".into());
        }
    } else {
        details.push("Software encoding selected in Settings.".into());
    }
    for name in ["libx264", "mpeg4", "libvpx"] {
        if has_encoder(text, name) {
            candidates.push(Codecs::software(name, audio));
        }
    }
    for codecs in candidates {
        ensure!(!cancelled(), "Recording cancelled before it started");
        report(&format!("Checking {}…", codecs.label()));
        let result = check(&codecs);
        ensure!(!cancelled(), "Recording cancelled before it started");
        match result {
            Ok(()) => {
                let label = if mode == RecordingEncoder::Auto && !codecs.hardware() {
                    "Hardware encoding unavailable; using software."
                } else {
                    "Encoder check passed."
                };
                details.insert(0, format!("{}\n{label}", codecs.label()));
                return Ok(Selection {
                    codecs,
                    details: details.join("\n"),
                });
            }
            Err(e) => details.push(format!(
                "{}: {}",
                codecs.label(),
                format!("{e:#}").chars().take(1800).collect::<String>()
            )),
        }
    }
    bail!(
        "No working recording encoder found.\n{}",
        details.join("\n")
    )
}
pub(super) fn select(
    canvas: &Canvas,
    mode: RecordingEncoder,
    cancelled: impl Fn() -> bool,
    report: impl FnMut(&str),
) -> Result<Selection> {
    let output = crate::audio::command("ffmpeg", &["-hide_banner", "-encoders"])
        .context("Recording needs FFmpeg. On Fedora, install ffmpeg-free, then try again")?;
    let hardware_deadline = Instant::now() + Duration::from_secs(10);
    choose(
        &String::from_utf8_lossy(&output),
        &render_nodes(Path::new("/dev/dri")),
        mode,
        |codec| {
            let mut deadline = Instant::now() + Duration::from_secs(3);
            if codec.hardware() {
                deadline = deadline.min(hardware_deadline);
            }
            probe(canvas, codec, deadline, &cancelled)
        },
        &cancelled,
        report,
    )
}

pub(super) fn select_stream(
    canvas: &Canvas,
    mode: RecordingEncoder,
    kbps: u32,
    cancelled: impl Fn() -> bool,
    report: impl FnMut(&str),
) -> Result<Selection> {
    let output = crate::audio::command("ffmpeg", &["-hide_banner", "-encoders"])
        .context("Streaming needs FFmpeg with H.264 and AAC encoders")?;
    let text = String::from_utf8_lossy(&output);
    ensure!(
        has_encoder(&text, "aac"),
        "Streaming requires an AAC encoder in FFmpeg"
    );
    let compatible = text
        .lines()
        .filter(|line| {
            matches!(
                line.split_whitespace().nth(1),
                Some("aac" | "h264_vaapi" | "h264_nvenc" | "h264_amf" | "libx264")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let hardware_deadline = Instant::now() + Duration::from_secs(10);
    let mut selection = choose(
        &compatible,
        &render_nodes(Path::new("/dev/dri")),
        mode,
        |codec| {
            let mut codec = codec.clone();
            codec.stream_kbps = Some(kbps);
            let mut deadline = Instant::now() + Duration::from_secs(4);
            if codec.hardware() {
                deadline = deadline.min(hardware_deadline);
            }
            probe(canvas, &codec, deadline, &cancelled)
        },
        &cancelled,
        report,
    )?;
    selection.codecs.stream_kbps = Some(kbps);
    Ok(selection)
}
pub(super) fn launch_stream(
    canvas: &Canvas,
    codec: &Codecs,
) -> Result<(Encoder, InputWriter, InputWriter)> {
    ensure!(
        codec.stream_kbps.is_some(),
        "Streaming encoder was not checked"
    );
    launch_target(canvas, codec, Target::Stream)
}

#[cfg(unix)]
pub(super) type InputWriter = std::io::PipeWriter;
#[cfg(windows)]
pub(super) type InputWriter = rosorah_windows::pipe::Writer;
pub(super) struct Encoder(pub Child, #[cfg(windows)] std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for Encoder {
    fn drop(&mut self) {
        #[cfg(windows)] self.1.store(true,std::sync::atomic::Ordering::Relaxed);
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
enum Target<'a> {
    File(&'a Path),
    Probe,
    Stream,
    /// Plain frames to a virtual camera device (no encoding; audio is read and dropped).
    Camera(&'a str),
}
/// Sends the canvas to a virtual camera: on Linux a v4l2loopback device, on Windows the pipe the
/// Rosorah camera component reads.
pub(super) fn launch_camera(canvas: &Canvas, device: &str) -> Result<(Encoder, InputWriter, InputWriter)> {
    launch_target(canvas, &Codecs::software("rawvideo", "pcm_f32le"), Target::Camera(device))
}
#[cfg(test)]
pub(super) fn launch(
    canvas: &Canvas,
    codec: &Codecs,
    path: &Path,
) -> Result<(Encoder, InputWriter, InputWriter)> {
    ensure!(
        !path.exists(),
        "Recording already exists: {}",
        path.display()
    );
    launch_target(canvas, codec, Target::File(path))
}
pub(super) fn launch_tracks(
    canvas: &Canvas,
    codec: &Codecs,
    path: &Path,
    tracks: usize,
) -> Result<(Encoder, InputWriter, InputWriter)> {
    ensure!(!path.exists(), "Recording already exists");
    launch_target_tracks(canvas, codec, Target::File(path), tracks)
}
fn launch_target(
    canvas: &Canvas,
    codec: &Codecs,
    target: Target<'_>,
) -> Result<(Encoder, InputWriter, InputWriter)> {
    launch_target_tracks(canvas, codec, target, 1)
}
fn launch_target_tracks(
    canvas: &Canvas,
    codec: &Codecs,
    target: Target<'_>,
    tracks: usize,
) -> Result<(Encoder, InputWriter, InputWriter)> {
    ensure!((1..=6).contains(&tracks), "Invalid recording track count");
    #[cfg(unix)] let (video_read, video_write) = std::io::pipe()?;
    #[cfg(unix)] let (audio_read, audio_write) = std::io::pipe()?;
    #[cfg(unix)] let audio_fd = audio_read.as_raw_fd();
    #[cfg(unix)] let (video_input,audio_input)=(String::from("pipe:0"),format!("pipe:{audio_fd}"));
    #[cfg(windows)] let cancel=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(windows)] let mut video_write=InputWriter::new(cancel.clone())?;
    #[cfg(windows)] let mut audio_write=InputWriter::new(cancel.clone())?;
    #[cfg(windows)] let (video_input,audio_input)=(video_write.name().to_owned(),audio_write.name().to_owned());
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-filter_threads",
        "1",
    ]);
    codec.device_args(&mut cmd);
    cmd.args([
        "-thread_queue_size",
        "4",
        "-probesize",
        "32",
        "-analyzeduration",
        "1",
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgba",
        "-video_size",
    ])
    .arg(format!("{}x{}", canvas.width, canvas.height))
    .args([
        "-framerate",
        &canvas.fps.to_string(),
        "-i",
        &video_input,
        "-thread_queue_size",
        "32",
        "-probesize",
        "32",
        "-analyzeduration",
        "1",
        "-f",
        "f32le",
        "-ar",
        "48000",
        "-ac",
        &(tracks * 2).to_string(),
        "-i",
    ])
    .arg(&audio_input);
    let camera = matches!(target, Target::Camera(_));
    if camera {
        // Video to the camera as plain YUV; the audio input is still read (so it never blocks) and dropped.
        let Target::Camera(device) = &target else { unreachable!() };
        #[cfg(not(windows))]
        cmd.args(["-map", "0:v:0", "-vf", "format=yuv420p", "-f", "v4l2"]).arg(device);
        #[cfg(windows)]
        cmd.args(["-map", "0:v:0", "-vf", "format=nv12", "-f", "rawvideo", "-y"]).arg(device);
        cmd.args(["-map", "1:a:0", "-f", "null", "-"]).stderr(Stdio::piped()).stdout(Stdio::null());
        #[cfg(unix)]
        cmd.stdin(Stdio::from(video_read.try_clone()?));
    } else if tracks == 1 {
        cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
    } else {
        let split = format!(
            "[1:a]asplit={tracks}{};",
            (0..tracks).map(|t| format!("[in{t}]")).collect::<String>()
        );
        let pans = (0..tracks)
            .map(|t| format!("[in{t}]pan=stereo|c0=c{}|c1=c{}[a{t}]", t * 2, t * 2 + 1))
            .collect::<Vec<_>>()
            .join(";");
        cmd.args([
            "-filter_complex_threads",
            "1",
            "-filter_complex",
            &(split + &pans),
            "-map",
            "0:v:0",
        ]);
        for t in 0..tracks {
            cmd.args([
                "-map",
                &format!("[a{t}]"),
                &format!("-metadata:s:a:{t}"),
                &format!("title=Track {}", t + 1),
            ]);
        }
    }
    if !camera {
    codec.video_args(&mut cmd, canvas.fps);
    cmd.args([
        "-c:a",
        codec.audio,
        "-threads:a",
        "1",
        "-metadata",
        "title=Rosorah Studio recording",
        "-metadata:s:a:0",
        "title=Track 1 mix",
    ])
    .stderr(Stdio::piped());
    #[cfg(unix)] cmd.stdin(Stdio::from(video_read));
    #[cfg(windows)] cmd.stdin(Stdio::null());
    if codec.stream_kbps.is_some() {
        cmd.args([
            "-f",
            "flv",
            "-flvflags",
            "no_duration_filesize",
            "-flush_packets",
            "1",
            "-b:a",
            "160k",
        ]);
    } else {
        cmd.args(["-f", "matroska", "-cluster_time_limit", "1000", "-flush_packets", "1"]);
    }
    if codec.stream_kbps.is_none() && codec.audio == "aac" {
        cmd.args(["-b:a", "192k"]);
    } else if codec.audio != "aac" {
        cmd.args(["-compression_level", "1"]);
    }
    match target {
        Target::Stream => {
            cmd.arg("pipe:1").stdout(Stdio::piped());
        }
        Target::File(path) => {
            cmd.arg("-n").arg(path).stdout(Stdio::null());
        }
        Target::Probe => {
            // Exercise the exact input/filter/encoder/muxer path without a file.
            cmd.args(["-progress", "pipe:1", "-y", if cfg!(windows) { "NUL" } else { "/dev/null" }])
                .stdout(Stdio::piped());
        }
        Target::Camera(_) => {}
    }
    }
    // Only the audio read end crosses exec. fcntl is async-signal-safe; no fd
    // number is guessed or overwritten, and all writer ends remain CLOEXEC.
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(move || {
            if libc::fcntl(audio_fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn().context("Start FFmpeg")?;
    #[cfg(unix)] drop(audio_read);
    #[cfg(windows)] { video_write.set_peer(child.id()); audio_write.set_peer(child.id()); }
    #[cfg(unix)] let encoder=Encoder(child);
    #[cfg(windows)] let encoder=Encoder(child,cancel);
    Ok((encoder, video_write, audio_write))
}
fn drain(mut pipe: impl Read, limit: usize) -> Vec<u8> {
    let mut out = VecDeque::new();
    let mut bytes = [0; 4096];
    while let Ok(n) = pipe.read(&mut bytes) {
        if n == 0 {
            break;
        }
        out.extend(&bytes[..n]);
        while out.len() > limit {
            out.pop_front();
        }
    }
    out.into_iter().collect()
}
fn probe(
    canvas: &Canvas,
    codec: &Codecs,
    deadline: Instant,
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    ensure!(!cancelled(), "Recording cancelled before it started");
    ensure!(
        Instant::now() < deadline,
        "GPU detection time limit reached"
    );
    let (mut child, mut video, mut audio) = launch_target(canvas, codec, Target::Probe)?;
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let progress = thread::spawn(move || drain(stdout, 16_384));
    let errors = thread::spawn(move || drain(stderr, 16_384));
    let pixels = canvas.width as usize * canvas.height as usize;
    let audio_frames = 4 * 48_000 / canvas.fps as usize;
    let video = thread::spawn(move || -> std::io::Result<()> {
        let rgba = [32, 96, 160, 255].repeat(pixels);
        for _ in 0..4 {
            video.write_all(&rgba)?;
        }
        Ok(())
    });
    let audio = thread::spawn(move || audio.write_all(&vec![0; audio_frames * 2 * 4]));
    let outcome = (|| -> Result<_> {
        loop {
            ensure!(!cancelled(), "Recording cancelled before it started");
            ensure!(Instant::now() < deadline, "Encoder check timed out");
            if let Some(status) = child.0.try_wait()? {
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(5));
        }
    })();
    // Close read ends even on cancellation/timeouts, unblocking every writer.
    // Drain threads continue reading until EOF and retain only bounded tails.
    drop(child);
    let video = video
        .join()
        .map_err(|_| anyhow::anyhow!("Encoder check video writer failed"));
    let audio = audio
        .join()
        .map_err(|_| anyhow::anyhow!("Encoder check audio writer failed"));
    let progress = progress.join().unwrap_or_default();
    let errors = errors.join().unwrap_or_default();
    let status = outcome?;
    ensure!(
        status.success() && errors.is_empty(),
        "FFmpeg encoder check failed ({status}): {}",
        String::from_utf8_lossy(&errors)
    );
    video??;
    audio??;
    let progress = String::from_utf8_lossy(&progress);
    ensure!(
        progress.lines().any(|line| line
            .strip_prefix("frame=")
            .and_then(|s| s.trim().parse::<u32>().ok())
            .is_some_and(|n| n >= 4))
            && progress.lines().any(|line| line == "progress=end"),
        "Encoder check produced no completed video"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const ENCODERS: &str = " V..... h264_vaapi VAAPI\n V..... h264_nvenc NVENC\n V..... libx264 H264\n V..... mpeg4 MPEG4\n V..... libvpx VP8\n A..... aac AAC\n A..... flac FLAC\n";
    #[test]
    fn broken_first_gpu_does_not_hide_second_gpu() {
        let paths = [
            PathBuf::from("/dev/dri/renderD128"),
            PathBuf::from("/dev/dri/renderD129"),
        ];
        let mut tried = Vec::new();
        let selection = choose(
            ENCODERS,
            &paths,
            RecordingEncoder::Auto,
            |c| {
                tried.push(c.clone());
                match &c.acceleration {
                    Acceleration::Vaapi {
                        device,
                        low_power: false,
                    } if device == &paths[1] => Ok(()),
                    _ => bail!("driver unavailable"),
                }
            },
            || false,
            |_| {},
        )
        .unwrap();
        assert!(selection.codecs.hardware());
        assert_eq!(tried.len(), 3);
        assert!(selection.details.contains("driver unavailable"));
    }
    #[test]
    fn unavailable_hardware_falls_back_and_explicit_software_skips_it() {
        let selection = choose(
            ENCODERS,
            &[],
            RecordingEncoder::Auto,
            |c| {
                if c.hardware() {
                    bail!("driver missing");
                }
                Ok(())
            },
            || false,
            |_| {},
        )
        .unwrap();
        assert_eq!(selection.codecs, Codecs::software("libx264", "aac"));
        assert!(selection.details.contains("Hardware encoding unavailable"));
        assert!(selection.details.contains("driver missing"));
        choose(
            ENCODERS,
            &["/dev/dri/renderD128".into()],
            RecordingEncoder::Software,
            |c| {
                assert!(!c.hardware());
                Ok(())
            },
            || false,
            |_| {},
        )
        .unwrap();
    }
    #[test]
    fn restricted_builds_and_hardware_only_builds_can_record() {
        let chosen = choose(
            " V..... libvpx VP8\n A..... flac FLAC\n",
            &[],
            RecordingEncoder::Auto,
            |_| Ok(()),
            || false,
            |_| {},
        )
        .unwrap();
        assert_eq!(chosen.codecs, Codecs::software("libvpx", "flac"));
        let chosen = choose(
            " V..... h264_nvenc NVENC\n A..... aac AAC\n",
            &[],
            RecordingEncoder::Auto,
            |_| Ok(()),
            || false,
            |_| {},
        )
        .unwrap();
        assert!(chosen.codecs.hardware());
        assert!(
            choose(
                " V..... libvpx VP8",
                &[],
                RecordingEncoder::Auto,
                |_| Ok(()),
                || false,
                |_| {}
            )
            .is_err()
        );
    }
    #[test]
    fn cancel_stops_selection_without_trying_another_encoder() {
        let cancelled = std::cell::Cell::new(false);
        let mut calls = 0;
        let result = choose(
            ENCODERS,
            &[],
            RecordingEncoder::Auto,
            |_| {
                calls += 1;
                cancelled.set(true);
                bail!("cancelled")
            },
            || cancelled.get(),
            |_| {},
        );
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert_eq!(calls, 1);
    }
    #[test]
    #[ignore = "Requires FFmpeg; checks real encode failure and software recovery"]
    fn real_probe_rejects_missing_gpu_and_recovers_with_software() -> Result<()> {
        let canvas = Canvas {
            width: 160,
            height: 90,
            fps: 30,
            ..auroracast_core::Project::default().canvases[0].clone()
        };
        let output = crate::audio::command("ffmpeg", &["-hide_banner", "-encoders"])?;
        // Force the known-missing VAAPI device even on a developer NVIDIA PC.
        let advertised = String::from_utf8_lossy(&output)
            .lines()
            .filter(|line| line.split_whitespace().nth(1) != Some("h264_nvenc"))
            .collect::<Vec<_>>()
            .join("\n");
        let selection = choose(
            &advertised,
            &["/dev/rosorah-missing-render-device".into()],
            RecordingEncoder::Auto,
            |c| {
                probe(&canvas, c, Instant::now() + Duration::from_secs(3), || {
                    false
                })
            },
            || false,
            |_| {},
        )?;
        assert!(!selection.codecs.hardware());
        assert!(selection.details.contains("FFmpeg encoder check failed"));
        println!("{}", selection.details);
        for codec in [
            Codecs::software("mpeg4", "aac"),
            Codecs::software("libvpx", "flac"),
        ] {
            if has_encoder(&advertised, codec.video) && has_encoder(&advertised, codec.audio) {
                probe(
                    &canvas,
                    &codec,
                    Instant::now() + Duration::from_secs(3),
                    || false,
                )?;
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "Requires FFmpeg; checks active child timeout cleanup"]
    fn probe_timeout_does_not_leave_blocked_writers() {
        let canvas = Canvas {
            width: 1920,
            height: 1080,
            fps: 30,
            ..auroracast_core::Project::default().canvases[0].clone()
        };
        let start = Instant::now();
        let result = probe(
            &canvas,
            &Codecs::software("mpeg4", "aac"),
            start + Duration::from_millis(1),
            || false,
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

#[cfg(test)]
mod multitrack_tests {
    use super::*;
    #[test]
    fn six_stereo_tracks_remain_separate_in_mkv() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("rosorah-six-tracks-{}.mkv", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = Canvas {
            id: 1,
            name: "test".into(),
            width: 64,
            height: 64,
            fps: 10,
            scene: 1,
        };
        let (mut encoder, mut video, mut audio) =
            launch_tracks(&c, &Codecs::software("mpeg4", "flac"), &path, 6)?;
        let v = thread::spawn(move || video.write_all(&vec![128; 64 * 64 * 4 * 10]));
        let a = thread::spawn(move || {
            let mut bytes = Vec::with_capacity(48000 * 12 * 4);
            for _ in 0..48000 {
                for t in 0..6 {
                    for value in [0.05 * (t + 1) as f32, -0.05 * (t + 1) as f32] {
                        bytes.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            audio.write_all(&bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(s) = encoder.0.try_wait()? {
                ensure!(s.success(), "Multitrack encoder failed");
                break;
            }
            ensure!(Instant::now() < deadline, "Multitrack encoder stalled");
            thread::sleep(Duration::from_millis(10));
        }
        v.join().unwrap()?;
        a.join().unwrap()?;
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-show_streams", "-of", "json"])
            .arg(&path)
            .output()?;
        let j: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        assert_eq!(j["streams"].as_array().unwrap().len(), 7);
        for t in 0..6 {
            let out = Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(&path)
                .args(["-map", &format!("0:a:{t}"), "-f", "f32le", "pipe:1"])
                .output()?;
            ensure!(out.status.success(), "Decode track");
            assert_eq!(out.stdout.len(), 48000 * 2 * 4);
            let left = f32::from_le_bytes(out.stdout[..4].try_into().unwrap());
            let right = f32::from_le_bytes(out.stdout[4..8].try_into().unwrap());
            assert!((left - 0.05 * (t + 1) as f32).abs() < 0.00001);
            assert!((right + 0.05 * (t + 1) as f32).abs() < 0.00001);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
}
