//! Native settings live in a short-lived process. Only the parent owns settings
//! and media files; this process exchanges JSON lines over inherited pipes.
use eframe::egui::{self, Color32, Vec2};
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    sync::{mpsc, OnceLock},
    time::{Duration, Instant},
};

mod design;
mod keys;
mod pages;
mod resize_trace;
mod thumbnails;
mod wheel_editor;

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
    textures: thumbnails::Cache,
    search: String,
    page: pages::Page,
    emote_page: bool,
    ping_editor: wheel_editor::EditorState,
    emote_editor: wheel_editor::EditorState,
    preview_ping: String,
    test_exit: Option<Instant>,
    resize_trace: Option<resize_trace::Trace>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        design::install(&cc.egui_ctx);
        // Local visual QA uses app scaling without changing Windows display
        // settings. No preference or production IPC field is created.
        if let Some(scale) = std::env::var("LOLPING_UI_TEST_SCALE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| (1.0..=2.0).contains(v))
        {
            cc.egui_ctx.set_zoom_factor(scale);
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
            textures: thumbnails::Cache::default(),
            search: String::new(),
            page: pages::Page::default(),
            emote_page: false,
            ping_editor: wheel_editor::EditorState::default(),
            emote_editor: wheel_editor::EditorState::default(),
            preview_ping: "missing".into(),
            test_exit: std::env::var("LOLPING_UI_TEST_EXIT_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(|ms| Instant::now() + Duration::from_millis(ms)),
            resize_trace: resize_trace::Trace::from_env(),
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
    fn wayland(&self) -> bool {
        self.platform() == "linux" && self.state["linuxSession"]["wayland"] == true
    }
    fn gnome_wayland(&self) -> bool {
        self.wayland() && self.state["linuxSession"]["desktop"] == "gnome"
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
        if let Some(patch) = &self.failed_save {
            merge(&mut value, patch);
        }
        if let Some(pending) = self.saving.and_then(|id| self.pending.get(&id)) {
            if let Some(patch) = pending.params.as_object() {
                merge(&mut value, patch);
            }
        }
        merge(&mut value, &self.dirty);
        self.state["settings"] = value;
    }
    fn flush(&mut self) {
        if self.saving.is_none() && !self.dirty.is_empty() {
            // A failed acknowledgement can arrive after the next edit was queued.
            // Retry those fields with the next write, keeping the latest edits.
            if let Some(failed) = self.failed_save.take() {
                for (key, value) in failed {
                    self.dirty.entry(key).or_insert(value);
                }
                self.error.clear();
            }
            let patch = std::mem::take(&mut self.dirty);
            let id = self.request("settings", Value::Object(patch));
            self.saving = Some(id);
            self.save_at = None;
        }
    }
    fn prepare_close(&mut self) -> bool {
        self.end_capture();
        self.flush();
        self.pending.is_empty()
            && self.dirty.is_empty()
            && self.failed_save.is_none()
            && !self.file_dialog
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
                            "quit" => self.closing = true,
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
                        "wayland/install" => {
                            self.hint = self
                                .t(if result["reloginRequired"] == true {
                                    "waylandRelogin"
                                } else {
                                    "waylandInstalled"
                                })
                                .into();
                        }
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
    fn texture(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        bytes: &'static [u8],
    ) -> Option<egui::TextureHandle> {
        self.textures
            .request(ctx, id, || thumbnails::Source::Embedded(bytes))
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
            if self.prepare_close() {
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
            .unwrap_or("dark");
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let sampled_at = self.resize_trace.as_ref().map(|_| Instant::now());
        let ctx = ui.ctx().clone();
        if self.close_allowed {
            return;
        }
        self.draw_settings(ui);
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
        if let (Some(trace), Some(started)) = (&mut self.resize_trace, sampled_at) {
            trace.record(
                ctx.content_rect().size(),
                frame.info().cpu_usage,
                started.elapsed().as_secs_f64() * 1000.0,
            );
        }
    }
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
    let preview_width = std::env::var("LOLPING_UI_TEST_WIDTH")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (760.0..=1600.0).contains(v))
        .unwrap_or(1120.0);
    let preview_height = std::env::var("LOLPING_UI_TEST_HEIGHT")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (560.0..=1200.0).contains(v))
        .unwrap_or(840.0);
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([preview_width, preview_height])
        .with_min_inner_size([760.0, 560.0])
        .with_clamp_size_to_monitor_size(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let mut options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        persist_window: false,
        centered: true,
        ..Default::default()
    };
    if let Ok(value) = std::env::var("LOLPING_UI_TEST_VSYNC") {
        options.glow_options.vsync = value != "0";
    }
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

    fn visible_text(app: &mut App) -> String {
        fn text(shape: &egui::Shape, output: &mut String) {
            match shape {
                egui::Shape::Text(shape) => {
                    output.push_str(shape.galley.text());
                    output.push('\n');
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        text(shape, output);
                    }
                }
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1120.0, 1600.0),
                )),
                ..Default::default()
            },
            |ui| app.draw_settings(ui),
        );
        let mut content = String::new();
        for shape in &output.shapes {
            text(&shape.shape, &mut content);
        }
        output.textures_delta.clear();
        content
    }

    #[test]
    fn wayland_uses_actual_shortcuts_and_preserves_existing_trigger_preferences() {
        let (mut app, output) = test_app();
        app.state["platform"] = json!("linux");
        app.state["linuxSession"] = json!({"wayland":true,"desktop":"gnome"});
        let original = app.state["settings"].clone();
        for (page, shortcut, inactive) in [
            (pages::Page::Trigger, "Super + Alt + Q", "Drag threshold"),
            (pages::Page::Hotkey, "Super + Alt + P", "Record shortcut"),
            (pages::Page::Emotes, "Super + Alt + E", "Click to emote"),
        ] {
            app.page = page;
            app.emote_page = true;
            let text = visible_text(&mut app);
            assert!(text.contains(shortcut), "missing {shortcut}: {text}");
            assert!(
                !text.contains(inactive),
                "inactive control still shown: {inactive}"
            );
            assert!(text.contains("Install / enable GNOME extension"));
        }
        assert_eq!(app.state["settings"], original);
        assert!(app.dirty.is_empty());
        assert!(output.try_recv().is_err());
        app.state["linuxSession"]["wayland"] = json!(false);
        app.page = pages::Page::Trigger;
        assert!(visible_text(&mut app).contains("Drag threshold"));
    }

    #[test]
    fn other_wayland_desktops_do_not_offer_incompatible_extension_installation() {
        let (mut app, _) = test_app();
        app.state["platform"] = json!("linux");
        app.state["linuxSession"] = json!({"wayland":true,"desktop":"kde"});
        let text = visible_text(&mut app);
        assert!(text.contains("Other Wayland desktops are not supported yet"));
        assert!(!text.contains("Install / enable GNOME extension"));
    }

    #[test]
    fn extension_install_reply_explains_when_a_new_login_is_required() {
        for relogin in [false, true] {
            let (mut app, output) = test_app();
            app.act("wayland/install", json!({}));
            let request = output.recv().unwrap();
            app.local
                .send(Incoming::Message(json!({
                    "id": request["id"], "result": {"ok":true,"reloginRequired":relogin}
                })))
                .unwrap();
            app.messages(&egui::Context::default());
            assert!(app.error.is_empty());
            assert!(app
                .hint
                .contains(if relogin { "Sign out" } else { "Retry start" }));
        }
    }

    #[test]
    fn oversized_window_keeps_all_visible_emotes_resident_after_warmup() {
        use base64::Engine;
        let (mut app, _) = test_app();
        let thumbnail = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD
                .encode(include_bytes!("../../resources/emotes/bee_happy.png"))
        );
        let library_count = crate::settings::catalog().as_array().unwrap().len() + 24;
        app.state["emotes"] = Value::Array(
            (0..library_count)
                .map(|i| {
                    json!({
                        "id": format!("test-emote-{i}"), "name": format!("Emote {i}"),
                        "thumbnail": thumbnail, "custom": i >= 44,
                    })
                })
                .collect(),
        );
        let expected = library_count + crate::settings::PINGS.len() + 1;
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(10);
        app.page = pages::Page::Emotes;
        loop {
            for id in crate::settings::PINGS {
                app.texture(&ctx, &format!("ping-{id}"), ping_icon(id));
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(3840.0, 2560.0),
                    )),
                    ..Default::default()
                },
                |ui| app.draw_settings(ui),
            );
            output.textures_delta.clear();
            if app.textures.counts() == (expected, 0) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "whole library never became resident: {:?}",
                app.textures.counts()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        for frame in 0..100 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(3840.0 - (frame % 300) as f32, 2560.0),
                    )),
                    ..Default::default()
                },
                |ui| app.draw_settings(ui),
            );
            output.textures_delta.clear();
            assert_eq!(
                app.textures.counts(),
                (expected, 0),
                "resize restarted decoding"
            );
        }
    }

    #[test]
    #[ignore = "opt-in CPU-only resize benchmark; creates no native window"]
    fn resize_layout_cpu_profile() {
        use base64::Engine;
        let mut report = Vec::new();
        let thumbnail = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD
                .encode(include_bytes!("../../resources/emotes/bee_happy.png"))
        );
        for page in [
            pages::Page::Trigger,
            pages::Page::Sound,
            pages::Page::Wheel,
            pages::Page::Emotes,
        ] {
            let (mut app, _) = test_app();
            app.textures.synchronous =
                std::env::var("LOLPING_UI_TEST_SYNC_THUMBNAILS").as_deref() == Ok("1");
            app.page = page;
            app.state["emotes"] = Value::Array(
                (0..68)
                    .map(|index| {
                        json!({
                            "id": format!("test-emote-{index}"),
                            "name": format!("Test emote {index}"), "thumbnail": thumbnail,
                            "custom": index >= 44,
                        })
                    })
                    .collect(),
            );
            let ctx = egui::Context::default();
            design::install(&ctx);
            let mut samples = Vec::new();
            let mut cold = Vec::new();
            for frame in 0..320 {
                let size = Vec2::new(
                    760.0 + ((frame * 13) % 601) as f32,
                    560.0 + ((frame * 7) % 401) as f32,
                );
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                };
                let started = Instant::now();
                let mut output = ctx.run_ui(input, |ui| app.draw_settings(ui));
                std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
                output.textures_delta.clear();
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                if frame < 20 {
                    cold.push(elapsed);
                } else {
                    samples.push(elapsed);
                }
                // Let asynchronous decode progress without counting worker time
                // or sleep in UI CPU measurements. No window / display is opened.
                std::thread::sleep(Duration::from_millis(2));
            }
            samples.sort_by(f64::total_cmp);
            report.push(json!({"page": format!("{page:?}"),
                "frames": samples.len(), "medianMs": samples[samples.len()/2],
                "p95Ms": samples[samples.len()*95/100],
                "maxMs": samples.last(), "coldFramesMs": cold,
            }));
        }
        let report = json!({"scope":"CPU UI layout + tessellation; excludes OpenGL, vsync, OS event loop and presentation", "pages": report});
        eprintln!("{}", serde_json::to_string_pretty(&report).unwrap());
        if let Some(path) = std::env::var_os("LOLPING_UI_TEST_CPU_REPORT") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires the private Xvfb started by scripts/test-linux-surface.py"]
    fn linux_native_import_dialog_selects_and_cancels_without_portal() {
        use std::{
            ffi::{CStr, CString},
            sync::{
                atomic::{AtomicBool, Ordering},
                Arc,
            },
        };
        assert_eq!(std::env::var("LOLPING_PRIVATE_XVFB").as_deref(), Ok("1"));
        assert_eq!(std::env::var("GTK_USE_PORTAL").as_deref(), Ok("0"));
        let directory = tempfile::tempdir().unwrap();
        let selected = directory.path().join("import-test.png");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/plugin.png"),
            &selected,
        )
        .unwrap();
        let (mut app, output) = test_app();
        let context = egui::Context::default();
        // Exercise exactly the settings page's worker-thread picker. The GTK
        // actions below execute on rfd's own GTK thread via its main context.
        // They only address this process's mapped file chooser; no real input
        // is injected and no portal or external zenity program is involved.
        for accept in [true, false] {
            let seen = Arc::new(AtomicBool::new(false));
            let observed = seen.clone();
            let filename = CString::new(selected.to_str().unwrap()).unwrap();
            let title = app.t("selectFile").to_owned();
            let began = Instant::now();
            let mut selection_set = false;
            gtk::glib::timeout_add(Duration::from_millis(150), move || unsafe {
                use gtk::{
                    ffi as g,
                    glib::{ffi, gobject_ffi, ControlFlow},
                };
                if began.elapsed() > Duration::from_secs(10) {
                    return ControlFlow::Break;
                }
                let windows = g::gtk_window_list_toplevels();
                let mut item = windows;
                let mut completed = false;
                while !item.is_null() {
                    let widget = (*item).data as *mut g::GtkWidget;
                    let window = widget as *mut g::GtkWindow;
                    let caption = g::gtk_window_get_title(window);
                    if !caption.is_null()
                        && CStr::from_ptr(caption).to_bytes() == title.as_bytes()
                        && g::gtk_widget_get_mapped(widget) != 0
                        && gobject_ffi::g_type_check_instance_is_a(
                            widget.cast(),
                            g::gtk_file_chooser_get_type(),
                        ) != 0
                        && gobject_ffi::g_type_check_instance_is_a(
                            widget.cast(),
                            g::gtk_dialog_get_type(),
                        ) != 0
                    {
                        observed.store(true, Ordering::Release);
                        if !accept {
                            g::gtk_dialog_response(widget.cast(), g::GTK_RESPONSE_CANCEL);
                            completed = true;
                        } else if !selection_set {
                            g::gtk_file_chooser_set_filename(widget.cast(), filename.as_ptr());
                            selection_set = true;
                        } else {
                            let picked = g::gtk_file_chooser_get_filename(widget.cast());
                            if !picked.is_null() {
                                let ready =
                                    CStr::from_ptr(picked).to_bytes() == filename.as_bytes();
                                ffi::g_free(picked.cast());
                                if ready {
                                    g::gtk_dialog_response(widget.cast(), g::GTK_RESPONSE_ACCEPT);
                                    completed = true;
                                }
                            }
                        }
                        break;
                    }
                    item = (*item).next;
                }
                ffi::g_list_free(windows);
                if completed {
                    ControlFlow::Break
                } else {
                    ControlFlow::Continue
                }
            });
            app.choose_file(&context, "image", "");
            let deadline = Instant::now() + Duration::from_secs(12);
            while app.file_dialog && Instant::now() < deadline {
                app.messages(&context);
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(
                seen.load(Ordering::Acquire),
                "native GTK file chooser was never mapped"
            );
            assert!(!app.file_dialog, "native GTK file chooser did not complete");
            if accept {
                let message = output.recv_timeout(Duration::from_secs(1)).unwrap();
                assert_eq!(message["method"], "import");
                assert_eq!(message["params"]["kind"], "image");
                assert_eq!(message["params"]["path"], selected.to_str().unwrap());
            } else {
                assert!(
                    output.try_recv().is_err(),
                    "cancelling must not submit an import"
                );
            }
        }
    }

    #[test]
    fn changing_tabs_keeps_queued_edits_and_independent_wheel_selection() {
        let (mut app, output) = test_app();
        app.patch("volume", json!(42), true);
        app.ping_editor.target = wheel_editor::Target::Slot(6);
        app.page = pages::Page::Emotes;
        app.emote_editor.target = wheel_editor::Target::Center;
        app.search = "poro".into();
        app.page = pages::Page::Sound;
        assert_eq!(app.state["settings"]["volume"], 42);
        assert_eq!(app.ping_editor.target, wheel_editor::Target::Slot(6));
        assert_eq!(app.emote_editor.target, wheel_editor::Target::Center);
        assert_eq!(app.search, "poro");
        app.flush();
        assert_eq!(output.recv().unwrap()["params"], json!({"volume":42}));
        assert_eq!(
            app.pending
                .values()
                .filter(|p| p.method == "settings")
                .count(),
            1
        );
        assert!(!app
            .pending
            .values()
            .any(|p| p.method == "start" || p.method == "stop" || p.method == "suspend"));
    }
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
    #[test]
    fn failed_reply_preserves_a_newer_edit_already_queued() {
        let (mut app, output) = test_app();
        app.patch("volume", json!(15), false);
        app.flush();
        let first = output.recv().unwrap();
        app.patch("volume", json!(80), true);
        app.local
            .send(Incoming::Message(
                json!({"id":first["id"],"error":"temporary write failure"}),
            ))
            .unwrap();
        app.local
            .send(Incoming::Message(json!({
                "event":"lolping/settings",
                "params":{"settings":crate::settings::defaults()}
            })))
            .unwrap();
        app.messages(&egui::Context::default());
        assert_eq!(app.state["settings"]["volume"], 80);
        app.flush();
        let retry = output.recv().unwrap();
        assert_eq!(retry["params"], json!({"volume":80}));
        assert!(app.failed_save.is_none());
        let mut acknowledged = crate::settings::defaults();
        acknowledged["volume"] = json!(80);
        app.local
            .send(Incoming::Message(
                json!({"id":retry["id"],"result":acknowledged}),
            ))
            .unwrap();
        app.messages(&egui::Context::default());
        assert_eq!(app.state["settings"]["volume"], 80);
        assert!(app.error.is_empty());
        assert!(app.prepare_close());
    }
    #[test]
    fn failed_reply_retains_other_fields_with_an_edit_already_queued() {
        let (mut app, output) = test_app();
        app.patch("volume", json!(15), false);
        app.flush();
        let first = output.recv().unwrap();
        app.patch("muted", json!(true), true);
        app.local
            .send(Incoming::Message(
                json!({"id":first["id"],"error":"temporary write failure"}),
            ))
            .unwrap();
        app.messages(&egui::Context::default());
        app.flush();
        assert_eq!(
            output.recv().unwrap()["params"],
            json!({"volume":15,"muted":true})
        );
        assert!(app.failed_save.is_none());
    }
    #[test]
    fn parent_quit_flushes_debounced_edits_and_releases_capture_before_close() {
        let (mut app, output) = test_app();
        app.begin_capture("trigger");
        let suspend = output.recv().unwrap();
        app.local
            .send(Incoming::Message(
                json!({"id":suspend["id"],"result":{"ok":true}}),
            ))
            .unwrap();
        app.patch("volume", json!(80), true);
        app.local
            .send(Incoming::Message(json!({"event":"quit","params":{}})))
            .unwrap();
        app.messages(&egui::Context::default());
        assert!(app.closing);
        assert!(!app.close_allowed);
        assert!(!app.prepare_close());
        assert!(app.capture.is_none());
        let resume = output.recv().unwrap();
        let save = output.recv().unwrap();
        assert_eq!(resume["method"], "suspend");
        assert_eq!(resume["params"], json!({"on":false}));
        assert_eq!(save["method"], "settings");
        assert_eq!(save["params"], json!({"volume":80}));
        app.local
            .send(Incoming::Message(
                json!({"id":resume["id"],"result":{"ok":true}}),
            ))
            .unwrap();
        app.messages(&egui::Context::default());
        assert!(!app.prepare_close());
        let mut acknowledged = crate::settings::defaults();
        acknowledged["volume"] = json!(80);
        app.local
            .send(Incoming::Message(
                json!({"id":save["id"],"result":acknowledged}),
            ))
            .unwrap();
        app.messages(&egui::Context::default());
        assert!(app.prepare_close());
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
                textures: thumbnails::Cache::default(),
                search: String::new(),
                page: pages::Page::default(),
                emote_page: false,
                ping_editor: wheel_editor::EditorState::default(),
                emote_editor: wheel_editor::EditorState::default(),
                preview_ping: "missing".into(),
                test_exit: None,
                resize_trace: None,
            },
            output,
        )
    }
}
