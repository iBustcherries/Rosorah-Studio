//! Reads paid messages aloud (Super Chats, donations, cheers and Kicks) with Piper, a free offline
//! voice engine that's downloaded, with the chosen voice, the first time it's needed. Speech
//! plays through a sound source of its own, so it's in the stream and recordings and has its own
//! strip in the mixer.
use anyhow::{Context, Result, bail, ensure};
use auroracast_core::Speech;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

pub struct Voice {
    pub id: &'static str,
    pub label: &'static str,
    /// The voice's folder in Piper's voice collection.
    path: &'static str,
    onnx: &'static str,
    json: &'static str,
}
pub const VOICES: [Voice; 4] = [
    Voice {
        id: "en_US-lessac-medium",
        label: "Lessac (American English)",
        path: "en/en_US/lessac/medium",
        onnx: "5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f",
        json: "efe19c417bed055f2d69908248c6ba650fa135bc868b0e6abb3da181dab690a0",
    },
    Voice {
        id: "en_US-amy-medium",
        label: "Amy (American English)",
        path: "en/en_US/amy/medium",
        onnx: "b3a6e47b57b8c7fbe6a0ce2518161a50f59a9cdd8a50835c02cb02bdd6206c18",
        json: "95a23eb4d42909d38df73bb9ac7f45f597dbfcde2d1bf9526fdeaf5466977d77",
    },
    Voice {
        id: "en_US-ryan-medium",
        label: "Ryan (American English)",
        path: "en/en_US/ryan/medium",
        onnx: "abf4c274862564ed647ba0d2c47f8ee7c9b717d27bdad9219100eb310db4047a",
        json: "44034c056cb15681b2ad494307c7f3f2e4499d1253c700c711fa0a4607ffe78d",
    },
    Voice {
        id: "en_GB-alan-medium",
        label: "Alan (British English)",
        path: "en/en_GB/alan/medium",
        onnx: "0a309668932205e762801f1efc2736cd4b0120329622adf62be09e56339d3330",
        json: "c0f0d124e5895c00e7c03b35dcc8287f319a6998a365b182deb5c8e752ee8c1e",
    },
];
const VOICE_BASE: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0";
/// Piper's own release for this system: (address, SHA-256).
const PIPER: (&str, &str) = if cfg!(windows) {
    ("https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip", "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea")
} else {
    ("https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz", "a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992")
};
/// Waiting lines are dropped beyond this, so a flood of small cheers can't queue for an hour.
const QUEUE: usize = 20;

pub fn voice(id: &str) -> &'static Voice {
    VOICES.iter().find(|v| v.id == id).unwrap_or(&VOICES[0])
}
fn folder() -> PathBuf {
    auroracast_core::platform::config_root().join("rosorah-studio/speech")
}
fn piper_program() -> PathBuf {
    folder().join("piper").join(if cfg!(windows) { "piper.exe" } else { "piper" })
}
fn voice_file(v: &Voice) -> PathBuf {
    folder().join("voices").join(format!("{}.onnx", v.id))
}
/// The recording the speech sound plays; each line replaces the last once it has finished.
pub fn output() -> PathBuf {
    folder().join("speech.wav")
}
/// Whether the engine and this voice are downloaded already.
fn installed(v: &Voice) -> bool {
    let model = voice_file(v);
    piper_program().is_file() && model.is_file() && model.with_extension("onnx.json").is_file()
}

