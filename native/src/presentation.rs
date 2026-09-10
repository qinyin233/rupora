//! Shared visual language for the application chrome and document rendering.
//!
//! Colors, typography, icon geometry and font installation live together so a
//! rendering or application caller does not need to duplicate style decisions.

use std::{fs, path::PathBuf, sync::Arc};

use eframe::egui::{
    self, Align, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Layout, Margin,
    RichText, Stroke, TextStyle, Ui, Vec2, text::TextFormat,
};

use crate::{export, markdown::Heading, wysiwyg::VisualStyle};

pub(crate) const WYSIWYG_STRONG_FAMILY: &str = "rupora-wysiwyg-strong";
pub(crate) const WYSIWYG_BODY_LINE_HEIGHT: f32 = 27.0;
pub(crate) const WYSIWYG_INLINE_CODE_LINE_HEIGHT: f32 = 20.0;
pub(crate) const WYSIWYG_INLINE_CODE_VERTICAL_PADDING: f32 = 2.0;

#[derive(Clone, Copy)]
pub(crate) struct AppPalette {
    pub(crate) canvas: Color32,
    pub(crate) surface: Color32,
    pub(crate) toolbar: Color32,
    pub(crate) sidebar: Color32,
    pub(crate) text: Color32,
    pub(crate) secondary: Color32,
    pub(crate) border: Color32,
    pub(crate) accent: Color32,
    pub(crate) accent_soft: Color32,
    pub(crate) code_bg: Color32,
    pub(crate) code_keyword: Color32,
    pub(crate) code_string: Color32,
    pub(crate) code_comment: Color32,
    pub(crate) code_number: Color32,
    pub(crate) hover: Color32,
}

pub(crate) fn app_palette(dark: bool) -> AppPalette {
    if dark {
        AppPalette {
            canvas: Color32::from_rgb(24, 25, 27),
            surface: Color32::from_rgb(32, 33, 36),
            toolbar: Color32::from_rgb(29, 30, 33),
            sidebar: Color32::from_rgb(28, 29, 32),
            text: Color32::from_rgb(237, 238, 240),
            secondary: Color32::from_rgb(155, 159, 168),
            border: Color32::from_rgb(48, 50, 55),
            accent: Color32::from_rgb(92, 164, 255),
            accent_soft: Color32::from_rgb(37, 58, 83),
            code_bg: Color32::from_rgb(39, 41, 45),
            code_keyword: Color32::from_rgb(198, 149, 255),
            code_string: Color32::from_rgb(143, 203, 157),
            code_comment: Color32::from_rgb(126, 132, 146),
            code_number: Color32::from_rgb(235, 184, 116),
            hover: Color32::from_rgb(43, 45, 50),
        }
    } else {
        AppPalette {
            canvas: Color32::from_rgb(245, 245, 247),
            surface: Color32::WHITE,
            toolbar: Color32::from_rgb(249, 249, 251),
            sidebar: Color32::from_rgb(241, 242, 245),
            text: Color32::from_rgb(29, 29, 31),
            secondary: Color32::from_rgb(115, 119, 128),
            border: Color32::from_rgb(229, 230, 234),
            accent: Color32::from_rgb(0, 113, 227),
            accent_soft: Color32::from_rgb(228, 240, 254),
            code_bg: Color32::from_rgb(247, 248, 250),
            code_keyword: Color32::from_rgb(126, 65, 196),
            code_string: Color32::from_rgb(32, 128, 86),
            code_comment: Color32::from_rgb(113, 121, 132),
            code_number: Color32::from_rgb(177, 91, 22),
            hover: Color32::from_rgb(235, 237, 241),
        }
    }
}

pub(crate) fn document_page_frame(palette: AppPalette, dark: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(0.5, palette.border))
        .corner_radius(14)
        .shadow(egui::epaint::Shadow {
            offset: [0, 5],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if dark { 32 } else { 8 }),
        })
        .inner_margin(Margin::symmetric(56, 44))
}

pub(crate) fn code_block_frame(palette: AppPalette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.code_bg)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(16, 12))
}

