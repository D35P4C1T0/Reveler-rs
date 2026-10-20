//! Compact Swiss editor. All animation, font loading and painting stay on the GUI thread.
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use egui::{
    Align2, Color32, FontFamily, FontId, Id, Rect, Response, Sense, Stroke, Ui, UiBuilder, pos2,
    vec2,
};
use nice_plug::prelude::*;
use nice_plug_core::context::gui::{GuiContext, ParamSetter};
use nice_plug_egui::{Frame, NiceEguiApp};

use crate::{Meters, RevelerParams};

pub(crate) const WIDTH: f32 = 520.0;
pub(crate) const EXPANDED_HEIGHT: f32 = 590.0;
const COLLAPSED_HEIGHT: f32 = 388.0;
const FLIP_SECONDS: f32 = 0.44;
const ACCENT: Color32 = Color32::from_rgb(223, 72, 44);

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    fg: Color32,
    muted: Color32,
    line: Color32,
    track: Color32,
}
impl Palette {
    fn for_ui(ui: &Ui) -> Self {
        let rgb = Color32::from_rgb;
        if ui.visuals().dark_mode {
            Self {
                bg: rgb(29, 30, 28),
                fg: rgb(238, 238, 230),
                muted: rgb(170, 169, 158),
                line: rgb(68, 70, 62),
                track: rgb(53, 55, 47),
            }
        } else {
            Self {
                bg: rgb(244, 243, 238),
                fg: rgb(32, 35, 31),
                muted: rgb(98, 102, 94),
                line: rgb(204, 206, 196),
                track: rgb(222, 223, 213),
            }
        }
    }
}

/// Linear progress makes interruption reversible without a jump; easing is applied at paint time.
#[derive(Default)]
struct PageFlip {
    open: bool,
    progress: f32,
}
impl PageFlip {
    fn advance(&mut self, dt: f32) {
        let step = dt.clamp(0.0, 0.05) / FLIP_SECONDS;
        self.progress = if self.open {
            (self.progress + step).min(1.0)
        } else {
            (self.progress - step).max(0.0)
        };
    }
    fn moving(&self) -> bool {
        self.progress > 0.0 && self.progress < 1.0
    }
    fn eased(&self) -> f32 {
        // CSS cubic-bezier(.4, 0, .2, 1), matching the approved preview.
        let x = self.progress;
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..16 {
            let t = (lo + hi) * 0.5;
            let bx = 3.0 * (1.0 - t).powi(2) * t * 0.4 + 3.0 * (1.0 - t) * t * t * 0.2 + t.powi(3);
            if bx < x {
                lo = t;
            } else {
                hi = t;
            }
        }
        let t = (lo + hi) * 0.5;
        3.0 * (1.0 - t) * t * t + t.powi(3)
    }
}

