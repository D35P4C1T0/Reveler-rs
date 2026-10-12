use reveler_dsp::{Parameters, Processor, correction_gain, db_to_gain};
use std::f64::consts::TAU;

fn fixed() -> Parameters {
    Parameters {
        target_lufs: -20.0,
        input_lufs: -20.0,
        correction_high: 0.0,
        correction_low: 0.0,
        max_gain_db: 30.0,
        ..Parameters::default()
    }
}
fn run_signal(
    rate: f64,
    channels: usize,
    n: usize,
    params: Parameters,
    mut signal: impl FnMut(usize, usize) -> f32,
) -> Vec<Vec<f32>> {
    let mut p = Processor::new(rate, channels).unwrap();
    p.set_parameters(params);
    p.reset();
    let mut result = vec![Vec::with_capacity(n); channels];
    for i in 0..n {
        let mut frame = [0.0; 6];
        for (c, x) in frame.iter_mut().enumerate().take(channels) {
            *x = signal(i, c);
        }
        p.process_frame(&mut frame[..channels]);
        for c in 0..channels {
            result[c].push(frame[c]);
        }
    }
    result
}
fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

#[test]
fn rejects_invalid_configurations() {
    for r in [f64::NAN, f64::INFINITY, 0.0, 7999.0, 192001.0] {
        assert!(Processor::new(r, 2).is_err());
    }
    for c in [0, 3, 4, 5, 7] {
        assert!(Processor::new(48000.0, c).is_err());
    }
}
#[test]
fn mix_laws_and_mode_bits() {
    let p = Parameters {
        target_lufs: -16.0,
        input_lufs: -23.0,
        max_gain_db: 30.0,
        correction_high: 0.25,
        correction_low: 0.75,
        ..Parameters::default()
    };
    let fixed = db_to_gain(7.0);
    assert!((correction_gain(p, -13.0) - (0.75 * fixed + 0.25 * db_to_gain(-3.0))).abs() < 1e-12);
    assert!((correction_gain(p, -33.0) - (0.25 * fixed + 0.75 * db_to_gain(17.0))).abs() < 1e-12);
    let log = Parameters { mix_mode: 3, ..p };
    assert!((correction_gain(log, -13.0) - db_to_gain(4.5)).abs() < 1e-12);
    assert!((correction_gain(log, -33.0) - db_to_gain(14.5)).abs() < 1e-12);
    assert_eq!(
        correction_gain(Parameters { mix_mode: 1, ..p }, -13.0),
        correction_gain(p, -13.0)
    );
    assert_eq!(
        correction_gain(Parameters { mix_mode: 2, ..p }, -33.0),
        correction_gain(p, -33.0)
    );
}
#[test]
fn max_gain_includes_fixed_makeup() {
    let p = Parameters {
        target_lufs: -10.0,
        input_lufs: -40.0,
        max_gain_db: 3.0,
        correction_high: 0.0,
        correction_low: 0.0,
        ..Parameters::default()
    };
    assert!((correction_gain(p, -40.0) - db_to_gain(3.0)).abs() < 1e-12);
}
#[test]
fn delay_matches_host_report_and_impulse_center() {
    for (r, expected) in [
        (8000.0, 288),
        (44100.0, 1560),
        (48000.0, 1698),
        (96000.0, 3390),
        (192000.0, 6774),
    ] {
        let p = Processor::new(r, 2).unwrap();
        let latency = p.latency_samples();
        assert_eq!(latency, expected);
        let y = run_signal(
            r,
            2,
            latency + 64,
            fixed(),
            |i, _| if i == 0 { 0.1 } else { 0.0 },
        );
        let peak = y[0]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert_eq!(peak, latency, "rate {r}");
    }
}
#[test]
fn bypass_is_exact_and_latency_compensated_after_reset() {
    let p = Parameters {
        bypass: true,
        ..fixed()
    };
    let latency = Processor::new(48000.0, 2).unwrap().latency_samples();
    let x: Vec<f32> = (0..4000)
        .map(|i| ((i as f32) * 0.127).sin() * 0.25)
        .collect();
    let y = run_signal(48000.0, 2, x.len() + latency, p, |i, _| {
        x.get(i).copied().unwrap_or(0.0)
    });
    assert!(y[0][..latency].iter().all(|x| *x == 0.0));
    assert_eq!(&y[0][latency..], x);
}
#[test]
fn stereo_link_preserves_channel_ratio() {
    let y = run_signal(48000.0, 2, 12000, fixed(), |i, c| {
        (3.0 * (TAU * 1000.0 * i as f64 / 48000.0).sin() * if c == 0 { 1.0 } else { 0.5 }) as f32
    });
    for (l, r) in y[0].iter().zip(&y[1]) {
        assert!((*l - 2.0 * r).abs() < 1e-6);
    }
}
#[test]
fn static_gain_mode_keeps_level_and_waveform() {
    let p = Parameters {
        target_lufs: -16.0,
        input_lufs: -23.0,
        ..fixed()
    };
    let y = run_signal(48000.0, 2, 24000, p, |i, _| {
        (0.05 * (TAU * 1000.0 * i as f64 / 48000.0).sin()) as f32
    });
    let ratio = rms(&y[0][8000..8000 + 15984]) / (0.05 / 2.0_f64.sqrt());

    let calibrated_gain = 0.11149929463863373_f64 / 0.05;
    assert!((20.0 * ratio.log10() - 20.0 * calibrated_gain.log10()).abs() < 0.0001);
}
#[test]
fn silence_freezes_upward_gain_and_flushes_state() {
    let mut p = Processor::new(48000.0, 2).unwrap();
    p.set_parameters(Parameters {
        max_gain_db: 30.0,
        ..Parameters::default()
    });
    p.reset();
    for _ in 0..48000 {
        let mut f = [0.0; 2];
        p.process_frame(&mut f);
        assert_eq!(f, [0.0; 2]);
    }

    assert!((p.meters().gain_db - 30.0).abs() < 0.0001);
    assert!(p.meters().input_lufs < -49.9);
}
#[test]
fn lfe_does_not_drive_main_loudness_or_limiter() {
    let a = run_signal(48000.0, 6, 12000, fixed(), |i, c| {
        if c == 0 {
            (0.02 * (TAU * 1000.0 * i as f64 / 48000.0).sin()) as f32
        } else {
            0.0
        }
    });
    let b = run_signal(48000.0, 6, 12000, fixed(), |i, c| {
        if c == 0 {
            (0.02 * (TAU * 1000.0 * i as f64 / 48000.0).sin()) as f32
        } else if c == 3 {
            (8.0 * (TAU * 60.0 * i as f64 / 48000.0).sin()) as f32
        } else {
            0.0
        }
    });
    assert_eq!(a[0], b[0]);
    assert!(b[3].iter().all(|x| x.abs() <= 1.001));
}
#[test]
fn output_is_finite_under_nonfinite_input_and_parameters() {
    let p = Parameters {
        target_lufs: f64::NAN,
        max_gain_db: f64::INFINITY,
        ..fixed()
    };
    let y = run_signal(48000.0, 2, 5000, p, |i, _| match i % 4 {
        0 => f32::NAN,
        1 => f32::INFINITY,
        2 => f32::NEG_INFINITY,
        _ => 0.1,
    });
    assert!(y.iter().flatten().all(|x| x.is_finite()));
}

