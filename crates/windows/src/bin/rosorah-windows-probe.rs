//! Opt-in hardware diagnostic; never captures until a display ID is supplied.
#[cfg(not(windows))]
fn main() {
    eprintln!("This diagnostic requires Windows.");
    std::process::exit(1);
}
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use rosorah_capture::{Engine, Options, Phase, Target};
    use rosorah_windows::capture::{ScreenBackend, monitors};
    use std::time::{Duration, Instant};
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "--audio" {
        use rosorah_windows::audio::{self, Input};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let input = match args[1].as_str() {
            "desktop" => Input::Desktop,
            "microphone" => Input::Microphone,
            _ => anyhow::bail!("Choose --audio desktop or --audio microphone"),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || -> anyhow::Result<(u64, f32)> {
            let mut samples = 0;
            let mut peak = 0.0f32;
            audio::run(input, &worker_cancel, |packet| {
                samples += packet.samples.len() as u64;
                for sample in packet.samples {
                    peak = peak.max(sample.abs());
                }
                Ok(())
            })?;
            Ok((samples, peak))
        });
        std::thread::sleep(Duration::from_secs(5));
        cancel.store(true, Ordering::Relaxed);
        let (samples, peak) = worker
            .join()
            .map_err(|_| anyhow::anyhow!("Audio worker panicked"))??;
        println!("Audio stopped cleanly: {samples} samples, peak {peak:.3}. No audio saved.");
        if samples == 0 {
            println!("No packets received. Desktop loopback can be idle when no sound is playing.");
        }
        return Ok(());
    }
    let displays = monitors()?;
    for display in &displays {
        println!("{}  {} × {}", display.id, display.width, display.height);
    }
    if args.is_empty() {
        println!(
            "Use --monitor DISPLAY_ID to test five seconds of capture. No frames are saved. Audio test: --audio desktop or --audio microphone."
        );
        return Ok(());
    }
    anyhow::ensure!(
        args.len() == 2 && args[0] == "--monitor",
        "Usage: rosorah-windows-probe [--monitor DISPLAY_ID]"
    );
    anyhow::ensure!(
        displays.iter().any(|m| m.id == args[1]),
        "Choose one of the listed display IDs"
    );
    let mut engine = Engine::default();
    engine.start(
        1,
        Box::new(ScreenBackend {
            monitor_id: args[1].clone(),
        }),
        Options {
            target: Target::Screen,
            cursor: false,
            ..Default::default()
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut frames = 0;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        let _ = engine.drain();
        if let Some(s) = engine.snapshot(1) {
            anyhow::ensure!(s.phase != Phase::Failed, "{}", s.detail);
            frames = s.received;
        }
    }
    engine.stop(1);
    let deadline = Instant::now() + Duration::from_secs(2);
    while engine
        .snapshot(1)
        .is_some_and(|s| s.phase != Phase::Stopped)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    anyhow::ensure!(
        engine
            .snapshot(1)
            .is_some_and(|s| s.phase == Phase::Stopped),
        "Capture did not stop within two seconds"
    );
    anyhow::ensure!(frames > 0, "No display frames received");
    println!("Received {frames} frames; capture stopped cleanly.");
    Ok(())
}
