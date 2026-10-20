//! Settings-only wheel editing. All persistence and desktop previews stay in the parent UI.
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, Layout, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, TextureId, Vec2,
};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};

use super::design::{font, Palette};

pub struct WheelItem {
    pub id: String,
    pub name: String,
    pub texture: Option<TextureId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Slot(usize),
    Center,
}

pub struct EditorState {
    pub target: Target,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            target: Target::Slot(0),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Assign { target: Target, id: String },
    Preview(String),
    Size(u32),
    LoadTexture(String),
}

fn text(zh: bool, chinese: &'static str, english: &'static str) -> &'static str {
    if zh {
        chinese
    } else {
        english
    }
}

fn direction(target: Target, zh: bool) -> &'static str {
    const ZH: [&str; 8] = ["上", "右上", "右", "右下", "下", "左下", "左", "左上"];
    const EN: [&str; 8] = [
        "North",
        "North-east",
        "East",
        "South-east",
        "South",
        "South-west",
        "West",
        "North-west",
    ];
    match target {
        Target::Slot(index) => {
            if zh {
                ZH[index % 8]
            } else {
                EN[index % 8]
            }
        }
        Target::Center => text(zh, "中心", "Center"),
    }
}

fn selected_id<'a>(state: &EditorState, wheel: &'a [String], center: &'a str) -> &'a str {
    match state.target {
        Target::Slot(index) => wheel.get(index).map(String::as_str).unwrap_or(""),
        Target::Center => center,
    }
}

/// Eight clockwise sectors beginning at north. The center belongs to its own target;
/// positions outside the circle never select the nearest sector.
pub fn hit_test(offset: Vec2, outer_radius: f32, inner_radius: f32) -> Option<Target> {
    let radius = offset.length();
    if !radius.is_finite()
        || !outer_radius.is_finite()
        || !inner_radius.is_finite()
        || radius > outer_radius
        || outer_radius <= 0.0
        || inner_radius < 0.0
        || inner_radius >= outer_radius
    {
        return None;
    }
    if radius <= inner_radius {
        return Some(Target::Center);
    }
    let angle = (offset.y.atan2(offset.x) + FRAC_PI_2 + FRAC_PI_4 / 2.0).rem_euclid(TAU);
    Some(Target::Slot((angle / FRAC_PI_4).floor() as usize % 8))
}

/// Returns a new valid layout only when a selection changes. Reusing an outer item
/// swaps the two slots; the click/center item is intentionally independent.
pub fn assign(
    wheel: &[String],
    target: Target,
    center: &str,
    id: &str,
) -> Option<(Vec<String>, String)> {
    if id.is_empty() {
        return None;
    }
    match target {
        Target::Center if center != id => Some((wheel.to_vec(), id.to_owned())),
        Target::Slot(index) if wheel.len() == 8 && index < wheel.len() && wheel[index] != id => {
            let mut next = wheel.to_vec();
            if let Some(other) = next.iter().position(|value| value == id) {
                next.swap(index, other);
            } else {
                next[index] = id.to_owned();
            }
            Some((next, center.to_owned()))
        }
        _ => None,
    }
}

fn step_target(target: Target, clockwise: bool) -> Target {
    match target {
        Target::Slot(index) => Target::Slot((index + if clockwise { 1 } else { 7 }) % 8),
        Target::Center => Target::Slot(if clockwise { 0 } else { 7 }),
    }
}

fn card(palette: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.line))
        .corner_radius(14)
        .inner_margin(20.0)
}

fn caption(ui: &mut egui::Ui, label: &str, palette: Palette) {
    ui.label(
        RichText::new(label)
            .font(font(14.0, false))
            .color(palette.muted),
    );
}