pub(crate) struct RevelerEditor {
    params: Arc<RevelerParams>,
    meters: Arc<Meters>,
    context: Option<GuiContext>,
    flip: PageFlip,
    more: bool,
    first_frame: bool,
}
impl RevelerEditor {
    pub(crate) fn new(params: Arc<RevelerParams>, meters: Arc<Meters>) -> Self {
        Self {
            params,
            meters,
            context: None,
            flip: PageFlip::default(),
            more: true,
            first_frame: true,
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let c = Palette::for_ui(ui);
        let window = ui.max_rect();
        if self.first_frame {
            self.more = window.height() > COLLAPSED_HEIGHT + 1.0;
            self.first_frame = false;
        }
        ui.painter().rect_filled(window, 0, c.bg);
        let content =
            Rect::from_min_max(window.min + vec2(22.0, 20.0), window.max - vec2(22.0, 14.0));
        let header = Rect::from_min_size(content.min, vec2(content.width(), 50.0));
        let p = ui.painter();
        let brand = p.text(
            header.min + vec2(0.0, 2.0),
            Align2::LEFT_TOP,
            "reveler",
            font(26.0),
            c.fg,
        );
        p.text(
            pos2(brand.right() - 1.0, header.top() + 2.0),
            Align2::LEFT_TOP,
            ".",
            font(26.0),
            ACCENT,
        );
        p.hline(
            header.x_range(),
            header.bottom() - 1.0,
            Stroke::new(2.0, c.fg),
        );

        let book = Rect::from_min_size(pos2(header.right() - 84.0, header.top()), vec2(32.0, 32.0));
        let book_response = ui.interact(book, Id::new("manual-button"), Sense::click());
        if book_response.clicked() {
            self.flip.open = !self.flip.open;
        }
        book_response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Manual"));
        book_response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if self.flip.open {
                "Close manual"
            } else {
                "Manual"
            });
        paint_book(ui, book, if self.flip.open { ACCENT } else { c.muted });

        let bypass = self.params.bypass.value();
        let toggle =
            Rect::from_min_size(pos2(header.right() - 44.0, header.top()), vec2(44.0, 32.0));
        let toggle_response = ui.interact(toggle, Id::new("processing-toggle"), Sense::click());
        if toggle_response.clicked()
            && let Some(ctx) = &self.context
        {
            let setter = ctx.param_setter();
            setter.begin_set_parameter(&self.params.bypass);
            setter.set_parameter(&self.params.bypass, !bypass);
            setter.end_set_parameter(&self.params.bypass);
        }
        toggle_response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                true,
                !bypass,
                "Processing enabled",
            )
        });
        toggle_response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if bypass {
                "Enable processing"
            } else {
                "Bypass processing"
            });
        let track = Rect::from_center_size(toggle.center(), vec2(36.0, 20.0));
        ui.painter().rect_filled(
            track,
            10,
            if bypass {
                Color32::from_rgb(75, 77, 71)
            } else {
                ACCENT
            },
        );
        ui.painter().circle_filled(
            pos2(
                track.left() + if bypass { 10.0 } else { 26.0 },
                track.center().y,
            ),
            7.0,
            Color32::from_rgb(244, 243, 238),
        );

        self.flip.advance(ui.input(|i| i.stable_dt));
        let page = Rect::from_min_max(pos2(content.left(), header.bottom()), content.max);
        // Render on the existing layer, then replace only this page's shapes during a flip.
        // Stationary frames use ordinary egui text/meshes, with no extra tessellation.
        let layer = ui.layer_id();
        let start = ui.ctx().graphics_mut(|g| g.entry(layer).next_idx());
        let mut page_ui = ui.new_child(UiBuilder::new().id_salt("page").max_rect(page));
        page_ui.set_clip_rect(page.intersect(ui.clip_rect()));
        if self.flip.moving() {
            page_ui.disable();
            page_ui.set_opacity(1.0);
        }
        page_ui.painter().rect_filled(page, 0, c.bg);
        if self.flip.eased() >= 0.5 {
            self.manual(&mut page_ui, c);
        } else {
            if bypass {
                page_ui.set_opacity(0.45);
            }
            self.controls(&mut page_ui, page, c);
        }
        if self.flip.moving() {
            flip_page(ui, page, layer, start, self.flip.eased());
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        } else if !self.flip.open {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    fn controls(&mut self, ui: &mut Ui, page: Rect, c: Palette) {
        let Some(context) = self.context.clone() else {
            return;
        };
        let setter = context.param_setter();
        let x = page.left();
        let y = page.top();
        let w = page.width();
        let half = (w - 20.0) * 0.5;
        for (offset, label, meter, color) in [
            (0.0, "INPUT", &self.meters.input, c.fg),
            (half + 20.0, "OUTPUT", &self.meters.output, ACCENT),
        ] {
            let db = peak_db(meter);
            text(ui, pos2(x + offset, y + 14.0), label, 11.0, c.fg);
            right_text(
                ui,
                pos2(x + offset + half, y + 14.0),
                &format_db(db, "dBFS"),
                11.0,
                c.fg,
            );
            let bar = Rect::from_min_size(pos2(x + offset, y + 35.0), vec2(half, 5.0));
            ui.painter().rect_filled(bar, 0, c.track);
            let amount = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            ui.painter().rect_filled(
                Rect::from_min_size(bar.min, vec2(bar.width() * amount, 5.0)),
                0,
                color,
            );
        }
        ui.painter()
            .hline(page.x_range(), y + 54.0, Stroke::new(1.0, c.line));
        for (offset, label, value) in [
            (
                0.0,
                "Detector",
                format_db(read(&self.meters.input_lufs), "LUFS"),
            ),
            (
                w * 0.43,
                "Gain",
                format!("{:+.1} dB", read(&self.meters.gain_db)),
            ),
            (
                w * 0.76,
                "Limiter",
                format!("{:.1} dB", read(&self.meters.limiter_db)),
            ),
        ] {
            let r = ui.painter().text(
                pos2(x + offset, y + 65.0),
                Align2::LEFT_TOP,
                label,
                font(11.0),
                c.muted,
            );
            text(ui, pos2(r.right() + 5.0, y + 65.0), &value, 11.0, c.fg);
        }
        let main_y = y + 108.0;
        let left_w = (w - 24.0) * 1.35 / 2.35;
        parameter(
            ui,
            &self.params.target,
            &setter,
            Rect::from_min_size(pos2(x, main_y), vec2(left_w, 146.0)),
            true,
            true,
            c,
        );
        let side_x = x + left_w + 24.0;
        let side_w = w - left_w - 24.0;
        parameter(
            ui,
            &self.params.true_peak,
            &setter,
            Rect::from_min_size(pos2(side_x, main_y), vec2(side_w, 65.0)),
            false,
            true,
            c,
        );
        parameter(
            ui,
            &self.params.max_gain,
            &setter,
            Rect::from_min_size(pos2(side_x, main_y + 83.0), vec2(side_w, 65.0)),
            false,
            true,
            c,
        );

        let summary_y = y + 278.0;
        ui.painter()
            .hline(page.x_range(), summary_y, Stroke::new(1.0, c.line));
        let summary = Rect::from_min_size(pos2(x, summary_y), vec2(w, 40.0));
        let response = ui.interact(summary, Id::new("more-controls"), Sense::click());
        if response.clicked() {
            self.more = !self.more;
            ui.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(
                WIDTH,
                if self.more {
                    EXPANDED_HEIGHT
                } else {
                    COLLAPSED_HEIGHT
                },
            )));
        }
        let tip = pos2(x + 4.0, summary_y + 20.0);
        let points = if self.more {
            vec![
                tip + vec2(-3.0, -2.0),
                tip + vec2(3.0, -2.0),
                tip + vec2(0.0, 2.0),
            ]
        } else {
            vec![
                tip + vec2(-2.0, -3.0),
                tip + vec2(-2.0, 3.0),
                tip + vec2(2.0, 0.0),
            ]
        };
        ui.painter()
            .add(egui::Shape::convex_polygon(points, c.fg, Stroke::NONE));
        text(
            ui,
            pos2(x + 15.0, summary_y + 13.0),
            "MORE CONTROLS",
            11.0,
            c.muted,
        );
        if !self.more {
            return;
        }
        let gw = (w - 20.0) * 0.5;
        let gy = summary.bottom() + 4.0;
        for (index, param) in [
            &self.params.freeze,
            &self.params.input,
            &self.params.correction_high,
            &self.params.correction_low,
            &self.params.lfe_gain,
        ]
        .into_iter()
        .enumerate()
        {
            let rect = Rect::from_min_size(
                pos2(
                    x + (index % 2) as f32 * (gw + 20.0),
                    gy + (index / 2) as f32 * 61.0,
                ),
                vec2(gw, 49.0),
            );
            parameter(ui, param, &setter, rect, false, false, c);
        }
        let mix = Rect::from_min_size(pos2(x + gw + 20.0, gy + 122.0), vec2(gw, 54.0));
        text(ui, mix.min, "MIX MODE", 11.0, c.muted);
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt("mix")
                .max_rect(Rect::from_min_max(mix.min + vec2(0.0, 21.0), mix.max)),
        );
        child.visuals_mut().widgets.inactive.weak_bg_fill = c.bg;
        child.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, c.line);
        child.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0, c.fg);
        child.visuals_mut().widgets.hovered.weak_bg_fill = c.track;
        child.visuals_mut().widgets.hovered.fg_stroke = Stroke::new(1.0, c.fg);
        child.visuals_mut().window_fill = c.bg;
        child.visuals_mut().override_text_color = Some(c.fg);
        let old = self.params.mix_mode.value();
        let mut selected = old;
        egui::ComboBox::from_id_salt("mode")
            .width(gw)
            .selected_text(MIX_MODES[old as usize])
            .show_ui(&mut child, |ui| {
                for (i, label) in MIX_MODES.iter().enumerate() {
                    ui.selectable_value(&mut selected, i as i32, *label);
                }
            });
        if selected != old {
            setter.begin_set_parameter(&self.params.mix_mode);
            setter.set_parameter(&self.params.mix_mode, selected);
            setter.end_set_parameter(&self.params.mix_mode);
        }
    }

    fn manual(&self, ui: &mut Ui, c: Palette) {
        egui::ScrollArea::vertical().id_salt("manual-scroll").auto_shrink([false,false]).show(ui,|ui| {
            ui.add_space(16.0);
            ui.label(egui::RichText::new("Manual").size(16.0).color(c.fg));
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Reveler automatically evens out changes in loudness, then limits true peaks. It uses frequency-weighted detection and lookahead to adjust gain smoothly. Set a target level, a peak ceiling, and the maximum gain it may apply.").size(13.0).line_height(Some(19.5)).color(c.muted));
            for (heading,body) in MANUAL {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(*heading).size(13.0).color(c.fg));
                ui.label(egui::RichText::new(*body).size(13.0).line_height(Some(19.5)).color(c.muted));
            }
            ui.add_space(14.0);
            ui.label(egui::RichText::new("Drag a slider or type its value. Double-click a slider to reset. Shift-drag for fine adjustment.").size(13.0).line_height(Some(19.5)).color(c.muted));
            ui.add_space(12.0);
        });
    }
}

