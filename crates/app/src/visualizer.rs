//! Music levels for the Now playing visualizer. The mixer feeds each 10 ms block in while a
//! widget is asking, and widgets read smoothed band levels from low to high frequencies.
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

/// Bands from 50 Hz to 14 kHz, spaced evenly in pitch.
pub const BANDS: usize = 64;
const WINDOW: usize = 2048;
const RATE: f32 = 48_000.;

struct Feed {
    samples: VecDeque<f32>,
    levels: [f32; BANDS],
    updated: Option<Instant>,
    wanted: Option<Instant>,
}
fn feed() -> &'static Mutex<Feed> {
    static FEED: OnceLock<Mutex<Feed>> = OnceLock::new();
    FEED.get_or_init(|| {
        Mutex::new(Feed { samples: VecDeque::with_capacity(WINDOW), levels: [0.; BANDS], updated: None, wanted: None })
    })
}

/// Whether a widget read the levels in the last two seconds, so the mixer only does the work then.
pub fn wanted() -> bool {
    feed().lock().unwrap().wanted.is_some_and(|t| t.elapsed() < Duration::from_secs(2))
}

/// Takes one block of interleaved stereo samples.
pub fn push(stereo: &[f32]) {
    let mut f = feed().lock().unwrap();
    for frame in stereo.chunks_exact(2) {
        if f.samples.len() == WINDOW {
            f.samples.pop_front();
        }
        f.samples.push_back((frame[0] + frame[1]) * 0.5);
    }
    if f.samples.len() < WINDOW {
        return;
    }
    let target = spectrum(f.samples.make_contiguous());
    // Quick to rise, slower to fall, so the bars move with the beat without flickering.
    for (level, t) in f.levels.iter_mut().zip(target) {
        *level = if t > *level { *level + (t - *level) * 0.55 } else { (*level - 0.018).max(t) };
    }
    f.updated = Some(Instant::now());
}

/// The current band levels (0 to 1), or `None` when no sound has come in lately.
pub fn levels() -> Option<[f32; BANDS]> {
    let mut f = feed().lock().unwrap();
    f.wanted = Some(Instant::now());
    f.updated.filter(|t| t.elapsed() < Duration::from_millis(500)).map(|_| f.levels)
}

/// A gentle made-up shape for previews and screenshots, moving with `clock` (seconds).
pub fn sample(clock: f32) -> [f32; BANDS] {
    std::array::from_fn(|i| {
        let x = i as f32 / BANDS as f32;
        let base = 0.75 - x * 0.45 + (x * 9. + clock * 1.7).sin() * 0.12 + (x * 23. - clock * 2.9).sin() * 0.08;
        base.clamp(0.05, 1.)
    })
}

fn spectrum(samples: &[f32]) -> [f32; BANDS] {
    let mut re: Vec<f32> = samples
        .iter()
        .enumerate()
        .map(|(i, s)| s * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (WINDOW - 1) as f32).cos()))
        .collect();
    let mut im = vec![0.; WINDOW];
    fft(&mut re, &mut im);
    // A full-scale sine through the Hann window peaks at WINDOW / 4.
    let mags: Vec<f32> = re.iter().zip(&im).take(WINDOW / 2).map(|(r, i)| r.hypot(*i) / (WINDOW as f32 / 4.)).collect();
    let bin = RATE / WINDOW as f32;
    let (low, high) = (50f32, 14_000f32);
    std::array::from_fn(|b| {
        let edge = |k: f32| low * (high / low).powf(k / BANDS as f32);
        let (from, to) = (edge(b as f32) / bin, edge(b as f32 + 1.) / bin);
        let mag = if to - from < 1. {
            // Narrower than one bin: read between the two nearest.
            let c = (from + to) / 2.;
            let i = c.floor() as usize;
            let t = c - i as f32;
            mags[i] * (1. - t) + mags[(i + 1).min(mags.len() - 1)] * t
        } else {
            mags[from.ceil() as usize..=(to.floor() as usize).min(mags.len() - 1)].iter().fold(0f32, |a, m| a.max(*m))
        };
        // Music is quieter up high; tilt 3 dB per octave so the treble shows too.
        let centre = edge(b as f32 + 0.5);
        let db = 20. * mag.max(1e-6).log10() + 3. * (centre / 1000.).log2();
        ((db + 54.) / 50.).clamp(0., 1.)
    })
}

/// In-place radix-2 FFT; `re.len()` must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(freq: f32) -> Vec<f32> {
        (0..WINDOW).map(|i| (std::f32::consts::TAU * freq * i as f32 / RATE).sin() * 0.5).collect()
    }
    #[test]
    fn a_tone_lights_its_own_band() {
        for freq in [110., 1000., 6000.] {
            let bands = spectrum(&tone(freq));
            let loudest = (0..BANDS).max_by(|a, b| bands[*a].total_cmp(&bands[*b])).unwrap();
            let expected = ((freq / 50f32).ln() / (14_000f32 / 50.).ln() * BANDS as f32) as usize;
            assert!(loudest.abs_diff(expected) <= 1, "{freq} Hz lit band {loudest}, expected {expected}");
            assert!(bands[loudest] > 0.6);
            assert!(bands[(expected + 12).min(BANDS - 1)] < 0.25, "{freq} Hz leaks: {bands:?}");
        }
    }
    #[test]
    fn silence_is_flat() {
        assert!(spectrum(&vec![0.; WINDOW]).iter().all(|b| *b == 0.));
    }
}
