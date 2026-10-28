//! Per-channel frequency-weighted power tracking and adaptive gain.
use crate::Parameters;

const DB: f64 = 0.11512925464970228;
const OFFSET: f32 = f32::from_bits(0x3f30e560);
#[inline]
fn rounded(x: f64, bias: f64) -> f64 {
    (((x + bias) as f32) as f64 - bias) as f32 as f64
}
#[inline]
fn smooth(old: f32, target: f64, pole: f64) -> f32 {
    ((old as f64 - target) * pole + target) as f32
}
#[inline]
fn even(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & !1)
}

#[derive(Clone, Copy, Default)]
struct Channel {
    hp1: f32,
    hp2: f32,
    shelf1: f32,
    shelf2: f32,
    fast: f32,
    held: f32,
    slow: f32,
    shelf_power: f32,
    crest: f32,
    crest_slow: f32,
    hold: u16,
    startup: u32,
}

pub struct Adaptive {
    channels: usize,
    rate: f64,
    params: Parameters,
    states: [Channel; 5],
    poles: [f64; 4],
    filter: [f32; 7],
    hold_limit: u16,
    dither: f32,
    delay: Vec<[f32; 6]>,
    pos: usize,
    started: bool,
    freeze: f32,
    max_gain: f32,
    fixed: f32,
    lfe_gain: f32,
    reference_power: f64,
    linear: [(f64, f64); 2],
    logarithmic: [(f64, f64); 2],
    log_mode: [bool; 2],
    fixed_mode: bool,
    power: f64,
    gain: f32,
}
impl Adaptive {
    pub fn new(rate: f64, channels: usize) -> Self {
        let rate = rate as f32 as f64;
        let hp = (128.8052987971815 / rate).tan();
        let hp_norm = 1.0 / (hp * hp + 1.0 + hp * 2.0);
        let k = (5283.593356660385 / rate).tan();
        let k2 = k * k;
        let q = k * std::f64::consts::SQRT_2;
        let norm = 1.0 / (q + (k2 + 1.0));
        let v = q * 1.2562344526401112;
        let filter = [
            (2.0 * (1.0 - hp * hp) * hp_norm) as f32,
            ((hp * hp + 1.0 - hp * 2.0) * hp_norm) as f32,
            (2.0 * (1.0 - k2) * norm) as f32,
            ((k2 + 1.0 - q) * norm) as f32,
            ((v + (k2 + 1.578125)) * norm) as f32,
            (2.0 * (1.578125 - k2) * norm) as f32,
            ((k2 + 1.578125 - v) * norm) as f32,
        ];
        let mut result = Self {
            channels,
            rate,
            params: Parameters::default(),
            states: [Channel::default(); 5],
            poles: [0.1, 0.5, 0.8, 8.0].map(|t| rounded((-1.0 / (rate * t)).exp(), 1.0)),
            filter,
            hold_limit: (rate * 0.12 + 0.49999999) as u16,
            dither: f32::from_bits(0x2f800000),
            delay: vec![[0.0; 6]; (rate * 0.034 + 0.49999999) as usize],
            pos: 0,
            started: false,
            freeze: 0.0,
            max_gain: 1.0,
            fixed: 1.0,
            lfe_gain: 1.0,
            reference_power: 1.0,
            linear: [(0.0, 0.0); 2],
            logarithmic: [(0.0, 0.0); 2],
            log_mode: [false; 2],
            fixed_mode: false,
            power: 0.0,
            gain: 1.0,
        };
        result.configure();
        result.reset();
        result
    }
    pub fn delay_samples(&self) -> usize {
        self.delay.len()
    }
    pub fn measured_power(&self) -> f64 {
        self.power
    }
    pub fn gain(&self) -> f32 {
        self.gain
    }
    pub fn set_parameters(&mut self, p: Parameters) {
        if p == self.params {
            return;
        }
        let gain_changed = p.target_lufs != self.params.target_lufs
            || p.input_lufs != self.params.input_lufs
            || p.max_gain_db != self.params.max_gain_db
            || p.correction_high != self.params.correction_high
            || p.correction_low != self.params.correction_low
            || p.mix_mode != self.params.mix_mode;
        self.params = p;
        self.configure();
        if gain_changed && !self.started {
            self.reset_envelopes()
        }
    }
    fn configure(&mut self) {
        let p = self.params;
        let target = (p.target_lufs as f32 + OFFSET) as f64;
        let input = (p.input_lufs as f32 + OFFSET) as f64;
        let target_amp = rounded((target * DB).exp(), 1.0);
        let input_amp = (input * DB).exp();
        let inv = rounded(1.0 / input_amp, 1.0);
        self.reference_power = (input_amp * input_amp) as f32 as f64;
        self.max_gain = rounded(((p.max_gain_db as f32 as f64) * DB).exp(), 16.0) as f32;
        self.lfe_gain = rounded(((p.lfe_gain_db as f32 as f64) * DB).exp(), 16.0) as f32;
        let n = if self.channels == 6 {
            f32::from_bits(0x3e4ccccd)
        } else {
            0.5f32
        };
        let freeze_db = (p.freeze_lufs as f32 + OFFSET) * 2.0;
        self.freeze = (((freeze_db as f64) * DB).exp() * n as f64) as f32;

        let amounts = [p.correction_high, p.correction_low].map(|a| {
            let x = ((a * 100.0) as f32) * f32::from_bits(0x3c23d70a);
            (((x + 16.0) as f64 - 16.0) as f32 as f64).clamp(0.0, 1.0)
        });
        self.fixed_mode = amounts[0] <= 1e-8 && amounts[1] <= 1e-8;
        self.log_mode = [
            p.mix_mode & 2 != 0 && amounts[0] < 1.0,
            p.mix_mode & 1 != 0 && amounts[1] < 1.0,
        ];
        let base = target_amp * inv;
        self.fixed = (rounded(base, 4.0) as f32).min(self.max_gain);
        for (i, &a) in amounts.iter().enumerate() {
            self.linear[i] = (rounded(target_amp * a, 1.0), rounded((1.0 - a) * base, 1.0));
            let target_minus_input = (target as f32 - input as f32) as f64 * DB;
            self.logarithmic[i] = (
                ((1.0 - a) * target_minus_input + (a * DB) * target) as f32 as f64,
                (a * -0.5) as f32 as f64,
            );
        }
    }
    fn reset_envelopes(&mut self) {
        let n = if self.channels == 6 {
            f32::from_bits(0x3e4ccccd)
        } else {
            0.5f32
        };
        let (a, b) = self.linear[1];
        let (log_a, log_b) = self.logarithmic[1];
        let initial = if self.log_mode[1] {
            if log_b < 0.0 {
                ((((self.max_gain as f64).ln() - log_a) / log_b).exp() * n as f64) as f32
            } else {
                f32::from_bits(0x2f800000)
            }
        } else {
            let x = (a / (self.max_gain as f64 - b).max(1e-8)) as f32;
            (x * x) * n
        };
        for s in &mut self.states {
            s.fast = initial;
            s.held = initial;
            s.slow = initial;
            s.shelf_power = initial;
            s.crest = 1.0;
            s.crest_slow = 1.0;
            s.startup = self.rate as u32;
        }
        self.dither = f32::from_bits(0x2f800000);
    }
    pub fn reset(&mut self) {
        self.states = [Channel::default(); 5];
        self.started = false;
        self.reset_envelopes();
        self.delay.fill([0.0; 6]);
        self.pos = 0;
        self.power = 0.0;
        self.gain = self.fixed;
    }
    fn detect(&mut self, index: usize, input: f32) -> f32 {
        let s = &mut self.states[index];
        let [ha, hb, sa, sb, b0, b1, b2] = self.filter;
        // f32 operations follow scalar SSE order, including the alternating tiny offset.
        let h = ((ha * s.hp1 + self.dither) - hb * s.hp2) + input;
        let high = (s.hp2 - (s.hp1 + s.hp1)) + h;
        s.hp2 = s.hp1;
        s.hp1 = h;
        let shelf = (sa * s.shelf1 - sb * s.shelf2) + high;
        let y = (s.shelf2 * b2 - b1 * s.shelf1) + b0 * shelf;
        s.shelf2 = s.shelf1;
        s.shelf1 = shelf;
        let shelf_square = y * y;
        let delta = (high * high - shelf_square) * 0.25f32;
        let fast_input = delta + shelf_square;
        let peak_input = (delta + delta) + fast_input;
        let [p100, p500, p800, p8] = self.poles;
        if fast_input as f64 > 1e-7 {
            s.fast = smooth(s.fast, fast_input as f64, p100)
        }
        // The trough gate uses the previous slow envelope. The native sample
        // loop saves this quarter value before updating the slow follower.
        let slow_quarter = s.slow * 0.25;
        s.slow = smooth(s.slow, peak_input as f64, p500);
        let mut unconditional_crest = false;
        if peak_input > s.held {
            if s.fast > self.freeze || s.fast > s.held || s.slow > self.freeze {
                s.held = smooth(s.held, peak_input as f64, p500)
            }
            s.hold = 1;
        } else if s.hold > self.hold_limit {
            if s.fast > self.freeze {
                let target = peak_input as f64 * 1.5848931924611134;
                if s.held as f64 > target {
                    s.held = smooth(s.held, target, p500)
                }
                unconditional_crest = true;
            }
        } else {
            if peak_input < slow_quarter {
                s.hold = s.hold.wrapping_add(1)
            } else if s.fast > self.freeze {
                s.held = smooth(s.held, peak_input as f64, p500);
                unconditional_crest = true;
            }
        }
        let power = s.fast.max(s.held);
        if unconditional_crest || s.slow > self.freeze || s.fast > self.freeze {
            s.shelf_power = smooth(s.shelf_power, shelf_square as f64, p800);
            let mut ratio = power / s.shelf_power;
            if s.startup != 0 {
                if s.shelf_power > f32::from_bits(0x38d1b717) {
                    ratio = ratio.min(2.5);
                    if ratio > s.crest_slow {
                        s.crest_slow = smooth(s.crest_slow, ratio as f64, p500);
                        s.crest = s.crest_slow;
                    }
                    if s.crest_slow > std::f32::consts::SQRT_2 || ratio < 1.0 {
                        s.startup = 0
                    } else {
                        s.startup -= 1
                    }
                }
            } else {
                let target = if ratio > 0.875 { ratio as f64 } else { 0.875 };
                s.crest = smooth(s.crest, target, p8).min(2.0);
                s.crest_slow = smooth(s.crest_slow, s.crest as f64, p800);
            }
        }
        power / s.crest_slow
    }