impl NiceEguiApp for RevelerEditor {
    fn build(
        &mut self,
        ctx: egui::Context,
        context: GuiContext,
        _: &mut Frame,
    ) -> Result<(), nice_plug_egui::baseview::HandlerError> {
        configure(&ctx);
        self.first_frame = true;
        self.context = Some(context);
        Ok(())
    }
    fn ui(&mut self, ui: &mut Ui, _: &mut Frame) {
        self.draw(ui);
    }
    fn editor_closed(&mut self) {
        self.context = None;
        self.flip = PageFlip::default();
    }
}

fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
fn text(ui: &Ui, at: egui::Pos2, value: &str, size: f32, color: Color32) {
    ui.painter()
        .text(at, Align2::LEFT_TOP, value, font(size), color);
}
fn right_text(ui: &Ui, at: egui::Pos2, value: &str, size: f32, color: Color32) {
    ui.painter()
        .text(at, Align2::RIGHT_TOP, value, font(size), color);
}
fn read(v: &AtomicU32) -> f32 {
    f32::from_bits(v.load(Ordering::Relaxed))
}
fn peak_db(v: &AtomicU32) -> f32 {
    let a = read(v);
    if a > 1e-6 {
        20.0 * a.log10()
    } else {
        f32::NEG_INFINITY
    }
}
fn format_db(v: f32, unit: &str) -> String {
    if v.is_finite() {
        format!("{v:.1} {unit}")
    } else {
        format!("−∞ {unit}")
    }
}

