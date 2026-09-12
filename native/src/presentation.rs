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
            canvas: Color32::from_rgb(23, 25, 29),
            surface: Color32::from_rgb(31, 34, 39),
            sidebar: Color32::from_rgb(26, 29, 34),
            text: Color32::from_rgb(238, 241, 246),
            secondary: Color32::from_rgb(171, 180, 195),
            border: Color32::from_rgb(53, 59, 69),
            accent: Color32::from_rgb(115, 181, 255),
            accent_soft: Color32::from_rgb(36, 58, 84),
            code_bg: Color32::from_rgb(39, 41, 45),
            code_keyword: Color32::from_rgb(198, 149, 255),
            code_string: Color32::from_rgb(143, 203, 157),
            code_comment: Color32::from_rgb(164, 174, 190),
            code_number: Color32::from_rgb(235, 184, 116),
            hover: Color32::from_rgb(43, 45, 50),
        }
    } else {
        AppPalette {
            canvas: Color32::from_rgb(243, 244, 246),
            surface: Color32::WHITE,
            sidebar: Color32::from_rgb(236, 238, 242),
            text: Color32::from_rgb(32, 36, 44),
            secondary: Color32::from_rgb(89, 97, 112),
            border: Color32::from_rgb(217, 222, 230),
            accent: Color32::from_rgb(0, 103, 217),
            accent_soft: Color32::from_rgb(225, 236, 252),
            code_bg: Color32::from_rgb(247, 248, 250),
            code_keyword: Color32::from_rgb(126, 65, 196),
            code_string: Color32::from_rgb(25, 113, 75),
            code_comment: Color32::from_rgb(93, 104, 118),
            code_number: Color32::from_rgb(177, 91, 22),
            hover: Color32::from_rgb(235, 237, 241),
        }
    }
}

/// Shared by all document views, including narrow split panes.
pub(crate) struct DocumentPageLayout {
    pub(crate) side_margin: f32,
    pub(crate) top_margin: f32,
    pub(crate) content_width: f32,
    pub(crate) min_content_height: f32,
    horizontal_padding: i8,
    vertical_padding: i8,
}

pub(crate) fn document_page_layout(width: f32, height: f32) -> DocumentPageLayout {
    let gutter = if width < 280.0 {
        0.0
    } else if width < 640.0 {
        12.0
    } else {
        24.0
    };
    let page_width = (width - 2.0 * gutter).clamp(1.0, 880.0);
    let horizontal_padding = if page_width < 280.0 {
        12
    } else if page_width < 440.0 {
        20
    } else if page_width < 680.0 {
        32
    } else {
        56
    };
    let vertical_padding = if width < 640.0 { 24 } else { 40 };
    let top_margin = if width < 640.0 { 12.0 } else { 24.0 };
    DocumentPageLayout {
        side_margin: ((width - page_width) * 0.5).max(0.0),
        top_margin,
        content_width: (page_width - 2.0 * horizontal_padding as f32).max(1.0),
        min_content_height: (height - 2.0 * top_margin - 2.0 * vertical_padding as f32).max(80.0),
        horizontal_padding,
        vertical_padding,
    }
}

pub(crate) fn document_page_frame(
    palette: AppPalette,
    _dark: bool,
    layout: &DocumentPageLayout,
) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .inner_margin(Margin::symmetric(
            layout.horizontal_padding,
            layout.vertical_padding,
        ))
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
    Search,
    Write,
    Read,
    Split,
    More,
    Refresh,
    ArrowUp,
    ArrowDown,
    Copy,
    Check,
    ChevronRight,
    ChevronDown,
}

