//! Shared native settings tokens, font families and static Fluent icons.
//!
//! Resources are owned by the settings process: closing it also drops its font
//! atlas and the small, lazily-created icon textures.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Rect, Response, Sense, Stroke, TextureHandle,
    TextureOptions, Ui, Vec2,
};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub line: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub danger: Color32,
    pub good: Color32,
}

impl Palette {
    pub fn of(ui: &Ui) -> Self {
        Self::for_theme(ui.visuals().dark_mode)
    }

    fn for_theme(dark: bool) -> Self {
        if dark {
            Self {
                bg: Color32::from_rgb(24, 26, 32),
                surface: Color32::from_rgb(32, 35, 42),
                raised: Color32::from_rgb(40, 44, 52),
                line: Color32::from_rgb(56, 61, 71),
                text: Color32::from_rgb(238, 241, 245),
                muted: Color32::from_rgb(164, 171, 182),
                accent: Color32::from_rgb(243, 201, 90),
                accent_hover: Color32::from_rgb(255, 219, 122),
                accent_soft: Color32::from_rgb(63, 55, 36),
                danger: Color32::from_rgb(245, 133, 137),
                good: Color32::from_rgb(116, 211, 165),
            }
        } else {
            Self {
                bg: Color32::from_rgb(246, 247, 249),
                surface: Color32::WHITE,
                raised: Color32::from_rgb(238, 240, 244),
                line: Color32::from_rgb(216, 220, 227),
                text: Color32::from_rgb(33, 38, 47),
                muted: Color32::from_rgb(94, 103, 118),
                accent: Color32::from_rgb(127, 87, 12),
                accent_hover: Color32::from_rgb(101, 65, 5),
                accent_soft: Color32::from_rgb(250, 239, 207),
                danger: Color32::from_rgb(181, 53, 64),
                good: Color32::from_rgb(32, 119, 78),
            }
        }
    }
}

/// Use an actual medium font face, rather than simulated bold or a thin default.
pub fn font(size: f32, strong: bool) -> FontId {
    FontId::new(
        size,
        if strong {
            FontFamily::Name("lolping-medium".into())
        } else {
            FontFamily::Proportional
        },
    )
}

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let palette = Palette::for_theme(theme == egui::Theme::Dark);
        let mut style = (*ctx.style_of(theme)).clone();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.spacing.interact_size = Vec2::new(40.0, 36.0);
        style.spacing.window_margin = egui::Margin::same(24);
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.slider_width = 180.0;
        style.spacing.slider_rail_height = 4.0;
        style.spacing.combo_width = 172.0;
        style.spacing.icon_width = 20.0;
        style.spacing.icon_width_inner = 12.0;
        style.spacing.icon_spacing = 8.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating_width = 3.0;
        style.spacing.scroll.bar_inner_margin = 6.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, font(16.0, false));
        style
            .text_styles
            .insert(egui::TextStyle::Button, font(16.0, true));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, font(28.0, true));
        style
            .text_styles
            .insert(egui::TextStyle::Small, font(14.0, false));
        style.animation_time = 0.12;
        let visuals = &mut style.visuals;
        visuals.override_text_color = Some(palette.text);
        visuals.weak_text_color = Some(palette.muted);
        visuals.panel_fill = palette.bg;
        visuals.window_fill = palette.surface;
        visuals.window_stroke = Stroke::new(1.0, palette.line);
        visuals.window_corner_radius = CornerRadius::same(8);
        visuals.menu_corner_radius = CornerRadius::same(8);
        visuals.extreme_bg_color = palette.bg;
        visuals.text_edit_bg_color = Some(palette.bg);
        visuals.faint_bg_color = palette.raised;
        visuals.code_bg_color = palette.raised;
        visuals.hyperlink_color = palette.accent;
        visuals.warn_fg_color = palette.accent;
        visuals.error_fg_color = palette.danger;
        visuals.selection.bg_fill = palette.accent_soft;
        visuals.selection.stroke = Stroke::new(1.5, palette.accent);
        visuals.slider_trailing_fill = true;
        visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
        visuals.disabled_alpha = 0.45;
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.bg_fill = palette.raised;
            widget.weak_bg_fill = palette.raised;
            widget.bg_stroke = Stroke::new(1.0, palette.line);
            widget.fg_stroke = Stroke::new(1.5, palette.text);
            widget.corner_radius = CornerRadius::same(8);
            widget.expansion = 0.0;
        }
        visuals.widgets.noninteractive.bg_fill = palette.surface;
        visuals.widgets.noninteractive.weak_bg_fill = palette.surface;
        visuals.widgets.hovered.bg_fill = palette.accent_soft;
        visuals.widgets.hovered.weak_bg_fill = palette.accent_soft;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, palette.accent_hover);
        visuals.widgets.active.bg_fill = palette.accent_soft;
        visuals.widgets.active.weak_bg_fill = palette.accent_soft;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.5, palette.accent);
        visuals.widgets.open = visuals.widgets.active;
        ctx.set_style_of(theme, style);
    }
    // The persisted theme is applied when the initial state arrives over IPC.
    // Starting dark avoids a bright default frame while that request is pending.
    ctx.set_theme(egui::ThemePreference::Dark);
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let embedded: [(&str, &[u8]); 4] = [
        (
            "lolping-latin",
            include_bytes!("../../../resources/fonts/LOLPingLatin-Regular.ttf"),
        ),
        (
            "lolping-latin-medium",
            include_bytes!("../../../resources/fonts/LOLPingLatin-Medium.ttf"),
        ),
        (
            "lolping-cjk",
            include_bytes!("../../../resources/fonts/LOLPingUI-Regular.otf"),
        ),
        (
            "lolping-cjk-medium",
            include_bytes!("../../../resources/fonts/LOLPingUI-Medium.otf"),
        ),
    ];
    for (name, bytes) in embedded {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    let mut regular = vec!["lolping-latin".into(), "lolping-cjk".into()];
    let mut medium = vec!["lolping-latin-medium".into(), "lolping-cjk-medium".into()];
    // Use consistent bundled faces for app labels on all platforms. A system
    // CJK face remains a fallback for names imported from the user's library.
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            "C:/Windows/Fonts/msyh.ttc",
            "C:/Windows/Fonts/msyh.ttf",
            "C:/Windows/Fonts/simhei.ttf",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansSC-Regular.ttf",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        ]
    };
    if let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) {
        fonts.font_data.insert(
            "system-cjk".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        regular.push("system-cjk".into());
        medium.push("system-cjk".into());
    }
    let defaults = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    regular.extend(defaults.clone());
    medium.extend(defaults);
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts
        .families
        .insert(FontFamily::Name("lolping-medium".into()), medium);
    if let Some(monospace) = fonts.families.get_mut(&FontFamily::Monospace) {
        monospace.push("lolping-cjk".into());
        if fonts.font_data.contains_key("system-cjk") {
            monospace.push("system-cjk".into());
        }
    }
    ctx.set_fonts(fonts);
}

