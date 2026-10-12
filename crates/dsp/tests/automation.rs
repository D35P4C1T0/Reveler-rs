use reveler_dsp::{Parameters, Processor};

#[test]
fn stereo_parameter_transitions_are_deterministic() {
    let settings = [
        [-20., -1., 20., -40., -20., 100., 100., 0.],
        [-18., -1., 15., -70., -25., 25., 75., 1.],
        [-23., -3., 12., -35., -30., 75., 25., 2.],
        [-17., 0., 10., -60., -22., 50., 50., 3.],
        [-20., -1., 20., -40., -20., 100., 100., 0.],
        [-20., -1., 20., -70., -30., 0., 100., 0.],
        [-19., -5., 18., -50., -28., 25., 75., 3.],
        [-20., -1., 20., -40., -20., 100., 100., 0.],
    ];
    let mut processor = Processor::new(48000., 2).unwrap();
    let mut seed = 0x06138a92u32;
    let mut hash = 14695981039346656037u64;
    for i in 0..65536 {
        if i % 8192 == 0 {
            let v = settings[i / 8192];
            processor.set_parameters(Parameters {
                target_lufs: v[0],
                true_peak_db: v[1],
                max_gain_db: v[2],
                freeze_lufs: v[3],
                input_lufs: v[4],
                correction_high: v[5] / 100.,
                correction_low: v[6] / 100.,
                mix_mode: v[7] as u8,
                ..Parameters::default()
            });
        }
        let amplitude = if i % 16384 >= 8192 { 0.6 } else { 0.04 };
        let mut frame = [0.; 2];
        for value in &mut frame {
            seed = 1664525u32.wrapping_mul(seed).wrapping_add(1013904223);
            *value = (((seed >> 8) as f64 / 8388608.0 - 1.0) * amplitude) as f32;
        }
        processor.process_frame(&mut frame);
        for value in frame {
            assert!(value.is_finite());
            for byte in value.to_le_bytes() {
                hash = (hash ^ byte as u64).wrapping_mul(1099511628211);
            }
        }
    }
    assert_eq!(hash, 0x3e8f1623ea69b9ef);
}