impl AppIcon {
    const fn accessible_label(self) -> &'static str {
        match self {
            Self::New => "新建",
            Self::Folder => "打开",
            Self::Save => "保存",
            Self::Sidebar => "文稿架",
            Self::Outline => "文档大纲",
            Self::Theme => "切换主题",
            Self::Source => "源码",
            Self::File => "Markdown 文档",
            Self::Close => "关闭",
            Self::Search => "查找",
            Self::Write => "写作",
            Self::Read => "阅读",
            Self::Split => "分栏",
            Self::More => "更多操作",
            Self::Refresh => "刷新",
            Self::ArrowUp => "上一个匹配项",
            Self::ArrowDown => "下一个匹配项",
            Self::Copy => "复制",
            Self::Check => "已完成",
            Self::ChevronRight => "展开",
            Self::ChevronDown => "折叠",
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
        let fill = if enabled && (self.selected || response.is_pointer_button_down_on()) {
            self.palette.accent_soft
        } else if response.hovered() && enabled {
            self.palette.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 6.0, fill);
        if response.has_focus() && enabled {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                Stroke::new(2.0, self.palette.accent),
                egui::StrokeKind::Inside,
            );
        }
        let color = if !enabled {
            self.palette.secondary.gamma_multiply(0.45)
        } else if self.selected {
            self.palette.accent
        } else {
            self.palette.secondary
        };
        paint_app_icon(
            ui.painter(),
            egui::Rect::from_center_size(
                rect.center(),
                Vec2::splat((self.size - 12.0).clamp(12.0, 20.0)),
            ),
            self.icon,
            color,
        );
        response.widget_info(|| {
            if matches!(
                self.icon,
                AppIcon::Sidebar
                    | AppIcon::Outline
                    | AppIcon::Source
                    | AppIcon::Write
                    | AppIcon::Read
                    | AppIcon::Split
                    | AppIcon::Search
            ) {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    enabled,
                    self.selected,
                    self.icon.accessible_label(),
                )
            } else {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    self.icon.accessible_label(),
                )
            }
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
        size: 32.0,
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

/// A single focusable target for an icon and its visible label. The icon is
/// decorative here; assistive technology receives the action once, as text.
pub(crate) fn app_action_button(
    ui: &mut Ui,
    icon: AppIcon,
    label: &str,
    selected: bool,
    palette: AppPalette,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 34.0), egui::Sense::click());
    let enabled = ui.is_enabled();
    let fill = if enabled && (selected || response.is_pointer_button_down_on()) {
        palette.accent_soft
    } else if enabled && response.hovered() {
        palette.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    if response.has_focus() && enabled {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            6.0,
            Stroke::new(2.0, palette.accent),
            egui::StrokeKind::Inside,
        );
    }
    let color = if !enabled {
        palette.secondary.gamma_multiply(0.45)
    } else if selected {
        palette.accent
    } else {
        palette.text
    };
    paint_app_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 19.0, rect.center().y),
            Vec2::splat(18.0),
        ),
        icon,
        color,
    );
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        FontId::proportional(13.0),
        color,
    );
    job.wrap.max_width = (width - 42.0).max(1.0);
    job.wrap.max_rows = 1;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.painter().galley(
        egui::pos2(rect.left() + 35.0, rect.center().y - galley.size().y * 0.5),
        galley,
        color,
    );
    response.widget_info(|| {
        if matches!(
            icon,
            AppIcon::Write | AppIcon::Read | AppIcon::Split | AppIcon::Source
        ) && label == icon.accessible_label()
        {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
        } else {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label)
        }
    });
    response
}

pub(crate) struct DocumentTabResponse {
    pub(crate) response: egui::Response,
    pub(crate) activate: bool,
    pub(crate) close: bool,
}