fn configure(ctx: &egui::Context) {
    // Use the platform's Swiss sans; don't redistribute proprietary system font files.
    let paths: &[&str] = if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/Helvetica.ttc"]
    } else if cfg!(target_os = "windows") {
        &["C:/Windows/Fonts/arial.ttf"]
    } else {
        &[
            "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
    };
    let mut fonts = egui::FontDefinitions::default();
    for path in paths {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("swiss".into(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts
                .families
                .get_mut(&FontFamily::Proportional)
                .unwrap()
                .insert(0, "swiss".into());
            break;
        }
    }
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = vec2(4.0, 4.0);
        style.spacing.button_padding = vec2(4.0, 3.0);
        style.animation_time = 0.0;
        for text_style in [egui::TextStyle::Body, egui::TextStyle::Button] {
            style.text_styles.insert(text_style, font(13.0));
        }
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::ZERO;
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::ZERO;
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::ZERO;
        style.visuals.selection.bg_fill = ACCENT;
    });
}

fn paint_book(ui: &Ui, rect: Rect, color: Color32) {
    let center = rect.center();
    let stroke = Stroke::new(1.3, color);
    for sign in [-1.0, 1.0] {
        let points = vec![
            center + vec2(0.0, -5.0),
            center + vec2(sign * 3.0, -6.0),
            center + vec2(sign * 8.0, -6.0),
            center + vec2(sign * 8.0, 5.0),
            center + vec2(sign * 3.0, 5.0),
            center + vec2(0.0, 6.0),
            center + vec2(0.0, -5.0),
        ];
        ui.painter().add(egui::Shape::line(points, stroke));
    }
}