pub(crate) fn show_code_header(ui: &mut Ui, language: Option<&str>, palette: AppPalette) {
    // Reserve a dedicated toolbar row even for an unlabelled block. The copy
    // control is overlaid here and must never cover editable code glyphs.
    ui.set_min_width(ui.available_width());
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_height(24.0);
            ui.label(
                RichText::new(language.unwrap_or("纯文本"))
                    .size(13.0)
                    .color(palette.secondary),
            );
        },
    );
    ui.add_space(8.0);
    // Keep short and empty blocks spacious in both reading and editing states.
    let body_offset = ui.next_widget_position().y - ui.min_rect().top();
    ui.set_min_height(body_offset + 54.0);
}

#[derive(Clone, Copy)]
pub(crate) enum AppIcon {
    New,
    Folder,
    Save,
    Sidebar,
    Outline,
    Theme,
    Source,
    File,
    Close,
}

impl AppIcon {
    const fn accessible_label(self) -> &'static str {
        match self {
            Self::New => "新建",
            Self::Folder => "打开",
            Self::Save => "保存",
            Self::Sidebar => "资源管理器",
            Self::Outline => "文档大纲",
            Self::Theme => "切换主题",
            Self::Source => "源码 / 所见即所得",
            Self::File => "Markdown 文档",
            Self::Close => "关闭",
        }
    }
}

pub(crate) struct AppIconButton {
    pub(crate) icon: AppIcon,
    pub(crate) selected: bool,
    pub(crate) palette: AppPalette,
    pub(crate) size: f32,
}

impl egui::Widget for AppIconButton {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::click());
        let enabled = ui.is_enabled();
        let fill = if self.selected {
            self.palette.accent_soft
        } else if response.hovered() && enabled {
            self.palette.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 6.0, fill);
        let color = if !enabled {
            self.palette.secondary.gamma_multiply(0.45)
        } else if self.selected {
            self.palette.accent
        } else {
            self.palette.secondary
        };
        paint_app_icon(
            ui.painter(),
            rect.shrink(self.size * 0.25),
            self.icon,
            color,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                self.icon.accessible_label(),
            )
        });
        response
    }
}

pub(crate) fn icon_button_widget(
    icon: AppIcon,
    selected: bool,
    palette: AppPalette,
) -> AppIconButton {
    AppIconButton {
        icon,
        selected,
        palette,
        size: 30.0,
    }
}

pub(crate) fn app_icon_button(
    ui: &mut Ui,
    icon: AppIcon,
    selected: bool,
    tooltip: &str,
    palette: AppPalette,
) -> egui::Response {
    ui.add(icon_button_widget(icon, selected, palette))
        .on_hover_text(tooltip)
}

pub(crate) fn paint_app_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: AppIcon,
    color: Color32,
) {
    let stroke = Stroke::new(1.45, color);
    let center = rect.center();
    let left = rect.left();
    let right = rect.right();
    let top = rect.top();
    let bottom = rect.bottom();
    match icon {
        AppIcon::New => {
            painter.line_segment(
                [egui::pos2(left, center.y), egui::pos2(right, center.y)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(center.x, top), egui::pos2(center.x, bottom)],
                stroke,
            );
        }
        AppIcon::Folder => {
            painter.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(left, top + 3.0),
                    egui::pos2(left + 5.0, top + 3.0),
                    egui::pos2(left + 7.0, top + 5.0),
                    egui::pos2(right, top + 5.0),
                    egui::pos2(right, bottom - 1.0),
                    egui::pos2(left, bottom - 1.0),
                ],
                stroke,
            ));
        }
        AppIcon::Save => {
            painter.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(left + 1.0, top),
                    egui::pos2(right - 2.0, top),
                    egui::pos2(right, top + 2.0),
                    egui::pos2(right, bottom),
                    egui::pos2(left + 1.0, bottom),
                ],
                stroke,
            ));
            painter.line_segment(
                [
                    egui::pos2(left + 4.0, top),
                    egui::pos2(left + 4.0, top + 5.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left + 4.0, bottom - 4.0),
                    egui::pos2(right - 3.0, bottom - 4.0),
                ],
                stroke,
            );
        }
        AppIcon::Sidebar => {
            painter.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(left, top),
                    egui::pos2(right, top),
                    egui::pos2(right, bottom),
                    egui::pos2(left, bottom),
                ],
                stroke,
            ));
            painter.line_segment(
                [egui::pos2(left + 4.5, top), egui::pos2(left + 4.5, bottom)],
                stroke,
            );
        }
        AppIcon::Outline => {
            for row in 0..3 {
                let y = top + 2.0 + row as f32 * 5.0;
                painter.circle_filled(egui::pos2(left + 1.5, y), 1.15, color);
                painter.line_segment([egui::pos2(left + 5.0, y), egui::pos2(right, y)], stroke);
            }
        }
        AppIcon::Theme => {
            painter.circle_stroke(center, 3.3, stroke);
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment([center + direction * 5.3, center + direction * 7.0], stroke);
            }
        }
        AppIcon::Source => {
            painter.line_segment(
                [
                    egui::pos2(center.x - 2.0, top + 1.5),
                    egui::pos2(left, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left, center.y),
                    egui::pos2(center.x - 2.0, bottom - 1.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 2.0, top + 1.5),
                    egui::pos2(right, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(right, center.y),
                    egui::pos2(center.x + 2.0, bottom - 1.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 1.5, top),
                    egui::pos2(center.x - 1.5, bottom),
                ],
                stroke,
            );
        }
        AppIcon::File => {
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(left + 2.0, top),
                    egui::pos2(right - 4.0, top),
                    egui::pos2(right, top + 4.0),
                    egui::pos2(right, bottom),
                    egui::pos2(left + 2.0, bottom),
                    egui::pos2(left + 2.0, top),
                ],
                stroke,
            ));
            painter.line_segment(
                [
                    egui::pos2(right - 4.0, top),
                    egui::pos2(right - 4.0, top + 4.0),
                ],
                stroke,
            );
        }
        AppIcon::Close => {
            painter.line_segment(
                [
                    egui::pos2(left + 2.0, top + 2.0),
                    egui::pos2(right - 2.0, bottom - 2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(right - 2.0, top + 2.0),
                    egui::pos2(left + 2.0, bottom - 2.0),
                ],
                stroke,
            );
        }
    }
}

