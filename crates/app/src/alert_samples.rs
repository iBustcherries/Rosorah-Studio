//! Built-in alert sounds and animations, generated on first use so new alerts work without files.
//! Sounds are synthesized WAV; animations are drawn frame by frame and encoded by FFmpeg
//! (transparent VP9 WebM). Nothing is downloaded.
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub name: &'static str,
    pub path: PathBuf,
    pub sound: bool,
}

pub fn folder() -> PathBuf {
    auroracast_core::platform::config_root().join("rosorah-studio/alert-samples")
}

/// Creates any missing samples and returns those available. Animations are skipped without FFmpeg.
pub fn ensure_all() -> Result<Vec<Sample>> {
    let dir = folder();
    std::fs::create_dir_all(&dir).context("Could not create the samples folder")?;
    let mut out = Vec::new();
    for (name, file, notes) in SOUNDS {
        let path = dir.join(file);
        if !path.is_file() {
            write_wav(&path, &synth(notes))?;
        }
        out.push(Sample { name, path, sound: true });
    }
    for (name, file, style) in [("Confetti burst", "confetti.webm", 0u8), ("Sparkle ring", "sparkle.webm", 1)] {
        let path = dir.join(file);
        if path.is_file() || animation(&path, style).is_ok() {
            out.push(Sample { name, path, sound: false });
        }
    }
    Ok(out)
}

/// (frequency Hz, start s, length s) notes per sound.
type Note = (f32, f32, f32);
const SOUNDS: [(&str, &str, &[Note]); 3] = [
    ("Chime", "chime.wav", &[(1046.5, 0.0, 0.9), (1318.5, 0.12, 0.9), (1568.0, 0.24, 1.1)]),
    ("Pop", "pop.wav", &[(660.0, 0.0, 0.12), (990.0, 0.05, 0.14)]),
    ("Fanfare", "fanfare.wav", &[(523.3, 0.0, 0.18), (659.3, 0.18, 0.18), (784.0, 0.36, 0.18), (1046.5, 0.54, 0.8)]),
];
const RATE: u32 = 44_100;

fn synth(notes: &[Note]) -> Vec<i16> {
    let end = notes.iter().map(|(_, s, l)| s + l).fold(0f32, f32::max) + 0.05;
    let mut mix = vec![0f32; (end * RATE as f32) as usize];
    for &(freq, start, length) in notes {
        let first = (start * RATE as f32) as usize;
        for i in 0..(length * RATE as f32) as usize {
            let t = i as f32 / RATE as f32;
            // Soft attack, exponential decay, and a quiet octave for a bell-like tone.
            let envelope = (t / 0.01).min(1.) * (-4.5 * t / length).exp();
            let wave = (std::f32::consts::TAU * freq * t).sin() + 0.25 * (std::f32::consts::TAU * freq * 2. * t).sin();
            if let Some(sample) = mix.get_mut(first + i) {
                *sample += wave * envelope;
            }
        }
    }
    let peak = mix.iter().fold(0f32, |m, s| m.max(s.abs())).max(1e-6);
    mix.iter().map(|s| (s / peak * 0.7 * i16::MAX as f32) as i16).collect()
}

fn write_wav(path: &Path, samples: &[i16]) -> Result<()> {
    let data = samples.len() as u32 * 2;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).context("Could not write a sample sound")
}

const SIZE: u32 = 480;
const FRAMES: u32 = 75; // 2.5 s at 30 fps

/// Draws a transparent animation and encodes it with FFmpeg.
fn animation(path: &Path, style: u8) -> Result<()> {
    let frames = folder().join(format!("frames-{style}"));
    std::fs::create_dir_all(&frames)?;
    for f in 0..FRAMES {
        let image = if style == 0 { confetti(f) } else { sparkle(f) };
        image.save(frames.join(format!("{f:03}.png")))?;
    }
    let partial = path.with_extension("part.webm");
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-framerate", "30", "-i"])
        .arg(frames.join("%03d.png"))
        .args(["-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0", "-crf", "30", "-an"])
        .arg(&partial)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = std::fs::remove_dir_all(&frames);
    ensure!(status.is_ok_and(|s| s.success()), "FFmpeg could not encode the animation");
    std::fs::rename(&partial, path)?;
    Ok(())
}

type Frame = image::RgbaImage;
const COLORS: [[u8; 3]; 6] = [[255, 92, 138], [255, 196, 61], [90, 214, 255], [155, 120, 255], [94, 230, 160], [255, 140, 70]];

