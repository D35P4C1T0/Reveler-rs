//! Plugin controls, native format exports and editor.
use nice_plug::prelude::*;
use nice_plug_core::editor::dpi::LogicalSize;
use nice_plug_egui::{
    EguiEditor, EguiEditorState, EguiNiceSettings, RepaintNotifier, create_egui_editor,
};
mod editor;
use editor::RevelerEditor;
use reveler_dsp::{Parameters, Processor};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

#[derive(Params)]
struct RevelerParams {
    #[id = "target_lufs"]
    target: FloatParam,
    #[id = "true_peak_db"]
    true_peak: FloatParam,
    #[id = "max_gain_db"]
    max_gain: FloatParam,
    #[id = "freeze_lufs"]
    freeze: FloatParam,
    #[id = "input_lufs"]
    input: FloatParam,
    #[id = "correction_high"]
    correction_high: FloatParam,
    #[id = "correction_low"]
    correction_low: FloatParam,
    #[id = "mix_mode"]
    mix_mode: IntParam,
    #[id = "lfe_gain_db"]
    lfe_gain: FloatParam,
    #[id = "bypass"]
    bypass: BoolParam,
}

fn continuous(name: &str, default: f32, min: f32, max: f32, unit: &'static str) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_step_size(0.1)
        .with_unit(unit)
}

impl Default for RevelerParams {
    fn default() -> Self {
        // Bounds are centralized here; see the investigation report for evidence.
        Self {
            target: continuous("Target Level", -20.0, -30.0, -10.0, " LUFS"),
            true_peak: continuous("True Peak", 0.0, -5.0, 0.0, " dBTP"),
            max_gain: continuous("Max Gain", 0.0, -10.0, 30.0, " dB"),
            freeze: continuous("Freeze Level", -40.0, -70.0, -10.0, " LUFS"),
            input: continuous("Input Level", -20.0, -40.0, 0.0, " LUFS"),
            correction_high: continuous("Correction High", 100.0, 0.0, 100.0, " %"),
            correction_low: continuous("Correction Low", 100.0, 0.0, 100.0, " %"),
            mix_mode: IntParam::new("Mix Mode", 0, IntRange::Linear { min: 0, max: 3 })
                .with_value_to_string(Arc::new(|mode| {
                    match mode {
                        1 => "Low log / high linear",
                        2 => "Low linear / high log",
                        3 => "Both logarithmic",
                        _ => "Both linear",
                    }
                    .to_owned()
                })),
            lfe_gain: continuous("LFE Gain", 0.0, -10.0, 30.0, " dB"),
            bypass: BoolParam::new("Bypass", false).make_bypass(),
        }
    }
}

impl RevelerParams {
    fn dsp(&self) -> Parameters {
        Parameters {
            target_lufs: self.target.value() as f64,
            true_peak_db: self.true_peak.value() as f64,
            max_gain_db: self.max_gain.value() as f64,
            freeze_lufs: self.freeze.value() as f64,
            input_lufs: self.input.value() as f64,
            correction_high: self.correction_high.value() as f64 / 100.0,
            correction_low: self.correction_low.value() as f64 / 100.0,
            mix_mode: self.mix_mode.value() as u8,
            lfe_gain_db: self.lfe_gain.value() as f64,
            bypass: self.bypass.value(),
        }
    }
}

struct Meters {
    input: AtomicU32,
    output: AtomicU32,
    input_lufs: AtomicU32,
    gain_db: AtomicU32,
    limiter_db: AtomicU32,
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            input: AtomicU32::new(0),
            output: AtomicU32::new(0),
            input_lufs: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            gain_db: AtomicU32::new(0),
            limiter_db: AtomicU32::new(0),
        }
    }
}

struct Reveler {
    params: Arc<RevelerParams>,
    dsp: Option<Processor>,
    meters: Arc<Meters>,
    editor_state: Arc<EguiEditorState>,
}

impl Default for Reveler {
    fn default() -> Self {
        Self {
            params: Arc::default(),
            dsp: None,
            meters: Arc::default(),
            editor_state: EguiEditorState::from_size(
                LogicalSize::new(editor::WIDTH, editor::EXPANDED_HEIGHT),
                1.0,
            ),
        }
    }
}

const fn layout(channels: u32, name: &'static str) -> AudioIOLayout {
    AudioIOLayout {
        main_input_channels: Some(new_nonzero_u32(channels)),
        main_output_channels: Some(new_nonzero_u32(channels)),
        names: PortNames {
            layout: Some(name),
            ..PortNames::const_default()
        },
        ..AudioIOLayout::const_default()
    }
}