pub(crate) fn outline_row(ui: &mut Ui, heading: &Heading) -> bool {
    let indent = (heading.level.saturating_sub(1) as f32) * 12.0;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.selectable_label(false, &heading.text)
            .on_hover_text(format!("第 {} 行 · H{}", heading.line, heading.level))
            .clicked()
    })
    .inner
}

pub(crate) fn visual_text_format(style: VisualStyle, palette: AppPalette) -> TextFormat {
    let size = if style.code && style.marker {
        17.0
    } else if style.footnote {
        11.0
    } else {
        match (style.code, style.heading) {
            (_, 1) => 34.0,
            (_, 2) => 27.0,
            (_, 3) => 22.0,
            (_, 4) => 18.0,
            (true, _) => 15.0,
            _ => 17.0,
        }
    };
    let family = if style.code {
        FontFamily::Monospace
    } else if style.strong || style.heading > 0 || style.table_header {
        FontFamily::Name(WYSIWYG_STRONG_FAMILY.into())
    } else {
        FontFamily::Proportional
    };
    let mut format = TextFormat::simple(
        FontId::new(size, family),
        if style.code && style.marker {
            palette.accent
        } else if style.marker || style.quote {
            palette.secondary
        } else if style.link {
            palette.accent
        } else {
            palette.text
        },
    );
    format.line_height = Some(match (style.code, style.heading) {
        (_, 1) => 44.0,
        (_, 2) => 36.0,
        (_, 3) => 31.0,
        (_, 4) => WYSIWYG_BODY_LINE_HEIGHT,
        (true, _) => WYSIWYG_INLINE_CODE_LINE_HEIGHT,
        _ => WYSIWYG_BODY_LINE_HEIGHT,
    });
    if style.code {
        format.valign = Align::Center;
    } else if style.footnote {
        format.valign = Align::Min;
    }
    if style.strong || style.heading > 0 || style.table_header {
        format.extra_letter_spacing = 0.2;
    }
    if style.table_header {
        format.background = palette.code_bg.gamma_multiply(0.7);
    } else if style.table {
        format.background = palette.surface.gamma_multiply(0.98);
    }
    format.italics = style.emphasis;
    if style.strikethrough {
        format.strikethrough = Stroke::new(1.0, format.color);
    }
    if style.link {
        format.underline = Stroke::new(1.0, palette.accent);
    }
    if style.code {
        format.extra_letter_spacing = 0.1;
    }
    format
}