#[test]
fn zero_low_correction_startup_does_not_emit_nan() {
    let y = run_signal(
        48000.0,
        2,
        5000,
        Parameters {
            correction_high: 1.0,
            correction_low: 0.0,
            input_lufs: -30.0,
            max_gain_db: 20.0,
            freeze_lufs: -70.0,
            ..Parameters::default()
        },
        |i, _| (0.03 * (TAU * 997.0 * i as f64 / 48000.0).sin()) as f32,
    );
    assert!(y.iter().flatten().all(|x| x.is_finite()));
}
#[test]
fn reset_erases_history_and_is_deterministic() {
    let mut p = Processor::new(48000.0, 2).unwrap();
    p.set_parameters(fixed());
    p.reset();
    let mut first = Vec::new();
    for i in 0..4000 {
        let mut f = [(i as f32 * 0.3).sin() * 0.1; 2];
        p.process_frame(&mut f);
        first.push(f);
    }
    p.reset();
    for (i, &expected) in first.iter().enumerate() {
        let mut f = [(i as f32 * 0.3).sin() * 0.1; 2];
        p.process_frame(&mut f);
        assert_eq!(f, expected);
    }
}

#[test]
fn mono_pan_law_matches_duplicate_stereo() {
    let params = Parameters {
        max_gain_db: 20.0,
        ..Parameters::default()
    };
    let mono = run_signal(48000.0, 1, 75000, params, |i, _| {
        (0.05 * (TAU * 997.0 * i as f64 / 48000.0).sin()) as f32
    });
    let stereo = run_signal(48000.0, 2, 75000, params, |i, _| {
        (0.05 * (TAU * 997.0 * i as f64 / 48000.0).sin()) as f32
    });
    assert_eq!(mono[0], stereo[0]);
}
#[test]
fn automatic_leveling_reaches_the_control_target() {
    let mut p = Processor::new(48000.0, 2).unwrap();
    p.set_parameters(Parameters {
        max_gain_db: 20.0,
        ..Parameters::default()
    });
    p.reset();
    for i in 0..288000 {
        let mut f = [(0.02 * (TAU * 997.0 * i as f64 / 48000.0).sin()) as f32; 2];
        p.process_frame(&mut f);
    }
    let m = p.meters();
    assert!((m.input_lufs + m.gain_db + 20.0).abs() < 0.2, "{m:?}");
}