/// Draws one static, interactive editor. Actions are applied by the owner after this call.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    wheel: &[String],
    center: &str,
    size: u32,
    items: &[WheelItem],
    zh: bool,
    emotes: bool,
) -> Vec<Action> {
    let palette = Palette::of(ui);
    let mut actions = Vec::new();
    ui.push_id(
        if emotes {
            "emote-editor"
        } else {
            "ping-editor"
        },
        |ui| {
            let width = ui.available_width();
            let compact = ui.ctx().content_rect().width() >= 960.0;
            if compact {
                let left = (width - 20.0) * 0.59;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.allocate_ui_with_layout(
                        vec2(left, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            canvas(ui, state, wheel, center, items, zh, emotes, &mut actions);
                        },
                    );
                    ui.add_space(20.0);
                    ui.allocate_ui_with_layout(
                        vec2(width - left - 20.0, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            parameters(
                                ui,
                                state,
                                wheel,
                                center,
                                size,
                                items,
                                zh,
                                emotes,
                                &mut actions,
                            );
                        },
                    );
                });
            } else {
                canvas(ui, state, wheel, center, items, zh, emotes, &mut actions);
                ui.add_space(16.0);
                parameters(
                    ui,
                    state,
                    wheel,
                    center,
                    size,
                    items,
                    zh,
                    emotes,
                    &mut actions,
                );
            }
            ui.add_space(if compact { 12.0 } else { 20.0 });
            card(palette)
                .inner_margin(if compact { 14.0 } else { 20.0 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = if compact { 4.0 } else { 8.0 };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(text(
                                zh,
                                if emotes {
                                    "表情素材"
                                } else {
                                    "信号素材"
                                },
                                if emotes {
                                    "Emote library"
                                } else {
                                    "Ping library"
                                },
                            ))
                            .font(font(18.0, true))
                            .color(palette.text),
                        );
                        ui.label(
                            RichText::new(format!("{:02}", items.len()))
                                .font(font(13.0, true))
                                .color(palette.muted),
                        );
                        if compact {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                caption(
                                    ui,
                                    text(
                                        zh,
                                        "点击方向，再选择素材",
                                        "Select a direction, then a tile",
                                    ),
                                    palette,
                                );
                            });
                        }
                    });
                    if !compact {
                        caption(
                    ui,
                    text(
                        zh,
                        "先选择方向，再点击素材分配；已有素材会交换位置。",
                        "Select a direction, then a tile. Items already on the wheel swap places.",
                    ),
                    palette,
                );
                    }
                    ui.add_space(if compact { 6.0 } else { 16.0 });
                    // At most four partially-visible rows of eight tiles plus nine
                    // wheel textures fit inside the shared 48-texture cache.
                    let count = if compact && !emotes && items.len() == 12 {
                        if ui.available_width() >= 960.0 {
                            12
                        } else {
                            6
                        }
                    } else {
                        (ui.available_width() / 120.0).floor().clamp(1.0, 8.0) as usize
                    };
                    let gap = if compact { 6.0 } else { 10.0 };
                    let chosen = selected_id(state, wheel, center);
                    ui.spacing_mut().item_spacing = vec2(gap, gap);
                    egui::ScrollArea::vertical()
                        .id_salt("wheel-materials")
                        .max_height(if compact { 220.0 } else { 300.0 })
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let tile_width = (ui.available_width()
                                - gap * (count.saturating_sub(1)) as f32)
                                / count as f32;
                            for row in items.chunks(count) {
                                ui.horizontal(|ui| {
                                    for item in row {
                                        let slot = wheel.iter().position(|id| id == &item.id);
                                        let response = tile(
                                            ui,
                                            item,
                                            chosen == item.id,
                                            slot,
                                            tile_width,
                                            zh,
                                            compact,
                                        );
                                        if ui.is_rect_visible(tile_image_rect(response.rect)) {
                                            request_texture(&mut actions, &item.id);
                                        }
                                        if response.gained_focus() {
                                            response.scroll_to_me(Some(Align::Center));
                                        }
                                        if response.clicked() {
                                            actions.push(Action::Assign {
                                                target: state.target,
                                                id: item.id.clone(),
                                            });
                                        }
                                    }
                                });
                            }
                        });
                    if items.is_empty() {
                        caption(
                            ui,
                            text(
                                zh,
                                "导入表情后即可分配到轮盘。",
                                "Import an emote to add it to the wheel.",
                            ),
                            palette,
                        );
                    }
                });
        },
    );
    actions
}

