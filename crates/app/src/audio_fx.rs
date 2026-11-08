//! Rosorah's own sound filters, run on each strip's audio in place before mixing:
//! low cut, voice isolation, gate, expander, compressor, three-band EQ, mono/balance and
//! limiter. Audio is 48 kHz interleaved stereo; apart from voice isolation (a small neural
//! network, RNNoise), everything is a few multiplications per sample.
use auroracast_core::AudioFilters;
use nnnoiseless::DenoiseState;
use std::collections::VecDeque;

const RATE: f32 = 48_000.;
/// RNNoise works on 10 ms frames and hands each one back a frame later.
const FRAME: usize = DenoiseState::FRAME_SIZE;

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.)
}
fn gain_to_db(gain: f32) -> f32 {
    20. * gain.max(1e-9).log10()
}
/// How much a smoothed value moves toward its target per sample, for a time constant in ms.
fn coefficient(ms: f32) -> f32 {
    (-1. / (ms.max(0.1) * 0.001 * RATE)).exp()
}

/// A second-order filter section (one EQ band), one state per side.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    state: [[f32; 2]; 2],
}
impl Biquad {
    /// Shelf (low or high) or peak band at `freq` Hz, boosting or cutting by `db`.
    fn band(kind: Band, freq: f32, db: f32) -> Self {
        let a = 10f32.powf(db / 40.);
        let w = std::f32::consts::TAU * freq / RATE;
        let (sin, cos) = w.sin_cos();
        let q = 0.707;
        let alpha = sin / (2. * q);
        let (b, a_coeffs) = match kind {
            Band::Peak => ([1. + alpha * a, -2. * cos, 1. - alpha * a], [1. + alpha / a, -2. * cos, 1. - alpha / a]),
            Band::LowShelf => {
                let s = 2. * a.sqrt() * alpha;
                (
                    [a * ((a + 1.) - (a - 1.) * cos + s), 2. * a * ((a - 1.) - (a + 1.) * cos), a * ((a + 1.) - (a - 1.) * cos - s)],
                    [(a + 1.) + (a - 1.) * cos + s, -2. * ((a - 1.) + (a + 1.) * cos), (a + 1.) + (a - 1.) * cos - s],
                )
            }
            Band::HighShelf => {
                let s = 2. * a.sqrt() * alpha;
                (
                    [a * ((a + 1.) + (a - 1.) * cos + s), -2. * a * ((a - 1.) + (a + 1.) * cos), a * ((a + 1.) + (a - 1.) * cos - s)],
                    [(a + 1.) - (a - 1.) * cos + s, 2. * ((a - 1.) - (a + 1.) * cos), (a + 1.) - (a - 1.) * cos - s],
                )
            }
        };
        Self::normalised(b, a_coeffs)
    }
    /// High-pass section at `freq` Hz with quality `q`.
    fn high_pass(freq: f32, q: f32) -> Self {
        let w = std::f32::consts::TAU * freq / RATE;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2. * q);
        Self::normalised([(1. + cos) / 2., -(1. + cos), (1. + cos) / 2.], [1. + alpha, -2. * cos, 1. - alpha])
    }
    fn normalised(b: [f32; 3], a: [f32; 3]) -> Self {
        Self { b: [b[0] / a[0], b[1] / a[0], b[2] / a[0]], a: [a[1] / a[0], a[2] / a[0]], state: [[0.; 2]; 2] }
    }
    fn run(&mut self, side: usize, x: f32) -> f32 {
        // Transposed direct form II: two remembered values per side.
        let s = &mut self.state[side];
        let y = self.b[0] * x + s[0];
        s[0] = self.b[1] * x - self.a[0] * y + s[1];
        s[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}
#[derive(Clone, Copy)]
enum Band {
    LowShelf,
    Peak,
    HighShelf,
}

/// One strip's running filter state (envelopes, EQ memory), kept between 10 ms blocks.
#[derive(Default)]
pub struct Chain {
    gate_gain: f32,
    gate_hold: u32,
    envelope: f32,
    compress_gain: f32,
    expand_gain: f32,
    limit_gain: f32,
    eq: Option<([f32; 3], [Biquad; 3])>,
    low_cut: Option<(f32, [Biquad; 2])>,
    isolation: Option<Isolation>,
}

/// Voice isolation's state for both sides: the network, and the sound waiting for a whole frame.
struct Isolation {
    networks: [Box<DenoiseState<'static>>; 2],
    waiting: [Vec<f32>; 2],
    cleaned: [VecDeque<f32>; 2],
    /// The untouched sound, held back as long as the cleaned sound is, so blending lines up.
    untouched: [VecDeque<f32>; 2],
}
impl Isolation {
    fn new() -> Self {
        Self {
            networks: [DenoiseState::new(), DenoiseState::new()],
            waiting: [Vec::with_capacity(FRAME * 2), Vec::with_capacity(FRAME * 2)],
            cleaned: Default::default(),
            untouched: std::array::from_fn(|_| VecDeque::from(vec![0.; FRAME])),
        }
    }
    /// Cleans one block in place; `strength` 0…1 blends the cleaned sound with the untouched one.
    fn run(&mut self, pcm: &mut [f32], strength: f32) {
        let frames = pcm.len() / 2;
        let mut out = [0f32; FRAME];
        for side in 0..2 {
            // RNNoise expects 16-bit sample values.
            self.waiting[side].extend(pcm.iter().skip(side).step_by(2).map(|s| s * 32768.));
            self.untouched[side].extend(pcm.iter().skip(side).step_by(2));
            while self.waiting[side].len() >= FRAME {
                self.networks[side].process_frame(&mut out, &self.waiting[side][..FRAME]);
                self.cleaned[side].extend(out.iter().map(|s| s / 32768.));
                self.waiting[side].drain(..FRAME);
            }
            // Blocks that aren't whole frames wait a little longer, once.
            let short = frames.saturating_sub(self.cleaned[side].len());
            for _ in 0..short {
                self.cleaned[side].push_front(0.);
                self.untouched[side].push_front(0.);
            }
            for (sample, (clean, dry)) in pcm.iter_mut().skip(side).step_by(2).zip(self.cleaned[side].drain(..frames).zip(self.untouched[side].drain(..frames))) {
                *sample = clean * strength + dry * (1. - strength);
            }
        }
    }
}
impl Chain {
    pub fn process(&mut self, f: &AudioFilters, pcm: &mut [f32]) {
        if !f.active() {
            return;
        }
        // Low cut first: a fourth-order high-pass (two Butterworth sections, 24 dB per octave).
        if f.low_cut {
            let hz = f.low_cut_hz.clamp(20., 500.);
            if self.low_cut.as_ref().is_none_or(|(have, _)| *have != hz) {
                let mut sections = [Biquad::high_pass(hz, 0.5412), Biquad::high_pass(hz, 1.3066)];
                if let Some((_, old)) = &self.low_cut {
                    for (s, previous) in sections.iter_mut().zip(old) {
                        s.state = previous.state;
                    }
                }
                self.low_cut = Some((hz, sections));
            }
            if let Some((_, sections)) = &mut self.low_cut {
                for frame in pcm.chunks_exact_mut(2) {
                    for s in sections.iter_mut() {
                        frame[0] = s.run(0, frame[0]);
                        frame[1] = s.run(1, frame[1]);
                    }
                }
            }
        } else {
            self.low_cut = None;
        }
        // Then voice isolation, so the gate, expander and compressor follow the cleaned voice.
        if f.voice_isolation {
            self.isolation.get_or_insert_with(Isolation::new).run(pcm, f.isolation_strength.clamp(0., 1.));
        } else {
            self.isolation = None;
        }
        if self.limit_gain == 0. {
            self.gate_gain = 1.;
            self.compress_gain = 1.;
            self.expand_gain = 1.;
            self.limit_gain = 1.;
        }
        if f.eq {
            let wanted = [f.bass, f.middle, f.treble];
            if self.eq.as_ref().is_none_or(|(have, _)| *have != wanted) {
                let mut bands = [Biquad::band(Band::LowShelf, 120., f.bass), Biquad::band(Band::Peak, 1000., f.middle), Biquad::band(Band::HighShelf, 6000., f.treble)];
                // Keep the filters' memory when only the settings change, so a slider move doesn't click.
                if let Some((_, old)) = &self.eq {
                    for (band, previous) in bands.iter_mut().zip(old) {
                        band.state = previous.state;
                    }
                }
                self.eq = Some((wanted, bands));
            }
        }
        let gate_open = db_to_gain(f.gate_open);
        let gate_close = db_to_gain(f.gate_open - 6.);
        let gate_release = coefficient(f.gate_release);
        let follow_attack = coefficient(5.);
        let follow_release = coefficient(80.);
        let compress_attack = coefficient(f.compress_attack);
        let compress_release = coefficient(f.compress_release);
        let makeup = db_to_gain(f.makeup);
        let limit = db_to_gain(f.limit_to.min(0.));
        let limit_release = coefficient(60.);
        let (pan_left, pan_right) = {
            // Equal-loudness placement: the centre keeps both sides at full level.
            let p = f.balance.clamp(-1., 1.);
            ((1. - p).min(1.), (1. + p).min(1.))
        };
        for frame in pcm.chunks_exact_mut(2) {
            let (mut l, mut r) = (frame[0], frame[1]);
            if f.mono {
                let m = (l + r) * 0.5;
                l = m;
                r = m;
            }
            // How loud it is right now (fast to rise, slower to fall).
            let level = l.abs().max(r.abs());
            let k = if level > self.envelope { follow_attack } else { follow_release };
            self.envelope = level + (self.envelope - level) * k;
            if f.gate {
                // Opens at once above the threshold, holds 50 ms, then closes smoothly (with 6 dB of
                // hysteresis so it doesn't flutter on the edge).
                if self.envelope >= gate_open {
                    self.gate_hold = (RATE * 0.05) as u32;
                    self.gate_gain = 1.;
                } else if self.gate_hold > 0 {
                    self.gate_hold -= 1;
                } else if self.envelope < gate_close {
                    self.gate_gain *= gate_release;
                }
                l *= self.gate_gain;
                r *= self.gate_gain;
            }
            if f.expander {
                let db = gain_to_db(self.envelope);
                let target = if db < f.expand_below { db_to_gain((db - f.expand_below) * (f.expand_ratio.max(1.) - 1.)) } else { 1. };
                let k = if target < self.expand_gain { compress_attack } else { compress_release };
                self.expand_gain = target + (self.expand_gain - target) * k;
                l *= self.expand_gain;
                r *= self.expand_gain;
            }
            if f.compressor {
                let db = gain_to_db(self.envelope);
                let over = db - f.compress_above;
                let target = if over > 0. { db_to_gain(-over * (1. - 1. / f.compress_ratio.max(1.))) } else { 1. };
                let k = if target < self.compress_gain { compress_attack } else { compress_release };
                self.compress_gain = target + (self.compress_gain - target) * k;
                l *= self.compress_gain * makeup;
                r *= self.compress_gain * makeup;
            }
            if f.eq
                && let Some((_, bands)) = &mut self.eq
            {
                for band in bands.iter_mut() {
                    l = band.run(0, l);
                    r = band.run(1, r);
                }
            }
            l *= pan_left;
            r *= pan_right;
            if f.limiter {
                // Turns down instantly when a peak would cross the ceiling, recovers smoothly.
                let peak = l.abs().max(r.abs());
                let needed = if peak * self.limit_gain > limit { limit / peak } else { 1. };
                self.limit_gain = if needed < self.limit_gain { needed } else { needed + (self.limit_gain - needed) * limit_release };
                l *= self.limit_gain;
                r *= self.limit_gain;
            }
            frame[0] = l;
            frame[1] = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(freq: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        (0..(RATE * seconds) as usize).flat_map(|i| {
            let v = (std::f32::consts::TAU * freq * i as f32 / RATE).sin() * amplitude;
            [v, v]
        }).collect()
    }
    fn peak(pcm: &[f32]) -> f32 {
        pcm.iter().fold(0f32, |m, v| m.max(v.abs()))
    }
    fn run(f: &AudioFilters, mut pcm: Vec<f32>) -> Vec<f32> {
        let mut chain = Chain::default();
        for block in pcm.chunks_mut(960) {
            chain.process(f, block);
        }
        pcm
    }
    #[test]
    fn gate_silences_quiet_sound_and_passes_loud_sound() {
        let f = AudioFilters { gate: true, gate_open: -30., ..Default::default() };
        let quiet = run(&f, tone(440., 0.005, 1.));
        assert!(peak(&quiet[48_000..]) < 0.0005, "hiss removed");
        let loud = run(&f, tone(440., 0.5, 1.));
        assert!(peak(&loud[48_000..]) > 0.45, "voice kept");
    }
    #[test]
    fn compressor_and_limiter_keep_peaks_down() {
        let f = AudioFilters { compressor: true, compress_above: -20., compress_ratio: 4., ..Default::default() };
        let squeezed = run(&f, tone(440., 0.9, 1.));
        assert!(peak(&squeezed[48_000..]) < 0.4, "{}", peak(&squeezed[48_000..]));
        let f = AudioFilters { limiter: true, limit_to: -6., ..Default::default() };
        let limited = run(&f, tone(440., 1.0, 0.5));
        assert!(peak(&limited) <= db_to_gain(-6.) + 0.001, "never above the ceiling: {}", peak(&limited));
    }
    #[test]
    fn eq_boosts_its_band_and_mono_balance_place_sound() {
        let f = AudioFilters { eq: true, bass: 9., ..Default::default() };
        let bass = run(&f, tone(80., 0.1, 1.));
        let treble = run(&f, tone(8000., 0.1, 1.));
        assert!(peak(&bass[24_000..]) > 0.2 && peak(&treble[24_000..]) < 0.12, "bass up, treble about the same");
        let f = AudioFilters { balance: -1., ..Default::default() };
        let left = run(&f, tone(440., 0.5, 0.1));
        assert!(left.chunks(2).all(|s| s[1] == 0.), "fully left");
        let f = AudioFilters { mono: true, ..Default::default() };
        let mut one_sided: Vec<f32> = tone(440., 0.5, 0.1).chunks(2).flat_map(|s| [s[0], 0.]).collect();
        Chain::default().process(&f, &mut one_sided);
        assert!(one_sided.chunks(2).all(|s| s[0] == s[1]), "both sides the same");
    }
    fn noise(amplitude: f32, seconds: f32) -> Vec<f32> {
        // A simple repeatable noise source, the same on both sides.
        let mut x = 12345u32;
        (0..(RATE * seconds) as usize).flat_map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let v = (x as f32 / u32::MAX as f32 * 2. - 1.) * amplitude;
            [v, v]
        }).collect()
    }
    fn rms(pcm: &[f32]) -> f32 {
        (pcm.iter().map(|v| v * v).sum::<f32>() / pcm.len().max(1) as f32).sqrt()
    }
    #[test]
    fn voice_isolation_lines_up_with_the_untouched_sound() {
        // Below RNNoise's silence threshold the network leaves the sound as it is, so the cleaned
        // and untouched paths must match sample for sample, a frame late.
        let input = noise(0.000_001, 0.5);
        for strength in [0., 0.5, 1.] {
            let f = AudioFilters { voice_isolation: true, isolation_strength: strength, ..Default::default() };
            let out = run(&f, input.clone());
            let late = &input[..input.len() - FRAME * 2];
            let err = out[FRAME * 2 * 2..].iter().zip(&late[FRAME * 2..]).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(err < 0.000_000_2, "strength {strength}: off by {err}");
        }
    }
    #[test]
    fn voice_isolation_removes_steady_noise() {
        let f = AudioFilters { voice_isolation: true, ..Default::default() };
        // Fan-like noise (treble rolled off); RNNoise is trained on real noise, not flat hiss.
        let mut lp = 0f32;
        let input: Vec<f32> = noise(0.2, 3.).chunks(2).flat_map(|s| { lp += (s[0] - lp) * 0.05; [lp, lp] }).collect();
        let out = run(&f, input.clone());
        let (before, after) = (rms(&input[96_000..]), rms(&out[96_000..]));
        assert!(after < before * db_to_gain(-30.), "noise only fell from {before} to {after}");
        let pitch = run(&f, tone(120., 0.05, 3.));
        assert!(rms(&pitch[96_000..]) > rms(&tone(120., 0.05, 3.)[96_000..]) * db_to_gain(-3.), "a voice-range pitch stays");
        let half = run(&AudioFilters { isolation_strength: 0.5, ..f }, input);
        assert!(rms(&half[96_000..]) > after * 1.5, "a lower strength keeps more of the original");
    }
    #[test]
    fn odd_sized_blocks_still_come_out_whole() {
        let f = AudioFilters { voice_isolation: true, ..Default::default() };
        let mut chain = Chain::default();
        let mut pcm = noise(0.05, 0.3);
        let len = pcm.len();
        for block in pcm.chunks_mut(2 * 300) {
            chain.process(&f, block);
        }
        assert_eq!(pcm.len(), len);
        assert!(pcm.iter().all(|v| v.is_finite()));
    }
    #[test]
    fn low_cut_removes_rumble_and_keeps_the_voice() {
        let f = AudioFilters { low_cut: true, low_cut_hz: 80., ..Default::default() };
        let rumble = run(&f, tone(30., 0.5, 1.));
        assert!(peak(&rumble[48_000..]) < 0.5 * db_to_gain(-24.), "30 Hz left at {}", peak(&rumble[48_000..]));
        let voice = run(&f, tone(300., 0.5, 1.));
        assert!((peak(&voice[48_000..]) - 0.5).abs() < 0.5 * (1. - db_to_gain(-0.5)), "300 Hz changed to {}", peak(&voice[48_000..]));
    }
}
