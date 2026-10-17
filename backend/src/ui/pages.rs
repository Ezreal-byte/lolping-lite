//! Presentation-only state. Settings and input ownership stay in the parent.
use super::{
    design::{self, Palette},
    keys, ping_icon, tr, wheel_editor, App, Confirm,
};
use eframe::egui::{self, Color32, RichText, Vec2};
use serde_json::json;
use std::time::Instant;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub(super) enum Page {
    #[default]
    Trigger,
    Hotkey,
    Sound,
    Wheel,
    Emotes,
    App,
}
impl Page {
    const ALL: [Self; 6] = [
        Self::Trigger,
        Self::Hotkey,
        Self::Sound,
        Self::Wheel,
        Self::Emotes,
        Self::App,
    ];
    fn label(self, zh: bool) -> &'static str {
        match self {
            Self::Trigger => copy(zh, "触发", "Trigger"),
            Self::Hotkey => copy(zh, "快捷开关", "Shortcuts"),
            Self::Sound => copy(zh, "信号与声音", "Pings & sound"),
            Self::Wheel => copy(zh, "轮盘", "Wheel"),
            Self::Emotes => copy(zh, "表情", "Emotes"),
            Self::App => copy(zh, "应用", "App"),
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::Hotkey => "hotkey",
            Self::Sound => "sound",
            Self::Wheel => "wheel",
            Self::Emotes => "emotes",
            Self::App => "app",
        }
    }
    fn subtitle(self, zh: bool) -> &'static str {
        match self {
            Self::Trigger => copy(
                zh,
                "让每一次操作，都恰到好处。",
                "Make every gesture feel just right.",
            ),
            Self::Hotkey => copy(
                zh,
                "需要专注时，一键暂停。随时继续。",
                "Pause when you need to focus. Resume when you’re ready.",
            ),
            Self::Sound => copy(
                zh,
                "调好大小与声音，让信号更有表现力。",
                "Find the right presence for every ping.",
            ),
            Self::Wheel => copy(
                zh,
                "你的信号，由你掌控。点击方向，再选择素材。",
                "Your pings, your way. Choose a direction, then assign a ping.",
            ),
            Self::Emotes => copy(
                zh,
                "给桌面多一点表情。",
                "A little more expression for your desktop.",
            ),
            Self::App => copy(
                zh,
                "熟悉的外观，轻巧的日常。",
                "A familiar look. A lighter everyday companion.",
            ),
        }
    }
}
fn copy(zh: bool, cn: &'static str, en: &'static str) -> &'static str {
    if zh {
        cn
    } else {
        en
    }
}
fn secondary(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(design::font(14.0, false))
            .color(Palette::of(ui).muted),
    );
}
fn card(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    let p = Palette::of(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.line))
        .corner_radius(10)
        .inner_margin(20)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            f(ui);
        });
}
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).font(design::font(17.0, true)));
    ui.add_space(6.0);
}
fn row(ui: &mut egui::Ui, title: &str, help: &str, f: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    let control = (width * 0.43).clamp(238.0, 300.0);
    let label_width = (width - control - 24.0).max(180.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(
            Vec2::new(label_width, 64.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(label_width);
                ui.add_space(8.0);
                ui.label(RichText::new(title).font(design::font(16.0, true)));
                if !help.is_empty() {
                    secondary(ui, help);
                }
            },
        );
        ui.add_space(24.0);
        ui.allocate_ui_with_layout(
            Vec2::new(control, 64.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.set_width(control);
                ui.spacing_mut().item_spacing.x = 8.0;
                f(ui);
            },
        );
    });
}
fn divider(ui: &mut egui::Ui) {
    ui.add(egui::Separator::default().spacing(10.0));
}
fn toggle(ui: &mut egui::Ui, value: &mut bool, name: &str) -> egui::Response {
    let (hit_rect, mut r) = ui.allocate_exact_size(Vec2::new(44.0, 36.0), egui::Sense::click());
    let rect = egui::Rect::from_center_size(hit_rect.center(), Vec2::new(44.0, 26.0));
    if r.clicked() {
        *value = !*value;
        r.mark_changed();
    }
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *value, name)
    });
    let p = Palette::of(ui);
    ui.painter()
        .rect_filled(rect, 13, if *value { p.accent } else { p.line });
    ui.painter().circle_filled(
        egui::pos2(
            if *value {
                rect.right() - 13.0
            } else {
                rect.left() + 13.0
            },
            rect.center().y,
        ),
        9.0,
        if *value && ui.visuals().dark_mode {
            p.bg
        } else {
            Color32::WHITE
        },
    );
    if r.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(3.0),
            15,
            egui::Stroke::new(1.0, p.accent),
            egui::StrokeKind::Outside,
        );
    }
    r
}
fn action(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response {
    let p = Palette::of(ui);
    let label = RichText::new(text).font(design::font(15.0, true));
    let mut button = egui::Button::new(if primary {
        label.color(if ui.visuals().dark_mode {
            p.bg
        } else {
            Color32::WHITE
        })
    } else {
        label
    });
    if primary {
        button = button.fill(p.accent).stroke(egui::Stroke::NONE);
    }
    ui.add(button)
}

impl App {
    pub(super) fn draw_settings(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        let p = Palette::of(ui);
        egui::Panel::top("settings-header")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(24, 0)),
            )
            .show(ui, |ui| {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if let Some(icon) = self.texture(
                        ui.ctx(),
                        "brand",
                        include_bytes!("../../../assets/plugin.png"),
                    ) {
                        ui.image((icon.id(), Vec2::splat(32.0)));
                    }
                    ui.add_space(2.0);
                    ui.label(RichText::new("LOLPing Lite").font(design::font(22.0, true)));
                    ui.label(
                        RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                            .font(design::font(13.0, false))
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.status_actions(ui)
                    });
                });
                ui.add_space(13.0);
                ui.separator();
                ui.add_enabled_ui(
                    self.capture.is_none() && self.confirm.is_none() && !self.closing,
                    |ui| {
                        let tab_width =
                            (ui.available_width() - ui.spacing().item_spacing.x * 5.0) / 6.0;
                        ui.horizontal(|ui| {
                            for page in Page::ALL {
                                let selected = self.page == page;
                                let (rect, r) = ui.allocate_exact_size(
                                    Vec2::new(tab_width, 49.0),
                                    egui::Sense::click(),
                                );
                                r.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::Role::Tab,
                                        ui.is_enabled(),
                                        selected,
                                        page.label(zh),
                                    )
                                });
                                let color = if selected {
                                    p.accent
                                } else if r.hovered() || r.has_focus() {
                                    p.text
                                } else {
                                    p.muted
                                };
                                if r.hovered() {
                                    ui.painter().rect_filled(
                                        rect.shrink2(Vec2::new(0.0, 5.0)),
                                        6,
                                        p.raised,
                                    );
                                }
                                let font = design::font(
                                    if tab_width < 140.0 { 14.0 } else { 16.0 },
                                    selected,
                                );
                                let text = ui.painter().layout_no_wrap(
                                    page.label(zh).into(),
                                    font.clone(),
                                    color,
                                );
                                let icon_width = if tab_width < 128.0 { 0.0 } else { 28.0 };
                                let x = rect.center().x - (text.size().x + icon_width) * 0.5;
                                if icon_width > 0.0 {
                                    design::paint_icon(
                                        ui,
                                        egui::Rect::from_center_size(
                                            egui::pos2(x + 10.0, rect.center().y),
                                            Vec2::splat(20.0),
                                        ),
                                        page.icon(),
                                        color,
                                    );
                                }
                                ui.painter()
                                    .with_clip_rect(rect.intersect(ui.clip_rect()))
                                    .galley(
                                        egui::pos2(
                                            x + icon_width,
                                            rect.center().y - text.size().y * 0.5,
                                        ),
                                        text,
                                        color,
                                    );
                                if selected {
                                    ui.painter().line_segment(
                                        [
                                            egui::pos2(rect.left() + 12.0, rect.bottom() - 1.0),
                                            egui::pos2(rect.right() - 12.0, rect.bottom() - 1.0),
                                        ],
                                        egui::Stroke::new(2.0, p.accent),
                                    );
                                }
                                if r.has_focus() {
                                    ui.painter().rect_stroke(
                                        rect.shrink(3.0),
                                        6,
                                        egui::Stroke::new(1.0, p.accent),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                if r.clicked() {
                                    self.page = page;
                                }
                            }
                        });
                    },
                );
            });
        egui::Panel::bottom("settings-save")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(24, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let working = self.saving.is_some() || !self.dirty.is_empty() || self.busy();
                    if working {
                        ui.spinner();
                    } else {
                        design::icon(
                            ui,
                            if self.failed_save.is_some() {
                                "close"
                            } else {
                                "check"
                            },
                            16.0,
                            if self.failed_save.is_some() {
                                p.danger
                            } else {
                                p.good
                            },
                        );
                    }
                    secondary(
                        ui,
                        if self.failed_save.is_some() {
                            copy(zh, "保存失败，请重试", "Couldn’t save. Please retry.")
                        } else {
                            self.t(if self.closing {
                                "closeSaving"
                            } else if self.saving.is_some() || !self.dirty.is_empty() {
                                "saving"
                            } else if self.busy() {
                                "working"
                            } else {
                                "saved"
                            })
                        },
                    );
                    if ui.available_width() > 300.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            secondary(
                                ui,
                                copy(
                                    zh,
                                    "关闭设置后，信号继续运行",
                                    "Pings keep running when settings close",
                                ),
                            )
                        });
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(24))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt((
                        "settings-page",
                        self.page,
                        self.emote_page && self.page == Page::Emotes,
                    ))
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        self.banners(ui);
                        if self.state.is_null() {
                            ui.heading(copy(zh, "正在加载设置", "Loading settings"));
                            if self.pending.is_empty() && ui.button(self.t("retryLoad")).clicked() {
                                self.request("state", json!({}));
                            }
                            return;
                        }
                        ui.label(
                            RichText::new(if self.page == Page::Wheel {
                                copy(zh, "你的轮盘，你的习惯", "Your wheel, your way")
                            } else {
                                self.page.label(zh)
                            })
                            .font(design::font(28.0, true)),
                        );
                        secondary(ui, self.page.subtitle(zh));
                        ui.add_space(16.0);
                        ui.add_enabled_ui(
                            !self.busy()
                                && !self.closing
                                && self.capture.is_none()
                                && self.confirm.is_none(),
                            |ui| match self.page {
                                Page::Trigger => self.trigger_page(ui),
                                Page::Hotkey => self.hotkey_page(ui),
                                Page::Sound => self.sound_page(ui),
                                Page::Wheel => self.wheel_page(ui, false),
                                Page::Emotes => self.emotes_page(ui),
                                Page::App => self.app_page(ui),
                            },
                        );
                        ui.add_space(12.0);
                    });
            });
    }
    fn status_actions(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let zh = self.zh();
        let running = self.state["status"]["state"] == "running";
        let starting = self.state["status"]["state"] == "starting";
        let paused = running && self.state["status"]["enabled"] != true;
        let capture = self.state["status"]["pauseReason"] == "capture";
        ui.add_enabled_ui(
            !self.state.is_null() && !self.busy() && self.capture.is_none() && !self.closing,
            |ui| {
                if action(
                    ui,
                    self.t(if running || starting { "stop" } else { "start" }),
                    !running && !starting,
                )
                .clicked()
                {
                    self.act(
                        if running || starting { "stop" } else { "start" },
                        json!({}),
                    );
                }
                if running
                    && ui
                        .add_enabled_ui(!capture, |ui| {
                            action(ui, self.t(if paused { "resume" } else { "pause" }), false)
                        })
                        .inner
                        .clicked()
                {
                    self.act("pause", json!({"on":!paused}));
                }
            },
        );
        let status = if self.state.is_null() {
            "loading"
        } else if capture {
            "capturePaused"
        } else if paused {
            "paused"
        } else {
            self.state["status"]["state"].as_str().unwrap_or("stopped")
        };
        ui.label(
            RichText::new(tr(zh, status))
                .font(design::font(14.0, true))
                .color(if running && !paused { p.good } else { p.muted }),
        );
    }
    fn banners(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        let status_error = self.state["status"]["error"]
            .as_str()
            .map(|e| self.human_error(e));
        if let Some(e) = status_error {
            ui.colored_label(p.danger, e);
            ui.add_space(8.0);
        }
        if !self.error.is_empty() {
            ui.colored_label(p.danger, &self.error);
            ui.add_space(8.0);
        }
        if self.capture.is_none() && !self.hint.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(&self.hint);
                if ui.button(self.t("confirm")).clicked() {
                    self.hint.clear();
                }
            });
        }
        if self.failed_save.is_some() {
            ui.horizontal(|ui| {
                if ui.button(self.t("retrySave")).clicked() {
                    if let Some(patch) = self.failed_save.take() {
                        for (k, v) in patch {
                            self.dirty.entry(k).or_insert(v);
                        }
                    }
                    self.error.clear();
                    self.flush();
                }
                if self.closing && ui.button(self.t("discardClose")).clicked() {
                    self.close_allowed = true;
                    ui.ctx().request_repaint();
                }
            });
        }
        if self.state["status"]["state"] == "noAccess" {
            ui.label(self.t("permissionHelp"));
            if ui.button(self.t("permission")).clicked() {
                self.act("permission", json!({}));
            }
            ui.add_space(14.0);
        }
        if self.platform() == "linux"
            && (self.state["wayland"] == true
                || self.state["status"]["error"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains("wayland"))
        {
            ui.label(self.t("unsupported"));
            ui.add_space(14.0);
        }
    }
    fn switch_row(&mut self, ui: &mut egui::Ui, key: &str, title: &str, help: &str) {
        let mut value = self.state["settings"][key].as_bool().unwrap_or(false);
        row(ui, title, help, |ui| {
            if toggle(ui, &mut value, title).changed() {
                self.patch(key, json!(value), false);
            }
        });
    }
    fn range_row(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        title: &str,
        help: &str,
        min: f64,
        max: f64,
        step: f64,
        unit: &str,
    ) {
        let mut value = self.state["settings"][key].as_f64().unwrap_or(min);
        row(ui, title, help, |ui| {
            let response = ui
                .scope(|ui| {
                    ui.visuals_mut().selection.bg_fill = Palette::of(ui).accent;
                    ui.add(
                        egui::Slider::new(&mut value, min..=max)
                            .step_by(step)
                            .fixed_decimals(if step >= 1.0 { 0 } else { 1 })
                            .suffix(unit)
                            .handle_shape(egui::style::HandleShape::Circle),
                    )
                    .accessible_name(title)
                })
                .inner;
            if response.changed() {
                self.patch(
                    key,
                    if step >= 1.0 {
                        json!(value as u64)
                    } else {
                        json!(value)
                    },
                    true,
                );
            }
            if response.drag_stopped() {
                self.save_at = Some(Instant::now());
            }
        });
    }
    fn trigger_row(&mut self, ui: &mut egui::Ui, key: &'static str, title: &str, help: &str) {
        let current = self.state["settings"][key].clone();
        let platform = self.platform().to_owned();
        let zh = self.zh();
        row(ui, title, help, |ui| {
            if ui.button(copy(zh, "录制", "Record")).clicked() {
                self.begin_capture(key);
            }
            egui::ComboBox::from_id_salt(key)
                .width(150.0)
                .selected_text(keys::trigger_label(&current, &platform, zh))
                .show_ui(ui, |ui| {
                    for choice in [
                        "alt", "ctrl", "shift", "win", "capslock", "mouse4", "mouse5", "off",
                    ] {
                        if (platform == "mac" && choice == "capslock")
                            || (key == "trigger" && choice == "off")
                        {
                            continue;
                        }
                        if ui
                            .selectable_label(
                                current == choice,
                                keys::trigger_label(&json!(choice), &platform, zh),
                            )
                            .clicked()
                        {
                            self.patch(key, json!(choice), false);
                        }
                    }
                })
                .response
                .accessible_name(title);
        });
    }
    fn trigger_page(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        section(ui, copy(zh, "手势触发", "Gesture controls"));
        card(ui, |ui| {
            self.trigger_row(
                ui,
                "trigger",
                copy(zh, "信号触发键", "Ping trigger"),
                copy(
                    zh,
                    "按住触发键，用鼠标左键拖动展开轮盘。",
                    "Hold the trigger and left-drag to open the wheel.",
                ),
            );
            divider(ui);
            self.range_row(
                ui,
                "dragThresholdPx",
                copy(zh, "拖动距离", "Drag threshold"),
                copy(
                    zh,
                    "超过此距离后显示轮盘。",
                    "Distance before the wheel opens.",
                ),
                2.0,
                40.0,
                1.0,
                " px",
            );
        });
        ui.add_space(18.0);
        section(ui, copy(zh, "点击行为", "Click behavior"));
        card(ui, |ui| {
            self.switch_row(
                ui,
                "clickPing",
                copy(zh, "单击发送信号", "Click to ping"),
                copy(
                    zh,
                    "按住触发键单击，发送中心信号。",
                    "Trigger + click sends the center ping.",
                ),
            );
            if self.platform() == "win" {
                divider(ui);
                self.switch_row(
                    ui,
                    "gameCheck",
                    copy(zh, "全屏游戏点击保护", "Protect fullscreen game clicks"),
                    copy(
                        zh,
                        "在全屏游戏中保留普通点击操作。",
                        "Let fullscreen games receive their clicks.",
                    ),
                );
            }
        });
        ui.add_space(18.0);
        secondary(
            ui,
            copy(
                zh,
                "提示：右键或 Esc 取消选择；中心信号可在「轮盘」中调整。",
                "Tip: right-click or Esc cancels. Change the center ping in Wheel.",
            ),
        );
    }
    fn hotkey_page(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        card(ui, |ui| {
            row(
                ui,
                copy(zh, "暂停 / 恢复", "Pause / resume"),
                copy(
                    zh,
                    "无需打开设置，就能切换信号状态。",
                    "Toggle pings without opening settings.",
                ),
                |ui| {
                    if action(ui, copy(zh, "录制快捷键", "Record shortcut"), false).clicked() {
                        self.begin_capture("toggleHotkey");
                    }
                },
            );
            divider(ui);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                for key in keys::shortcut(&self.state["settings"]["toggleHotkey"], self.platform())
                    .split('+')
                {
                    egui::Frame::new()
                        .fill(Palette::of(ui).raised)
                        .stroke(egui::Stroke::new(1.0, Palette::of(ui).line))
                        .corner_radius(7)
                        .inner_margin(egui::Margin::symmetric(18, 12))
                        .show(ui, |ui| {
                            ui.label(RichText::new(key.trim()).font(design::font(22.0, true)));
                        });
                }
            });
            ui.add_space(20.0);
            secondary(ui, copy(zh, "修饰键", "Modifiers"));
            ui.horizontal_wrapped(|ui| {
                let current = self.state["settings"]["toggleHotkey"].clone();
                let mut mods = current["mods"].as_u64().unwrap_or(3);
                let platform = self.platform().to_owned();
                for (bit, label) in [
                    (1, "Ctrl"),
                    (2, if platform == "mac" { "Option" } else { "Alt" }),
                    (4, "Shift"),
                    (
                        8,
                        if platform == "mac" {
                            "Command"
                        } else if platform == "linux" {
                            "Super"
                        } else {
                            "Win"
                        },
                    ),
                ] {
                    if ui.selectable_label(mods & bit != 0, label).clicked() {
                        let next = mods ^ bit;
                        if next & 11 != 0 {
                            mods = next;
                            self.patch(
                                "toggleHotkey",
                                json!({"mods":mods,"vk":current["vk"]}),
                                false,
                            );
                        } else {
                            self.error = self.t("hintNeedModifier").into();
                        }
                    }
                }
            });
            ui.add_space(12.0);
            secondary(ui,copy(zh,"录制期间会暂时停用手势，完成或取消后恢复原状态。按 Esc 取消。","Gestures pause during recording and return to their previous state afterward. Esc cancels."));
        });
    }
    fn sound_page(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        card(ui, |ui| {
            self.range_row(
                ui,
                "pingSizePx",
                copy(zh, "信号大小", "Ping size"),
                copy(
                    zh,
                    "桌面上信号的显示尺寸。",
                    "The size of pings on your desktop.",
                ),
                60.0,
                220.0,
                1.0,
                " px",
            );
            divider(ui);
            self.range_row(
                ui,
                "pingDurationS",
                copy(zh, "显示时间", "Duration"),
                copy(
                    zh,
                    "信号消失前的停留时间。",
                    "How long a ping stays visible.",
                ),
                1.0,
                8.0,
                0.1,
                " s",
            );
            divider(ui);
            self.switch_row(
                ui,
                "keepLastPing",
                copy(zh, "保留最后一个信号", "Keep last ping"),
                copy(
                    zh,
                    "最后一个信号持续显示，直到下次发送。",
                    "Keep the latest ping until the next one.",
                ),
            );
        });
        ui.add_space(14.0);
        section(ui, copy(zh, "声音", "Sound"));
        card(ui, |ui| {
            self.range_row(
                ui,
                "volume",
                copy(zh, "音量", "Volume"),
                copy(
                    zh,
                    "控制信号与表情音效的音量。",
                    "Volume for ping and emote sounds.",
                ),
                0.0,
                100.0,
                1.0,
                " %",
            );
            divider(ui);
            self.switch_row(
                ui,
                "muted",
                copy(zh, "静音", "Mute"),
                copy(
                    zh,
                    "关闭所有信号与表情音效。",
                    "Silence ping and emote sounds.",
                ),
            );
            divider(ui);
            self.switch_row(
                ui,
                "tickSound",
                copy(zh, "轮盘选择音效", "Selection sound"),
                copy(
                    zh,
                    "切换方向时播放轻提示音。",
                    "Play a subtle sound when changing direction.",
                ),
            );
        });
        ui.add_space(14.0);
        section(ui, copy(zh, "试发一个信号", "Try a ping"));
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for id in crate::settings::PINGS {
                    if let Some(tex) = self.texture(ui.ctx(), &format!("ping-{id}"), ping_icon(id))
                    {
                        let response = ui
                            .add(
                                egui::Button::image((tex.id(), Vec2::splat(38.0)))
                                    .selected(self.preview_ping == *id),
                            )
                            .on_hover_text(self.ping_name(id));
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::Button,
                                ui.is_enabled(),
                                self.preview_ping == *id,
                                self.ping_name(id),
                            )
                        });
                        if response.clicked() {
                            self.preview_ping = id.to_string();
                        }
                    }
                }
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                secondary(ui, &self.ping_name(&self.preview_ping));
                if action(ui, copy(zh, "试发信号", "Preview ping"), true).clicked() {
                    self.act("preview", json!({"id":self.preview_ping,"wheel":"ping"}));
                }
            });
        });
    }
    fn emote_texture(&mut self, ctx: &egui::Context, id: &str) -> Option<egui::TextureHandle> {
        use base64::Engine;
        if let Some(tex) = self.textures.get(id) {
            return Some(tex.clone());
        }
        let source = self.state["emotes"]
            .as_array()?
            .iter()
            .find(|e| e["id"] == id)?["thumbnail"]
            .as_str()?;
        let (_, data) = source.split_once(',')?;
        if data.len() > 2 * 1024 * 1024 {
            return None;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .ok()?;
        self.texture(ctx, id, &bytes)
    }
    fn wheel_page(&mut self, ui: &mut egui::Ui, emotes: bool) {
        let key = if emotes { "emoteWheel" } else { "wheel" };
        let center_key = if emotes {
            "clickEmoteId"
        } else {
            "clickPingId"
        };
        let wheel: Vec<String> = self.state["settings"][key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let center = self.state["settings"][center_key]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let items = self
            .choices(emotes)
            .into_iter()
            .map(|(id, name)| {
                let texture = if emotes {
                    if wheel.contains(&id) || center == id {
                        self.emote_texture(ui.ctx(), &id).map(|t| t.id())
                    } else {
                        self.textures.get(&id).map(|t| t.id())
                    }
                } else {
                    self.texture(ui.ctx(), &format!("ping-{id}"), ping_icon(&id))
                        .map(|t| t.id())
                };
                wheel_editor::WheelItem { id, name, texture }
            })
            .collect::<Vec<_>>();
        let size = self.state["settings"]["wheelSizePx"]
            .as_u64()
            .unwrap_or(600) as u32;
        let zh = self.zh();
        let editor = if emotes {
            &mut self.emote_editor
        } else {
            &mut self.ping_editor
        };
        let actions = wheel_editor::show(ui, editor, &wheel, &center, size, &items, zh, emotes);
        for action in actions {
            match action {
                wheel_editor::Action::Assign { target, id } => {
                    let current: Vec<String> = self.state["settings"][key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                    let current_center = self.state["settings"][center_key].as_str().unwrap_or("");
                    if let Some((next, next_center)) =
                        wheel_editor::assign(&current, target, current_center, &id)
                    {
                        if next != current {
                            self.patch(key, json!(next), false);
                        }
                        if next_center != self.state["settings"][center_key] {
                            self.patch(center_key, json!(next_center), false);
                        }
                    }
                }
                wheel_editor::Action::Preview(id) => self.act(
                    "preview",
                    json!({"id":id,"wheel":if emotes{"emote"}else{"ping"}}),
                ),
                wheel_editor::Action::Size(value) => self.patch("wheelSizePx", json!(value), true),
                wheel_editor::Action::LoadTexture(id) => {
                    if emotes && self.emote_texture(ui.ctx(), &id).is_some() {
                        ui.ctx().request_repaint();
                    }
                }
            }
        }
    }
    fn emotes_page(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.emote_page, false, copy(zh, "素材库", "Library"));
            ui.selectable_value(
                &mut self.emote_page,
                true,
                copy(zh, "触发与轮盘", "Trigger & wheel"),
            );
        });
        ui.add_space(16.0);
        if self.emote_page {
            card(ui, |ui| {
                self.trigger_row(
                    ui,
                    "emoteTrigger",
                    copy(zh, "表情触发键", "Emote trigger"),
                    copy(
                        zh,
                        "按住并拖动鼠标，打开表情轮盘。",
                        "Hold and drag to open the emote wheel.",
                    ),
                );
                divider(ui);
                self.switch_row(
                    ui,
                    "emoteClick",
                    copy(zh, "单击发送表情", "Click to emote"),
                    copy(
                        zh,
                        "按住触发键单击，发送中心表情。",
                        "Trigger + click sends the center emote.",
                    ),
                );
                divider(ui);
                self.range_row(
                    ui,
                    "emoteSizePx",
                    copy(zh, "表情大小", "Emote size"),
                    copy(
                        zh,
                        "桌面上表情的显示尺寸。",
                        "Size of emotes on your desktop.",
                    ),
                    80.0,
                    300.0,
                    1.0,
                    " px",
                );
                divider(ui);
                self.switch_row(
                    ui,
                    "emoteSound",
                    copy(zh, "表情声音", "Emote sounds"),
                    copy(
                        zh,
                        "发送表情时播放对应音效。",
                        "Play the emote’s assigned sound.",
                    ),
                );
            });
            ui.add_space(20.0);
            self.wheel_page(ui, true);
            return;
        }
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if action(ui, copy(zh, "导入图片", "Import image"), true).clicked() {
                    self.choose_file(ui.ctx(), "image", "");
                }
                if ui
                    .add_enabled(
                        self.state["extraPackInstalled"] != true,
                        egui::Button::new(copy(zh, "导入扩展包", "Import pack")),
                    )
                    .clicked()
                {
                    self.choose_file(ui.ctx(), "pack", "");
                }
                if ui
                    .button(copy(zh, "去 GitHub 下载", "Download on GitHub"))
                    .clicked()
                {
                    self.act("releases", json!({}));
                }
            });
            ui.add_space(8.0);
            secondary(
                ui,
                self.t(if self.state["extraPackInstalled"] == true {
                    "packInstalled"
                } else {
                    "packHint"
                }),
            );
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            design::icon(ui, "search", 20.0, Palette::of(ui).muted);
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(tr(zh, "search"))
                    .desired_width(300.0),
            )
            .accessible_name(tr(zh, "search"));
        });
        ui.add_space(14.0);
        let search = self.search.to_lowercase();
        // Collect only lightweight metadata. Base64 images stay in state until a
        // visible card asks for one; no entire-library clone or animation decode.
        let items: Vec<(String, String, bool)> = self.state["emotes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let id = e["id"].as_str()?;
                let name = self.emote_name(e);
                if !search.is_empty()
                    && !name.to_lowercase().contains(&search)
                    && !id.to_lowercase().contains(&search)
                {
                    return None;
                }
                Some((id.into(), name, e["custom"] == true))
            })
            .collect();
        if items.is_empty() {
            card(ui, |ui| {
                ui.label(copy(zh, "没有找到表情", "No emotes found"));
                if ui.button(copy(zh, "清除搜索", "Clear search")).clicked() {
                    self.search.clear();
                }
            });
            return;
        }
        let columns = ((ui.available_width() / 154.0).floor() as usize).clamp(2, 7);
        let width = (ui.available_width() - (columns - 1) as f32 * 12.0) / columns as f32;
        for chunk in items.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (id, name, custom) in chunk {
                    let p = Palette::of(ui);
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(egui::Stroke::new(1.0, p.line))
                        .corner_radius(10)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width(width - 26.0);
                            ui.vertical_centered(|ui| {
                                let (rect, r) = ui.allocate_exact_size(
                                    Vec2::new(width - 26.0, 100.0),
                                    egui::Sense::click(),
                                );
                                r.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::Role::Button,
                                        ui.is_enabled(),
                                        format!("{} {name}", tr(zh, "preview")),
                                    )
                                });
                                if ui.is_rect_visible(rect) {
                                    if let Some(t) = self.emote_texture(ui.ctx(), id) {
                                        let size = t.size_vec2();
                                        let scale = (88.0 / size.x).min(88.0 / size.y);
                                        ui.painter().image(
                                            t.id(),
                                            egui::Rect::from_center_size(
                                                rect.center(),
                                                size * scale,
                                            ),
                                            egui::Rect::from_min_max(
                                                egui::Pos2::ZERO,
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            Color32::WHITE,
                                        );
                                    }
                                }
                                if r.hovered() || r.has_focus() {
                                    ui.painter().rect_stroke(
                                        rect,
                                        6,
                                        egui::Stroke::new(1.0, p.accent),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                if r.on_hover_text(self.t("preview")).clicked() {
                                    self.act("preview", json!({"id":id,"wheel":"emote"}));
                                }
                                ui.add_sized(
                                    [width - 26.0, 24.0],
                                    egui::Label::new(
                                        RichText::new(name).font(design::font(14.0, true)),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(name);
                                if *custom {
                                    ui.horizontal(|ui| {
                                        if ui.small_button(copy(zh, "声音", "Sound")).clicked() {
                                            self.choose_file(ui.ctx(), "sound", id);
                                        }
                                        if ui.small_button(copy(zh, "删除", "Delete")).clicked() {
                                            self.confirm = Some(Confirm::Remove(id.clone()));
                                        }
                                    });
                                } else {
                                    secondary(ui, copy(zh, "点击预览", "Click to preview"));
                                }
                            });
                        });
                }
            });
            ui.add_space(4.0);
        }
    }
    fn app_page(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        card(ui, |ui| {
            let theme = self.state["preferences"]["theme"]
                .as_str()
                .unwrap_or("dark")
                .to_owned();
            row(
                ui,
                copy(zh, "外观", "Appearance"),
                copy(
                    zh,
                    "选择适合你的界面主题。",
                    "Choose a theme that feels at home.",
                ),
                |ui| {
                    egui::ComboBox::from_id_salt("theme")
                        .width(180.0)
                        .selected_text(tr(zh, &theme))
                        .show_ui(ui, |ui| {
                            for next in ["dark", "light", "system"] {
                                if ui.selectable_label(theme == next, tr(zh, next)).clicked() {
                                    self.state["preferences"]["theme"] = json!(next);
                                    self.act("preferences", json!({"theme":next}));
                                }
                            }
                        })
                        .response
                        .accessible_name(tr(zh, "theme"));
                },
            );
            divider(ui);
            let language = self.state["settings"]["language"]
                .as_str()
                .unwrap_or("auto")
                .to_owned();
            row(
                ui,
                copy(zh, "语言", "Language"),
                copy(
                    zh,
                    "立即切换，无需重启。",
                    "Changes take effect immediately.",
                ),
                |ui| {
                    let label = match language.as_str() {
                        "zh-CN" => "简体中文",
                        "en" => "English",
                        _ => tr(zh, "auto"),
                    };
                    egui::ComboBox::from_id_salt("language")
                        .width(180.0)
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (id, label) in [
                                ("auto", tr(zh, "auto")),
                                ("zh-CN", "简体中文"),
                                ("en", "English"),
                            ] {
                                if ui.selectable_label(language == id, label).clicked() {
                                    self.patch("language", json!(id), false);
                                }
                            }
                        })
                        .response
                        .accessible_name(tr(zh, "language"));
                },
            );
            divider(ui);
            let mut autostart = self.state["preferences"]["autostart"]
                .as_bool()
                .unwrap_or(false);
            row(
                ui,
                copy(zh, "开机启动", "Launch at login"),
                copy(
                    zh,
                    "登录系统后自动在后台运行。",
                    "Run in the background when you sign in.",
                ),
                |ui| {
                    if toggle(ui, &mut autostart, tr(zh, "autostart")).changed() {
                        self.state["preferences"]["autostart"] = json!(autostart);
                        self.act("preferences", json!({"autostart":autostart}));
                    }
                },
            );
        });
        ui.add_space(18.0);
        section(ui, copy(zh, "关于 LOLPing Lite", "About LOLPing Lite"));
        card(ui, |ui| {
            ui.label(
                RichText::new(concat!("LOLPing Lite  ", env!("CARGO_PKG_VERSION")))
                    .font(design::font(17.0, true)),
            );
            secondary(
                ui,
                copy(
                    zh,
                    "Rust 原生界面 · 关闭设置即释放界面资源",
                    "Native Rust UI · Closing settings releases UI resources",
                ),
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to(
                    copy(zh, "项目主页", "Project"),
                    "https://github.com/Ezreal-byte/lolping-lite",
                );
                ui.hyperlink_to(
                    copy(
                        zh,
                        "原版 lolPing · darrenprx",
                        "Original lolPing · darrenprx",
                    ),
                    "https://github.com/darrenprx/lolPing",
                );
            });
            secondary(ui, self.t("attribution"));
        });
        ui.add_space(18.0);
        section(ui, copy(zh, "重置设置", "Reset settings"));
        card(ui, |ui| {
            row(
                ui,
                copy(zh, "恢复默认设置", "Restore defaults"),
                copy(
                    zh,
                    "将重置触发键、轮盘和信号设置，保留自定义素材。",
                    "Reset triggers, wheels and pings. Keep imported media.",
                ),
                |ui| {
                    if ui
                        .button(
                            RichText::new(copy(zh, "重置设置…", "Reset settings…"))
                                .color(Palette::of(ui).danger),
                        )
                        .clicked()
                    {
                        self.confirm = Some(Confirm::Reset);
                    }
                },
            );
        });
    }
}