/// Host gestures bracket a whole drag; text entry, keyboard and resets are discrete edits.
fn apply_edit(
    param: &FloatParam,
    setter: &ParamSetter<'_>,
    response: &Response,
    value: f32,
    changed: bool,
) {
    if response.drag_started() {
        setter.begin_set_parameter(param);
    }
    if changed {
        let discrete = !response.dragged() && !response.drag_stopped();
        if discrete {
            setter.begin_set_parameter(param);
        }
        setter.set_parameter(param, value);
        if discrete {
            setter.end_set_parameter(param);
        }
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }
}

#[allow(clippy::too_many_arguments)]
fn parameter(
    ui: &mut Ui,
    param: &FloatParam,
    setter: &ParamSetter<'_>,
    rect: Rect,
    hero: bool,
    limits: bool,
    c: Palette,
) {
    let id = Id::new(param.name());
    let label = match param.name() {
        "Correction High" => "HIGH CORRECTION".to_owned(),
        "Correction Low" => "LOW CORRECTION".to_owned(),
        name => name.to_uppercase(),
    };
    text(ui, rect.min, &label, 11.0, c.muted);
    let value_rect = if hero {
        Rect::from_min_size(rect.min + vec2(0.0, 22.0), vec2(rect.width(), 57.0))
    } else {
        Rect::from_min_size(
            pos2(rect.right() - 100.0, rect.top() - 4.0),
            vec2(100.0, 22.0),
        )
    };
    let unit = param.unit().trim();
    let unit_width = if hero {
        0.0
    } else {
        ui.painter()
            .layout_no_wrap(unit.to_owned(), font(11.0), c.muted)
            .size()
            .x
            + 5.0
    };
    let mut value_ui = ui.new_child(
        UiBuilder::new()
            .id(id.with("value"))
            .max_rect(Rect::from_min_max(
                value_rect.min,
                pos2(value_rect.right() - unit_width, value_rect.bottom()),
            ))
            .layout(if hero {
                egui::Layout::left_to_right(egui::Align::Center)
            } else {
                egui::Layout::right_to_left(egui::Align::Center)
            }),
    );
    value_ui.style_mut().override_font_id = Some(font(if hero { 48.0 } else { 14.0 }));
    value_ui.spacing_mut().button_padding = vec2(0.0, 0.0);
    value_ui.visuals_mut().button_frame = false;
    value_ui.visuals_mut().override_text_color = Some(c.fg);
    let mut value = param.value();
    let response = value_ui.add(
        egui::DragValue::new(&mut value)
            .range(param.preview_plain(0.0)..=param.preview_plain(1.0))
            .speed(0.1)
            .fixed_decimals(1)
            .update_while_editing(false)
            .custom_formatter(|v, _| format!("{v:.1}")),
    );
    apply_edit(param, setter, &response, value, response.changed());
    if hero {
        text(ui, rect.min + vec2(0.0, 82.0), unit, 11.0, c.muted);
    } else {
        right_text(
            ui,
            pos2(rect.right(), rect.top() + 1.0),
            unit,
            11.0,
            c.muted,
        );
    }
    let slider_y = rect.top() + if hero { 116.0 } else { 35.0 };
    let slider = rect.shrink2(vec2(5.0, 0.0));
    let hit = Rect::from_min_max(
        pos2(rect.left(), slider_y - 12.0),
        pos2(rect.right(), slider_y + 12.0),
    );
    let response = ui.interact(hit, id.with("slider"), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::slider(ui.is_enabled(), param.value() as f64, param.name())
    });
    let mut normalized = param.unmodulated_normalized_value();
    let mut changed = false;
    if response.double_clicked() || (response.clicked() && ui.input(|i| i.modifiers.command)) {
        normalized = param.preview_normalized(param.default_plain_value());
        changed = true;
    } else if response.dragged() {
        if ui.input(|i| i.modifiers.shift) {
            normalized = (normalized + response.drag_delta().x * 0.0015).clamp(0.0, 1.0);
        } else if let Some(pos) = response.interact_pointer_pos() {
            normalized = ((pos.x - slider.left()) / slider.width()).clamp(0.0, 1.0);
        }
        changed = true;
    } else if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        normalized = ((pos.x - slider.left()) / slider.width()).clamp(0.0, 1.0);
        changed = true;
    }
    if response.has_focus() {
        let direction = ui.input(|i| {
            (i.key_pressed(egui::Key::ArrowRight) as i32
                - i.key_pressed(egui::Key::ArrowLeft) as i32) as f32
        });
        if direction != 0.0 {
            normalized = param.preview_normalized(param.value() + direction * 0.1);
            changed = true;
        }
    }
    apply_edit(
        param,
        setter,
        &response,
        param.preview_plain(normalized),
        changed,
    );
    let track = Rect::from_center_size(pos2(rect.center().x, slider_y), vec2(rect.width(), 4.0));
    ui.painter().rect_filled(track, 0, c.track);
    ui.painter().rect_filled(
        Rect::from_center_size(
            pos2(slider.left() + normalized * slider.width(), slider_y),
            vec2(10.0, 20.0),
        ),
        0,
        if hero { ACCENT } else { c.fg },
    );
    if limits {
        text(
            ui,
            pos2(rect.left(), slider_y + 14.0),
            &format!("{:.0}", param.preview_plain(0.0)),
            11.0,
            c.muted,
        );
        right_text(
            ui,
            pos2(rect.right(), slider_y + 14.0),
            &format!("{:.0}", param.preview_plain(1.0)),
            11.0,
            c.muted,
        );
    }
}

