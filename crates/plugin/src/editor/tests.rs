use super::*;
use nice_plug_core::{
    context::{PluginApi, gui::GuiContextInner},
    params::internals::ParamPtr,
    plugin::PluginState,
};
use std::sync::Mutex;

#[derive(Default)]
struct Host {
    edits: Mutex<Vec<(String, &'static str)>>,
}
impl GuiContextInner for Host {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }
    unsafe fn raw_begin_set_parameter(&self, p: ParamPtr) {
        self.edits
            .lock()
            .unwrap()
            .push((unsafe { p.name() }.into(), "begin"));
    }
    unsafe fn raw_set_parameter_normalized(&self, p: ParamPtr, v: f32) {
        // The editor and test retain the Arc containing every parameter for the host's lifetime.
        unsafe {
            p._internal_set_normalized_value(v);
        }
        self.edits
            .lock()
            .unwrap()
            .push((unsafe { p.name() }.into(), "set"));
    }
    unsafe fn raw_end_set_parameter(&self, p: ParamPtr) {
        self.edits
            .lock()
            .unwrap()
            .push((unsafe { p.name() }.into(), "end"));
    }
    fn get_state(&self) -> PluginState {
        unreachable!()
    }
    fn set_state(&self, _: PluginState) {
        unreachable!()
    }
    fn request_restart(&self) {}
}
struct Harness {
    ctx: egui::Context,
    editor: RevelerEditor,
    host: Arc<Host>,
    time: f64,
    size: egui::Vec2,
}
impl Harness {
    fn new(theme: egui::Theme) -> Self {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        configure(&ctx);
        let host = Arc::new(Host::default());
        let mut editor = RevelerEditor::new(Arc::default(), Arc::default());
        editor.context = Some(GuiContext::new(host.clone()));
        Self {
            ctx,
            editor,
            host,
            time: 0.0,
            size: vec2(WIDTH, EXPANDED_HEIGHT),
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| self.editor.draw(ui),
        );
        output.textures_delta.clear();
        output
    }
    fn idle(&mut self, frames: usize) -> egui::FullOutput {
        let mut out = self.frame(vec![]);
        for _ in 1..frames {
            out = self.frame(vec![]);
        }
        out
    }
    fn click(&mut self, at: egui::Pos2) -> egui::FullOutput {
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![pointer(at, true)]);
        self.frame(vec![pointer(at, false)])
    }
    fn texts(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    Some(t.galley.job.text.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}
fn pointer(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
fn assert_balanced(host: &Host) {
    let mut open = None;
    let edits = host.edits.lock().unwrap();
    for (name, kind) in edits.iter() {
        match *kind {
            "begin" => {
                assert!(open.is_none(), "overlapping gesture for {name}");
                open = Some(name);
            }
            "set" => assert_eq!(open, Some(name)),
            "end" => {
                assert_eq!(open, Some(name));
                open = None;
            }
            _ => unreachable!(),
        }
    }
    assert!(open.is_none(), "unterminated host gesture");
}

#[test]
fn manual_replaces_controls_scrolls_and_never_resizes_window() {
    let mut h = Harness::new(egui::Theme::Light);
    let initial = h.idle(2);
    assert!(Harness::texts(&initial).contains(&"TARGET LEVEL".into()));
    h.click(pos2(430.0, 36.0));
    let manual = h.idle(40);
    assert!(h.editor.flip.open);
    let texts = Harness::texts(&manual);
    assert!(texts.contains(&"Manual".into()));
    assert!(!texts.contains(&"TARGET LEVEL".into()));
    for out in [&initial, &manual] {
        assert!(out.viewport_output.values().all(|v| {
            v.commands
                .iter()
                .all(|cmd| !matches!(cmd, egui::ViewportCommand::InnerSize(_)))
        }));
    }
    h.frame(vec![
        egui::Event::PointerMoved(pos2(200.0, 220.0)),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: vec2(0.0, -1800.0),
            modifiers: Default::default(),
        },
    ]);
    let scrolled = h.idle(40);
    assert!(
        Harness::texts(&scrolled)
            .iter()
            .any(|t| t.starts_with("Drag a slider")),
        "manual must scroll to its final instructions"
    );
    assert!(!Harness::texts(&scrolled).contains(&"Manual".into()));
    h.click(pos2(200.0, 294.0));
    assert!(
        h.host.edits.lock().unwrap().is_empty(),
        "hidden controls must not receive clicks"
    );
    h.click(pos2(430.0, 36.0));
    let controls = h.idle(40);
    assert!(!h.editor.flip.open);
    assert!(Harness::texts(&controls).contains(&"TARGET LEVEL".into()));
}

#[test]
fn bypass_and_slider_drag_text_entry_reset_preserve_host_gestures() {
    let mut h = Harness::new(egui::Theme::Light);
    h.idle(2);
    h.click(pos2(476.0, 36.0));
    assert!(h.editor.params.bypass.value());
    h.click(pos2(476.0, 36.0));
    assert!(!h.editor.params.bypass.value());
    // A full mouse drag through the target slider.
    let start = pos2(60.0, 294.0);
    let end = pos2(220.0, 294.0);
    h.frame(vec![egui::Event::PointerMoved(start)]);
    h.frame(vec![pointer(start, true)]);
    h.frame(vec![egui::Event::PointerMoved(end)]);
    h.frame(vec![pointer(end, false)]);
    assert!(
        h.editor.params.target.value() > -15.0,
        "target {} edits {:?}",
        h.editor.params.target.value(),
        h.host.edits.lock().unwrap()
    );
    // Click the large number and replace it by typing.
    h.click(pos2(65.0, 226.0));
    h.frame(vec![
        egui::Event::Text("-23.4".into()),
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        },
    ]);
    assert!((h.editor.params.target.value() + 23.4).abs() < 0.11);
    h.idle(30);
    h.click(pos2(130.0, 294.0));
    h.click(pos2(130.0, 294.0));
    assert!(
        (h.editor.params.target.value() + 20.0).abs() < 0.01,
        "double click resets target"
    );
    assert_balanced(&h.host);
}

