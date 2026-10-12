//! Real-time adaptive audio leveling and true-peak limiting.
pub mod adaptive;
pub mod limiter;

use adaptive::Adaptive;
use limiter::PeakLimiter;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    pub target_lufs: f64,
    pub true_peak_db: f64,
    pub max_gain_db: f64,
    pub freeze_lufs: f64,
    pub input_lufs: f64,
    pub correction_high: f64,
    pub correction_low: f64,
    pub mix_mode: u8,
    pub lfe_gain_db: f64,
    pub bypass: bool,
}
impl Default for Parameters {
    fn default() -> Self {
        Self {
            target_lufs: -20.0,
            true_peak_db: 0.0,
            max_gain_db: 0.0,
            freeze_lufs: -40.0,
            input_lufs: -20.0,
            correction_high: 1.0,
            correction_low: 1.0,
            mix_mode: 0,
            lfe_gain_db: 0.0,
            bypass: false,
        }
    }
}

fn finite_clamp(x: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if x.is_finite() {
        x.clamp(lo, hi)
    } else {
        default
    }
}
impl Parameters {
    fn sanitize(self) -> Self {
        let d = Self::default();
        Self {
            target_lufs: finite_clamp(self.target_lufs, -30.0, -10.0, d.target_lufs),
            true_peak_db: finite_clamp(self.true_peak_db, -5.0, 0.0, d.true_peak_db),
            max_gain_db: finite_clamp(self.max_gain_db, -10.0, 30.0, d.max_gain_db),
            freeze_lufs: finite_clamp(self.freeze_lufs, -70.0, -10.0, d.freeze_lufs),
            input_lufs: finite_clamp(self.input_lufs, -40.0, 0.0, d.input_lufs),
            correction_high: finite_clamp(self.correction_high, 0.0, 1.0, d.correction_high),
            correction_low: finite_clamp(self.correction_low, 0.0, 1.0, d.correction_low),
            mix_mode: self.mix_mode.min(3),
            lfe_gain_db: finite_clamp(self.lfe_gain_db, -10.0, 30.0, d.lfe_gain_db),
            bypass: self.bypass,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Meters {
    pub input_lufs: f64,
    pub gain_db: f64,
    pub limiter_reduction_db: f64,
}

#[inline]
pub fn db_to_gain(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// Correction mix laws: linear interpolation of amplitudes, or logarithmic
/// interpolation of amplitudes (linear in dB). Mode bits: 1=low log, 2=high log.
pub fn correction_gain(params: Parameters, measured_lufs: f64) -> f64 {
    let measured = if measured_lufs.is_finite() {
        measured_lufs
    } else {
        params.input_lufs
    };
    let high = measured > params.input_lufs;
    let amount = if high {
        params.correction_high
    } else {
        params.correction_low
    };
    let fixed_db = params.target_lufs - params.input_lufs;
    let controlled_db = params.target_lufs - measured;
    let log = params.mix_mode & if high { 2 } else { 1 } != 0;
    let gain = if log {
        db_to_gain(fixed_db + amount * (controlled_db - fixed_db))
    } else {
        (1.0 - amount) * db_to_gain(fixed_db) + amount * db_to_gain(controlled_db)
    };
    gain.min(db_to_gain(params.max_gain_db))
}

/// Adaptive controller with linked true-peak limiting.
/// Construction allocates; processing and reset do not. 5.1 order is
/// L, R, C, LFE, Ls, Rs. LFE has an independent limiter.
pub struct Processor {
    channels: usize,
    params: Parameters,
    adaptive: Adaptive,
    main_limiter: PeakLimiter,
    lfe_limiter: Option<PeakLimiter>,
    latency: usize,
    dry: Vec<[f32; 6]>,
    dry_pos: usize,
    wet: f64,
    bypass_coeff: f64,
    meter_count: usize,
    meters: Meters,
}

impl Processor {
    pub fn new(sample_rate: f64, channels: usize) -> Result<Self, &'static str> {
        if !sample_rate.is_finite() || !(8000.0..=192000.0).contains(&sample_rate) {
            return Err("sample rate must be between 8 and 192 kHz");
        }
        if !matches!(channels, 1 | 2 | 6) {
            return Err("supported layouts: mono, stereo, 5.1");
        }
        let params = Parameters::default();
        let adaptive = Adaptive::new(sample_rate, channels);
        let main_limiter = PeakLimiter::new(
            sample_rate,
            if channels == 6 { 5 } else { channels },
            params.true_peak_db,
        );
        let latency = adaptive.delay_samples() + main_limiter.latency_samples();
        let lfe_limiter = if channels == 6 {
            {
                let mut limiter = PeakLimiter::new(sample_rate, 1, params.true_peak_db);
                limiter.use_surround_fir();
                Some(limiter)
            }
        } else {
            None
        };
        Ok(Self {
            channels,
            params,
            adaptive,
            main_limiter,
            lfe_limiter,
            latency,
            dry: vec![[0.0; 6]; latency + 1],
            dry_pos: 0,
            wet: 1.0,
            bypass_coeff: (-1.0 / (sample_rate * 0.005)).exp(),
            meter_count: 0,
            meters: Meters {
                input_lufs: f64::NEG_INFINITY,
                gain_db: 0.0,
                limiter_reduction_db: 0.0,
            },
        })
    }
    pub fn latency_samples(&self) -> usize {
        self.latency
    }
    pub fn meters(&self) -> Meters {
        self.meters
    }
    pub fn set_parameters(&mut self, p: Parameters) {
        let p = p.sanitize();
        if p == self.params {
            return;
        }
        self.adaptive.set_parameters(p);
        if p.true_peak_db != self.params.true_peak_db {
            self.main_limiter.set_peak(p.true_peak_db);
            if let Some(lfe) = &mut self.lfe_limiter {
                lfe.set_peak(p.true_peak_db);
            }
        }
        self.params = p;
    }
    pub fn reset(&mut self) {
        self.adaptive.reset();
        self.main_limiter.reset();
        if let Some(lfe) = &mut self.lfe_limiter {
            lfe.reset();
        }
        self.dry.fill([0.0; 6]);
        self.dry_pos = 0;
        self.wet = if self.params.bypass { 0.0 } else { 1.0 };
        self.meter_count = 0;
        self.meters = Meters {
            input_lufs: f64::NEG_INFINITY,
            gain_db: 0.0,
            limiter_reduction_db: 0.0,
        };
    }
    pub fn process_frame(&mut self, frame: &mut [f32]) {
        assert_eq!(
            frame.len(),
            self.channels,
            "frame must match activated layout"
        );
        let mut dry = [0.0; 6];
        for (c, x) in frame.iter_mut().enumerate() {
            *x = if x.is_finite() {
                x.clamp(-1e6, 1e6)
            } else {
                0.0
            };
            dry[c] = *x;
        }
        let gain = self.adaptive.process_frame(frame);
        if self.channels == 6 {
            let mut main = [frame[0], frame[1], frame[2], frame[4], frame[5]];
            self.main_limiter.process_frame(&mut main);
            for (i, c) in [0, 1, 2, 4, 5].into_iter().enumerate() {
                frame[c] = main[i];
            }
            if let Some(lfe) = &mut self.lfe_limiter {
                lfe.process_frame(&mut frame[3..4]);
            }
        } else {
            self.main_limiter.process_frame(frame);
        }
        self.dry[self.dry_pos] = dry;
        self.dry_pos = (self.dry_pos + 1) % self.dry.len();
        let delayed_dry = self.dry[self.dry_pos];
        let target = if self.params.bypass { 0.0 } else { 1.0 };
        self.wet = target + self.bypass_coeff * (self.wet - target);
        if (self.wet - target).abs() < 1e-8 {
            self.wet = target;
        }
        if self.wet != 1.0 {
            for (c, x) in frame.iter_mut().enumerate() {
                *x = (*x as f64 * self.wet + delayed_dry[c] as f64 * (1.0 - self.wet)) as f32;
            }
        }
        // Prevent nonfinite gain calculations from contaminating the audio graph.
        for value in frame.iter_mut() {
            if !value.is_finite() {
                *value = 0.0;
            }
        }
        // Display work is decimated independently of the sample DSP.
        self.meter_count = (self.meter_count + 1) % 64;
        if self.meter_count == 0 {
            let power = self.adaptive.measured_power();
            self.meters.input_lufs = if power > 0.0 {
                -0.691 + 10.0 * power.log10()
            } else {
                f64::NEG_INFINITY
            };
            self.meters.gain_db = 20.0 * (gain as f64).max(1e-30).log10();
            self.meters.limiter_reduction_db = self.main_limiter.reduction_db() as f64;
        }
    }
}