const MIX_MODES: [&str; 4] = [
    "Both linear",
    "Low log / high linear",
    "Low linear / high log",
    "Both logarithmic",
];
const MANUAL: &[(&str, &str)] = &[
    (
        "Target level · LUFS",
        "The loudness the leveler aims for. Higher values make the result louder, subject to Max gain and True peak.",
    ),
    (
        "True peak · dBTP",
        "The output ceiling for the lookahead true-peak limiter. Lower values leave more peak headroom.",
    ),
    (
        "Max gain · dB",
        "Caps the gain applied by the leveler. At 0 dB it cannot boost; positive values allow quieter material to be raised.",
    ),
    (
        "Freeze level · LUFS",
        "Restrains the detector’s adaptation during very quiet passages, helping avoid gain rising through pauses and background noise. It does not mute the signal.",
    ),
    (
        "Input level · LUFS",
        "The reference loudness used to separate high and low correction. It also defines the fixed gain when correction is reduced; it is not an input trim.",
    ),
    (
        "High correction · %",
        "How strongly the leveler corrects passages above Input level. 100% applies full leveling; 0% uses fixed gain based on Target minus Input level.",
    ),
    (
        "Low correction · %",
        "The same correction amount for passages below Input level. High and low refer to loudness, not frequency bands.",
    ),
    (
        "LFE gain · dB",
        "Sets gain independently for the LFE channel in 5.1. LFE is excluded from loudness detection. This control has no effect in mono or stereo.",
    ),
    (
        "Mix mode",
        "Chooses linear-amplitude or logarithmic-gain blending between fixed gain and full correction, separately above and below Input level. It changes the response at partial correction amounts; it is not a wet/dry mix.",
    ),
    (
        "Processing toggle",
        "Accent: processing on. Grey: latency-compensated bypass, so the dry signal stays aligned.",
    ),
    (
        "Meters",
        "Input and output show sample peaks in dBFS. Detector shows measured loudness; Gain shows leveling gain; Limiter shows peak reduction.",
    ),
];