pub fn icon(ui: &mut Ui, name: &str, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui, rect, name, color);
    response
}

/// Paint an icon inside a navigation button without adding another layout item.
/// The fixed icon set is decoded only on first use and tinted at paint time.
pub fn paint_icon(ui: &Ui, rect: Rect, name: &str, color: Color32) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let Some(bytes) = icon_bytes(name) else {
        return;
    };
    let id = egui::Id::unique(("lolping-fluent-icon", name));
    let texture = ui.ctx().data_mut(|data| data.get_temp::<TextureHandle>(id));
    let texture = texture.unwrap_or_else(|| {
        let pixels = image::load_from_memory(bytes)
            .expect("bundled Fluent icon is a valid PNG")
            .into_rgba8();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            pixels.as_raw(),
        );
        let texture =
            ui.ctx()
                .load_texture(format!("fluent-{name}"), image, TextureOptions::LINEAR);
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        color,
    );
}

fn icon_bytes(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "trigger" => include_bytes!("../../../resources/icons/trigger.png"),
        "hotkey" => include_bytes!("../../../resources/icons/hotkey.png"),
        "sound" => include_bytes!("../../../resources/icons/sound.png"),
        "wheel" => include_bytes!("../../../resources/icons/wheel.png"),
        "emotes" => include_bytes!("../../../resources/icons/emotes.png"),
        "app" => include_bytes!("../../../resources/icons/app.png"),
        "play" => include_bytes!("../../../resources/icons/play.png"),
        "pause" => include_bytes!("../../../resources/icons/pause.png"),
        "stop" => include_bytes!("../../../resources/icons/stop.png"),
        "check" => include_bytes!("../../../resources/icons/check.png"),
        "download" => include_bytes!("../../../resources/icons/download.png"),
        "upload" => include_bytes!("../../../resources/icons/upload.png"),
        "search" => include_bytes!("../../../resources/icons/search.png"),
        "record" => include_bytes!("../../../resources/icons/record.png"),
        "close" => include_bytes!("../../../resources/icons/close.png"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color32) -> f32 {
        let linear = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }

    #[test]
    fn text_tokens_meet_normal_text_contrast_on_content_surfaces() {
        for dark in [true, false] {
            let palette = Palette::for_theme(dark);
            for foreground in [
                palette.text,
                palette.muted,
                palette.accent,
                palette.danger,
                palette.good,
            ] {
                for background in [palette.bg, palette.surface, palette.raised] {
                    let a = luminance(foreground);
                    let b = luminance(background);
                    let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(
                        ratio >= 4.5,
                        "theme dark={dark}, {foreground:?} on {background:?}: {ratio}"
                    );
                }
            }
        }
    }
}
