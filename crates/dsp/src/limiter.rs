//! Four-phase peak detection and linked lookahead gain control.

const STATE_BASE: u64 = 0x1000;
const INPUT: u64 = 0x2000;
const OUTPUT: u64 = 0x3000;
const SCRATCH_BASE: u64 = 0x4000;
const ENV: u64 = 0x100000;
const PEAK: u64 = 0x200000;
const AUDIO: u64 = 0x300000;
struct LimiterMemory {
    state: [u8; 0x460],
    scratch: [u8; 256],
    envelopes: Vec<f32>,
    peaks: Vec<f32>,
    audio: Vec<f32>,
    input: f32,
    output: f32,
    gain: f32,
}
impl LimiterMemory {
    fn get(&self, a: u64, w: usize) -> u64 {
        if a == INPUT {
            return self.input.to_bits() as u64;
        }
        if a == OUTPUT {
            return self.output.to_bits() as u64;
        }
        if a >= AUDIO {
            return self.audio[((a - AUDIO) / 4) as usize].to_bits() as u64;
        }
        if a >= PEAK {
            return self.peaks[((a - PEAK) / 4) as usize].to_bits() as u64;
        }
        if a >= ENV {
            return self.envelopes[((a - ENV) / 4) as usize].to_bits() as u64;
        }
        let (b, o) = if a >= SCRATCH_BASE {
            (&self.scratch[..], (a - SCRATCH_BASE) as usize)
        } else {
            (&self.state[..], (a - STATE_BASE) as usize)
        };
        let mut v = 0;
        for i in 0..w / 8 {
            v |= (b[o + i] as u64) << (i * 8)
        }
        v
    }
    fn put(&mut self, a: u64, w: usize, v: u64) {
        if a == INPUT {
            self.input = f32::from_bits(v as u32);
            return;
        }
        if a == OUTPUT {
            self.output = f32::from_bits(v as u32);
            return;
        }
        if a >= AUDIO {
            self.audio[((a - AUDIO) / 4) as usize] = f32::from_bits(v as u32);
            return;
        }
        if a >= PEAK {
            self.peaks[((a - PEAK) / 4) as usize] = f32::from_bits(v as u32);
            return;
        }
        if a >= ENV {
            self.envelopes[((a - ENV) / 4) as usize] = f32::from_bits(v as u32);
            return;
        }
        let (b, o) = if a >= SCRATCH_BASE {
            (&mut self.scratch[..], (a - SCRATCH_BASE) as usize)
        } else {
            (&mut self.state[..], (a - STATE_BASE) as usize)
        };
        for i in 0..w / 8 {
            b[o + i] = (v >> (i * 8)) as u8
        }
    }
    fn f(&mut self, o: u64, v: f32) {
        self.put(STATE_BASE + o, 32, v.to_bits() as u64)
    }
    fn d(&mut self, o: u64, v: f64) {
        self.put(STATE_BASE + o, 64, v.to_bits())
    }
    fn n(&mut self, o: u64, v: usize) {
        self.put(STATE_BASE + o, 16, v as u64)
    }
}

pub struct PeakLimiter {
    memory: LimiterMemory,
    fir: Vec<[f32; 12]>,
    delay: Vec<Vec<f32>>,
    fir_index: usize,
    surround_fir: bool,
    delay_index: usize,
    rate: f64,
    peak_db: f64,
}
impl PeakLimiter {
    pub fn new(rate: f64, channels: usize, true_peak_db: f64) -> Self {
        let rate = (rate as f32) as f64;
        let rate = if rate < 10. {
            48000.
        } else {
            rate.clamp(8000., 384000.)
        };
        let n = ((rate as f32) * f32::from_bits(0x3aa3d70a)).round_ties_even() as usize;
        let mut s = Self {
            memory: LimiterMemory {
                state: [0; 0x460],
                scratch: [0; 256],
                envelopes: vec![0.; n],
                peaks: vec![0.; n],
                audio: vec![0.; n],
                input: 0.,
                output: 0.,
                gain: 1.,
            },
            fir: vec![[0.; 12]; channels.max(1)],
            delay: vec![vec![0.; n]; channels.max(1)],
            fir_index: 0,
            surround_fir: channels == 5,
            delay_index: 0,
            rate,
            peak_db: true_peak_db,
        };
        s.reset();
        s
    }
    pub fn use_surround_fir(&mut self) {
        self.surround_fir = true;
    }
    pub fn reduction_db(&self) -> f32 {
        -20. * self.memory.gain.max(f32::MIN_POSITIVE).log10()
    }
    pub fn latency_samples(&self) -> usize {
        self.memory.peaks.len() + 6
    }
    pub fn reset(&mut self) {
        self.memory.state.fill(0);
        self.memory.scratch.fill(0);
        self.memory.envelopes.fill(0.);
        self.memory.peaks.fill(0.);
        self.memory.audio.fill(0.);
        self.fir.fill([0.; 12]);
        for d in &mut self.delay {
            d.fill(0.)
        }
        self.fir_index = 0;
        self.delay_index = 0;
        let m = &mut self.memory;
        let n = m.peaks.len();
        m.put(STATE_BASE, 64, ENV);
        m.put(STATE_BASE + 0x18, 64, PEAK);
        m.put(STATE_BASE + 0x30, 64, AUDIO);
        m.n(0x3ae, n);
        m.n(0x3b0, n - 2);
        let hold = ((self.rate as f32) * f32::from_bits(0x3b06594b)).round_ties_even() as usize;
        m.n(0x3b4, hold);
        m.n(0x3b8, 6 * hold);
        m.put(STATE_BASE + 0x3bc, 8, 200);
        m.d(
            0x400,
            ((1043359.2039800996 / n as f64) + 0.49999999999999994).trunc() * 9.5367431640625e-7,
        );
        for (o, e) in [(0x408, 100.65), (0x418, 0.70535), (0x410, 7.0535)] {
            let p = 0.01f64.powf(e / self.rate);
            let p = ((p + 1.) as f32) as f64 - 1.;
            m.d(o, (p as f32) as f64);
        }
        m.d(0x420, 1.52587890625e-5);
        m.f(0x428, -1.);
        m.f(0x42c, -1.);
        m.f(0x430, -1.);
        for o in [0x3d4, 0x3d8, 0x3e8] {
            m.f(o, 1.)
        }
        m.f(0x3dc, 32.);
        m.f(0x3e0, 32.);
        m.f(0x43c, 32.);
        m.f(0x1dc, -1.);
        m.put(STATE_BASE + 0x3f0, 64, STATE_BASE + 0x3c0);
        self.set_peak(self.peak_db);
    }
    pub fn set_peak(&mut self, db: f64) {
        self.peak_db = db;
        let amplitude = (((db as f32) as f64) * 0.11512925464970228).exp();
        let base = (31.999996185302734 * amplitude + 256.) as f32 - 256.;
        let m = &mut self.memory;
        if m.get(STATE_BASE + 0x3b6, 16) != 0
            && f32::from_bits(m.get(STATE_BASE + 0x1c8, 32) as u32) == base
        {
            return;
        }
        m.f(0x1c8, base);
        m.f(0x1cc, base);
        let floor = ((base as f64) * 0.4556253375000625) as f32;
        m.f(0x1d0, floor);
        m.f(0x1d8, ((base as f64) * 0.953125 + 256.) as f32 - 256.);
        if m.get(STATE_BASE + 0x3b6, 16) == 0 {
            for o in [
                0x3c0, 0x3c4, 0x3c8, 0x3cc, 0x3d0, 0x3e4, 0x3f8, 0x440, 0x444,
            ] {
                m.f(o, base)
            }
            m.f(0x1d4, floor);
            m.f(0x434, base * f32::from_bits(0x3d257a93));
            m.f(0x438, base * f32::from_bits(0x3d257a93));
        } else if floor > f32::from_bits(m.get(STATE_BASE + 0x1d4, 32) as u32) {
            m.f(0x1d4, floor);
        }
        let exponent = ((amplitude.to_bits() >> 52) & 0x7ff) as i32 - 1022;
        let mantissa = amplitude / 2.0f64.powi(exponent);
        let mut number = (mantissa * 4194303.5).trunc() as i64;
        if number & 1 == 0 {
            number -= 1
        }
        let denominator = (1u32).wrapping_shl((17 - exponent) as u32) as f32;
        let numerator = (number as f32) * (1. / denominator);
        m.f(0x1e4, numerator.min(f32::from_bits(0x41ff4004)));
    }
    pub fn process_frame(&mut self, frame: &mut [f32]) {
        let idx = self.fir_index;
        let old = (idx + 6) % 12;
        let mut peak = 0f32;
        for (ch, &sample) in frame.iter().enumerate().take(self.fir.len()) {
            self.fir[ch][idx] = sample;
            let mut phase = [0f32; 4];
            for lane in 0..4 {
                let mut p = [0f32; 12];
                for tap in 0..12 {
                    p[tap] = self.fir[ch][(idx + 12 - tap) % 12] * f32::from_bits(FIR[tap][lane]);
                }
                phase[lane] = if self.surround_fir {
                    surround_fir_sum(&p)
                } else if self.fir.len() == 2 {
                    stereo_fir_sum(idx, &p)
                } else {
                    fir_sum(idx, &p)
                };
            }
            for v in phase {
                peak = peak.max(v.abs())
            }
            peak = peak.max(self.fir[ch][old].abs());
        }
        self.memory.input = 0.;
        self.memory.output = peak;
        let gain = update_gain(&mut self.memory);
        for (ch, sample) in frame.iter_mut().enumerate().take(self.fir.len()) {
            let delayed = self.delay[ch][self.delay_index];
            self.delay[ch][self.delay_index] = self.fir[ch][old];
            *sample = delayed * gain;
        }
        self.fir_index = (idx + 1) % 12;
        self.delay_index = (self.delay_index + 1) % self.memory.peaks.len();
    }
}