pub(crate) fn document_tab(
    ui: &mut Ui,
    title: &str,
    dirty: bool,
    selected: bool,
    palette: AppPalette,
    width: f32,
) -> DocumentTabResponse {
    let mut activate = false;
    let mut close = false;
    let tab = egui::Frame::new()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(0.5, Color32::TRANSPARENT))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.horizontal(|ui| {
                let (marker, _) =
                    ui.allocate_exact_size(Vec2::new(14.0, 28.0), egui::Sense::hover());
                if dirty {
                    ui.painter()
                        .circle_filled(marker.center(), 3.0, palette.accent);
                } else {
                    paint_app_icon(
                        ui.painter(),
                        egui::Rect::from_center_size(marker.center(), Vec2::splat(13.0)),
                        AppIcon::File,
                        if selected {
                            palette.accent
                        } else {
                            palette.secondary
                        },
                    );
                }
                let response = ui
                    .add_sized(
                        [(width - 71.0).max(40.0), 28.0],
                        egui::Button::new("")
                            .left_text(RichText::new(title).size(13.0).color(if selected {
                                palette.text
                            } else {
                                palette.secondary
                            }))
                            .frame(false)
                            .truncate(),
                    )
                    .on_hover_text(if dirty {
                        format!("{title}\n有未保存的修改")
                    } else {
                        title.to_owned()
                    });
                activate = response.clicked();
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        selected,
                        title,
                    )
                });
                close = ui
                    .add(AppIconButton {
                        icon: AppIcon::Close,
                        selected: false,
                        palette,
                        size: 24.0,
                    })
                    .on_hover_text(format!("关闭 {title}"))
                    .clicked();
            });
        });
    if selected {
        let rect = tab.response.rect;
        ui.painter().line_segment(
            [
                egui::pos2(rect.left() + 10.0, rect.bottom() - 1.0),
                egui::pos2(rect.right() - 10.0, rect.bottom() - 1.0),
            ],
            Stroke::new(2.0, palette.accent),
        );
    }
    DocumentTabResponse {
        response: tab.response,
        activate,
        close,
    }
}