pub(crate) fn apply_theme(ctx: &Context, dark: bool) {
    let palette = app_palette(dark);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.secondary);
    visuals.panel_fill = palette.canvas;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.faint_bg_color = palette.hover;
    visuals.extreme_bg_color = palette.surface;
    visuals.text_edit_bg_color = Some(palette.surface);
    visuals.code_bg_color = palette.code_bg;
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent_soft;
    visuals.selection.stroke = Stroke::new(1.5, palette.accent);
    visuals.button_frame = true;
    visuals.collapsing_header_frame = false;
    visuals.indent_has_left_vline = false;

    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.noninteractive.weak_bg_fill = palette.surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);

    visuals.widgets.inactive.bg_fill = palette.surface;
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);

    visuals.widgets.hovered.bg_fill = palette.hover;
    visuals.widgets.hovered.weak_bg_fill = palette.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = palette.accent_soft;
    visuals.widgets.active.weak_bg_fill = palette.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.active.expansion = 0.0;
    visuals.widgets.open = visuals.widgets.active;

    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.global_style_mut(|style| {
        style.visuals = visuals;
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.interact_size = Vec2::new(32.0, 29.0);
        style.spacing.window_margin = Margin::same(18);
        style.spacing.scroll.bar_width = 7.0;
        style.spacing.scroll.floating_width = 2.0;
        style.spacing.scroll.foreground_color = false;
        style.spacing.scroll.active_handle_opacity = 0.35;
        style.spacing.scroll.active_background_opacity = 0.0;
        style.spacing.scroll.interact_background_opacity = 0.0;
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(32.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(15.0, FontFamily::Monospace),
        );
        for (name, size) in [
            ("rupora-title", 30.0),
            ("rupora-h2", 24.0),
            ("rupora-h3", 20.0),
            ("rupora-h4", 17.0),
        ] {
            style.text_styles.insert(
                TextStyle::Name(name.into()),
                FontId::new(size, FontFamily::Proportional),
            );
        }
    });
}

pub(crate) fn install_fonts(ctx: &Context) {
    let regular_font = export::cjk_font_candidates()
        .into_iter()
        .find_map(|path| fs::read(&path).ok().map(|bytes| (path, bytes)));
    let mut fonts = FontDefinitions::default();
    if let Some((path, bytes)) = regular_font {
        let font_name = format!("rupora-cjk-{}", path.display());
        fonts
            .font_data
            .insert(font_name.clone(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, font_name.clone());
        let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
        monospace.insert(monospace.len().min(1), font_name);
    }
    #[cfg(target_os = "windows")]
    if let Ok(bytes) = fs::read(r"C:\Windows\Fonts\seguiemj.ttf") {
        let name = "rupora-emoji".to_owned();
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }
    let mut strong_fonts = Vec::new();
    if let Some((bold_path, bold_bytes)) = cjk_bold_font_candidates()
        .into_iter()
        .find_map(|path| fs::read(&path).ok().map(|bytes| (path, bytes)))
    {
        let bold_name = format!("rupora-cjk-bold-{}", bold_path.display());
        fonts.font_data.insert(
            bold_name.clone(),
            Arc::new(FontData::from_owned(bold_bytes)),
        );
        strong_fonts.push(bold_name);
    }
    strong_fonts.extend(
        fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts
        .families
        .insert(FontFamily::Name(WYSIWYG_STRONG_FAMILY.into()), strong_fonts);
    ctx.set_fonts(fonts);
}

fn cjk_bold_font_candidates() -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        vec![
            PathBuf::from(r"C:\Windows\Fonts\msyhbd.ttc"),
            PathBuf::from(r"C:\Windows\Fonts\msyhbd.ttf"),
            PathBuf::from(r"C:\Windows\Fonts\simhei.ttf"),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
            PathBuf::from("/System/Library/Fonts/STHeiti Medium.ttc"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
        ]
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "windows")]
    use super::*;
    #[cfg(target_os = "windows")]
    use std::path::Path;

    #[cfg(target_os = "windows")]
    #[test]
    fn human_entered_emoji_has_a_real_glyph_in_body_and_code_fonts() {
        if !Path::new(r"C:\Windows\Fonts\seguiemj.ttf").is_file() {
            return;
        }
        let context = Context::default();
        install_fonts(&context);
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                assert!(
                    ui.fonts_mut(|fonts| fonts.has_glyph(&FontId::new(17.0, family.clone()), '🙂')),
                    "the emoji must not be rendered as a missing-glyph box: {family:?}"
                );
            }
        });
    }
}