const FIR: [[u32; 4]; 12] = [
    [0xbcb4c500, 0xbd039800, 0xbceee000, 0xbc87ec00],
    [0x3d2c4440, 0x3d77b240, 0x3d5967a0, 0x3ce979c0],
    [0xbd940f10, 0xbddb8910, 0xbdc64740, 0xbd5a16c0],
    [0x3de04bb0, 0x3e2921f0, 0x3e1c3a10, 0x3db077f0],
    [0xbe364730, 0xbe83641c, 0xbe6ca0d8, 0xbe043be8],
    [0x3f740844, 0x3f497b0c, 0x3f08e126, 0x3e80f1a8],
    [0x3e80f1a8, 0x3f08e126, 0x3f497b0c, 0x3f740844],
    [0xbe043be8, 0xbe6ca0d8, 0xbe83641c, 0xbe364730],
    [0x3db077f0, 0x3e1c3a10, 0x3e2921f0, 0x3de04bb0],
    [0xbd5a16c0, 0xbdc64740, 0xbddb8910, 0xbd940f10],
    [0x3ce979c0, 0x3d5967a0, 0x3d77b240, 0x3d2c4440],
    [0xbc87ec00, 0xbceee000, 0xbd039800, 0xbcb4c500],
];

#[allow(
    unused_assignments,
    unused_variables,
    unused_parens,
    unused_comparisons
)]
#[allow(
    clippy::double_parens,
    clippy::eq_op,
    clippy::absurd_extreme_comparisons,
    clippy::unnecessary_cast
)]
fn update_gain(m: &mut LimiterMemory) -> f32 {
    let mut words = [0u64; 16];
    let mut scalars = [0u64; 16];
    words[7] = STATE_BASE;
    words[6] = INPUT;
    words[2] = OUTPUT;
    words[1] = 1;
    words[4] = SCRATCH_BASE + 128;
    let (mut eq, mut lt, mut le) = (false, false, false);
    let mut step = 0u32;
    loop {
        match step {
            0 => {
                eq = ((words[1] & 0xffffffff) & (words[1] & 0xffffffff)) == 0;
                lt = false;
                le = eq;

                if le {
                    step = 168;
                    continue;
                }
                step = 1;
                continue;
            }
            1 => {
                words[13] = m.get(words[7].wrapping_add(0x18u64), 64);

                words[14] = m.get(words[7].wrapping_add(0x30u64), 64);

                words[9] = (m.get(words[7].wrapping_add(0x3b2u64), 16)) & 0xffffffff;

                words[0] = (m.get(words[7].wrapping_add(0x3aeu64), 16)) & 0xffffffff;

                m.put(
                    words[4].wrapping_add((-0x14i64 as u64)),
                    16,
                    words[0] & 0xffff,
                );

                words[8] = (m.get(words[7].wrapping_add(0x3b6u64), 16)) & 0xffffffff;

                words[3] = (m.get(words[7].wrapping_add(0x3b4u64), 16)) & 0xffffffff;

                m.put(
                    words[4].wrapping_add((-0x8i64 as u64)),
                    16,
                    words[3] & 0xffff,
                );

                words[10] = (m.get(words[7].wrapping_add(0x3bau64), 16)) & 0xffffffff;

                words[3] = (m.get(words[7].wrapping_add(0x3b8u64), 16)) & 0xffffffff;

                m.put(
                    words[4].wrapping_add((-0x6i64 as u64)),
                    16,
                    words[3] & 0xffff,
                );

                words[12] = m.get(words[7], 64);

                words[3] = (m.get(words[7].wrapping_add(0x3b0u64), 16)) & 0xffffffff;

                m.put(
                    words[4].wrapping_add((-0x12i64 as u64)),
                    16,
                    words[3] & 0xffff,
                );

                words[0] = (words[0].wrapping_add(4u64.wrapping_mul(words[0]))) & 0xffffffff;

                words[0] = ((words[0] & 0xffffffff).wrapping_shl((0x5u64) as u32)) & 0xffffffff;

                m.put(
                    words[4].wrapping_add((-0x4i64 as u64)),
                    32,
                    words[0] & 0xffffffff,
                );

                words[0] = (words[1].wrapping_add((-0x1i64 as u64))) & 0xffffffff;

                words[0] = words[2].wrapping_add(4u64.wrapping_mul(words[0]).wrapping_add(0x4u64));

                m.put(words[4].wrapping_add((-0x10i64 as u64)), 64, words[0]);

                scalars[9] = (0x3f800000u64) & 0xffffffff;

                scalars[11] = (0x41fffffeu64) & 0xffffffff;

                scalars[8] = (0x42000000u64) & 0xffffffff;

                scalars[10] = 0;

                words[15] = (0x3f802000u64) & 0xffffffff;

                step = 35;
                continue;
            }
            2 => {
                scalars[0] = (m.get(words[7].wrapping_add(0x1c8u64), 32)) & 0xffffffff;

                m.put(words[2], 32, (scalars[0]) & 0xffffffff);

                m.put(
                    words[7].wrapping_add(0x43cu64),
                    32,
                    (scalars[8]) & 0xffffffff,
                );

                scalars[2] = (m.get(words[2], 32)) & 0xffffffff;

                scalars[7] = scalars[8];

                scalars[4] = scalars[10];
                step = 3;
                continue;
            }
            3 => {
                words[0] = (m.get(words[7].wrapping_add(0x454u64), 16)) & 0xffffffff;

                words[1] = words[0];

                words[0] = (words[0]).wrapping_shl((0x2u64) as u32);

                words[11] = words[13].wrapping_add(words[0]);

                scalars[6] = (m.get(words[11], 32)) & 0xffffffff;

                words[3] = words[14].wrapping_add(words[0]);

                scalars[3] = (m.get(words[3], 32)) & 0xffffffff;

                words[9] = ((words[9] & 0xffffffff).wrapping_add((0x1u64))) & 0xffffffff;

                scalars[5] = (m.get(words[7].wrapping_add(0x3f8u64), 32)) & 0xffffffff;

                eq = (words[9] & 0xffff) == (m.get(words[4].wrapping_add((-0x14i64 as u64)), 16));
                lt = (words[9] & 0xffff) < (m.get(words[4].wrapping_add((-0x14i64 as u64)), 16));
                le = lt || eq;

                if !le {
                    step = 37;
                    continue;
                }
                step = 4;
                continue;
            }
            4 => {
                words[5] = m.get(words[7].wrapping_add(0x3f0u64), 64);

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(m.get(words[5], 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 37;
                    continue;
                }
                step = 5;
                continue;
            }
            5 => {
                scalars[1] = (m.get(words[7].wrapping_add(0x3d0u64), 32)) & 0xffffffff;

                scalars[0] = scalars[7];
                step = 6;
                continue;
            }
            6 => {
                words[8] = ((words[8] & 0xffffffff).wrapping_add((0x1u64))) & 0xffffffff;

                eq = (words[8] & 0xffff) == (m.get(words[4].wrapping_add((-0x8i64 as u64)), 16));
                lt = (words[8] & 0xffff) < (m.get(words[4].wrapping_add((-0x8i64 as u64)), 16));
                le = lt || eq;

                if !le {
                    step = 8;
                    continue;
                }
                step = 7;
                continue;
            }
            7 => {
                scalars[5] = (m.get(words[7].wrapping_add(0x3c4u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[2] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 11;
                    continue;
                }
                step = 8;
                continue;
            }
            8 => {
                m.put(
                    words[7].wrapping_add(0x3c4u64),
                    32,
                    (scalars[2]) & 0xffffffff,
                );

                scalars[5] = (m.get(words[7].wrapping_add(0x1d4u64), 32)) & 0xffffffff;

                scalars[5] =
                    (if f32::from_bits(scalars[5] as u32) > f32::from_bits(scalars[2] as u32) {
                        f32::from_bits(scalars[5] as u32)
                    } else {
                        f32::from_bits(scalars[2] as u32)
                    })
                    .to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x3ccu64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                scalars[7] = (m.get(words[7].wrapping_add(0x3c8u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 67;
                    continue;
                }
                step = 9;
                continue;
            }
            9 => {
                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x440u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 67;
                    continue;
                }
                step = 10;
                continue;
            }
            10 => {
                m.put(
                    words[7].wrapping_add(0x3ccu64),
                    32,
                    (scalars[7]) & 0xffffffff,
                );

                scalars[5] = scalars[2];

                words[8] = (0x1u64) & 0xffffffff;
                step = 11;
                continue;
            }
            11 => {
                words[10] = ((words[10] & 0xffffffff).wrapping_add((0x1u64))) & 0xffffffff;

                eq = (words[10] & 0xffff) == (m.get(words[4].wrapping_add((-0x6i64 as u64)), 16));
                lt = (words[10] & 0xffff) < (m.get(words[4].wrapping_add((-0x6i64 as u64)), 16));
                le = lt || eq;

                if !le {
                    step = 13;
                    continue;
                }
                step = 12;
                continue;
            }
            12 => {
                {
                    let a = f32::from_bits(scalars[2] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3c8u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if lt {
                    step = 14;
                    continue;
                }
                step = 13;
                continue;
            }
            13 => {
                m.put(
                    words[7].wrapping_add(0x3c8u64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                words[10] = (words[8] & 0xffffffff) & 0xffffffff;
                step = 14;
                continue;
            }
            14 => {
                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[0] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 68;
                    continue;
                }
                step = 15;
                continue;
            }
            15 => {
                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x428u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !eq {
                    step = 74;
                    continue;
                }
                step = 16;
                continue;
            }
            16 => {
                scalars[0] = (f32::from_bits(scalars[0] as u32) as f64).to_bits() as u64;

                scalars[7] = m.get(words[7].wrapping_add(0x420u64), 64);
                step = 17;
                continue;
            }
            17 => {
                scalars[0] =
                    (f64::from_bits(scalars[0]) + f64::from_bits(scalars[7])).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x43cu64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                {
                    let a = f32::from_bits(scalars[0] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if lt {
                    step = 19;
                    continue;
                }
                step = 18;
                continue;
            }
            18 => {
                m.put(
                    words[7].wrapping_add(0x3e0u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                m.put(
                    words[7].wrapping_add(0x43cu64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                scalars[7] = (f64::from_bits(scalars[7]) * f64::from_bits(0x3fe7f254dab9cc3bu64))
                    .to_bits() as u64;

                scalars[7] = (f64::from_bits(scalars[7]) as f32).to_bits() as u64;

                scalars[7] = (f32::from_bits(scalars[7] as u32) as f64).to_bits() as u64;

                m.put(words[7].wrapping_add(0x420u64), 64, scalars[7]);

                m.put(words[7].wrapping_add(0x428u64), 32, 0xbf800000u64);

                scalars[0] = scalars[1];
                step = 19;
                continue;
            }
            19 => {
                m.put(words[7].wrapping_add(0x448u64), 8, 0x0u64);

                m.put(words[7].wrapping_add(0x45au64), 8, 0x1u64);

                words[5] = words[7].wrapping_add(0x3c0u64);

                m.put(words[7].wrapping_add(0x3f0u64), 64, words[5]);
                step = 20;
                continue;
            }
            20 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[0] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 28;
                    continue;
                }
                step = 21;
                continue;
            }
            21 => {
                m.put(
                    words[7].wrapping_add(0x43cu64),
                    32,
                    (scalars[6]) & 0xffffffff,
                );

                m.put(words[7].wrapping_add(0x448u64), 8, 0x0u64);

                m.put(words[7].wrapping_add(0x45au64), 8, 0x1u64);

                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 23;
                    continue;
                }
                step = 22;
                continue;
            }
            22 => {
                m.put(
                    words[7].wrapping_add(0x3d0u64),
                    32,
                    (scalars[6]) & 0xffffffff,
                );
                step = 23;
                continue;
            }
            23 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3e0u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 25;
                    continue;
                }
                step = 24;
                continue;
            }
            24 => {
                m.put(
                    words[7].wrapping_add(0x3e0u64),
                    32,
                    (scalars[6]) & 0xffffffff,
                );
                step = 25;
                continue;
            }
            25 => {
                eq = ((words[9] & 0xffff) & (words[9] & 0xffff)) == 0;
                lt = false;
                le = eq;

                if !eq {
                    step = 28;
                    continue;
                }
                step = 26;
                continue;
            }
            26 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3e4u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 28;
                    continue;
                }
                step = 27;
                continue;
            }
            27 => {
                m.put(
                    words[7].wrapping_add(0x3e4u64),
                    32,
                    (scalars[6]) & 0xffffffff,
                );
                step = 28;
                continue;
            }
            28 => {
                m.put(
                    words[7].wrapping_add(0x3f8u64),
                    32,
                    (scalars[2]) & 0xffffffff,
                );

                m.put(words[11], 32, (scalars[2]) & 0xffffffff);

                scalars[0] = (m.get(words[7].wrapping_add(0x3ccu64), 32)) & 0xffffffff;

                scalars[5] = (m.get(words[7].wrapping_add(0x440u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[0] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 72;
                    continue;
                }
                step = 29;
                continue;
            }
            29 => {
                {
                    let a = f32::from_bits(scalars[2] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x438u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 88;
                    continue;
                }
                step = 30;
                continue;
            }
            30 => {
                scalars[5] = (f32::from_bits(scalars[5] as u32) as f64).to_bits() as u64;

                scalars[5] = (f64::from_bits(scalars[5]) * f64::from_bits(0x3ff000001ad7f29bu64))
                    .to_bits() as u64;

                scalars[0] = 0;

                scalars[0] = (f64::from_bits(scalars[5]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x440u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                words[11] = (m.get(words[7].wrapping_add(0x44cu64), 32)) & 0xffffffff;

                eq = ((words[11] & 0xffffffff) & (words[11] & 0xffffffff)) == 0;
                lt = false;
                le = eq;

                if eq {
                    step = 32;
                    continue;
                }
                step = 31;
                continue;
            }
            31 => {
                words[11] = ((words[11] & 0xffffffff).wrapping_sub((0x1u64))) & 0xffffffff;

                m.put(words[7].wrapping_add(0x44cu64), 32, words[11] & 0xffffffff);
                step = 32;
                continue;
            }
            32 => {
                m.put(
                    words[12].wrapping_add(words[0]),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                m.put(words[3], 32, (scalars[4]) & 0xffffffff);

                eq = (words[1] & 0xffff) == (m.get(words[4].wrapping_add((-0x12i64 as u64)), 16));
                lt = (words[1] & 0xffff) < (m.get(words[4].wrapping_add((-0x12i64 as u64)), 16));
                le = lt || eq;

                if !le {
                    step = 90;
                    continue;
                }
                step = 33;
                continue;
            }
            33 => {
                words[1] = ((words[1] & 0xffffffff).wrapping_add((0x1u64))) & 0xffffffff;

                m.put(words[7].wrapping_add(0x454u64), 16, words[1] & 0xffff);
                step = 34;
                continue;
            }
            34 => {
                scalars[0] = scalars[9];

                scalars[0] = (f32::from_bits(scalars[0] as u32)
                    / f32::from_bits(m.get(words[7].wrapping_add(0x43cu64), 32) as u32))
                .to_bits() as u64;

                scalars[0] = (f32::from_bits(scalars[0] as u32)
                    * f32::from_bits(m.get(words[7].wrapping_add(0x1e4u64), 32) as u32))
                .to_bits() as u64;

                words[0] = ((scalars[0]) & 0xffffffff) & 0xffffffff;

                words[0] = ((words[0] & 0xffffffff) & (-0x2i64 as u64)) & 0xffffffff;

                scalars[4] = (words[0] & 0xffffffff) & 0xffffffff;
                m.gain = f32::from_bits(scalars[4] as u32);

                scalars[3] = (f32::from_bits(scalars[3] as u32) * f32::from_bits(scalars[4] as u32))
                    .to_bits() as u64;

                m.put(words[2], 32, (scalars[3]) & 0xffffffff);

                words[2] = (words[2]).wrapping_add((0x4u64));

                words[6] = (words[6]).wrapping_add((0x4u64));

                eq = (m.get(words[4].wrapping_add((-0x10i64 as u64)), 64)) == (words[2]);
                lt = (m.get(words[4].wrapping_add((-0x10i64 as u64)), 64)) < (words[2]);
                le = lt || eq;

                if eq {
                    step = 167;
                    continue;
                }
                step = 35;
                continue;
            }
            35 => {
                scalars[2] = (m.get(words[2], 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[2] as u32);
                    let b = f32::from_bits(0x7cf0bdc2u64 as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 2;
                    continue;
                }
                step = 36;
                continue;
            }
            36 => {
                scalars[4] = (m.get(words[6], 32)) & 0xffffffff;

                scalars[2] = (f32::from_bits(scalars[2] as u32)
                    * f32::from_bits(scalars[11] as u32))
                .to_bits() as u64;

                m.put(words[2], 32, (scalars[2]) & 0xffffffff);

                scalars[7] = (m.get(words[7].wrapping_add(0x43cu64), 32)) & 0xffffffff;

                step = 3;
                continue;
            }
            37 => {
                m.put(
                    words[7].wrapping_add(0x3c0u64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                scalars[12] = (m.get(words[7].wrapping_add(0x3d0u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(scalars[12] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if lt {
                    step = 48;
                    continue;
                }
                step = 38;
                continue;
            }
            38 => {
                words[9] = (m.get(words[7].wrapping_add(0x448u64), 8)) & 0xffffffff;

                eq = ((words[9] & 0xff) & (words[9] & 0xff)) == 0;
                lt = false;
                le = eq;

                if eq {
                    step = 143;
                    continue;
                }
                step = 39;
                continue;
            }
            39 => {
                scalars[0] = (0x3c800000u64) & 0xffffffff;

                scalars[0] = (f32::from_bits(scalars[0] as u32) * f32::from_bits(scalars[7] as u32))
                    .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[0] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 172;
                    continue;
                }
                step = 40;
                continue;
            }
            40 => {
                m.put(words[7].wrapping_add(0x448u64), 8, 0x0u64);

                eq = (m.get(words[7].wrapping_add(0x45au64), 8)) == (0x0u64);
                lt = (m.get(words[7].wrapping_add(0x45au64), 8)) < (0x0u64);
                le = lt || eq;

                if eq {
                    step = 43;
                    continue;
                }
                step = 41;
                continue;
            }
            41 => {
                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3e4u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 43;
                    continue;
                }
                step = 42;
                continue;
            }
            42 => {
                m.put(words[7].wrapping_add(0x45au64), 8, 0x0u64);

                scalars[0] = (0x3f80002au64) & 0xffffffff;

                scalars[0] = (f32::from_bits(scalars[0] as u32)
                    * f32::from_bits(m.get(words[7].wrapping_add(0x3e0u64), 32) as u32))
                .to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x3e0u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );
                step = 43;
                continue;
            }
            43 => {
                scalars[0] = (m.get(words[12].wrapping_add(words[0]), 32)) & 0xffffffff;

                scalars[1] = (m.get(words[7].wrapping_add(0x1ccu64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[0] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 180;
                    continue;
                }
                step = 44;
                continue;
            }
            44 => {
                scalars[1] = (0x3f802000u64) & 0xffffffff;

                scalars[1] = (f32::from_bits(scalars[1] as u32) * f32::from_bits(scalars[0] as u32))
                    .to_bits() as u64;

                words[9] = words[7].wrapping_add(0x3e4u64);
                step = 45;
                continue;
            }
            45 => {
                m.put(words[7].wrapping_add(0x3f0u64), 64, words[9]);

                m.put(
                    words[7].wrapping_add(0x3e4u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                step = 49;
                continue;
            }
            46 => {
                m.put(words[7].wrapping_add(0x448u64), 8, 0x1u64);

                words[9] = words[7].wrapping_add(0x3c0u64);

                eq = (m.get(words[7].wrapping_add(0x3f0u64), 64)) == (words[9]);
                lt = (m.get(words[7].wrapping_add(0x3f0u64), 64)) < (words[9]);
                le = lt || eq;

                if !eq {
                    step = 48;
                    continue;
                }
                step = 47;
                continue;
            }
            47 => {
                words[9] = words[7].wrapping_add(0x3e4u64);

                m.put(words[7].wrapping_add(0x3f0u64), 64, words[9]);
                step = 48;
                continue;
            }
            48 => {
                scalars[1] = (m.get(words[7].wrapping_add(0x3e4u64), 32)) & 0xffffffff;
                step = 49;
                continue;
            }
            49 => {
                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 54;
                    continue;
                }
                step = 50;
                continue;
            }
            50 => {
                m.put(
                    words[7].wrapping_add(0x3e4u64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[12] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !lt {
                    step = 53;
                    continue;
                }
                step = 51;
                continue;
            }
            51 => {
                {
                    let a = f32::from_bits(scalars[12] as u32);
                    let b = f32::from_bits(scalars[7] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if eq {
                    step = 53;
                    continue;
                }
                step = 52;
                continue;
            }
            52 => {
                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3c4u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 183;
                    continue;
                }
                step = 53;
                continue;
            }
            53 => {
                m.put(
                    words[7].wrapping_add(0x3d0u64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                words[9] = words[7].wrapping_add(0x3c0u64);

                m.put(words[7].wrapping_add(0x3f0u64), 64, words[9]);

                scalars[1] = scalars[5];
                step = 54;
                continue;
            }
            54 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 89;
                    continue;
                }
                step = 55;
                continue;
            }
            55 => {
                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(scalars[6] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if lt {
                    step = 142;
                    continue;
                }
                step = 56;
                continue;
            }
            56 => {
                scalars[1] = 0;

                scalars[1] = (f32::from_bits(scalars[6] as u32) as f64).to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) * f64::from_bits(0x3ff000002d000000u64))
                    .to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x3e4u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                words[9] = words[7].wrapping_add(0x3c0u64);

                m.put(words[7].wrapping_add(0x3f0u64), 64, words[9]);

                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[7] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 58;
                    continue;
                }
                step = 57;
                continue;
            }
            57 => {
                m.put(
                    words[7].wrapping_add(0x3e4u64),
                    32,
                    (scalars[7]) & 0xffffffff,
                );

                scalars[1] = scalars[7];
                step = 58;
                continue;
            }
            58 => {
                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3d0u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !eq {
                    step = 60;
                    continue;
                }
                step = 59;
                continue;
            }
            59 => {
                m.put(
                    words[7].wrapping_add(0x3d0u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );
                step = 60;
                continue;
            }
            60 => {
                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3e0u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 62;
                    continue;
                }
                step = 61;
                continue;
            }
            61 => {
                m.put(
                    words[7].wrapping_add(0x3e0u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );
                step = 62;
                continue;
            }
            62 => {
                scalars[0] = (m.get(words[7].wrapping_add(0x43cu64), 32)) & 0xffffffff;
                step = 63;
                continue;
            }
            63 => {
                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3c4u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 73;
                    continue;
                }
                step = 64;
                continue;
            }
            64 => {
                scalars[12] = (m.get(words[7].wrapping_add(0x3d0u64), 32)) & 0xffffffff;
                step = 65;
                continue;
            }
            65 => {
                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[12] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 73;
                    continue;
                }
                step = 66;
                continue;
            }
            66 => {
                scalars[1] = scalars[12];

                words[9] = ((words[9] & 0xffffffff) ^ (words[9] & 0xffffffff)) & 0xffffffff;

                step = 6;
                continue;
            }
            67 => {
                scalars[5] = scalars[2];

                words[8] = (0x1u64) & 0xffffffff;

                step = 11;
                continue;
            }
            68 => {
                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[6] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 82;
                    continue;
                }
                step = 69;
                continue;
            }
            69 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x430u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 140;
                    continue;
                }
                step = 70;
                continue;
            }
            70 => {
                scalars[5] = 0;

                scalars[5] = (f32::from_bits(scalars[1] as u32) as f64).to_bits() as u64;
                step = 71;
                continue;
            }
            71 => {
                scalars[0] = (f32::from_bits(scalars[0] as u32) as f64).to_bits() as u64;

                scalars[0] =
                    (f64::from_bits(scalars[0]) - f64::from_bits(scalars[5])).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0])
                    * f64::from_bits(m.get(words[7].wrapping_add(0x408u64), 64)))
                .to_bits() as u64;

                scalars[5] =
                    (f64::from_bits(scalars[5]) + f64::from_bits(scalars[0])).to_bits() as u64;

                scalars[0] = 0;

                scalars[0] = (f64::from_bits(scalars[5]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x43cu64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                step = 20;
                continue;
            }
            72 => {
                scalars[1] = 0;

                scalars[1] = (f32::from_bits(scalars[0] as u32) as f64).to_bits() as u64;

                scalars[0] = 0;

                scalars[0] = (f32::from_bits(scalars[5] as u32) as f64).to_bits() as u64;

                scalars[0] =
                    (f64::from_bits(scalars[0]) - f64::from_bits(scalars[1])).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0])
                    * f64::from_bits(m.get(words[7].wrapping_add(0x410u64), 64)))
                .to_bits() as u64;

                scalars[0] =
                    (f64::from_bits(scalars[0]) + f64::from_bits(scalars[1])).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x440u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                step = 32;
                continue;
            }
            73 => {
                m.put(
                    words[7].wrapping_add(0x3d0u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                words[9] = ((words[9] & 0xffffffff) ^ (words[9] & 0xffffffff)) & 0xffffffff;

                step = 6;
                continue;
            }
            74 => {
                scalars[5] = (0x7e967699u64) & 0xffffffff;

                scalars[7] = 0;

                scalars[7] = (f32::from_bits(scalars[1] as u32) as f64).to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 139;
                    continue;
                }
                step = 75;
                continue;
            }
            75 => {
                scalars[12] = 0x47da9930c0000000u64;

                scalars[13] = (0x7ed4c986u64) & 0xffffffff;
                step = 76;
                continue;
            }
            76 => {
                scalars[0] = (f32::from_bits(scalars[0] as u32) as f64).to_bits() as u64;

                scalars[5] = m.get(words[7].wrapping_add(0x420u64), 64);

                scalars[7] =
                    (f64::from_bits(scalars[7]) - f64::from_bits(scalars[0])).to_bits() as u64;

                scalars[7] = (f64::from_bits(scalars[7])
                    * f64::from_bits(m.get(words[7].wrapping_add(0x400u64), 64)))
                .to_bits() as u64;

                scalars[7] =
                    (f64::from_bits(scalars[7]) + f64::from_bits(scalars[12])).to_bits() as u64;

                scalars[7] = (f64::from_bits(scalars[7]) as f32).to_bits() as u64;

                scalars[7] = (f32::from_bits(scalars[7] as u32)
                    - f32::from_bits(scalars[13] as u32))
                .to_bits() as u64;

                scalars[7] = (f32::from_bits(scalars[7] as u32) as f64).to_bits() as u64;

                {
                    let a = f64::from_bits(scalars[7]);
                    let b = f64::from_bits(scalars[5]);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 129;
                    continue;
                }
                step = 77;
                continue;
            }
            77 => {
                {
                    let a = f64::from_bits(scalars[5]);
                    let b = f64::from_bits(scalars[0]);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !lt {
                    step = 129;
                    continue;
                }
                step = 78;
                continue;
            }
            78 => {
                scalars[7] = scalars[5];
                step = 79;
                continue;
            }
            79 => {
                scalars[5] = (0x35000000u64) & 0xffffffff;

                scalars[5] = (f32::from_bits(scalars[5] as u32) * f32::from_bits(scalars[1] as u32))
                    .to_bits() as u64;

                scalars[5] = (f32::from_bits(scalars[5] as u32) as f64).to_bits() as u64;

                {
                    let a = f64::from_bits(scalars[5]);
                    let b = f64::from_bits(scalars[7]);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 81;
                    continue;
                }
                step = 80;
                continue;
            }
            80 => {
                m.put(words[7].wrapping_add(0x420u64), 64, scalars[5]);

                scalars[7] = scalars[5];
                step = 81;
                continue;
            }
            81 => {
                m.put(
                    words[7].wrapping_add(0x3e0u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                m.put(
                    words[7].wrapping_add(0x428u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                scalars[5] = (0x3f7eb77fu64) & 0xffffffff;

                scalars[5] = (f32::from_bits(scalars[5] as u32) * f32::from_bits(scalars[1] as u32))
                    .to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x3dcu64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                scalars[5] = (0x3c27a805u64) & 0xffffffff;

                scalars[5] = (f32::from_bits(scalars[5] as u32) * f32::from_bits(scalars[1] as u32))
                    .to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x42cu64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                m.put(
                    words[7].wrapping_add(0x430u64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                step = 17;
                continue;
            }
            82 => {
                {
                    let a = f32::from_bits(scalars[0] as u32);
                    let b = f32::from_bits(scalars[6] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 20;
                    continue;
                }
                step = 83;
                continue;
            }
            83 => {
                {
                    let a = f32::from_bits(scalars[0] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3dcu64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if lt {
                    step = 87;
                    continue;
                }
                step = 84;
                continue;
            }
            84 => {
                words[5] = (words[1].wrapping_add(0x1u64)) & 0xffffffff;

                eq = (m.get(words[4].wrapping_add((-0x14i64 as u64)), 16)) == (words[5] & 0xffff);
                lt = (m.get(words[4].wrapping_add((-0x14i64 as u64)), 16)) < (words[5] & 0xffff);
                le = lt || eq;

                if le {
                    step = 182;
                    continue;
                }
                step = 85;
                continue;
            }
            85 => {
                words[5] = (words[5] & 0xffff) & 0xffffffff;

                words[5] = (words[5]).wrapping_shl((0x2u64) as u32);
                step = 86;
                continue;
            }
            86 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(m.get(words[13].wrapping_add(words[5]), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 20;
                    continue;
                }
                step = 87;
                continue;
            }
            87 => {
                scalars[5] = 0;

                scalars[5] = (f32::from_bits(scalars[6] as u32) as f64).to_bits() as u64;

                step = 71;
                continue;
            }
            88 => {
                scalars[2] = 0;

                scalars[2] = (f32::from_bits(scalars[0] as u32) as f64).to_bits() as u64;

                scalars[0] = 0;

                scalars[0] = (f32::from_bits(scalars[5] as u32) as f64).to_bits() as u64;

                scalars[0] =
                    (f64::from_bits(scalars[0]) - f64::from_bits(scalars[2])).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0])
                    * f64::from_bits(m.get(words[7].wrapping_add(0x418u64), 64)))
                .to_bits() as u64;

                scalars[0] =
                    (f64::from_bits(scalars[0]) + f64::from_bits(scalars[2])).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x440u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                step = 32;
                continue;
            }
            89 => {
                scalars[0] = scalars[7];

                step = 63;
                continue;
            }
            90 => {
                m.put(words[7].wrapping_add(0x454u64), 16, 0x0u64);

                words[0] = (m.get(words[7].wrapping_add(0x448u64), 8)) & 0xffffffff;

                words[1] = (words[0].wrapping_add((-0x1i64 as u64))) & 0xffffffff;

                eq = (words[1] & 0xff) == (0x12u64);
                lt = (words[1] & 0xff) < (0x12u64);
                le = lt || eq;

                if !le {
                    step = 92;
                    continue;
                }
                step = 91;
                continue;
            }
            91 => {
                words[0] = ((words[0] & 0xffffffff).wrapping_add((0x1u64))) & 0xffffffff;

                m.put(words[7].wrapping_add(0x448u64), 8, words[0] & 0xff);
                step = 92;
                continue;
            }
            92 => {
                scalars[5] = (m.get(words[7].wrapping_add(0x3c4u64), 32)) & 0xffffffff;

                scalars[7] = (m.get(words[7].wrapping_add(0x42cu64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[7] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 163;
                    continue;
                }
                step = 93;
                continue;
            }
            93 => {
                m.put(words[7].wrapping_add(0x449u64), 8, 0x64u64);
                step = 94;
                continue;
            }
            94 => {
                eq = (m.get(words[7].wrapping_add(0x450u64), 16)) == (0xfu64);
                lt = (m.get(words[7].wrapping_add(0x450u64), 16)) < (0xfu64);
                le = lt || eq;

                if le {
                    step = 96;
                    continue;
                }
                step = 95;
                continue;
            }
            95 => {
                m.put(
                    words[7].wrapping_add(0x430u64),
                    32,
                    (scalars[7]) & 0xffffffff,
                );
                step = 96;
                continue;
            }
            96 => {
                scalars[1] = (m.get(words[7].wrapping_add(0x434u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 147;
                    continue;
                }
                step = 97;
                continue;
            }
            97 => {
                m.put(words[7].wrapping_add(0x44au64), 8, 0x64u64);

                scalars[2] = (m.get(words[7].wrapping_add(0x440u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[2] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 101;
                    continue;
                }
                step = 98;
                continue;
            }
            98 => {
                words[0] = (0x64u64) & 0xffffffff;
                step = 99;
                continue;
            }
            99 => {
                words[3] = (m.get(words[4].wrapping_add((-0x4i64 as u64)), 32)) & 0xffffffff;

                m.put(words[7].wrapping_add(0x44cu64), 32, words[3] & 0xffffffff);
                step = 100;
                continue;
            }
            100 => {
                eq = ((words[0] & 0xff) & (words[0] & 0xff)) == 0;
                lt = false;
                le = eq;

                if eq {
                    step = 138;
                    continue;
                }
                step = 101;
                continue;
            }
            101 => {
                words[0] = (m.get(words[7].wrapping_add(0x44cu64), 32)) & 0xffffffff;

                eq = ((words[0] & 0xffffffff) & (words[0] & 0xffffffff)) == 0;
                lt = false;
                le = eq;

                if eq {
                    step = 138;
                    continue;
                }
                step = 102;
                continue;
            }
            102 => {
                scalars[0] = scalars[1];
                step = 103;
                continue;
            }
            103 => {
                m.put(
                    words[7].wrapping_add(0x438u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                scalars[6] = (m.get(words[7].wrapping_add(0x3ccu64), 32)) & 0xffffffff;

                scalars[4] = 0;

                scalars[4] = (f32::from_bits(scalars[6] as u32) as f64).to_bits() as u64;

                scalars[0] = 0;

                scalars[0] = (f32::from_bits(m.get(words[7].wrapping_add(0x444u64), 32) as u32)
                    as f64)
                    .to_bits() as u64;

                scalars[0] =
                    (f64::from_bits(scalars[0]) - f64::from_bits(scalars[4])).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) * f64::from_bits(0x3fefdf0e835081f9u64))
                    .to_bits() as u64;

                scalars[0] =
                    (f64::from_bits(scalars[0]) + f64::from_bits(scalars[4])).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x444u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                words[0] = (m.get(words[7].wrapping_add(0x3bdu64), 8)) & 0xffffffff;

                words[0] = ((words[0] & 0xffffffff).wrapping_add((0x1u64))) & 0xffffffff;

                m.put(words[7].wrapping_add(0x3bdu64), 8, words[0] & 0xff);

                words[1] = (m.get(words[7].wrapping_add(0x3bcu64), 8)) & 0xffffffff;

                scalars[12] = (m.get(words[7].wrapping_add(0x3c8u64), 32)) & 0xffffffff;

                eq = (words[0] & 0xff) == (words[1] & 0xff);
                lt = (words[0] & 0xff) < (words[1] & 0xff);
                le = lt || eq;

                if !le {
                    step = 130;
                    continue;
                }
                step = 104;
                continue;
            }
            104 => {
                scalars[14] = (m.get(words[7].wrapping_add(0x3d8u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[12] as u32);
                    let b = f32::from_bits(scalars[14] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 130;
                    continue;
                }
                step = 105;
                continue;
            }
            105 => {
                scalars[13] = (m.get(words[7].wrapping_add(0x1dcu64), 32)) & 0xffffffff;

                scalars[12] = scalars[14];
                step = 106;
                continue;
            }
            106 => {
                scalars[14] = (m.get(words[7].wrapping_add(0x3d4u64), 32)) & 0xffffffff;
                step = 107;
                continue;
            }
            107 => {
                words[0] = (m.get(words[7].wrapping_add(0x452u64), 16)) & 0xffffffff;

                eq = ((words[0] & 0xffff) & (words[0] & 0xffff)) == 0;
                lt = false;
                le = eq;

                if eq {
                    step = 109;
                    continue;
                }
                step = 108;
                continue;
            }
            108 => {
                words[0] = ((words[0] & 0xffffffff).wrapping_sub((0x1u64))) & 0xffffffff;

                m.put(words[7].wrapping_add(0x452u64), 16, words[0] & 0xffff);
                step = 109;
                continue;
            }
            109 => {
                {
                    let a = f32::from_bits(scalars[2] as u32);
                    let b = f32::from_bits(scalars[13] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 150;
                    continue;
                }
                step = 110;
                continue;
            }
            110 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[2] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 176;
                    continue;
                }
                step = 111;
                continue;
            }
            111 => {
                scalars[15] = (0x3d257a93u64) & 0xffffffff;

                scalars[15] = (f32::from_bits(scalars[15] as u32)
                    * f32::from_bits(scalars[13] as u32))
                .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[15] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 186;
                    continue;
                }
                step = 112;
                continue;
            }
            112 => {
                m.put(
                    words[7].wrapping_add(0x434u64),
                    32,
                    (scalars[15]) & 0xffffffff,
                );

                m.put(
                    words[7].wrapping_add(0x438u64),
                    32,
                    (scalars[15]) & 0xffffffff,
                );

                m.put(
                    words[7].wrapping_add(0x3e8u64),
                    32,
                    (scalars[6]) & 0xffffffff,
                );

                m.put(words[7].wrapping_add(0x452u64), 16, 0x18fu64);

                {
                    let a = f32::from_bits(scalars[12] as u32);
                    let b = f32::from_bits(scalars[6] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if lt {
                    step = 114;
                    continue;
                }
                step = 113;
                continue;
            }
            113 => {
                m.put(
                    words[7].wrapping_add(0x3e8u64),
                    32,
                    (scalars[12]) & 0xffffffff,
                );
                step = 114;
                continue;
            }
            114 => {
                scalars[1] = (m.get(words[7].wrapping_add(0x1ccu64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[13] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 153;
                    continue;
                }
                step = 115;
                continue;
            }
            115 => {
                scalars[15] = (0x3eb851ecu64) & 0xffffffff;

                scalars[15] = (f32::from_bits(scalars[15] as u32)
                    * f32::from_bits(scalars[2] as u32))
                .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[0] as u32);
                    let b = f32::from_bits(scalars[15] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 185;
                    continue;
                }
                step = 116;
                continue;
            }
            116 => {
                m.put(
                    words[7].wrapping_add(0x1ccu64),
                    32,
                    (scalars[13]) & 0xffffffff,
                );

                m.put(words[7].wrapping_add(0x458u64), 8, 0x0u64);

                scalars[1] = 0;

                scalars[1] = (f32::from_bits(scalars[13] as u32) as f64).to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) * f64::from_bits(0x3fdd28f72cf2abb8u64))
                    .to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1d0u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                m.put(words[7].wrapping_add(0x450u64), 16, 0x190u64);

                {
                    let a = f32::from_bits(scalars[13] as u32);
                    let b = f32::from_bits(scalars[12] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 122;
                    continue;
                }
                step = 117;
                continue;
            }
            117 => {
                {
                    let a = f32::from_bits(scalars[13] as u32);
                    let b = f32::from_bits(m.get(words[12], 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 166;
                    continue;
                }
                step = 118;
                continue;
            }
            118 => {
                scalars[1] = scalars[13];

                scalars[13] = (m.get(words[7].wrapping_add(0x1c8u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[13] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 156;
                    continue;
                }
                step = 119;
                continue;
            }
            119 => {
                eq = (m.get(words[7].wrapping_add(0x458u64), 8)) == (0x0u64);
                lt = (m.get(words[7].wrapping_add(0x458u64), 8)) < (0x0u64);
                le = lt || eq;

                if eq {
                    step = 166;
                    continue;
                }
                step = 120;
                continue;
            }
            120 => {
                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[2] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 166;
                    continue;
                }
                step = 121;
                continue;
            }
            121 => {
                m.put(words[7].wrapping_add(0x458u64), 8, 0x0u64);

                scalars[1] = 0;

                scalars[1] = (f32::from_bits(m.get(words[7].wrapping_add(0x1ccu64), 32) as u32)
                    as f64)
                    .to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) * f64::from_bits(0x3fdd28f72cf2abb8u64))
                    .to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1d0u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );
                step = 122;
                continue;
            }
            122 => {
                {
                    let a = f32::from_bits(scalars[12] as u32);
                    let b = f32::from_bits(scalars[14] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !lt {
                    step = 124;
                    continue;
                }
                step = 123;
                continue;
            }
            123 => {
                scalars[14] = (f32::from_bits(scalars[14] as u32)
                    * f32::from_bits(0x3f7fe39du64 as u32))
                .to_bits() as u64;

                scalars[12] = scalars[14];
                step = 124;
                continue;
            }
            124 => {
                m.put(
                    words[7].wrapping_add(0x3d4u64),
                    32,
                    (scalars[12]) & 0xffffffff,
                );

                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 34;
                    continue;
                }
                step = 125;
                continue;
            }
            125 => {
                scalars[0] = (m.get(words[7].wrapping_add(0x1d4u64), 32)) & 0xffffffff;

                scalars[5] = (0x3f5e55dau64) & 0xffffffff;

                scalars[5] = (f32::from_bits(scalars[5] as u32)
                    * f32::from_bits(scalars[12] as u32))
                .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[2] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 169;
                    continue;
                }
                step = 126;
                continue;
            }
            126 => {
                scalars[5] = (0x3f6b8184u64) & 0xffffffff;

                scalars[5] = (f32::from_bits(scalars[5] as u32)
                    * f32::from_bits(scalars[12] as u32))
                .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[2] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 181;
                    continue;
                }
                step = 127;
                continue;
            }
            127 => {
                scalars[12] = (f32::from_bits(scalars[12] as u32)
                    * f32::from_bits(0x3f73fc65u64 as u32))
                .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[2] as u32);
                    let b = f32::from_bits(scalars[12] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 191;
                    continue;
                }
                step = 128;
                continue;
            }
            128 => {
                scalars[4] = (f64::from_bits(scalars[4]) * f64::from_bits(0x3fef50765b6e4540u64))
                    .to_bits() as u64;

                scalars[4] = (f64::from_bits(scalars[4]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1d4u64),
                    32,
                    (scalars[4]) & 0xffffffff,
                );

                step = 170;
                continue;
            }
            129 => {
                m.put(words[7].wrapping_add(0x420u64), 64, scalars[7]);

                step = 79;
                continue;
            }
            130 => {
                m.put(words[7].wrapping_add(0x3bdu64), 8, 0x1u64);

                m.put(
                    words[7].wrapping_add(0x3d8u64),
                    32,
                    (scalars[12]) & 0xffffffff,
                );

                scalars[13] = 0;

                scalars[13] = (f32::from_bits(scalars[12] as u32) as f64).to_bits() as u64;

                scalars[13] = (f64::from_bits(scalars[13]) * f64::from_bits(0x3fd70a3d70000000u64))
                    .to_bits() as u64;

                scalars[13] = (f64::from_bits(scalars[13]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1dcu64),
                    32,
                    (scalars[13]) & 0xffffffff,
                );

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[2] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 106;
                    continue;
                }
                step = 131;
                continue;
            }
            131 => {
                scalars[14] = (m.get(words[7].wrapping_add(0x3d4u64), 32)) & 0xffffffff;

                scalars[15] = (0x3e800000u64) & 0xffffffff;

                scalars[15] = (f32::from_bits(scalars[15] as u32)
                    * f32::from_bits(scalars[14] as u32))
                .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[15] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 133;
                    continue;
                }
                step = 132;
                continue;
            }
            132 => {
                scalars[14] = scalars[5];

                scalars[14] = (f32::from_bits(scalars[14] as u32)
                    + f32::from_bits(scalars[5] as u32))
                .to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x3d4u64),
                    32,
                    (scalars[14]) & 0xffffffff,
                );
                step = 133;
                continue;
            }
            133 => {
                eq = (m.get(words[7].wrapping_add(0x459u64), 8)) == (0x0u64);
                lt = (m.get(words[7].wrapping_add(0x459u64), 8)) < (0x0u64);
                le = lt || eq;

                if eq {
                    step = 107;
                    continue;
                }
                step = 134;
                continue;
            }
            134 => {
                words[0] = (m.get(words[7].wrapping_add(0x452u64), 16)) & 0xffffffff;

                eq = ((words[0] & 0xffff) & (words[0] & 0xffff)) == 0;
                lt = false;
                le = eq;

                if !eq {
                    step = 108;
                    continue;
                }
                step = 135;
                continue;
            }
            135 => {
                m.put(words[7].wrapping_add(0x459u64), 8, 0x0u64);

                scalars[15] = (0x3e800000u64) & 0xffffffff;

                scalars[15] = (f32::from_bits(scalars[15] as u32)
                    * f32::from_bits(m.get(words[7].wrapping_add(0x3e8u64), 32) as u32))
                .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[15] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 109;
                    continue;
                }
                step = 136;
                continue;
            }
            136 => {
                m.put(
                    words[7].wrapping_add(0x3e8u64),
                    32,
                    (scalars[5]) & 0xffffffff,
                );

                step = 109;
                continue;
            }
            137 => {
                scalars[2] = (m.get(words[7].wrapping_add(0x440u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[2] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 99;
                    continue;
                }
                step = 138;
                continue;
            }
            138 => {
                m.put(words[7].wrapping_add(0x459u64), 8, 0x1u64);

                scalars[0] = (0xbf800000u64) & 0xffffffff;

                step = 103;
                continue;
            }
            139 => {
                scalars[13] = 0x3ff6a09e667f3bcdu64;

                scalars[13] =
                    (f64::from_bits(scalars[13]) * f64::from_bits(scalars[7])).to_bits() as u64;

                scalars[13] = (f64::from_bits(scalars[13]) as f32).to_bits() as u64;

                scalars[12] = 0;

                scalars[12] = (f32::from_bits(scalars[13] as u32) as f64).to_bits() as u64;

                step = 76;
                continue;
            }
            140 => {
                {
                    let a = f32::from_bits(scalars[0] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3e0u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !lt {
                    step = 20;
                    continue;
                }
                step = 141;
                continue;
            }
            141 => {
                scalars[0] = (f32::from_bits(scalars[0] as u32) as f64).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) * f64::from_bits(0x3ff000001ad7f29bu64))
                    .to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x43cu64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                step = 20;
                continue;
            }
            142 => {
                m.put(words[7].wrapping_add(0x428u64), 32, 0xbf800000u64);

                scalars[1] = (m.get(words[7].wrapping_add(0x3e4u64), 32)) & 0xffffffff;

                scalars[0] = (m.get(words[7].wrapping_add(0x43cu64), 32)) & 0xffffffff;

                step = 63;
                continue;
            }
            143 => {
                eq = (m.get(words[7].wrapping_add(0x45au64), 8)) == (0x0u64);
                lt = (m.get(words[7].wrapping_add(0x45au64), 8)) < (0x0u64);
                le = lt || eq;

                if eq {
                    step = 46;
                    continue;
                }
                step = 144;
                continue;
            }
            144 => {
                words[9] = words[7].wrapping_add(0x3c0u64);

                m.put(words[7].wrapping_add(0x3f0u64), 64, words[9]);

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[2] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if lt {
                    step = 48;
                    continue;
                }
                step = 145;
                continue;
            }
            145 => {
                if le {
                    step = 192;
                    continue;
                }
                step = 146;
                continue;
            }
            146 => {
                m.put(words[7].wrapping_add(0x448u64), 8, 0x1u64);

                scalars[1] = (m.get(words[7].wrapping_add(0x3e4u64), 32)) & 0xffffffff;

                step = 49;
                continue;
            }
            147 => {
                words[0] = (m.get(words[7].wrapping_add(0x44au64), 8)) & 0xffffffff;

                eq = ((words[0] & 0xff) & (words[0] & 0xff)) == 0;
                lt = false;
                le = eq;

                if eq {
                    step = 137;
                    continue;
                }
                step = 148;
                continue;
            }
            148 => {
                words[0] = ((words[0] & 0xffffffff).wrapping_sub((0x1u64))) & 0xffffffff;

                m.put(words[7].wrapping_add(0x44au64), 8, words[0] & 0xff);

                scalars[2] = (m.get(words[7].wrapping_add(0x440u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(scalars[2] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 99;
                    continue;
                }
                step = 149;
                continue;
            }
            149 => {
                step = 100;
                continue;
            }
            150 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[2] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 152;
                    continue;
                }
                step = 151;
                continue;
            }
            151 => {
                scalars[0] = (0x3d257a93u64) & 0xffffffff;

                scalars[0] = (f32::from_bits(scalars[0] as u32) * f32::from_bits(scalars[2] as u32))
                    .to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[0] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 188;
                    continue;
                }
                step = 152;
                continue;
            }
            152 => {
                scalars[1] = (m.get(words[7].wrapping_add(0x1ccu64), 32)) & 0xffffffff;
                step = 153;
                continue;
            }
            153 => {
                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[12] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 119;
                    continue;
                }
                step = 154;
                continue;
            }
            154 => {
                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(m.get(words[12], 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 119;
                    continue;
                }
                step = 155;
                continue;
            }
            155 => {
                scalars[13] = (m.get(words[7].wrapping_add(0x1c8u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[13] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 119;
                    continue;
                }
                step = 156;
                continue;
            }
            156 => {
                {
                    let a = f32::from_bits(scalars[5] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x438u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 158;
                    continue;
                }
                step = 157;
                continue;
            }
            157 => {
                scalars[0] = (m.get(words[7].wrapping_add(0x1d8u64), 32)) & 0xffffffff;

                scalars[0] =
                    (if f32::from_bits(scalars[0] as u32) > f32::from_bits(scalars[6] as u32) {
                        f32::from_bits(scalars[0] as u32)
                    } else {
                        f32::from_bits(scalars[6] as u32)
                    })
                    .to_bits() as u64;

                scalars[0] = (f32::from_bits(scalars[0] as u32) as f64).to_bits() as u64;

                scalars[1] = (f32::from_bits(scalars[1] as u32) as f64).to_bits() as u64;

                scalars[1] =
                    (f64::from_bits(scalars[1]) - f64::from_bits(scalars[0])).to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) * f64::from_bits(0x3fefe60000000000u64))
                    .to_bits() as u64;

                scalars[1] =
                    (f64::from_bits(scalars[1]) + f64::from_bits(scalars[0])).to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1ccu64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );
                step = 158;
                continue;
            }
            158 => {
                {
                    let a = f32::from_bits(scalars[13] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 160;
                    continue;
                }
                step = 159;
                continue;
            }
            159 => {
                m.put(
                    words[7].wrapping_add(0x1ccu64),
                    32,
                    (scalars[13]) & 0xffffffff,
                );

                scalars[1] = scalars[13];
                step = 160;
                continue;
            }
            160 => {
                words[0] = (m.get(words[7].wrapping_add(0x450u64), 16)) & 0xffffffff;

                eq = ((words[0] & 0xffff) & (words[0] & 0xffff)) == 0;
                lt = false;
                le = eq;

                if !eq {
                    step = 175;
                    continue;
                }
                step = 161;
                continue;
            }
            161 => {
                m.put(words[7].wrapping_add(0x450u64), 16, 0xfu64);

                m.put(words[7].wrapping_add(0x458u64), 8, 0x0u64);

                scalars[1] = (f32::from_bits(scalars[1] as u32) as f64).to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) * f64::from_bits(0x3fdd28f72cf2abb8u64))
                    .to_bits() as u64;

                scalars[1] = (f64::from_bits(scalars[1]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1d0u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 122;
                    continue;
                }
                step = 162;
                continue;
            }
            162 => {
                m.put(words[7].wrapping_add(0x449u64), 8, 0x0u64);

                m.put(words[7].wrapping_add(0x430u64), 32, 0xbf800000u64);

                step = 122;
                continue;
            }
            163 => {
                words[0] = (m.get(words[7].wrapping_add(0x449u64), 8)) & 0xffffffff;

                eq = ((words[0] & 0xff) & (words[0] & 0xff)) == 0;
                lt = false;
                le = eq;

                if eq {
                    step = 165;
                    continue;
                }
                step = 164;
                continue;
            }
            164 => {
                words[0] = ((words[0] & 0xffffffff).wrapping_sub((0x1u64))) & 0xffffffff;

                m.put(words[7].wrapping_add(0x449u64), 8, words[0] & 0xff);

                eq = ((words[0] & 0xff) & (words[0] & 0xff)) == 0;
                lt = false;
                le = eq;

                if !eq {
                    step = 94;
                    continue;
                }
                step = 165;
                continue;
            }
            165 => {
                m.put(words[7].wrapping_add(0x430u64), 32, 0xbf800000u64);

                step = 96;
                continue;
            }
            166 => {
                scalars[1] = (m.get(words[7].wrapping_add(0x1d0u64), 32)) & 0xffffffff;

                step = 122;
                continue;
            }
            167 => {
                m.put(words[7].wrapping_add(0x3b2u64), 16, words[9] & 0xffff);

                m.put(words[7].wrapping_add(0x3b6u64), 16, words[8] & 0xffff);

                m.put(words[7].wrapping_add(0x3bau64), 16, words[10] & 0xffff);
                step = 168;
                continue;
            }
            168 => {
                return m.gain;
            }
            169 => {
                scalars[4] = (f64::from_bits(scalars[4]) * f64::from_bits(0x3fe6a09e667f3bcdu64))
                    .to_bits() as u64;

                scalars[4] = (f64::from_bits(scalars[4]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1d4u64),
                    32,
                    (scalars[4]) & 0xffffffff,
                );
                step = 170;
                continue;
            }
            170 => {
                {
                    let a = f32::from_bits(scalars[1] as u32);
                    let b = f32::from_bits(scalars[4] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 34;
                    continue;
                }
                step = 171;
                continue;
            }
            171 => {
                scalars[0] = (f32::from_bits(scalars[0] as u32) as f64).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) * f64::from_bits(0x3feea4afa2a490dau64))
                    .to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) as f32).to_bits() as u64;

                scalars[1] =
                    (if f32::from_bits(scalars[1] as u32) > f32::from_bits(scalars[0] as u32) {
                        f32::from_bits(scalars[1] as u32)
                    } else {
                        f32::from_bits(scalars[0] as u32)
                    })
                    .to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1d4u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                step = 34;
                continue;
            }
            172 => {
                eq = (words[9] & 0xff) == (0x13u64);
                lt = (words[9] & 0xff) < (0x13u64);
                le = lt || eq;

                if le {
                    step = 48;
                    continue;
                }
                step = 173;
                continue;
            }
            173 => {
                eq = (m.get(words[7].wrapping_add(0x45au64), 8)) == (0x0u64);
                lt = (m.get(words[7].wrapping_add(0x45au64), 8)) < (0x0u64);
                le = lt || eq;

                if eq {
                    step = 43;
                    continue;
                }
                step = 174;
                continue;
            }
            174 => {
                step = 41;
                continue;
            }
            175 => {
                m.put(words[7].wrapping_add(0x458u64), 8, 0x1u64);

                words[0] = ((words[0] & 0xffffffff).wrapping_sub((0x1u64))) & 0xffffffff;

                m.put(words[7].wrapping_add(0x450u64), 16, words[0] & 0xffff);

                step = 120;
                continue;
            }
            176 => {
                scalars[1] = (m.get(words[7].wrapping_add(0x3e8u64), 32)) & 0xffffffff;
                step = 177;
                continue;
            }
            177 => {
                {
                    let a = f32::from_bits(scalars[12] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !lt {
                    step = 113;
                    continue;
                }
                step = 178;
                continue;
            }
            178 => {
                eq = ((words[0] & 0xffff) & (words[0] & 0xffff)) == 0;
                lt = false;
                le = eq;

                if !eq {
                    step = 114;
                    continue;
                }
                step = 179;
                continue;
            }
            179 => {
                scalars[1] = (f32::from_bits(scalars[1] as u32)
                    * f32::from_bits(0x3f7ffa52u64 as u32))
                .to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x3e8u64),
                    32,
                    (scalars[1]) & 0xffffffff,
                );

                step = 114;
                continue;
            }
            180 => {
                scalars[0] = (words[15] & 0xffffffff) & 0xffffffff;

                scalars[1] = (f32::from_bits(scalars[1] as u32) * f32::from_bits(scalars[0] as u32))
                    .to_bits() as u64;

                words[9] = words[7].wrapping_add(0x43cu64);

                step = 45;
                continue;
            }
            181 => {
                scalars[4] = (f64::from_bits(scalars[4]) * f64::from_bits(0x3feae89f995ad3adu64))
                    .to_bits() as u64;

                scalars[4] = (f64::from_bits(scalars[4]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1d4u64),
                    32,
                    (scalars[4]) & 0xffffffff,
                );

                step = 170;
                continue;
            }
            182 => {
                words[5] = ((words[5] & 0xffffffff) ^ (words[5] & 0xffffffff)) & 0xffffffff;

                step = 86;
                continue;
            }
            183 => {
                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[5] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if !le {
                    step = 55;
                    continue;
                }
                step = 184;
                continue;
            }
            184 => {
                scalars[0] = scalars[7];

                scalars[1] = scalars[5];

                step = 65;
                continue;
            }
            185 => {
                m.put(words[7].wrapping_add(0x3bdu64), 8, words[1] & 0xff);

                step = 153;
                continue;
            }
            186 => {
                scalars[1] = (m.get(words[7].wrapping_add(0x3e8u64), 32)) & 0xffffffff;

                {
                    let a = f32::from_bits(scalars[6] as u32);
                    let b = f32::from_bits(scalars[1] as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 177;
                    continue;
                }
                step = 187;
                continue;
            }
            187 => {
                step = 112;
                continue;
            }
            188 => {
                m.put(
                    words[7].wrapping_add(0x434u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                m.put(
                    words[7].wrapping_add(0x438u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );

                scalars[0] = 0;

                scalars[0] = (f32::from_bits(scalars[2] as u32) as f64).to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) * f64::from_bits(0x400638e38e38e38eu64))
                    .to_bits() as u64;

                scalars[0] = (f64::from_bits(scalars[0]) as f32).to_bits() as u64;

                {
                    let a = f32::from_bits(scalars[0] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3e8u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 190;
                    continue;
                }
                step = 189;
                continue;
            }
            189 => {
                m.put(
                    words[7].wrapping_add(0x3e8u64),
                    32,
                    (scalars[0]) & 0xffffffff,
                );
                step = 190;
                continue;
            }
            190 => {
                m.put(words[7].wrapping_add(0x452u64), 16, 0x18fu64);

                step = 152;
                continue;
            }
            191 => {
                scalars[4] = (f64::from_bits(scalars[4]) * f64::from_bits(0x3fed5818dcfba487u64))
                    .to_bits() as u64;

                scalars[4] = (f64::from_bits(scalars[4]) as f32).to_bits() as u64;

                m.put(
                    words[7].wrapping_add(0x1d4u64),
                    32,
                    (scalars[4]) & 0xffffffff,
                );

                step = 170;
                continue;
            }
            192 => {
                {
                    let a = f32::from_bits(scalars[7] as u32);
                    let b = f32::from_bits(m.get(words[7].wrapping_add(0x3c4u64), 32) as u32);
                    let unordered = a.is_nan() || b.is_nan();
                    eq = unordered || a == b;
                    lt = unordered || a < b;
                    le = lt || eq;
                }

                if le {
                    step = 48;
                    continue;
                }
                step = 193;
                continue;
            }
            193 => {
                step = 146;
                continue;
            }
            _ => unreachable!("invalid limiter control state"),
        }
    }
}
#[allow(unused_parens)]
fn fir_sum(index: usize, p: &[f32; 12]) -> f32 {
    match index {
        0 => {
            ((p[10]
                + ((p[8] + (p[7] + (p[6] + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + p[5]))))
                    + p[9]))
                + p[11])
        }
        1 => {
            ((p[10]
                + ((p[8] + (p[7] + (p[6] + ((((p[0] + p[1]) + (p[3] + p[2])) + p[4]) + p[5]))))
                    + p[9]))
                + p[11])
        }
        2 => {
            ((p[10]
                + ((p[8] + (p[7] + (p[6] + (((p[4] + p[3]) + (p[2] + (p[0] + p[1]))) + p[5]))))
                    + p[9]))
                + p[11])
        }
        3 => {
            ((p[10]
                + ((p[8] + (p[7] + (p[6] + ((p[3] + (p[2] + (p[0] + p[1]))) + (p[4] + p[5])))))
                    + p[9]))
                + p[11])
        }
        4 => {
            ((p[10]
                + ((p[8] + (p[7] + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + (p[5] + p[6]))))
                    + p[9]))
                + p[11])
        }
        5 => {
            ((p[10]
                + ((p[8] + ((p[7] + p[6]) + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + p[5])))
                    + p[9]))
                + p[11])
        }
        6 => {
            ((p[10]
                + (p[9]
                    + ((p[7] + p[8])
                        + (p[6] + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + p[5])))))
                + p[11])
        }
        7 => {
            ((p[10]
                + ((p[8] + p[9])
                    + (p[7] + (p[6] + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + p[5])))))
                + p[11])
        }
        8 => {
            (((p[10] + p[9])
                + (p[8] + (p[7] + (p[6] + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + p[5])))))
                + p[11])
        }
        9 => {
            ((p[10]
                + ((p[8] + (p[7] + (p[6] + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + p[5]))))
                    + p[9]))
                + p[11])
        }
        10 => {
            ((p[10]
                + ((p[8] + (p[7] + (p[6] + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + p[5]))))
                    + p[9]))
                + p[11])
        }
        11 => {
            ((p[10]
                + ((p[8] + (p[7] + (p[6] + (((p[3] + (p[2] + (p[0] + p[1]))) + p[4]) + p[5]))))
                    + p[9]))
                + p[11])
        }
        _ => unreachable!(),
    }
}
#[allow(clippy::assign_op_pattern)] // Preserve the floating-point operand order.
fn stereo_fir_sum(_index: usize, p: &[f32; 12]) -> f32 {
    let mut sum = p[0] + p[1];
    for &term in &p[2..] {
        sum = term + sum;
    }
    sum
}

fn surround_fir_sum(p: &[f32; 12]) -> f32 {
    let mut sum = p[0];
    for &term in &p[1..] {
        sum += term;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_replays_ring_and_shared_gain() {
        let mut limiter = PeakLimiter::new(44100., 2, -1.);
        let render = |limiter: &mut PeakLimiter| {
            (0..3000)
                .map(|i| {
                    let x = (i as f32 * 0.117).sin() * 3.;
                    let mut frame = [x, x * 0.5];
                    limiter.process_frame(&mut frame);
                    frame
                })
                .collect::<Vec<_>>()
        };
        let first = render(&mut limiter);
        limiter.reset();
        assert_eq!(first, render(&mut limiter));
        assert!(first.iter().all(|frame| frame[0] == frame[1] * 2.));
        assert_eq!(limiter.latency_samples(), 61);
    }
}
