//! Native settings live in a short-lived process. Only the parent owns settings
//! and media files; this process exchanges JSON lines over inherited pipes.
use base64::Engine;
use eframe::egui::{self, Color32, RichText, Vec2};
use serde_json::{json, Map, Value};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, Write},
    sync::{mpsc, OnceLock},
    time::{Duration, Instant},
};

mod keys;

fn strings() -> &'static Value {
    static STRINGS: OnceLock<Value> = OnceLock::new();
    STRINGS.get_or_init(|| serde_json::from_str(include_str!("ui/strings.json")).unwrap())
}
fn tr(zh: bool, key: &str) -> &'static str {
    strings()[if zh { "zh-CN" } else { "en" }][key]
        .as_str()
        .unwrap_or("")
}
fn merge(target: &mut Value, patch: &Map<String, Value>) {
    for (key, value) in patch {
        target[key] = value.clone();
    }
}

enum Incoming {
    Message(Value),
    Eof,
    File {
        kind: String,
        target: String,
        path: Option<String>,
    },
}
struct Pending {
    method: String,
    params: Value,
    at: Instant,
}
struct Capture {
    key: &'static str,
    started: Instant,
    armed: bool,
}
enum Confirm {
    Reset,
    Remove(String),
}

struct App {
    tx: mpsc::Sender<Value>,
    incoming: mpsc::Receiver<Incoming>,
    local: mpsc::Sender<Incoming>,
    pending: HashMap<u64, Pending>,
    next_id: u64,
    state: Value,
    dirty: Map<String, Value>,
    save_at: Option<Instant>,
    saving: Option<u64>,
    failed_save: Option<Map<String, Value>>,
    capture: Option<Capture>,
    confirm: Option<Confirm>,
    error: String,
    hint: String,
    file_dialog: bool,
    closing: bool,
    close_allowed: bool,
    ready_sent: bool,
    ui_frames: u8,
    theme: String,
    title: String,
    textures: HashMap<String, egui::TextureHandle>,
    texture_order: VecDeque<String>,
    search: String,
    test_exit: Option<Instant>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);
        let mut style = (*cc.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.spacing.item_spacing = Vec2::new(10.0, 10.0);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        style.spacing.interact_size.y = 30.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(23.0));
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let mut themed = (*cc.egui_ctx.style_of(theme)).clone();
            themed.spacing = style.spacing.clone();
            themed.text_styles = style.text_styles.clone();
            cc.egui_ctx.set_style_of(theme, themed);
        }
        let (wire_tx, wire_rx) = mpsc::channel::<Value>();
        let (local, incoming) = mpsc::channel();
        let wake = cc.egui_ctx.clone();
        let output_closed = local.clone();
        std::thread::spawn(move || {
            let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
            for value in wire_rx {
                let success = serde_json::to_writer(&mut stdout, &value)
                    .map_err(std::io::Error::other)
                    .and_then(|_| stdout.write_all(b"\n"))
                    .and_then(|_| stdout.flush());
                if success.is_err() {
                    let _ = output_closed.send(Incoming::Eof);
                    wake.request_repaint();
                    break;
                }
            }
        });
        let wake = cc.egui_ctx.clone();
        let input = local.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if input.send(Incoming::Message(value)).is_err() {
                        return;
                    }
                    wake.request_repaint();
                }
            }
            let _ = input.send(Incoming::Eof);
            wake.request_repaint();
        });
        let mut app = Self {
            tx: wire_tx,
            incoming,
            local,
            pending: HashMap::new(),
            next_id: 1,
            state: Value::Null,
            dirty: Map::new(),
            save_at: None,
            saving: None,
            failed_save: None,
            capture: None,
            confirm: None,
            error: String::new(),
            hint: String::new(),
            file_dialog: false,
            closing: false,
            close_allowed: false,
            ready_sent: false,
            ui_frames: 0,
            theme: String::new(),
            title: String::new(),
            textures: HashMap::new(),
            texture_order: VecDeque::new(),
            search: String::new(),
            test_exit: std::env::var("LOLPING_UI_TEST_EXIT_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(|ms| Instant::now() + Duration::from_millis(ms)),
        };
        app.request("state", json!({}));
        app
    }
    fn zh(&self) -> bool {
        match self.state["settings"]["language"]
            .as_str()
            .unwrap_or("auto")
        {
            "zh-CN" => true,
            "en" => false,
            _ => self.state["locale"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .starts_with("zh"),
        }
    }
    fn platform(&self) -> &str {
        self.state["platform"].as_str().unwrap_or("win")
    }
    fn t(&self, key: &str) -> &'static str {
        tr(self.zh(), key)
    }
    fn busy(&self) -> bool {
        self.file_dialog
            || self.pending.values().any(|p| {
                !matches!(
                    p.method.as_str(),
                    "settings" | "state" | "ui/ready" | "suspend"
                )
            })
    }
    fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        if self
            .tx
            .send(json!({"id":id,"method":method,"params":params}))
            .is_err()
        {
            self.close_allowed = true;
        } else {
            self.pending.insert(
                id,
                Pending {
                    method: method.into(),
                    params,
                    at: Instant::now(),
                },
            );
        }
        id
    }
    fn act(&mut self, method: &str, params: Value) {
        self.error.clear();
        self.request(method, params);
    }
    fn patch(&mut self, key: &str, value: Value, delayed: bool) {
        if let Some(failed) = self.failed_save.take() {
            for (key, value) in failed {
                self.dirty.entry(key).or_insert(value);
            }
        }
        self.state["settings"][key] = value.clone();
        self.dirty.insert(key.to_owned(), value);
        self.save_at = Some(
            Instant::now()
                + if delayed {
                    Duration::from_millis(220)
                } else {
                    Duration::ZERO
                },
        );
        self.failed_save = None;
        self.error.clear();
    }
    fn apply_settings(&mut self, mut value: Value) {
        if let Some(pending) = self.saving.and_then(|id| self.pending.get(&id)) {
            if let Some(patch) = pending.params.as_object() {
                merge(&mut value, patch);
            }
        }
        merge(&mut value, &self.dirty);
        if let Some(patch) = &self.failed_save {
            merge(&mut value, patch);
        }
        self.state["settings"] = value;
    }
    fn flush(&mut self) {
        if self.saving.is_none() && !self.dirty.is_empty() {
            let patch = std::mem::take(&mut self.dirty);
            let id = self.request("settings", Value::Object(patch));
            self.saving = Some(id);
            self.save_at = None;
        }
    }
    fn end_capture(&mut self) {
        if self.capture.take().is_some() {
            self.request("suspend", json!({"on":false}));
        }
    }
    fn begin_capture(&mut self, key: &'static str) {
        self.end_capture();
        self.hint.clear();
        self.capture = Some(Capture {
            key,
            started: Instant::now(),
            armed: false,
        });
        self.request("suspend", json!({"on":true}));
    }
    fn human_error(&self, error: &str) -> String {
        if let Some(errors) =
            strings()[if self.zh() { "zh-CN" } else { "en" }]["errors"].as_object()
        {
            if let Some((_, label)) = errors.iter().find(|(key, _)| error.contains(key.as_str())) {
                return label.as_str().unwrap_or(error).into();
            }
        }
        format!("{}: {error}", self.t("error"))
    }
    fn messages(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.incoming.try_recv() {
            match message {
                Incoming::Eof => self.close_allowed = true,
                Incoming::File { kind, target, path } => {
                    self.file_dialog = false;
                    if let Some(path) = path {
                        self.act("import", json!({"kind":kind,"path":path,"id":target}));
                    }
                }
                Incoming::Message(v) => {
                    if let Some(event) = v["event"].as_str() {
                        match event {
                            "focus" => {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                            }
                            "quit" => self.close_allowed = true,
                            "lolping/status" => self.state["status"] = v["params"].clone(),
                            "lolping/settings" => {
                                self.apply_settings(v["params"]["settings"].clone())
                            }
                            "lolping/library" => {
                                self.request("state", json!({}));
                            }
                            "lolping/audioError" => self.error = self.human_error("audio"),
                            _ => {}
                        }
                        continue;
                    }
                    let Some(id) = v["id"].as_u64() else { continue };
                    let Some(p) = self.pending.remove(&id) else {
                        continue;
                    };
                    if p.method == "settings" {
                        self.saving = None;
                    }
                    if let Some(error) = v["error"].as_str() {
                        self.error = self.human_error(error);
                        if p.method == "settings" {
                            self.failed_save = p.params.as_object().cloned();
                        }
                        if p.method == "suspend" && p.params["on"] == true {
                            self.end_capture();
                        }
                        if p.method == "preferences" {
                            self.request("state", json!({}));
                        }
                        continue;
                    }
                    let result = &v["result"];
                    match p.method.as_str() {
                        "state" => {
                            let old_settings = result["settings"].clone();
                            self.state = result.clone();
                            self.apply_settings(old_settings);
                        }
                        "settings" => self.apply_settings(result.clone()),
                        "preferences" => self.state["preferences"] = result.clone(),
                        "suspend" if p.params["on"] == true => {
                            if let Some(capture) = &mut self.capture {
                                capture.armed = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if self
            .pending
            .values()
            .any(|p| p.at.elapsed() > Duration::from_secs(45))
        {
            self.error = self.human_error(
                "The main process has not responded. Please retry after it becomes available.",
            );
        }
    }
    fn choose_file(&mut self, ctx: &egui::Context, kind: &str, target: &str) {
        self.file_dialog = true;
        let sender = self.local.clone();
        let wake = ctx.clone();
        let kind = kind.to_owned();
        let target = target.to_owned();
        let title = self.t("selectFile");
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new().set_title(title);
            let dialog = match kind.as_str() {
                "image" => dialog.add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"]),
                "sound" => dialog.add_filter("Sounds", &["wav", "ogg", "mp3"]),
                _ => dialog.add_filter("Emote pack", &["zip"]),
            };
            let path = dialog.pick_file().map(|p| p.to_string_lossy().into_owned());
            let _ = sender.send(Incoming::File { kind, target, path });
            wake.request_repaint();
        });
    }
    fn capture_input(&mut self, ctx: &egui::Context) {
        let Some(capture) = &self.capture else { return };
        if capture.started.elapsed() >= Duration::from_secs(15) {
            self.end_capture();
            self.hint = self.t("captureTimeout").into();
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        if !ctx.input(|i| i.focused) {
            self.end_capture();
            return;
        }
        if !capture.armed {
            return;
        }
        let key = capture.key;
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Key {
                key: logical,
                physical_key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            {
                let physical = physical_key.unwrap_or(logical);
                if physical == egui::Key::Escape {
                    self.end_capture();
                    break;
                }
                let Some(vk) = keys::vk(physical) else {
                    self.hint = self.t("hintUnusable").into();
                    continue;
                };
                let mods = keys::modifiers(modifiers, self.platform());
                if key == "toggleHotkey" && mods & 11 == 0 {
                    self.hint = self.t("hintNeedModifier").into();
                    continue;
                }
                if keys::conflicts(key, vk, &self.state["settings"]) {
                    self.hint = self.t("conflicts").into();
                    continue;
                }
                self.patch(
                    key,
                    if key == "toggleHotkey" {
                        json!({"mods":mods,"vk":vk})
                    } else {
                        json!({"vk":vk})
                    },
                    false,
                );
                self.end_capture();
                break;
            }
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        ui.horizontal(|ui| {
            let icon = self.texture(
                ui.ctx(),
                "app-icon",
                include_bytes!("../../resources/textures/enemy_missing.png"),
            );
            if let Some(texture) = icon {
                ui.image((texture.id(), Vec2::splat(48.0)));
            }
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.heading("LOLPing Lite");
                    ui.weak(
                        self.state["version"]
                            .as_str()
                            .unwrap_or(env!("CARGO_PKG_VERSION")),
                    );
                });
                ui.label(tr(zh, "subtitle"));
            });
        });
        ui.add_space(12.0);
        egui::Frame::group(ui.style())
            .inner_margin(16.0)
            .show(ui, |ui| {
                let running = self.state["status"]["state"] == "running";
                let starting = self.state["status"]["state"] == "starting";
                let paused = running && self.state["status"]["enabled"] != true;
                let capture = self.state["status"]["pauseReason"] == "capture";
                let status = if self.state.is_null() {
                    "loading"
                } else if capture {
                    "capturePaused"
                } else if paused {
                    "paused"
                } else {
                    self.state["status"]["state"].as_str().unwrap_or("stopped")
                }
                .to_owned();
                ui.horizontal_wrapped(|ui| {
                    let color = if running && !paused {
                        Color32::from_rgb(50, 180, 125)
                    } else {
                        Color32::from_rgb(210, 160, 65)
                    };
                    ui.label(RichText::new("●").color(color));
                    ui.strong(tr(zh, &status));
                    let enabled = !self.state.is_null() && !self.busy() && self.capture.is_none();
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new(tr(
                                zh,
                                if running || starting { "stop" } else { "start" },
                            )),
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
                            .add_enabled(
                                enabled && !capture,
                                egui::Button::new(tr(zh, if paused { "resume" } else { "pause" })),
                            )
                            .clicked()
                    {
                        self.act("pause", json!({"on":!paused}));
                    }
                    ui.weak(keys::shortcut(
                        &self.state["settings"]["toggleHotkey"],
                        self.platform(),
                    ));
                });
                ui.label(tr(
                    zh,
                    if capture {
                        "capturePauseHelp"
                    } else if paused {
                        "resumeHelp"
                    } else if running {
                        "intro"
                    } else {
                        "stoppedHelp"
                    },
                ));
                ui.weak(tr(zh, "lifecycle"));
                if self.platform() == "linux"
                    && (self.state["wayland"] == true
                        || self.state["status"]["error"]
                            .as_str()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains("wayland"))
                {
                    ui.colored_label(Color32::from_rgb(210, 140, 30), tr(zh, "unsupported"));
                }
                if self.state["status"]["state"] == "noAccess" {
                    ui.label(tr(zh, "permissionHelp"));
                    if ui.button(tr(zh, "permission")).clicked() {
                        self.act("permission", json!({}));
                    }
                }
            });
    }
    fn bool_row(&mut self, ui: &mut egui::Ui, key: &str) {
        let mut value = self.state["settings"][key].as_bool().unwrap_or(false);
        if ui.checkbox(&mut value, self.t(key)).changed() {
            self.patch(key, json!(value), false);
        }
    }
    fn slider(&mut self, ui: &mut egui::Ui, key: &str, min: f64, max: f64, step: f64, unit: &str) {
        let mut value = self.state["settings"][key].as_f64().unwrap_or(min);
        ui.horizontal_wrapped(|ui| {
            ui.add_sized([180.0, 25.0], egui::Label::new(self.t(key)));
            let response = ui.add(
                egui::Slider::new(&mut value, min..=max)
                    .step_by(step)
                    .suffix(unit),
            );
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
    fn trigger(&mut self, ui: &mut egui::Ui, key: &'static str) {
        let current = self.state["settings"][key].clone();
        let platform = self.platform().to_owned();
        let zh = self.zh();
        ui.horizontal_wrapped(|ui| {
            ui.add_sized([180.0, 25.0], egui::Label::new(tr(zh, key)));
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
                });
            if ui.button(tr(zh, "custom")).clicked() {
                self.begin_capture(key);
            }
        });
    }
    fn controls(&mut self, ui: &mut egui::Ui) {
        self.trigger(ui, "trigger");
        self.trigger(ui, "emoteTrigger");
        ui.horizontal_wrapped(|ui| {
            ui.add_sized([180.0, 25.0], egui::Label::new(self.t("toggle")));
            if ui
                .button(keys::shortcut(
                    &self.state["settings"]["toggleHotkey"],
                    self.platform(),
                ))
                .clicked()
            {
                self.begin_capture("toggleHotkey");
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.add_space(190.0);
            let current = self.state["settings"]["toggleHotkey"].clone();
            let mut mods = current["mods"].as_u64().unwrap_or(3);
            for (bit, label) in [
                (1, "Ctrl"),
                (
                    2,
                    if self.platform() == "mac" {
                        "Option"
                    } else {
                        "Alt"
                    },
                ),
                (4, "Shift"),
                (
                    8,
                    if self.platform() == "mac" {
                        "Command"
                    } else if self.platform() == "linux" {
                        "Super"
                    } else {
                        "Win"
                    },
                ),
            ] {
                let mut on = mods & bit != 0;
                if ui.checkbox(&mut on, label).changed() {
                    let next = if on { mods | bit } else { mods & !bit };
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
        self.slider(ui, "dragThresholdPx", 2.0, 40.0, 1.0, " px");
        for key in ["clickPing", "emoteClick"] {
            self.bool_row(ui, key);
        }
        if self.platform() == "win" {
            self.bool_row(ui, "gameCheck");
        }
    }
    fn ping_name(&self, id: &str) -> String {
        strings()[if self.zh() { "zh-CN" } else { "en" }]["pingNames"][id]
            .as_str()
            .unwrap_or(id)
            .to_owned()
    }
    fn emote_name(&self, emote: &Value) -> String {
        (if self.zh() {
            emote["nameZh"].as_str()
        } else {
            None
        })
        .or_else(|| emote["name"].as_str())
        .unwrap_or(self.t("customName"))
        .into()
    }
    fn selection(&mut self, ui: &mut egui::Ui, key: &str, emotes: bool) {
        let current = self.state["settings"][key]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let choices = self.choices(emotes);
        ui.horizontal_wrapped(|ui| {
            ui.add_sized([180.0, 25.0], egui::Label::new(self.t(key)));
            egui::ComboBox::from_id_salt(key)
                .width(195.0)
                .selected_text(
                    choices
                        .iter()
                        .find(|(id, _)| id == &current)
                        .map(|(_, label)| label.as_str())
                        .unwrap_or(&current),
                )
                .show_ui(ui, |ui| {
                    for (id, label) in &choices {
                        if ui.selectable_label(id == &current, label).clicked() {
                            self.patch(key, json!(id), false);
                        }
                    }
                });
        });
    }
    fn choices(&self, emotes: bool) -> Vec<(String, String)> {
        if emotes {
            self.state["emotes"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(|e| Some((e["id"].as_str()?.into(), self.emote_name(e))))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            crate::settings::PINGS
                .iter()
                .map(|id| ((*id).into(), self.ping_name(id)))
                .collect()
        }
    }
    fn wheel(&mut self, ui: &mut egui::Ui, key: &str, emotes: bool) {
        let choices = self.choices(emotes);
        let slots = self.state["settings"][key]
            .as_array()
            .cloned()
            .unwrap_or_default();
        ui.strong(self.t(key));
        let zh = self.zh();
        egui::Grid::new(key)
            .num_columns(2)
            .spacing([18.0, 8.0])
            .show(ui, |ui| {
                for (i, slot) in slots.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [70.0, 25.0],
                            egui::Label::new(
                                strings()[if zh { "zh-CN" } else { "en" }]["slots"][i]
                                    .as_str()
                                    .unwrap_or(""),
                            ),
                        );
                        let id = slot.as_str().unwrap_or("");
                        egui::ComboBox::from_id_salt((key, i))
                            .width(165.0)
                            .selected_text(
                                choices
                                    .iter()
                                    .find(|(p, _)| p == id)
                                    .map(|(_, label)| label.as_str())
                                    .unwrap_or(id),
                            )
                            .show_ui(ui, |ui| {
                                for (next, label) in &choices {
                                    if ui.selectable_label(next == id, label).clicked() {
                                        let mut changed = slots.clone();
                                        if let Some(other) = changed.iter().position(|v| v == next)
                                        {
                                            changed[other] = changed[i].clone();
                                        }
                                        changed[i] = json!(next);
                                        self.patch(key, json!(changed), false);
                                    }
                                }
                            });
                    });
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
    }
    fn pings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for id in crate::settings::PINGS {
                let icon = self.texture(ui.ctx(), &format!("ping-{id}"), ping_icon(id));
                let button = if let Some(icon) = icon {
                    egui::Button::image_and_text((icon.id(), Vec2::splat(32.0)), self.ping_name(id))
                } else {
                    egui::Button::new(self.ping_name(id))
                };
                if ui.add(button).on_hover_text(self.t("preview")).clicked() {
                    self.act("preview", json!({"id":id,"wheel":"ping"}));
                }
            }
        });
        ui.separator();
        self.slider(ui, "volume", 0.0, 100.0, 1.0, " %");
        self.bool_row(ui, "muted");
        self.bool_row(ui, "tickSound");
        self.slider(ui, "pingSizePx", 60.0, 220.0, 1.0, " px");
        self.slider(ui, "pingDurationS", 1.0, 8.0, 0.1, " s");
        self.bool_row(ui, "keepLastPing");
        self.slider(ui, "wheelSizePx", 300.0, 900.0, 10.0, " px");
        self.selection(ui, "clickPingId", false);
        ui.separator();
        self.wheel(ui, "wheel", false);
    }
    fn texture(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        bytes: &[u8],
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.textures.get(id) {
            return Some(texture.clone());
        }
        let image = image::load_from_memory(bytes)
            .ok()?
            .thumbnail(128, 128)
            .to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let texture = ctx.load_texture(
            id,
            egui::ColorImage::from_rgba_unmultiplied(size, &image),
            egui::TextureOptions::LINEAR,
        );
        while self.textures.len() >= 48 {
            if let Some(old) = self.texture_order.pop_front() {
                self.textures.remove(&old);
            } else {
                break;
            }
        }
        self.texture_order.push_back(id.into());
        self.textures.insert(id.into(), texture.clone());
        Some(texture)
    }
    fn emote_thumbnail(
        &mut self,
        ctx: &egui::Context,
        emote: &Value,
    ) -> Option<egui::TextureHandle> {
        let id = emote["id"].as_str()?;
        if let Some(texture) = self.textures.get(id) {
            return Some(texture.clone());
        }
        let (_, data) = emote["thumbnail"].as_str()?.split_once(',')?;
        if data.len() > 2 * 1024 * 1024 {
            return None;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .ok()?;
        self.texture(ctx, id, &bytes)
    }
    fn emotes(&mut self, ui: &mut egui::Ui) {
        ui.weak(self.t(if self.state["extraPackInstalled"] == true {
            "packInstalled"
        } else {
            "packHint"
        }));
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.t("addImage")).clicked() {
                self.choose_file(ui.ctx(), "image", "");
            }
            if ui
                .add_enabled(
                    self.state["extraPackInstalled"] != true,
                    egui::Button::new(self.t("importPack")),
                )
                .clicked()
            {
                self.choose_file(ui.ctx(), "pack", "");
            }
            if ui.button(self.t("downloadPack")).clicked() {
                self.act("releases", json!({}));
            }
        });
        let zh = self.zh();
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text(tr(zh, "search"))
                .desired_width(260.0),
        );
        let search = self.search.to_lowercase();
        let items: Vec<Value> = self.state["emotes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| {
                search.is_empty()
                    || self.emote_name(e).to_lowercase().contains(&search)
                    || e["id"].as_str().unwrap_or("").contains(&search)
            })
            .cloned()
            .collect();
        let columns = ((ui.available_width() / 160.0).floor() as usize).clamp(2, 5);
        for row in items.chunks(columns) {
            ui.horizontal(|ui| {
                for emote in row {
                    egui::Frame::group(ui.style())
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.set_width(132.0);
                            ui.vertical_centered(|ui| {
                                let (rect, response) =
                                    ui.allocate_exact_size(Vec2::splat(86.0), egui::Sense::click());
                                if ui.is_rect_visible(rect) {
                                    if let Some(texture) = self.emote_thumbnail(ui.ctx(), emote) {
                                        ui.painter().image(
                                            texture.id(),
                                            rect,
                                            egui::Rect::from_min_max(
                                                egui::Pos2::ZERO,
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            Color32::WHITE,
                                        );
                                    } else {
                                        ui.painter().text(
                                            rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "—",
                                            egui::FontId::proportional(20.0),
                                            ui.visuals().weak_text_color(),
                                        );
                                    }
                                }
                                if response.on_hover_text(self.t("preview")).clicked() {
                                    self.act("preview", json!({"id":emote["id"],"wheel":"emote"}));
                                }
                                ui.label(self.emote_name(emote));
                                if emote["custom"] == true {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.small_button(self.t("sound")).clicked() {
                                            self.choose_file(
                                                ui.ctx(),
                                                "sound",
                                                emote["id"].as_str().unwrap_or(""),
                                            );
                                        }
                                        if ui.small_button(self.t("remove")).clicked() {
                                            self.confirm = Some(Confirm::Remove(
                                                emote["id"].as_str().unwrap_or("").into(),
                                            ));
                                        }
                                    });
                                }
                            });
                        });
                }
            });
        }
        self.slider(ui, "emoteSizePx", 80.0, 300.0, 1.0, " px");
        self.bool_row(ui, "emoteSound");
        self.selection(ui, "clickEmoteId", true);
        ui.separator();
        self.wheel(ui, "emoteWheel", true);
    }
    fn preferences(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        let language = self.state["settings"]["language"]
            .as_str()
            .unwrap_or("auto")
            .to_owned();
        ui.horizontal(|ui| {
            ui.add_sized([180.0, 25.0], egui::Label::new(tr(zh, "language")));
            egui::ComboBox::from_id_salt("language")
                .width(195.0)
                .selected_text(match language.as_str() {
                    "zh-CN" => "简体中文",
                    "en" => "English",
                    _ => tr(zh, "auto"),
                })
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
                });
        });
        let theme = self.state["preferences"]["theme"]
            .as_str()
            .unwrap_or("system")
            .to_owned();
        ui.horizontal(|ui| {
            ui.add_sized([180.0, 25.0], egui::Label::new(tr(zh, "theme")));
            egui::ComboBox::from_id_salt("theme")
                .width(195.0)
                .selected_text(tr(zh, &theme))
                .show_ui(ui, |ui| {
                    for next in ["system", "light", "dark"] {
                        if ui.selectable_label(theme == next, tr(zh, next)).clicked() {
                            self.state["preferences"]["theme"] = json!(next);
                            self.act("preferences", json!({"theme":next}));
                        }
                    }
                });
        });
        let mut autostart = self.state["preferences"]["autostart"]
            .as_bool()
            .unwrap_or(false);
        if ui.checkbox(&mut autostart, tr(zh, "autostart")).changed() {
            self.state["preferences"]["autostart"] = json!(autostart);
            self.act("preferences", json!({"autostart":autostart}));
        }
        if ui.button(tr(zh, "reset")).clicked() {
            self.confirm = Some(Confirm::Reset);
        }
    }
    fn dialog(&mut self, ctx: &egui::Context) {
        if self.capture.is_some() {
            let zh = self.zh();
            let key = self.capture.as_ref().unwrap().key;
            egui::Window::new(tr(zh, "custom"))
                .id(egui::Id::unique("capture"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_min_width(350.0);
                    ui.label(tr(
                        zh,
                        if key == "toggleHotkey" {
                            "captureShortcut"
                        } else {
                            "capture"
                        },
                    ));
                    ui.weak(tr(zh, "capturePauseHelp"));
                    if !self.hint.is_empty() {
                        ui.colored_label(Color32::from_rgb(220, 150, 45), &self.hint);
                    }
                    if ui.button(tr(zh, "cancel")).clicked() {
                        self.end_capture();
                    }
                });
        }
        if self.confirm.is_some() {
            let zh = self.zh();
            let reset = matches!(self.confirm, Some(Confirm::Reset));
            egui::Window::new(tr(zh, "confirm"))
                .id(egui::Id::unique("confirm"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(tr(
                        zh,
                        if reset {
                            "resetConfirm"
                        } else {
                            "deleteConfirm"
                        },
                    ));
                    ui.horizontal(|ui| {
                        if ui.button(tr(zh, "cancel")).clicked() {
                            self.confirm = None;
                        }
                        if ui.button(tr(zh, "confirm")).clicked() {
                            match self.confirm.take() {
                                Some(Confirm::Reset) => {
                                    for (key, value) in
                                        crate::settings::defaults().as_object().unwrap()
                                    {
                                        if key != "customEmotes" && key != "version" {
                                            self.patch(key, value.clone(), false);
                                        }
                                    }
                                }
                                Some(Confirm::Remove(id)) => {
                                    self.act("emote/remove", json!({"id":id}))
                                }
                                None => {}
                            }
                        }
                    });
                });
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.messages(ctx);
        if self.close_allowed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.test_exit.is_some_and(|at| Instant::now() >= at) {
            self.closing = true;
            self.test_exit = None;
        }
        if let Some(at) = self.test_exit {
            ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
        }
        if self.closing {
            self.end_capture();
            self.flush();
            if self.pending.is_empty()
                && self.dirty.is_empty()
                && self.failed_save.is_none()
                && !self.file_dialog
            {
                self.close_allowed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        if self.save_at.is_some_and(|at| at <= Instant::now()) {
            self.flush();
        }
        if let Some(at) = self.save_at.filter(|_| self.saving.is_none()) {
            ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.capture_input(ctx);
        let theme = self.state["preferences"]["theme"]
            .as_str()
            .unwrap_or("system");
        if self.theme != theme {
            self.theme = theme.into();
            ctx.set_theme(match theme {
                "light" => egui::ThemePreference::Light,
                "dark" => egui::ThemePreference::Dark,
                _ => egui::ThemePreference::System,
            });
        }
        if self.title != self.t("settingsTitle") {
            self.title = self.t("settingsTitle").into();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title.clone()));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.close_allowed {
            return;
        }
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.set_max_width(930.0);
                    ui.add_space(14.0);
                    self.header(ui);
                    ui.horizontal(|ui| {
                        if self.saving.is_some() || !self.dirty.is_empty() || self.busy() {
                            ui.spinner();
                        }
                        ui.weak(self.t(if self.closing {
                            "closeSaving"
                        } else if self.saving.is_some() || !self.dirty.is_empty() {
                            "saving"
                        } else if self.busy() {
                            "working"
                        } else {
                            "saved"
                        }));
                    });
                    if let Some(error) = self.state["status"]["error"].as_str() {
                        ui.colored_label(Color32::from_rgb(220, 90, 70), self.human_error(error));
                    }
                    if !self.error.is_empty() {
                        ui.colored_label(Color32::from_rgb(220, 90, 70), &self.error);
                    }
                    if self.failed_save.is_some() {
                        ui.horizontal(|ui| {
                            if ui.button(self.t("retrySave")).clicked() {
                                if let Some(patch) = self.failed_save.take() {
                                    for (key, value) in patch {
                                        self.dirty.entry(key).or_insert(value);
                                    }
                                }
                                self.error.clear();
                                self.flush();
                            }
                            if self.closing && ui.button(self.t("discardClose")).clicked() {
                                self.close_allowed = true;
                                ctx.request_repaint();
                            }
                        });
                    }
                    if self.state.is_null() {
                        if self.pending.is_empty() && ui.button(self.t("retryLoad")).clicked() {
                            self.request("state", json!({}));
                        }
                        return;
                    }
                    let enabled = !self.busy()
                        && !self.closing
                        && self.capture.is_none()
                        && self.confirm.is_none();
                    ui.add_enabled_ui(enabled, |ui| {
                        for (name, section) in [
                            ("triggers", Self::controls as fn(&mut Self, &mut egui::Ui)),
                            ("pings", Self::pings),
                            ("emotes", Self::emotes),
                            ("preferences", Self::preferences),
                        ] {
                            ui.add_space(12.0);
                            ui.label(RichText::new(self.t(name)).size(18.0).strong());
                            egui::Frame::group(ui.style())
                                .inner_margin(16.0)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    section(self, ui);
                                });
                        }
                    });
                    ui.add_space(14.0);
                    ui.weak(self.t("native"));
                    ui.small(self.t("attribution"));
                    ui.add_space(16.0);
                });
        });
        self.dialog(&ctx);
        // Report readiness after a rendered frame, not merely after spawning a
        // process or opening its input pipe. This is also the lifecycle-test hook.
        if !self.ready_sent {
            self.ui_frames += 1;
            if self.ui_frames >= 2 {
                self.ready_sent = true;
                self.request("ui/ready", json!({"pid":std::process::id()}));
            } else {
                ctx.request_repaint();
            }
        }
    }
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // A compact OFL subset guarantees all bundled Chinese labels even on minimal
    // Linux images. Prefer a system CJK font for user-provided emote names.
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
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("system-cjk".into());
        }
    }
    fonts.font_data.insert(
        "lolping-cjk".into(),
        egui::FontData::from_static(include_bytes!(
            "../../resources/fonts/LOLPingUI-Regular.otf"
        ))
        .into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("lolping-cjk".into());
    }
    ctx.set_fonts(fonts);
}

