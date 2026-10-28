use reveler_dsp::{Parameters, adaptive::Adaptive};

#[test]
fn leveler_delay_is_exact_34ms() {
    let mut a = Adaptive::new(48000.0, 1);
    let p = Parameters {
        correction_high: 0.0,
        correction_low: 0.0,
        ..Parameters::default()
    };
    a.set_parameters(p);
    let delay = a.delay_samples();
    assert_eq!(delay, 1632);
    for t in 0..=delay {
        let mut x = [if t == 0 { 1.0 } else { 0.0 }];
        a.process_frame(&mut x);
        if t == delay {
            assert_eq!(x[0], 1.0)
        } else {
            assert_eq!(x[0], 0.0)
        }
    }
}

#[test]
fn lfe_is_not_changed_by_main_channel_adaptive_gain() {
    let mut a = Adaptive::new(48000.0, 6);
    let mut b = Adaptive::new(48000.0, 6);
    let p = Parameters {
        max_gain_db: 20.0,
        ..Parameters::default()
    };
    a.set_parameters(p);
    b.set_parameters(p);
    for t in 0..24000 {
        let lfe = ((t as f32) * 0.01).sin() * 0.03;
        let mut x = [0.0, 0.0, 0.0, lfe, 0.0, 0.0];
        let mut y = [0.8, -0.8, 0.8, lfe, 0.8, -0.8];
        a.process_frame(&mut x);
        b.process_frame(&mut y);
        assert_eq!(x[3].to_bits(), y[3].to_bits());
    }
}

#[test]
fn fixed_gain_keeps_detector_history_for_resuming_correction() {
    for channels in [1, 2, 6] {
        let mut reference = Adaptive::new(48000.0, channels);
        let mut switched = Adaptive::new(48000.0, channels);
        let params = Parameters {
            max_gain_db: 20.0,
            ..Parameters::default()
        };
        reference.set_parameters(params);
        switched.set_parameters(params);
        for i in 0..96000 {
            if i == 24000 {
                switched.set_parameters(Parameters {
                    correction_high: 0.0,
                    correction_low: 0.0,
                    ..params
                });
            } else if i == 72000 {
                switched.set_parameters(params);
            }
            let amplitude = if i < 24000 { 0.01 } else { 0.1 };
            let mut a = [amplitude * (i as f32 * 0.13).sin(); 6];
            let mut b = a;
            let expected = reference.process_frame(&mut a[..channels]);
            let actual = switched.process_frame(&mut b[..channels]);
            assert_eq!(switched.measured_power(), reference.measured_power());
            if i >= 72000 {
                assert_eq!(actual, expected);
                assert_eq!(a, b);
            }
        }
    }
}