/// What to say for a paid message, or None when there's nothing to read or it has a blocked word.
pub fn spoken(author: &str, text: &str, settings: &Speech) -> Option<String> {
    let mut words = Vec::new();
    // Emote pictures travel as U+E000 address U+E001 name U+E002; none of it is read.
    let plain: String = {
        let mut out = String::new();
        let mut skip = false;
        for c in text.chars() {
            match c {
                '\u{e000}' => skip = true,
                '\u{e002}' => {
                    skip = false;
                    out.push(' ');
                }
                c if skip || ('\u{e000}'..='\u{f8ff}').contains(&c) || c.is_control() => (),
                c => out.push(c),
            }
        }
        out
    };
    for word in plain.split_whitespace() {
        let lower = word.to_lowercase();
        // Links aren't read, and neither are cheer amounts written as emotes ("Cheer100").
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
            continue;
        }
        let letters = lower.trim_end_matches(|c: char| c.is_ascii_digit());
        if letters.len() < lower.len() && letters.len() >= 3 && letters.chars().all(|c| c.is_ascii_alphabetic()) && (letters == "cheer" || letters.ends_with("cheer")) {
            continue;
        }
        words.push(squeeze(word));
    }
    let blocked: Vec<String> = settings.blocked.split([',', '\n']).map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
    let clean = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    if words.iter().chain(std::iter::once(&author.to_string())).any(|w| blocked.contains(&clean(w))) {
        return None;
    }
    let max = settings.max_chars.clamp(20, 500) as usize;
    let mut message = String::new();
    for w in &words {
        if message.chars().count() + w.chars().count() + 1 > max {
            break;
        }
        if !message.is_empty() {
            message.push(' ');
        }
        message.push_str(w);
    }
    let message = message.replace(" · ", ", ");
    if !message.chars().any(char::is_alphanumeric) {
        return None;
    }
    let author = author.trim();
    if settings.read_names && !author.is_empty() && !message.to_lowercase().starts_with(&author.to_lowercase()) {
        Some(format!("{author} says: {message}"))
    } else {
        Some(message)
    }
}
/// "soooooo" → "sooo", so stretched words don't drone on.
fn squeeze(word: &str) -> String {
    let mut out = String::new();
    let (mut last, mut run) = ('\0', 0);
    for c in word.chars() {
        run = if c == last { run + 1 } else { 1 };
        last = c;
        if run <= 3 {
            out.push(c);
        }
    }
    out
}

/// Lines waiting to be read, and the one being turned into speech.
#[derive(Default)]
pub struct Reader {
    queue: VecDeque<String>,
    job: Option<mpsc::Receiver<Result<(), String>>>,
    started: Option<Instant>,
    pub status: String,
}
impl Reader {
    pub fn say(&mut self, line: String) {
        if self.queue.len() < QUEUE {
            self.queue.push_back(line);
        }
    }
    /// Drops everything waiting (the line playing is stopped by the caller).
    pub fn clear(&mut self) {
        self.queue.clear();
    }
    pub fn waiting(&self) -> usize {
        self.queue.len()
    }
    /// Call every frame with whether the speech sound is playing. Returns true when a new line
    /// has been recorded and should play now.
    pub fn tick(&mut self, settings: &Speech, playing: bool) -> bool {
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(result) => {
                    self.job = None;
                    match result {
                        Ok(()) => {
                            self.status.clear();
                            self.started = Some(Instant::now());
                            return true;
                        }
                        Err(e) => self.status = e,
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return false,
                Err(mpsc::TryRecvError::Disconnected) => self.job = None,
            }
        }
        // The sound takes a moment to start after it's asked to play.
        if playing || self.started.is_some_and(|t| t.elapsed() < Duration::from_millis(800)) || !settings.enabled {
            return false;
        }
        let Some(line) = self.queue.pop_front() else { return false };
        let v = voice(&settings.voice);
        if !installed(v) {
            self.status = "Downloading the voice (about 90 MB, only once)…".into();
        }
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        let _ = std::thread::Builder::new().name("speech".into()).spawn(move || {
            let _ = tx.send(install(v).and_then(|()| record(v, &line)).map_err(|e| format!("Couldn't read the message aloud: {e:#}")));
        });
        false
    }
}