#[test]
fn all_eight_sliders_and_mix_mode_remain_editable() {
    let mut h = Harness::new(egui::Theme::Light);
    h.idle(2);
    for (at, param_name) in [
        (pos2(80.0, 294.0), "Target Level"),
        (pos2(370.0, 213.0), "True Peak"),
        (pos2(370.0, 296.0), "Max Gain"),
        (pos2(80.0, 427.0), "Freeze Level"),
        (pos2(370.0, 427.0), "Input Level"),
        (pos2(80.0, 488.0), "Correction High"),
        (pos2(370.0, 488.0), "Correction Low"),
        (pos2(80.0, 549.0), "LFE Gain"),
    ] {
        h.click(at);
        assert!(
            h.host
                .edits
                .lock()
                .unwrap()
                .iter()
                .any(|(name, kind)| name == param_name && *kind == "set"),
            "slider {param_name} must reach the host"
        );
    }
    h.click(pos2(380.0, 551.0));
    let out = h.idle(2);
    assert!(Harness::texts(&out).iter().any(|s| s == "Both logarithmic"));
    let last_row = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == "Both logarithmic" => {
                Some(t.pos + vec2(12.0, 6.0))
            }
            _ => None,
        })
        .unwrap();
    h.click(last_row);
    assert_eq!(h.editor.params.mix_mode.value(), 3);
    assert_balanced(&h.host);
}