// The 3D flip below projects tessellated page geometry. Native text stays crisp at either end.
fn flip_page(
    ui: &Ui,
    page: Rect,
    layer: egui::LayerId,
    start: egui::layers::ShapeIdx,
    progress: f32,
) {
    let shapes = ui.ctx().graphics_mut(|g| {
        let list = g.entry(layer);
        let shapes: Vec<_> = list.all_entries().skip(start.0).cloned().collect();
        for index in start.0..start.0 + shapes.len() {
            list.reset_shape(egui::layers::ShapeIdx(index));
        }
        shapes
    });
    for primitive in ui.ctx().tessellate(shapes, ui.ctx().pixels_per_point()) {
        if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
            let projected =
                project_mesh(&mesh, primitive.clip_rect.intersect(page), page, progress);
            ui.painter()
                .with_clip_rect(page)
                .add(egui::Shape::mesh(projected));
        }
    }
}

fn project_point(point: egui::Pos2, page: Rect, progress: f32) -> egui::Pos2 {
    let angle = -std::f32::consts::PI * progress;
    let side = if progress < 0.5 { 1.0 } else { -1.0 };
    let dx = point.x - page.center().x;
    let perspective = 1400.0 / (1400.0 + dx * side * angle.sin());
    pos2(
        page.center().x + dx * angle.cos().abs() * perspective,
        page.center().y + (point.y - page.center().y) * perspective,
    )
}

/// Clip in page space before projecting: scrolled text must not leak across a turning edge.
fn project_mesh(
    mesh: &egui::epaint::Mesh,
    clip: Rect,
    page: Rect,
    progress: f32,
) -> egui::epaint::Mesh {
    use egui::epaint::Vertex;
    let mut out = egui::epaint::Mesh::with_texture(mesh.texture_id);
    for tri in mesh.indices.as_chunks::<3>().0 {
        let mut polygon = [Vertex::default(); 8];
        for i in 0..3 {
            polygon[i] = mesh.vertices[tri[i] as usize];
        }
        let mut count = 3;
        for edge in 0..4 {
            if count == 0 {
                break;
            }
            let previous = polygon;
            let old_count = count;
            count = 0;
            let distance = |v: Vertex| match edge {
                0 => v.pos.x - clip.left(),
                1 => clip.right() - v.pos.x,
                2 => v.pos.y - clip.top(),
                _ => clip.bottom() - v.pos.y,
            };
            let mut a = previous[old_count - 1];
            for b in previous.into_iter().take(old_count) {
                let da = distance(a);
                let db = distance(b);
                if (da >= 0.0) != (db >= 0.0) {
                    let t = da / (da - db);
                    let mut vertex = a;
                    vertex.pos = a.pos.lerp(b.pos, t);
                    vertex.uv = a.uv.lerp(b.uv, t);
                    let ac = a.color.to_array();
                    let bc = b.color.to_array();
                    let rgba: [u8; 4] = std::array::from_fn(|i| {
                        (ac[i] as f32 + (bc[i] as f32 - ac[i] as f32) * t).round() as u8
                    });
                    vertex.color =
                        Color32::from_rgba_premultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
                    polygon[count] = vertex;
                    count += 1;
                }
                if db >= 0.0 {
                    polygon[count] = b;
                    count += 1;
                }
                a = b;
            }
        }
        if count < 3 {
            continue;
        }
        let base = out.vertices.len() as u32;
        for mut vertex in polygon.into_iter().take(count) {
            vertex.pos = project_point(vertex.pos, page, progress);
            out.vertices.push(vertex);
        }
        for i in 1..count - 1 {
            out.indices
                .extend([base, base + i as u32, base + i as u32 + 1]);
        }
    }
    out
}

#[cfg(test)]
mod tests;