/// Alpha-blended filled circle.
fn dot(img: &mut Frame, x: f32, y: f32, r: f32, color: [u8; 3], alpha: f32) {
    let (x0, x1) = ((x - r).floor().max(0.) as u32, ((x + r).ceil() as u32).min(SIZE - 1));
    let (y0, y1) = ((y - r).floor().max(0.) as u32, ((y + r).ceil() as u32).min(SIZE - 1));
    if x - r > SIZE as f32 || y - r > SIZE as f32 || x + r < 0. || y + r < 0. {
        return;
    }
    for py in y0..=y1 {
        for px in x0..=x1 {
            let d = ((px as f32 - x).powi(2) + (py as f32 - y).powi(2)).sqrt();
            let a = ((r - d + 0.5).clamp(0., 1.) * alpha).clamp(0., 1.);
            if a <= 0. {
                continue;
            }
            let p = img.get_pixel_mut(px, py);
            let base = p[3] as f32 / 255.;
            let out = a + base * (1. - a);
            for c in 0..3 {
                p[c] = ((color[c] as f32 * a + p[c] as f32 * base * (1. - a)) / out.max(1e-6)) as u8;
            }
            p[3] = (out * 255.) as u8;
        }
    }
}

/// Deterministic pseudo-random numbers so every install draws the same animation.
fn noise(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(747_796_405).wrapping_add(salt.wrapping_mul(2_891_336_453));
    x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277_803_737);
    ((x >> 22) ^ x) as f32 / u32::MAX as f32
}

fn confetti(frame: u32) -> Frame {
    let mut img = Frame::new(SIZE, SIZE);
    let t = frame as f32 / 30.;
    let fade = (1. - ((t - 1.6) / 0.9).max(0.)).clamp(0., 1.);
    for i in 0..90 {
        let angle = noise(i, 1) * std::f32::consts::TAU;
        let speed = 160. + noise(i, 2) * 260.;
        let x = SIZE as f32 / 2. + angle.cos() * speed * t;
        let y = SIZE as f32 / 2. + angle.sin() * speed * t + 220. * t * t;
        dot(&mut img, x, y, 4. + noise(i, 3) * 5., COLORS[i as usize % COLORS.len()], fade);
    }
    img
}

fn sparkle(frame: u32) -> Frame {
    let mut img = Frame::new(SIZE, SIZE);
    let t = frame as f32 / 30.;
    let grow = 1. - (-3.5 * t).exp();
    let fade = (1. - ((t - 1.5) / 1.0).max(0.)).clamp(0., 1.);
    let center = SIZE as f32 / 2.;
    for i in 0..36 {
        let angle = i as f32 / 36. * std::f32::consts::TAU + t * 0.8;
        let radius = 40. + grow * 170.;
        let twinkle = 0.55 + 0.45 * (t * 9. + i as f32).sin();
        let (x, y) = (center + angle.cos() * radius, center + angle.sin() * radius);
        dot(&mut img, x, y, 9. * twinkle, [255, 255, 255], 0.35 * fade);
        dot(&mut img, x, y, 4.5 * twinkle, COLORS[i as usize % COLORS.len()], fade);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sounds_are_valid_mono_wav_without_clipping() {
        let path = std::env::temp_dir().join(format!("rosorah-sample-{}.wav", std::process::id()));
        let samples = synth(SOUNDS[0].2);
        write_wav(&path, &samples).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(bytes.len(), 44 + samples.len() * 2);
        assert!(samples.iter().all(|s| s.unsigned_abs() <= (i16::MAX as f32 * 0.71) as u16));
        assert!(samples.len() > RATE as usize, "chime should last over a second");
    }
    #[test]
    fn animations_start_visible_and_end_transparent() {
        for draw in [confetti, sparkle] {
            let visible = |f: &Frame| f.pixels().filter(|p| p[3] > 0).count();
            assert!(visible(&draw(3)) > 500);
            assert_eq!(visible(&draw(FRAMES + 30)), 0);
        }
    }
}
#[cfg(test)]
mod generation {
    /// Run with: XDG_CONFIG_HOME=<tmp> cargo test -- --ignored generates_real_samples
    #[test]
    #[ignore]
    fn generates_real_samples() {
        let samples = super::ensure_all().unwrap();
        for s in &samples {
            println!("{} {}", s.name, s.path.display());
        }
        assert_eq!(samples.len(), 5, "needs FFmpeg with libvpx-vp9 for the animations");
    }
}