#[test]
fn flip_reverses_without_jumping_and_projects_both_endpoints_identically() {
    let mut flip = PageFlip {
        open: true,
        progress: 0.0,
    };
    for _ in 0..8 {
        flip.advance(1.0 / 60.0);
    }
    let before = flip.progress;
    flip.open = false;
    flip.advance(1.0 / 60.0);
    assert!(
        flip.progress < before && (before - flip.progress - 1.0 / 60.0 / FLIP_SECONDS).abs() < 1e-5
    );
    for _ in 0..40 {
        flip.advance(1.0 / 60.0);
    }
    assert_eq!(flip.progress, 0.0);
    let page = Rect::from_min_max(pos2(22.0, 70.0), pos2(498.0, 576.0));
    for point in [page.min, page.max, page.center()] {
        assert!(project_point(point, page, 0.0).distance(point) < 1e-4);
        assert!(project_point(point, page, 1.0).distance(point) < 1e-4);
    }
    let mut mesh = egui::epaint::Mesh::default();
    mesh.add_colored_rect(page.expand(30.0), Color32::WHITE);
    for progress in [0.0, 0.25, 0.49, 0.5, 0.51, 0.75, 1.0] {
        let projected = project_mesh(&mesh, page, page, progress);
        assert!(projected.is_valid());
        assert!(projected.vertices.iter().all(|v| v.pos.is_finite()));
    }
}

#[test]
fn native_render_light_dark_bypass_manual_and_flip() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let mut h = Harness::new(theme);
        h.editor
            .meters
            .input
            .store(0.24_f32.to_bits(), Ordering::Relaxed);
        h.editor
            .meters
            .output
            .store(0.39_f32.to_bits(), Ordering::Relaxed);
        h.editor
            .meters
            .input_lufs
            .store((-24.3_f32).to_bits(), Ordering::Relaxed);
        for (name, progress, bypass) in [
            ("controls", 0.0, false),
            ("bypass", 0.0, true),
            ("manual", 1.0, false),
            ("flip-front", 0.16, false),
            ("flip-back", 0.68, false),
        ] {
            let ctx = h.editor.context.clone().unwrap();
            let setter = ctx.param_setter();
            setter.begin_set_parameter(&h.editor.params.bypass);
            setter.set_parameter(&h.editor.params.bypass, bypass);
            setter.end_set_parameter(&h.editor.params.bypass);
            h.editor.flip = PageFlip {
                open: progress > 0.0,
                progress,
            };
            let out = h.frame(vec![]);
            let shapes = out.shapes.clone();
            let primitives = h.ctx.tessellate(shapes, out.pixels_per_point);
            assert!(primitives.iter().all(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.is_valid(),
                _ => false,
            }));
            if let Ok(dir) = std::env::var("REVELER_UI_CAPTURE_DIR") {
                capture(
                    &h.ctx,
                    &primitives,
                    &std::path::Path::new(&dir).join(format!("{theme:?}-{name}.mesh")),
                );
            }
        }
    }
}

/// Export actual native meshes and font atlas for the optional software raster QA tool.
fn capture(ctx: &egui::Context, primitives: &[egui::ClippedPrimitive], path: &std::path::Path) {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = std::fs::File::create(path).unwrap();
    let atlas = ctx.fonts(|fonts| fonts.image());
    for n in [
        WIDTH as u32,
        EXPANDED_HEIGHT as u32,
        atlas.size[0] as u32,
        atlas.size[1] as u32,
        primitives.len() as u32,
    ] {
        f.write_all(&n.to_le_bytes()).unwrap();
    }
    for color in &atlas.pixels {
        f.write_all(&color.to_array()).unwrap();
    }
    for primitive in primitives {
        let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
            unreachable!()
        };
        for n in [
            primitive.clip_rect.min.x,
            primitive.clip_rect.min.y,
            primitive.clip_rect.max.x,
            primitive.clip_rect.max.y,
        ] {
            f.write_all(&n.to_le_bytes()).unwrap();
        }
        for n in [mesh.vertices.len() as u32, mesh.indices.len() as u32] {
            f.write_all(&n.to_le_bytes()).unwrap();
        }
        for v in &mesh.vertices {
            for n in [v.pos.x, v.pos.y, v.uv.x, v.uv.y] {
                f.write_all(&n.to_le_bytes()).unwrap();
            }
            f.write_all(&v.color.to_array()).unwrap();
        }
        for i in &mesh.indices {
            f.write_all(&i.to_le_bytes()).unwrap();
        }
    }
}