impl Plugin for Reveler {
    const NAME: &'static str = "Reveler-rs";
    const VENDOR: &'static str = "Reveler-rs Project";
    const URL: &'static str = "https://github.com/D35P4C1T0/Reveler-rs";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        layout(2, "Stereo"),
        layout(1, "Mono"),
        layout(6, "5.1 (L R C LFE Ls Rs)"),
    ];
    // Apply automation at processing-block boundaries.
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;
    type SysExMessage = ();
    type BackgroundTask = ();
    type Editor = EguiEditor<RevelerEditor>;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Self::Editor> {
        create_egui_editor(
            self.editor_state.clone(),
            RepaintNotifier::new(),
            EguiNiceSettings::default(),
            RevelerEditor::new(self.params.clone(), self.meters.clone()),
        )
    }

    fn activate(
        &mut self,
        layout: &AudioIOLayout,
        config: &BufferConfig,
        context: &mut impl ActivateContext<Self>,
    ) -> bool {
        let channels = layout.main_output_channels.map_or(0, |n| n.get() as usize);
        let Ok(mut dsp) = Processor::new(config.sample_rate as f64, channels) else {
            return false;
        };
        dsp.set_parameters(self.params.dsp());
        context.set_latency_samples(dsp.latency_samples() as u32);
        self.dsp = Some(dsp);
        true
    }

    fn reset(&mut self) {
        if let Some(dsp) = &mut self.dsp {
            dsp.reset();
        }
        self.meters
            .input
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        self.meters
            .output
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        self.meters
            .input_lufs
            .store(f32::NEG_INFINITY.to_bits(), Ordering::Relaxed);
        self.meters.gain_db.store(0, Ordering::Relaxed);
        self.meters.limiter_db.store(0, Ordering::Relaxed);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        _: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(dsp) = &mut self.dsp else {
            return ProcessStatus::Normal;
        };
        dsp.set_parameters(self.params.dsp());
        // Fixed-size stack scratch avoids allocation and handles all supported layouts.
        let channels = buffer.as_slice();
        let channel_count = channels.len();
        let mut frame = [0.0_f32; 6];
        let mut input_peak = 0.0_f32;
        let mut output_peak = 0.0_f32;
        for sample in 0..channels[0].len() {
            for (c, channel) in channels.iter().enumerate() {
                frame[c] = channel[sample];
                input_peak = input_peak.max(frame[c].abs());
            }
            dsp.process_frame(&mut frame[..channel_count]);
            for (c, channel) in channels.iter_mut().enumerate() {
                channel[sample] = frame[c];
                output_peak = output_peak.max(frame[c].abs());
            }
        }
        self.meters
            .input
            .store(input_peak.to_bits(), Ordering::Relaxed);
        self.meters
            .output
            .store(output_peak.to_bits(), Ordering::Relaxed);
        let meters = dsp.meters();
        self.meters
            .input_lufs
            .store((meters.input_lufs as f32).to_bits(), Ordering::Relaxed);
        self.meters
            .gain_db
            .store((meters.gain_db as f32).to_bits(), Ordering::Relaxed);
        self.meters.limiter_db.store(
            (meters.limiter_reduction_db as f32).to_bits(),
            Ordering::Relaxed,
        );
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Reveler {
    const CLAP_ID: &'static str = "org.reveler-rs.loudness-leveler";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Adaptive loudness leveler and true-peak limiter");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Utility,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for Reveler {
    const VST3_CLASS_ID: [u8; 16] = *b"RevelerRS2026v01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
}

nice_export_clap!(Reveler);
nice_export_vst3!(Reveler);

#[cfg(test)]
mod tests {
    use super::Reveler;
    use nice_plug::wrapper::vst3::{
        Wrapper,
        vst3::Steinberg::{
            Vst::{IAudioProcessorTrait, SpeakerArr},
            kResultFalse, kResultOk,
        },
    };

    #[test]
    fn vst3_accepts_only_the_speaker_maps_it_reports() {
        let wrapper = Wrapper::<Reveler>::new();
        for mut supported in [SpeakerArr::kMono, SpeakerArr::kStereo, SpeakerArr::k51] {
            let mut output = supported;
            assert_eq!(
                unsafe { wrapper.setBusArrangements(&mut supported, 1, &mut output, 1) },
                kResultOk
            );
            for direction in [0, 1] {
                let mut reported = 0;
                assert_eq!(
                    unsafe { wrapper.getBusArrangement(direction, 0, &mut reported) },
                    kResultOk
                );
                assert_eq!(reported, supported);
            }
            for mut unsupported in [SpeakerArr::k60Cine, SpeakerArr::k60Music] {
                assert_eq!(
                    unsafe { wrapper.setBusArrangements(&mut unsupported, 1, &mut output, 1) },
                    kResultFalse
                );
                assert_eq!(
                    unsafe { wrapper.setBusArrangements(&mut supported, 1, &mut unsupported, 1) },
                    kResultFalse
                );
            }
        }
    }
}
