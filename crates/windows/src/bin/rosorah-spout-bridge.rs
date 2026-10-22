//! Brings Spout to Linux: run under Wine/Proton in the same session as a Windows app, it lists
//! that session's Spout senders or streams one sender's pictures to Rosorah through stdout.
//!   rosorah-spout-bridge.exe --list           sender names, one per line
//!   rosorah-spout-bridge.exe <sender>         frames: "RSPF", width, height (u32 LE), RGBA bytes;
//!                                             width 0 means the sender isn't sending right now
//!   rosorah-spout-bridge.exe --serve <name>   the other way: frames in the same format on stdin
//!                                             are shared as a Spout sender in this Wine session
//!   rosorah-spout-bridge.exe --send <name>    a moving test picture, for testing the bridge
#[cfg(windows)]
fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
#[cfg(windows)]
fn run() -> anyhow::Result<()> {
    use rosorah_windows::spout;
    use std::{
        io::Write,
        time::{Duration, Instant},
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--list"] => {
            for name in spout::senders() {
                println!("{name}");
            }
        }
        ["--serve", name] => {
            use std::io::Read;
            let mut sender = spout::Sender::new(name)?;
            let mut input = std::io::stdin().lock();
            let mut header = [0u8; 12];
            let mut rgba = Vec::new();
            // Rosorah closing the pipe ends the sender.
            while input.read_exact(&mut header).is_ok() {
                anyhow::ensure!(&header[..4] == b"RSPF", "Unexpected input");
                let w = u32::from_le_bytes(header[4..8].try_into()?);
                let h = u32::from_le_bytes(header[8..12].try_into()?);
                anyhow::ensure!(w <= 8192 && h <= 8192, "Picture too large");
                rgba.resize((w * h * 4) as usize, 0);
                input.read_exact(&mut rgba)?;
                sender.send(&rgba, w, h)?;
            }
        }
        ["--send", name] => {
            let mut sender = spout::Sender::new(name)?;
            let (w, h) = (640u32, 360u32);
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            for tick in 0u32.. {
                for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
                    let (x, y) = (i as u32 % w, i as u32 / w);
                    px.copy_from_slice(&[((x + tick * 4) % 256) as u8, (y % 256) as u8, 128, 255]);
                }
                sender.send(&rgba, w, h)?;
                std::thread::sleep(Duration::from_millis(33));
            }
        }
        [name] => {
            let mut receiver = spout::Receiver::new(name)?;
            let mut out = std::io::stdout().lock();
            let mut next = Instant::now();
            loop {
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
                next = Instant::now() + Duration::from_millis(16);
                let frame = receiver.frame()?;
                let (w, h) = frame.as_ref().map_or((0, 0), |f| (f.width, f.height));
                out.write_all(b"RSPF")?;
                out.write_all(&w.to_le_bytes())?;
                out.write_all(&h.to_le_bytes())?;
                match frame {
                    Some(frame) => out.write_all(&frame.rgba)?,
                    None => next += Duration::from_millis(484),
                }
                // Rosorah closing the pipe ends the bridge.
                out.flush()?;
            }
        }
        _ => anyhow::bail!("usage: rosorah-spout-bridge.exe --list | <sender> | --serve <name> | --send <name>"),
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {}