fn ping_icon(id: &str) -> &'static [u8] {
    match id {
        "danger" => include_bytes!("../../resources/textures/pingwheel_retreatrender.png"),
        "push" => include_bytes!("../../resources/textures/push.png"),
        "omw" => include_bytes!("../../resources/textures/omw.png"),
        "allin" => include_bytes!("../../resources/textures/all_in.png"),
        "assist" => include_bytes!("../../resources/textures/assist_me.png"),
        "needvision" => include_bytes!("../../resources/textures/need_vision.png"),
        "missing" => include_bytes!("../../resources/textures/enemy_missing.png"),
        "enemyvision" => include_bytes!("../../resources/textures/pingwheel_enemyvisionrender.png"),
        "bait" => include_bytes!("../../resources/textures/pingwheel_baitrender.png"),
        "visioncleared" => include_bytes!("../../resources/textures/vision_cleared.png"),
        "missingflip" => include_bytes!("../../resources/textures/enemy_missing_flipped.png"),
        _ => include_bytes!("../../resources/textures/generic_ping.png"),
    }
}

pub fn run() -> Result<(), String> {
    let icon =
        image::load_from_memory(include_bytes!("../../resources/textures/enemy_missing.png"))
            .ok()
            .map(|image| {
                let image = image.to_rgba8();
                egui::IconData {
                    width: image.width(),
                    height: image.height(),
                    rgba: image.into_raw(),
                }
            });
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([900.0, 760.0])
        .with_min_inner_size([660.0, 500.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "LOLPing Lite · Settings",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_settings_have_bilingual_labels() {
        for key in crate::settings::defaults().as_object().unwrap().keys() {
            if !["version", "customEmotes", "toggleHotkey"].contains(&key.as_str()) {
                assert!(!tr(true, key).is_empty(), "missing Chinese label: {key}");
                assert!(!tr(false, key).is_empty(), "missing English label: {key}");
            }
        }
    }
    #[test]
    fn late_save_reply_preserves_newer_slider_input() {
        let (mut app, output) = test_app();
        app.patch("volume", json!(15), false);
        app.flush();
        let first = output.recv().unwrap();
        app.patch("volume", json!(80), true);
        let mut acknowledged = crate::settings::defaults();
        acknowledged["volume"] = json!(15);
        app.local
            .send(Incoming::Message(
                json!({"id":first["id"],"result":acknowledged}),
            ))
            .unwrap();
        app.messages(&egui::Context::default());
        assert_eq!(app.state["settings"]["volume"], 80);
        assert!(app.saving.is_none());
        app.flush();
        let next = output.recv().unwrap();
        assert_eq!(next["params"], json!({"volume":80}));
        assert!(app.saving.is_some());
    }
    #[test]
    fn failed_write_is_retained_when_another_control_changes() {
        let (mut app, output) = test_app();
        app.patch("volume", json!(15), false);
        app.flush();
        let first = output.recv().unwrap();
        app.local
            .send(Incoming::Message(
                json!({"id":first["id"],"error":"disk full"}),
            ))
            .unwrap();
        app.messages(&egui::Context::default());
        assert!(app.failed_save.is_some());
        app.patch("muted", json!(true), false);
        app.flush();
        assert_eq!(
            output.recv().unwrap()["params"],
            json!({"volume":15,"muted":true})
        );
    }
    fn test_app() -> (App, mpsc::Receiver<Value>) {
        let (tx, output) = mpsc::channel();
        let (local, incoming) = mpsc::channel();
        (
            App {
                tx,
                incoming,
                local,
                pending: HashMap::new(),
                next_id: 1,
                state: json!({"settings":crate::settings::defaults(),"platform":"win","locale":"en"}),
                dirty: Map::new(),
                save_at: None,
                saving: None,
                failed_save: None,
                capture: None,
                confirm: None,
                error: String::new(),
                hint: String::new(),
                file_dialog: false,
                closing: false,
                close_allowed: false,
                ready_sent: false,
                ui_frames: 0,
                theme: String::new(),
                title: String::new(),
                textures: HashMap::new(),
                texture_order: VecDeque::new(),
                search: String::new(),
                test_exit: None,
            },
            output,
        )
    }
}
