//! Rosorah's own sound filters, run on each strip's audio in place before mixing:
//! gate, expander, compressor, three-band EQ, mono/balance and limiter. Audio is 48 kHz
//! interleaved stereo; everything is a few multiplications per sample.
use auroracast_core::AudioFilters;

const RATE: f32 = 48_000.;

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
        let a0 = a_coeffs[0];
        Self { b: [b[0] / a0, b[1] / a0, b[2] / a0], a: [a_coeffs[1] / a0, a_coeffs[2] / a0], state: [[0.; 2]; 2] }
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
}
impl Chain {
    pub fn process(&mut self, f: &AudioFilters, pcm: &mut [f32]) {
        if !f.active() {
            return;
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
}