pub(crate) fn paint_app_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: AppIcon,
    color: Color32,
) {
    // Every icon uses the same 20 pt grid. Scale the paths and strokes together
    // so a close control and a navigation icon retain the same optical weight.
    let scale = rect.width().min(rect.height()) / 20.0;
    let origin = rect.center() - Vec2::splat(10.0 * scale);
    let point = |x: f32, y: f32| origin + egui::vec2(x, y) * scale;
    let stroke = Stroke::new(1.5 * scale, color);
    let line = |points: &[(f32, f32)]| {
        let points: Vec<_> = points.iter().map(|&(x, y)| point(x, y)).collect();
        painter.add(egui::Shape::line(points.clone(), stroke));
        if let (Some(first), Some(last)) = (points.first(), points.last()) {
            painter.circle_filled(*first, stroke.width * 0.5, color);
            painter.circle_filled(*last, stroke.width * 0.5, color);
        }
    };
    let rounded_rect = |min: (f32, f32), max: (f32, f32), radius: f32| {
        painter.rect_stroke(
            egui::Rect::from_min_max(point(min.0, min.1), point(max.0, max.1)),
            radius * scale,
            stroke,
            egui::StrokeKind::Middle,
        );
    };
    match icon {
        AppIcon::New => {
            line(&[(4.0, 10.0), (16.0, 10.0)]);
            line(&[(10.0, 4.0), (10.0, 16.0)]);
        }
        AppIcon::Folder => {
            line(&[
                (3.0, 16.0),
                (2.5, 5.0),
                (7.0, 5.0),
                (9.0, 7.0),
                (17.0, 7.0),
                (17.0, 9.0),
            ]);
            line(&[
                (3.0, 16.0),
                (5.0, 9.0),
                (18.0, 9.0),
                (16.0, 16.0),
                (3.0, 16.0),
            ]);
        }
        AppIcon::Save => {
            line(&[
                (3.0, 3.0),
                (13.5, 3.0),
                (17.0, 6.5),
                (17.0, 17.0),
                (3.0, 17.0),
                (3.0, 3.0),
            ]);
            line(&[(6.0, 3.0), (6.0, 8.0), (13.0, 8.0), (13.0, 3.0)]);
            rounded_rect((6.0, 11.5), (14.0, 17.0), 1.0);
        }
        AppIcon::Sidebar => {
            rounded_rect((2.5, 3.5), (17.5, 16.5), 2.0);
            line(&[(7.5, 3.5), (7.5, 16.5)]);
            line(&[(4.5, 7.0), (5.5, 7.0)]);
            line(&[(4.5, 10.0), (5.5, 10.0)]);
        }
        AppIcon::Outline => {
            for y in [5.0, 10.0, 15.0] {
                painter.circle_filled(point(3.5, y), scale, color);
                line(&[(7.0, y), (17.0, y)]);
            }
        }
        AppIcon::Theme => {
            painter.circle_stroke(point(10.0, 10.0), 3.25 * scale, stroke);
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                let (x, y) = (angle.cos(), angle.sin());
                line(&[
                    (10.0 + x * 6.0, 10.0 + y * 6.0),
                    (10.0 + x * 7.5, 10.0 + y * 7.5),
                ]);
            }
        }
        AppIcon::Source => {
            line(&[(6.0, 5.5), (2.0, 10.0), (6.0, 14.5)]);
            line(&[(14.0, 5.5), (18.0, 10.0), (14.0, 14.5)]);
            line(&[(11.5, 3.5), (8.5, 16.5)]);
        }
        AppIcon::File => {
            line(&[
                (11.5, 2.5),
                (4.0, 2.5),
                (4.0, 17.5),
                (16.0, 17.5),
                (16.0, 7.0),
                (11.5, 2.5),
                (11.5, 7.0),
                (16.0, 7.0),
            ]);
            line(&[(7.0, 11.0), (13.0, 11.0)]);
            line(&[(7.0, 14.0), (11.0, 14.0)]);
        }
        AppIcon::Close => {
            line(&[(5.0, 5.0), (15.0, 15.0)]);
            line(&[(15.0, 5.0), (5.0, 15.0)]);
        }
        AppIcon::Search => {
            painter.circle_stroke(point(8.5, 8.5), 5.5 * scale, stroke);
            line(&[(12.5, 12.5), (17.0, 17.0)]);
        }
        AppIcon::Write => {
            line(&[
                (4.0, 13.0),
                (13.5, 3.5),
                (16.5, 6.5),
                (7.0, 16.0),
                (3.0, 17.0),
                (4.0, 13.0),
                (7.0, 16.0),
            ]);
            line(&[(11.5, 5.5), (14.5, 8.5)]);
        }
        AppIcon::Read => {
            line(&[
                (10.0, 5.0),
                (6.0, 3.5),
                (2.5, 3.5),
                (2.5, 15.0),
                (6.0, 15.0),
                (10.0, 16.5),
                (14.0, 15.0),
                (17.5, 15.0),
                (17.5, 3.5),
                (14.0, 3.5),
                (10.0, 5.0),
                (10.0, 16.5),
            ]);
        }
        AppIcon::Split => {
            rounded_rect((2.5, 3.5), (17.5, 16.5), 2.0);
            line(&[(10.0, 3.5), (10.0, 16.5)]);
        }
        AppIcon::More => {
            for x in [4.0, 10.0, 16.0] {
                painter.circle_filled(point(x, 10.0), 1.25 * scale, color);
            }
        }
        AppIcon::Refresh => {
            let points: Vec<_> = (0..=20)
                .map(|step| {
                    let angle = (-45.0 + step as f32 * 14.0).to_radians();
                    (10.0 + 6.0 * angle.cos(), 10.0 + 6.0 * angle.sin())
                })
                .collect();
            line(&points);
            line(&[(14.3, 2.7), (14.3, 5.7), (17.3, 5.7)]);
        }
        AppIcon::ArrowUp => {
            line(&[(10.0, 16.0), (10.0, 4.0)]);
            line(&[(5.5, 8.5), (10.0, 4.0), (14.5, 8.5)]);
        }
        AppIcon::ArrowDown => {
            line(&[(10.0, 4.0), (10.0, 16.0)]);
            line(&[(5.5, 11.5), (10.0, 16.0), (14.5, 11.5)]);
        }
        AppIcon::Copy => {
            rounded_rect((6.5, 6.5), (17.0, 17.0), 2.0);
            line(&[(12.5, 3.0), (5.0, 3.0), (3.0, 5.0), (3.0, 12.5)]);
        }
        AppIcon::Check => line(&[(4.0, 10.0), (8.0, 14.0), (16.0, 6.0)]),
        AppIcon::ChevronRight => line(&[(7.0, 4.5), (12.5, 10.0), (7.0, 15.5)]),
        AppIcon::ChevronDown => line(&[(4.5, 7.0), (10.0, 12.5), (15.5, 7.0)]),
    }
}