// Independent 16x windowed-sinc reconstruction of the *output*. This uses a
// different cutoff, filter length, and window than the production oversampler.
fn reconstructed_peak(x: &[f32]) -> f64 {
    let mut peak: f64 = 0.0;
    for i in 24..x.len() - 24 {
        for phase in 0..16 {
            let t = phase as f64 / 16.0;
            let mut y = 0.0;
            let mut sum = 0.0;
            for k in -23_i32..=24 {
                let u = k as f64 - t;
                let sinc = if u.abs() < 1e-12 {
                    1.0
                } else {
                    (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u)
                };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * u / 25.0).cos();
                y += x[(i as i32 + k) as usize] as f64 * sinc * window;
                sum += sinc * window;
            }
            peak = peak.max((y / sum).abs());
        }
    }
    peak
}
#[test]
fn limiter_controls_intersample_peaks() {
    let p = Parameters {
        true_peak_db: -1.0,
        ..fixed()
    };
    for frequency in [1000.0, 10000.0, 18000.0, 21000.0] {
        let y = run_signal(48000.0, 2, 6000, p, |i, _| {
            (4.0 * (TAU * frequency * i as f64 / 48000.0 + 0.4).sin()) as f32
        });
        let peak = reconstructed_peak(&y[0][2500..]);
        assert!(
            20.0 * peak.log10() <= -0.8,
            "f={frequency}, dBTP={}",
            20.0 * peak.log10()
        );
    }
}

#[test]
fn limiter_controls_bursts_and_polarity_changes() {
    let p = Parameters {
        true_peak_db: -3.0,
        ..fixed()
    };
    let y = run_signal(48000.0, 2, 7000, p, |i, _| {
        if (100..150).contains(&i) {
            8.0
        } else if (300..350).contains(&i) {
            -8.0
        } else if (1000..1100).contains(&i) {
            if i % 2 == 0 { 8.0 } else { -8.0 }
        } else if (2000..2600).contains(&i) {
            (8.0 * (i as f64 * 0.37).sin()) as f32
        } else {
            0.0
        }
    });

    let sample_peak = y[0].iter().fold(0.0_f32, |p, x| p.max(x.abs()));
    assert_eq!(sample_peak.to_bits(), 0x3f2ea8c6);
}