#[allow(clippy::too_many_arguments)]
fn canvas(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    wheel: &[String],
    center_id: &str,
    items: &[WheelItem],
    zh: bool,
    emotes: bool,
    actions: &mut Vec<Action>,
) {
    let palette = Palette::of(ui);
    let compact = ui.ctx().content_rect().width() >= 960.0;
    card(palette)
        .inner_margin(if compact { 16.0 } else { 20.0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.x = 8.0;
            if compact {
                ui.spacing_mut().item_spacing.y = 4.0;
            }
            if !compact {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(text(zh, "轮盘布局", "Wheel layout"))
                            .font(font(18.0, true))
                            .color(palette.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(text(zh, "缩放预览", "Scaled preview"))
                                .font(font(12.0, false))
                                .color(palette.muted),
                        );
                    });
                });
                ui.add_space(4.0);
            }
            let height = if compact {
                300.0
            } else {
                ui.available_width().clamp(300.0, 370.0)
            };
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
            if compact {
                ui.painter().text(
                    rect.right_top(),
                    Align2::RIGHT_TOP,
                    text(zh, "缩放预览", "Scaled preview"),
                    font(12.0, false),
                    palette.muted,
                );
            }
            if ui.is_rect_visible(rect) {
                for id in wheel
                    .iter()
                    .map(String::as_str)
                    .chain(std::iter::once(center_id))
                {
                    if items
                        .iter()
                        .any(|item| item.id == id && item.texture.is_none())
                    {
                        request_texture(actions, id);
                    }
                }
            }
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    ui.is_enabled(),
                    text(
                        zh,
                        "选择轮盘方向；方向键切换，Home 选择中心",
                        "Wheel direction: arrow keys to change, Home for center",
                    ),
                )
            });
            let origin = rect.center();
            let outer = (rect.width().min(rect.height()) * 0.5 - if compact { 8.0 } else { 30.0 })
                .max(60.0);
            let inner = outer * 0.39;
            let hover = response
                .hover_pos()
                .and_then(|position| hit_test(position - origin, outer, inner));
            if response.clicked() {
                if let Some(target) = response
                    .interact_pointer_pos()
                    .and_then(|position| hit_test(position - origin, outer, inner))
                {
                    state.target = target;
                    response.request_focus();
                }
            }
            if response.has_focus() {
                let (previous, next, center) = ui.input_mut(|input| {
                    (
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                            || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
                            || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Home),
                    )
                });
                if previous {
                    state.target = step_target(state.target, false);
                }
                if next {
                    state.target = step_target(state.target, true);
                }
                if center {
                    state.target = Target::Center;
                }
            }
            if hover.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let painter = ui.painter_at(rect);
            painter.circle_stroke(origin, outer + 6.0, Stroke::new(1.0, palette.line));
            for index in 0..8 {
                let target = Target::Slot(index);
                let selected = state.target == target;
                let hovered = hover == Some(target);
                let angle = -FRAC_PI_2 + index as f32 * FRAC_PI_4;
                let from = angle - FRAC_PI_4 * 0.5 + 0.014;
                let to = angle + FRAC_PI_4 * 0.5 - 0.014;
                let fill = if selected {
                    palette.accent_soft
                } else if hovered {
                    palette.raised
                } else {
                    palette.bg
                };
                sector(&painter, origin, inner + 4.0, outer, from, to, fill);
                if selected {
                    arc(
                        &painter,
                        origin,
                        outer,
                        from,
                        to,
                        Stroke::new(2.5, palette.accent),
                    );
                    arc(
                        &painter,
                        origin,
                        inner + 4.0,
                        from,
                        to,
                        Stroke::new(1.0, palette.accent),
                    );
                }
                let icon_center = origin + Vec2::angled(angle) * (inner + (outer - inner) * 0.56);
                if let Some(item) = wheel
                    .get(index)
                    .and_then(|id| items.iter().find(|item| &item.id == id))
                {
                    if let Some(texture) = item.texture {
                        paint_texture(
                            ui,
                            &painter,
                            texture,
                            Rect::from_center_size(icon_center, Vec2::splat(outer * 0.27)),
                        );
                    } else {
                        painter.text(
                            icon_center,
                            Align2::CENTER_CENTER,
                            "—",
                            font(20.0, false),
                            palette.muted,
                        );
                    }
                }
                let label_origin =
                    origin + Vec2::angled(angle) * (outer + if compact { -12.0 } else { 20.0 });
                let label = if zh {
                    direction(target, true)
                } else {
                    ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][index]
                };
                painter.text(
                    label_origin,
                    Align2::CENTER_CENTER,
                    label,
                    font(11.0, selected),
                    if selected {
                        palette.accent
                    } else {
                        palette.muted
                    },
                );
            }
            let selected_center = state.target == Target::Center;
            painter.circle_filled(
                origin,
                inner,
                if selected_center {
                    palette.accent_soft
                } else {
                    palette.raised
                },
            );
            painter.circle_stroke(
                origin,
                inner,
                Stroke::new(
                    if selected_center { 2.0 } else { 1.0 },
                    if selected_center {
                        palette.accent
                    } else {
                        palette.line
                    },
                ),
            );
            if let Some(texture) = items
                .iter()
                .find(|item| item.id == center_id)
                .and_then(|item| item.texture)
            {
                paint_texture(
                    ui,
                    &painter,
                    texture,
                    Rect::from_center_size(origin - vec2(0.0, 11.0), Vec2::splat(inner * 0.67)),
                );
            }
            painter.text(
                origin + vec2(0.0, inner * 0.47),
                Align2::CENTER_CENTER,
                text(
                    zh,
                    if emotes {
                        "点击表情"
                    } else {
                        "中心信号"
                    },
                    "CLICK",
                ),
                font(11.0, true),
                if selected_center {
                    palette.accent
                } else {
                    palette.muted
                },
            );
            if response.has_focus() {
                painter.rect_stroke(
                    rect.shrink(1.0),
                    10.0,
                    Stroke::new(1.0, palette.accent),
                    StrokeKind::Inside,
                );
            }
            if !compact {
                ui.add_space(4.0);
                caption(
                    ui,
                    text(
                        zh,
                        "点击扇区编辑 · 方向键切换 · Home 选择中心",
                        "Click a sector · Arrow keys to change · Home for center",
                    ),
                    palette,
                );
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn parameters(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    wheel: &[String],
    center: &str,
    size: u32,
    items: &[WheelItem],
    zh: bool,
    emotes: bool,
    actions: &mut Vec<Action>,
) {
    let palette = Palette::of(ui);
    let compact = ui.ctx().content_rect().width() >= 960.0;
    card(palette)
        .inner_margin(if compact { 16.0 } else { 20.0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.x = 8.0;
            if compact {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.spacing_mut().interact_size.y = 30.0;
                ui.set_min_height(298.0);
            } else {
                caption(ui, text(zh, "正在编辑", "EDITING"), palette);
            }
            ui.horizontal(|ui| {
                if compact {
                    caption(ui, text(zh, "正在编辑", "EDITING"), palette);
                    ui.add_space(6.0);
                }
                ui.label(
                    RichText::new(direction(state.target, zh))
                        .font(font(23.0, true))
                        .color(palette.accent),
                );
                ui.label(
                    RichText::new(text(zh, "方向", "direction"))
                        .font(font(14.0, false))
                        .color(palette.muted),
                );
            });
            ui.add_space(if compact { 6.0 } else { 12.0 });
            let id = selected_id(state, wheel, center);
            let selected = items.iter().find(|item| item.id == id);
            ui.horizontal(|ui| {
                let icon_size = if compact { 44.0 } else { 62.0 };
                let (rect, _) = ui.allocate_exact_size(vec2(icon_size, icon_size), Sense::hover());
                ui.painter().rect_filled(rect, 12.0, palette.raised);
                if let Some(texture) = selected.and_then(|item| item.texture) {
                    paint_texture(ui, ui.painter(), texture, rect.shrink(9.0));
                } else if ui.is_rect_visible(rect) && !id.is_empty() {
                    request_texture(actions, id);
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(selected.map_or(id, |item| item.name.as_str()))
                            .font(font(18.0, true))
                            .color(palette.text),
                    );
                    caption(
                        ui,
                        text(zh, "从下方素材池更换", "Choose a tile below to replace"),
                        palette,
                    );
                });
            });
            ui.add_space(if compact { 8.0 } else { 16.0 });
            let preview = egui::Button::new(
                RichText::new(text(
                    zh,
                    if emotes {
                        "试发表情"
                    } else {
                        "试发信号"
                    },
                    if emotes {
                        "Preview emote"
                    } else {
                        "Preview ping"
                    },
                ))
                .font(font(15.0, true))
                .color(palette.bg),
            )
            .fill(palette.accent)
            .corner_radius(8);
            if ui
                .add_enabled(
                    !id.is_empty(),
                    preview.min_size(vec2(
                        ui.available_width(),
                        if compact { 32.0 } else { 38.0 },
                    )),
                )
                .clicked()
            {
                actions.push(Action::Preview(id.to_owned()));
            }
            ui.add_space(if compact { 8.0 } else { 18.0 });
            ui.separator();
            ui.add_space(if compact { 8.0 } else { 14.0 });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(text(zh, "轮盘大小", "Wheel size"))
                        .font(font(15.0, true))
                        .color(palette.text),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{size} px"))
                            .font(font(14.0, true))
                            .color(palette.accent),
                    );
                });
            });
            let mut next_size = size;
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = ui.available_width();
                ui.visuals_mut().selection.bg_fill = palette.accent;
                if ui
                    .add(
                        egui::Slider::new(&mut next_size, 300..=900)
                            .step_by(10.0)
                            .handle_shape(egui::style::HandleShape::Circle)
                            .trailing_fill(true)
                            .show_value(false),
                    )
                    .accessible_name(text(zh, "轮盘大小", "Wheel size"))
                    .changed()
                {
                    actions.push(Action::Size(next_size));
                }
            });
            caption(
                ui,
                text(
                    zh,
                    "300–900 px · 默认 600 px",
                    "300–900 px · Default 600 px",
                ),
                palette,
            );
            ui.add_space(if compact { 8.0 } else { 16.0 });
            if !compact {
                ui.label(
                    RichText::new(text(
                        zh,
                        if emotes {
                            "点击表情"
                        } else {
                            "中心信号"
                        },
                        if emotes { "Click emote" } else { "Center ping" },
                    ))
                    .font(font(15.0, true))
                    .color(palette.text),
                );
            }
            let current_name = items
                .iter()
                .find(|item| item.id == center)
                .map_or(center, |item| item.name.as_str());
            ui.horizontal(|ui| {
                if compact {
                    ui.label(
                        RichText::new(text(
                            zh,
                            if emotes {
                                "点击表情"
                            } else {
                                "中心信号"
                            },
                            if emotes { "Click emote" } else { "Center ping" },
                        ))
                        .font(font(14.0, true))
                        .color(palette.text),
                    );
                    ui.add_space(8.0);
                }
                egui::ComboBox::from_id_salt("center-selection")
                    .width(ui.available_width())
                    .selected_text(current_name)
                    .show_ui(ui, |ui| {
                        for item in items {
                            if ui.selectable_label(item.id == center, &item.name).clicked()
                                && item.id != center
                            {
                                state.target = Target::Center;
                                actions.push(Action::Assign {
                                    target: Target::Center,
                                    id: item.id.clone(),
                                });
                            }
                        }
                    })
                    .response
                    .accessible_name(text(
                        zh,
                        if emotes {
                            "点击表情"
                        } else {
                            "中心信号"
                        },
                        if emotes { "Click emote" } else { "Center ping" },
                    ))
                    .on_hover_text(text(
                        zh,
                        "独立于八个方向，可使用相同素材。",
                        "Independent of the eight outer directions.",
                    ));
            });
            if !compact {
                ui.add_space(5.0);
                caption(
                    ui,
                    text(
                        zh,
                        "独立于八个方向，可使用相同素材。",
                        "Independent of the eight outer directions.",
                    ),
                    palette,
                );
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn tile(
    ui: &mut egui::Ui,
    item: &WheelItem,
    selected: bool,
    slot: Option<usize>,
    width: f32,
    zh: bool,
    compact: bool,
) -> egui::Response {
    let palette = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, if compact { 83.0 } else { 109.0 }),
        Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, &item.name)
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            10.0,
            if selected {
                palette.accent_soft
            } else if response.hovered() {
                palette.raised
            } else {
                palette.bg
            },
        );
        painter.rect_stroke(
            rect,
            10.0,
            Stroke::new(
                if selected || response.has_focus() {
                    1.5
                } else {
                    1.0
                },
                if selected || response.has_focus() {
                    palette.accent
                } else {
                    palette.line
                },
            ),
            StrokeKind::Inside,
        );
        if let Some(texture) = item.texture {
            paint_texture(ui, painter, texture, tile_image_rect(rect));
        }
        // Layout text through a width-limited galley so imported names never overlap adjacent tiles.
        let galley = painter.layout(
            item.name.clone(),
            font(if compact { 12.0 } else { 13.0 }, selected),
            if selected {
                palette.accent
            } else {
                palette.text
            },
            width - 12.0,
        );
        let text_rect = Rect::from_min_size(
            pos2(
                rect.left() + 6.0,
                rect.top() + if compact { 43.0 } else { 60.0 },
            ),
            vec2(width - 12.0, if compact { 16.0 } else { 19.0 }),
        );
        painter.with_clip_rect(text_rect).galley(
            pos2(rect.center().x - galley.size().x * 0.5, text_rect.top()),
            galley,
            palette.text,
        );
        painter.text(
            pos2(
                rect.center().x,
                rect.bottom() - if compact { 12.0 } else { 16.0 },
            ),
            Align2::CENTER_CENTER,
            slot.map(|index| direction(Target::Slot(index), zh))
                .unwrap_or(text(zh, "未分配", "Available")),
            font(11.0, false),
            palette.muted,
        );
    }
    response
        .on_hover_text(&item.name)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn tile_image_rect(rect: Rect) -> Rect {
    let compact = rect.height() < 100.0;
    Rect::from_center_size(
        pos2(
            rect.center().x,
            rect.top() + if compact { 22.0 } else { 33.0 },
        ),
        Vec2::splat(if compact { 30.0 } else { 40.0 }),
    )
}