pub(crate) fn outline_row(ui: &mut Ui, heading: &Heading) -> bool {
    let indent = (heading.level.saturating_sub(1) as f32) * 12.0;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.add_sized(
            [ui.available_width(), 28.0],
            egui::Button::new("")
                .left_text(&heading.text)
                .frame(false)
                .truncate(),
        )
        .on_hover_text(format!(
            "{}\n第 {} 行 · H{}",
            heading.text, heading.line, heading.level
        ))
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
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, palette.accent);
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
        style.spacing.interact_size = Vec2::new(32.0, 30.0);
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
    use super::*;
    #[cfg(target_os = "windows")]
    use std::path::Path;

    #[test]
    fn normal_text_and_syntax_colors_remain_readable_in_both_themes() {
        fn luminance(color: Color32) -> f32 {
            let linear = |channel: u8| {
                let value = channel as f32 / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        }
        let check = |foreground, background| {
            let a = luminance(foreground);
            let b = luminance(background);
            let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(
                contrast >= 4.5,
                "{foreground:?} on {background:?}: {contrast:.2}:1"
            );
        };
        for dark in [false, true] {
            let palette = app_palette(dark);
            for background in [
                palette.surface,
                palette.canvas,
                palette.sidebar,
                palette.hover,
                palette.accent_soft,
            ] {
                for foreground in [palette.text, palette.secondary] {
                    check(foreground, background);
                }
            }
            for foreground in [
                palette.text,
                palette.code_comment,
                palette.code_keyword,
                palette.code_string,
                palette.code_number,
            ] {
                check(foreground, palette.code_bg);
            }
        }
    }

    #[test]
    fn rendered_document_page_fits_narrow_split_panes() {
        for width in [120.0, 180.0, 240.0, 340.0, 600.0, 1000.0] {
            let context = Context::default();
            let _ = context.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 560.0))),
                ..Default::default()
            }, |ui| {
                let available = ui.available_rect_before_wrap();
                let layout = document_page_layout(available.width(), available.height());
                ui.add_space(layout.top_margin);
                let frame = ui.horizontal(|ui| {
                    ui.add_space(layout.side_margin);
                    document_page_frame(app_palette(false), false, &layout).show(ui, |ui| {
                        ui.with_layout(Layout::top_down(Align::Min), |ui| {
                            ui.set_width(layout.content_width);
                            ui.set_min_height(layout.min_content_height);
                            ui.label("A narrow paragraph with enough words to wrap across multiple lines.");
                        });
                    }).response.rect
                }).inner;
                assert!(available.expand(1.0).contains_rect(frame), "width={width}: {frame:?} outside {available:?}");
            });
        }
    }

    #[test]
    fn long_document_tab_keeps_its_close_control_visible_and_clickable() {
        for selected in [false, true] {
            for dirty in [false, true] {
                let context = Context::default();
                let mut tab_rect = egui::Rect::NOTHING;
                for phase in 0..3 {
                    let position = egui::pos2(tab_rect.right() - 22.5, tab_rect.center().y);
                    let events = if phase == 0 {
                        vec![]
                    } else {
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed: phase == 1,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    };
                    let _ = context.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                        ui.horizontal(|ui| {
                            let tab = document_tab(ui, "An extremely long document title that must never hide the close control.md", dirty, selected, app_palette(false), 160.0);
                            tab_rect = tab.response.rect;
                            assert!(tab_rect.width() <= 161.0, "tab grew to {}", tab_rect.width());
                            assert_eq!(tab.close, phase == 2);
                            assert!(!tab.activate);
                        });
                    });
                }
            }
        }
    }

    #[test]
    fn icon_keyboard_focus_is_visible_without_moving_the_control() {
        let context = Context::default();
        let palette = app_palette(false);
        let mut initial = egui::Rect::NOTHING;
        for phase in 0..3 {
            let events = if phase == 2 {
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            } else {
                vec![]
            };
            let output = context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = ui.add(icon_button_widget(AppIcon::Sidebar, false, palette));
                    if phase == 0 {
                        initial = response.rect;
                        response.request_focus();
                    } else {
                        assert!(response.has_focus());
                        assert_eq!(response.rect, initial);
                        assert_eq!(response.clicked(), phase == 2);
                    }
                },
            );
            if phase > 0 {
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.color == palette.accent && rect.stroke.width >= 2.0)), "keyboard focus must be painted");
            }
        }
    }

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