    /// 34 ms delayed sample multiplied by the newly computed shared gain.
    /// 5.1 order: L, R, C, LFE, Ls, Rs. LFE is excluded from detection.
    pub fn process_frame(&mut self, frame: &mut [f32]) -> f32 {
        debug_assert_eq!(frame.len(), self.channels);
        let power = if self.channels == 1 {
            2.0 * self.detect(0, frame[0]) as f64
        } else if self.channels == 2 {
            let left = self.detect(0, frame[0]) as f64;
            left + self.detect(1, frame[1]) as f64
        } else {
            let l = self.detect(0, frame[0]) as f64;
            let r = self.detect(1, frame[1]) as f64;
            let c = self.detect(2, frame[2]) as f64;
            let ls = self.detect(3, frame[4]) as f64;
            let rs = self.detect(4, frame[5]) as f64;
            (ls + rs) * 1.4140625 + ((l + r) + c)
        };
        self.power = power;
        self.dither = -self.dither;
        let gain = if self.fixed_mode {
            self.fixed
        } else {
            let i = if power > self.reference_power { 0 } else { 1 };
            let g = if self.log_mode[i] {
                let (a, b) = self.logarithmic[i];
                (power.ln() * b + a).exp()
            } else {
                let (a, b) = self.linear[i];
                if a == 0.0 { b } else { a / power.sqrt() + b }
            };
            let g = g as f32;
            if g > self.max_gain {
                self.max_gain
            } else {
                even(g)
            }
        };
        let old = self.delay[self.pos];
        for i in 0..self.channels {
            self.delay[self.pos][i] = frame[i];
            frame[i] = old[i]
                * if self.channels == 6 && i == 3 {
                    self.lfe_gain
                } else {
                    gain
                }
        }
        self.pos += 1;
        if self.pos == self.delay.len() {
            self.pos = 0
        };
        self.started = true;
        self.gain = gain;
        gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trough_gate_uses_previous_slow_envelope() {
        let mut a = Adaptive::new(48000.0, 6);
        a.set_parameters(Parameters {
            max_gain_db: 20.0,
            true_peak_db: -1.0,
            ..Parameters::default()
        });
        for t in 0..=22757 {
            let mut frame = [0.0; 6];
            for (c, x) in frame.iter_mut().enumerate() {
                let value = 0.03
                    * (std::f64::consts::TAU * (997 + 83 * c) as f64 * t as f64 / 48000.0).sin();
                *x = (value * 32767.0).round_ties_even() as i16 as f32 / 32768.0;
            }
            a.process_frame(&mut frame);
        }

        assert_eq!(a.states[4].held.to_bits(), 0x39b03e75);
        assert_eq!(a.states[4].hold, 2);
    }
    #[test]
    fn trough_increments_hold_without_releasing_held_envelope() {
        let mut a = Adaptive::new(48000.0, 2);
        a.set_parameters(Parameters {
            true_peak_db: -1.0,
            max_gain_db: 20.0,
            ..Parameters::default()
        });
        for t in 0..=1727 {
            let value =
                (0.05 * (std::f64::consts::TAU * 1000.0 * t as f64 / 48000.0).sin() * 32767.0)
                    .round_ties_even() as i16 as f32
                    / 32768.0;
            let mut frame = [value, value];
            a.process_frame(&mut frame);
        }

        assert_eq!(a.states[0].held.to_bits(), 0x3918b906);
        assert_eq!(a.states[1].held.to_bits(), 0x3918b906);
        assert_eq!(a.states[0].hold, 2);
        assert_eq!(a.states[1].hold, 2);
    }
}