/// Downloads Piper and the voice when they're missing, checking each file against its known hash.
fn install(v: &Voice) -> Result<()> {
    let dir = folder();
    std::fs::create_dir_all(dir.join("voices")).context("Couldn't create the speech folder")?;
    if !piper_program().is_file() {
        let archive = dir.join(if cfg!(windows) { "piper.zip" } else { "piper.tar.gz" });
        download(PIPER.0, PIPER.1, &archive)?;
        // Windows 10 and later include tar, which also reads zip files.
        let status = quiet(std::process::Command::new("tar").arg("-xf").arg(&archive).arg("-C").arg(&dir)).status().context("Couldn't unpack the voice engine")?;
        let _ = std::fs::remove_file(&archive);
        ensure!(status.success() && piper_program().is_file(), "Couldn't unpack the voice engine");
    }
    let model = voice_file(v);
    if !model.with_extension("onnx.json").is_file() {
        download(&format!("{VOICE_BASE}/{}/{}.onnx.json", v.path, v.id), v.json, &model.with_extension("onnx.json"))?;
    }
    if !model.is_file() {
        download(&format!("{VOICE_BASE}/{}/{}.onnx", v.path, v.id), v.onnx, &model)?;
    }
    Ok(())
}
fn download(url: &str, sha256: &str, to: &Path) -> Result<()> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(600)).user_agent("Rosorah Studio").build()?;
    let mut reply = client.get(url).send()?.error_for_status()?;
    let part = to.with_extension("part");
    let mut file = std::fs::File::create(&part)?;
    let (mut hash, mut total, mut buf) = (Sha256::new(), 0usize, vec![0; 1 << 16]);
    loop {
        let n = reply.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n;
        if total > 200 << 20 {
            bail!("the download was larger than expected");
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    drop(file);
    if format!("{:x}", hash.finalize()) != sha256 {
        let _ = std::fs::remove_file(&part);
        bail!("the downloaded file didn't match (check your connection and try again)");
    }
    std::fs::rename(&part, to)?;
    Ok(())
}
/// Turns one line into the speech recording.
fn record(v: &Voice, line: &str) -> Result<()> {
    let part = folder().join("speech.part.wav");
    let mut child = quiet(std::process::Command::new(piper_program()).arg("--model").arg(voice_file(v)).arg("--output_file").arg(&part))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Couldn't start the voice engine")?;
    // One line, so a newline in the message can't split it into several.
    child.stdin.take().context("no input")?.write_all(format!("{}\n", line.replace(['\n', '\r'], " ")).as_bytes())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success() && part.is_file(), "the voice engine stopped");
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            bail!("the voice engine took too long");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::rename(&part, output())?;
    Ok(())
}
/// Helpers run without a console window on Windows.
fn quiet(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> Speech {
        Speech { enabled: true, ..Default::default() }
    }
    #[test]
    fn paid_messages_are_cleaned_up_for_reading() {
        let s = settings();
        assert_eq!(spoken("juniper", "$5.00 · great stream!", &s).as_deref(), Some("juniper says: $5.00, great stream!"));
        assert_eq!(spoken("juniper", "juniper cheered 100 Bits · Cheer100 lets gooooooo", &s).as_deref(), Some("juniper cheered 100 Bits, lets gooo"));
        assert_eq!(spoken("mia", "look https://example.com/x here", &s).as_deref(), Some("mia says: look here"));
        assert_eq!(spoken("mia", "hi \u{e000}https://files.kick.com/emotes/1/fullsize\u{e001}KEKW\u{e002} there", &s).as_deref(), Some("mia says: hi there"));
        assert_eq!(spoken("mia", "Cheer100", &s), None);
        let s = Speech { blocked: "badword, other".into(), read_names: false, max_chars: 20, ..settings() };
        assert_eq!(spoken("mia", "this has a BadWord!", &s), None);
        assert_eq!(spoken("mia", "one two three four five six seven", &s).as_deref(), Some("one two three four"));
    }
    /// Downloads Piper and a voice and records a line: XDG_CONFIG_HOME=<dir> cargo test speech_downloads -- --ignored
    #[test]
    #[ignore]
    fn speech_downloads_and_records() {
        let v = voice("en_US-amy-medium");
        install(v).unwrap();
        assert!(installed(v));
        record(v, "juniper says: thanks for the great stream").unwrap();
        let wav = std::fs::read(output()).unwrap();
        assert!(wav.starts_with(b"RIFF") && wav.len() > 20_000, "{} bytes", wav.len());
        // The recording plays into the mix the way the "Read aloud" sound does.
        let bus = crate::video::AudioBus::default();
        let mut engine = rosorah_capture::Engine::default();
        let settings = auroracast_core::VideoSettings { path: output().display().to_string(), audio: true, looped: false, automation_only: true, ..Default::default() };
        engine.start(5, Box::new(crate::video::VideoBackend { id: 5, settings, audio: bus.clone() }), rosorah_capture::Options::default()).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        let mut heard = false;
        while Instant::now() < until && !heard {
            heard = bus.read().values().flatten().any(|v| v.abs() > 0.01);
            std::thread::sleep(Duration::from_millis(10));
        }
        engine.stop(5);
        assert!(heard, "the speech made no sound in the mix");
    }
}