fn paint_texture(ui: &egui::Ui, painter: &egui::Painter, texture: TextureId, bounds: Rect) {
    let size = ui
        .ctx()
        .tex_manager()
        .read()
        .meta(texture)
        .map(|meta| vec2(meta.size[0] as f32, meta.size[1] as f32))
        .unwrap_or(Vec2::splat(1.0));
    let scale = (bounds.width() / size.x.max(1.0)).min(bounds.height() / size.y.max(1.0));
    painter.image(
        texture,
        Rect::from_center_size(bounds.center(), size * scale),
        Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn request_texture(actions: &mut Vec<Action>, id: &str) {
    if !actions
        .iter()
        .any(|action| matches!(action, Action::LoadTexture(existing) if existing == id))
    {
        actions.push(Action::LoadTexture(id.to_owned()));
    }
}

#[allow(clippy::too_many_arguments)]
fn sector(
    painter: &egui::Painter,
    center: Pos2,
    inner: f32,
    outer: f32,
    from: f32,
    to: f32,
    color: Color32,
) {
    const STEPS: usize = 16;
    let mut mesh = egui::Mesh::default();
    for step in 0..=STEPS {
        let angle = from + (to - from) * step as f32 / STEPS as f32;
        let radial = Vec2::angled(angle);
        mesh.colored_vertex(center + radial * inner, color);
        mesh.colored_vertex(center + radial * outer, color);
        if step < STEPS {
            let index = step as u32 * 2;
            mesh.add_triangle(index, index + 1, index + 2);
            mesh.add_triangle(index + 1, index + 3, index + 2);
        }
    }
    painter.add(Shape::mesh(mesh));
}

fn arc(painter: &egui::Painter, center: Pos2, radius: f32, from: f32, to: f32, stroke: Stroke) {
    let points = (0..=24)
        .map(|step| center + Vec2::angled(from + (to - from) * step as f32 / 24.0) * radius)
        .collect();
    painter.add(Shape::line(points, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Vec<String> {
        (0..8).map(|index| format!("ping-{index}")).collect()
    }

    #[test]
    fn north_clockwise_hits_all_eight_directions() {
        for index in 0..8 {
            let angle = -FRAC_PI_2 + index as f32 * FRAC_PI_4;
            assert_eq!(
                hit_test(Vec2::angled(angle) * 80.0, 100.0, 40.0),
                Some(Target::Slot(index))
            );
        }
        assert_eq!(hit_test(Vec2::ZERO, 100.0, 40.0), Some(Target::Center));
        assert_eq!(hit_test(vec2(40.0, 0.0), 100.0, 40.0), Some(Target::Center));
        assert_eq!(hit_test(vec2(101.0, 0.0), 100.0, 40.0), None);
        assert_eq!(hit_test(vec2(90.0, 90.0), 100.0, 40.0), None);
        assert_eq!(hit_test(vec2(f32::NAN, 0.0), 100.0, 40.0), None);
    }

    #[test]
    fn sector_boundaries_do_not_shift_the_north_origin() {
        let angle = -FRAC_PI_2 + std::f32::consts::PI / 8.0;
        assert_eq!(
            hit_test(Vec2::angled(angle - 0.001) * 80.0, 100.0, 40.0),
            Some(Target::Slot(0))
        );
        assert_eq!(
            hit_test(Vec2::angled(angle + 0.001) * 80.0, 100.0, 40.0),
            Some(Target::Slot(1))
        );
    }

    #[test]
    fn assignment_replaces_or_swaps_without_duplicates() {
        let initial = layout();
        let (replaced, center) = assign(&initial, Target::Slot(3), "center", "new").unwrap();
        assert_eq!(replaced[3], "new");
        assert_eq!(replaced[2], initial[2]);
        assert_eq!(center, "center");
        let (swapped, _) = assign(&initial, Target::Slot(0), "center", "ping-4").unwrap();
        assert_eq!(swapped[0], "ping-4");
        assert_eq!(swapped[4], "ping-0");
        assert_eq!(
            swapped
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            8
        );
        assert!(assign(&initial, Target::Slot(0), "center", "ping-0").is_none());
        assert!(assign(&initial, Target::Slot(8), "center", "new").is_none());
    }

    #[test]
    fn center_assignment_is_independent_and_same_value_is_noop() {
        let initial = layout();
        let (wheel, center) = assign(&initial, Target::Center, "center", "ping-4").unwrap();
        assert_eq!(wheel, initial);
        assert_eq!(center, "ping-4");
        assert!(assign(&initial, Target::Center, "center", "center").is_none());
    }

    #[test]
    fn keyboard_navigation_reaches_all_slots_and_wraps() {
        let mut target = Target::Center;
        for index in 0..8 {
            target = step_target(target, true);
            assert_eq!(target, Target::Slot(index));
        }
        assert_eq!(step_target(target, true), Target::Slot(0));
        assert_eq!(step_target(Target::Slot(0), false), Target::Slot(7));
        assert_eq!(step_target(Target::Center, false), Target::Slot(7));
    }
}
