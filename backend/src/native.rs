use crate::{
    audio::AudioWorker,
    render::{
        media::{Bank, DecoderPool},
        surface::Surface,
        Output, Scene,
    },
    service::Shared,
    settings,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc::SyncSender, Arc},
    time::{Duration, Instant},
};
use tao::{
    dpi::{PhysicalPosition, PhysicalSize},
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use tiny_skia::Pixmap;

#[cfg(all(test, target_os = "windows"))]
mod desktop_tests;

pub enum UserEvent {
    OpenSettings,
    SettingsGone(u64),
    Menu(String),
    Control {
        method: String,
        params: Value,
        reply: SyncSender<Result<Value, String>>,
    },
    Settings(Value, bool),
    Helper(u64, Value),
    HelperGone(u64),
    #[cfg(target_os = "linux")]
    Wayland(u64, Value),
    #[cfg(target_os = "linux")]
    DeadlineWake,
    Quit,
}
#[derive(Clone, PartialEq)]
pub struct Display {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
}
impl Display {
    pub fn local(&self, x: f64, y: f64, mac: bool) -> (f64, f64) {
        if mac {
            (
                x - self.x as f64 / self.scale,
                y - self.y as f64 / self.scale,
            )
        } else {
            (
                (x - self.x as f64) / self.scale,
                (y - self.y as f64) / self.scale,
            )
        }
    }
    fn contains(&self, x: f64, y: f64) -> bool {
        let (x, y) = self.local(x, y, cfg!(target_os = "macos"));
        x >= 0. && y >= 0. && x < self.w as f64 / self.scale && y < self.h as f64 / self.scale
    }
}
struct Overlay {
    // Surface must drop before its native window; Scene drops streaming receivers.
    surface: Surface,
    scene: Scene,
    window: Window,
    display: Display,
    ready: bool,
    visible: bool,
    pixels: Option<Pixmap>,
    frames: Vec<Instant>,
    frame_ms: Vec<f64>,
    next_frame: Instant,
}
impl Overlay {
    fn send(&mut self, name: &str, p: Value, pool: &DecoderPool) -> Vec<Output> {
        let now = Instant::now();
        if matches!(name, "wheelOpen" | "click" | "preview") {
            if self.frames.len() >= 128 {
                self.frames.remove(0);
            }
            self.frames.push(now);
        }
        self.scene.event(name, &p, pool, now)
    }
    fn tick(&mut self, now: Instant) -> Result<(), String> {
        if !self.scene.dirty && (!self.scene.animating(now) || now < self.next_frame) {
            return Ok(());
        }
        self.scene.advance(now);
        let Some(bounds) = self
            .scene
            .bounds(self.display.scale, self.display.w, self.display.h)
        else {
            if self.visible {
                self.window.set_visible(false);
                self.visible = false;
            }
            self.surface.clear(&self.window);
            self.pixels = None;
            self.scene.release_buffers();
            self.frames.clear();
            self.scene.dirty = false;
            return Ok(());
        };
        if self
            .pixels
            .as_ref()
            .is_none_or(|p| p.width() != bounds.w || p.height() != bounds.h)
        {
            self.pixels = Some(Pixmap::new(bounds.w, bounds.h).ok_or("nativeSurfaceTooLarge")?);
        }
        let pixels = self.pixels.as_mut().unwrap();
        self.scene.draw(pixels, bounds, self.display.scale, now);
        self.surface.present(
            &self.window,
            pixels,
            bounds,
            (self.display.x, self.display.y),
        )?;
        if !self.visible {
            self.window.set_visible(true);
            self.window.set_always_on_top(true);
            self.visible = true;
        }
        for start in self.frames.drain(..) {
            if self.frame_ms.len() >= 128 {
                self.frame_ms.remove(0);
            }
            self.frame_ms.push(start.elapsed().as_secs_f64() * 1000.);
        }
        self.next_frame = now + Duration::from_micros(16_667);
        Ok(())
    }
}
struct Helper {
    child: Child,
    input: ChildStdin,
    generation: u64,
    ready: bool,
    config: Option<Value>,
    suspended: Option<bool>,
    config_updates: u64,
}
impl Helper {
    fn send(&mut self, v: Value) -> Result<(), String> {
        writeln!(self.input, "{v}").map_err(|e| e.to_string())
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.send(json!({"type":"shutdown"}));
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct Native {
    shared: Arc<Shared>,
    proxy: EventLoopProxy<UserEvent>,
    windows: HashMap<WindowId, Overlay>,
    bank: Option<Arc<Bank>>,
    decoders: DecoderPool,
    helper: Option<Helper>,
    settings: Value,
    enabled: bool,
    generation: u64,
    wheel: Option<WindowId>,
    audio: AudioWorker,
    preview_until: Option<Instant>,
    suspend_until: Option<Instant>,
    last_displays: Vec<Display>,
    last_poll: Instant,
    pause_reason: Option<&'static str>,
}
impl Native {
    pub fn new(shared: Arc<Shared>, proxy: EventLoopProxy<UserEvent>) -> Self {
        let settings = shared.store.lock().unwrap().settings.clone();
        let emitter = shared.clone();
        let audio = AudioWorker::new(shared.root.clone(), shared.data.clone(), move |error| {
            emitter.emit("lolping/audioError", json!({"error":error}))
        });
        Self {
            shared,
            proxy,
            settings,
            windows: HashMap::new(),
            bank: None,
            decoders: DecoderPool::new(),
            helper: None,
            enabled: false,
            generation: 0,
            wheel: None,
            audio,
            preview_until: None,
            suspend_until: None,
            last_displays: Vec::new(),
            last_poll: Instant::now(),
            pause_reason: None,
        }
    }
    fn displays(target: &EventLoopWindowTarget<UserEvent>) -> Vec<Display> {
        target
            .available_monitors()
            .map(|m| {
                let p = m.position();
                let s = m.size();
                Display {
                    x: p.x,
                    y: p.y,
                    w: s.width,
                    h: s.height,
                    scale: m.scale_factor(),
                }
            })
            .collect()
    }
    fn rebuild(&mut self, target: &EventLoopWindowTarget<UserEvent>) -> Result<(), String> {
        let displays = Self::displays(target);
        if displays.is_empty() {
            return Err("noDisplay".into());
        }
        if displays == self.last_displays && !self.windows.is_empty() {
            return Ok(());
        }
        self.windows.clear();
        self.wheel = None;
        self.last_displays.clear();
        if let Some(h) = &mut self.helper {
            h.send(json!({"type":"suspend","on":true}))?;
            h.suspended = Some(true);
        }
        self.bank.get_or_insert_with(|| {
            Arc::new(Bank::load(
                &self.shared.root,
                &self.shared.data,
                &self.settings,
            ))
        });
        for display in &displays {
            let builder = WindowBuilder::new()
                .with_title("lolPing Lite overlay")
                .with_visible(false)
                .with_transparent(true)
                .with_decorations(false)
                .with_resizable(false)
                .with_always_on_top(true)
                .with_focusable(false)
                .with_focused(false)
                .with_inner_size(PhysicalSize::new(2, 2))
                .with_position(PhysicalPosition::new(display.x, display.y));
            #[cfg(target_os = "windows")]
            let builder = {
                use tao::platform::windows::WindowBuilderExtWindows;
                builder
                    .with_skip_taskbar(true)
                    .with_undecorated_shadow(false)
            };
            #[cfg(target_os = "macos")]
            let builder = {
                use tao::platform::macos::WindowBuilderExtMacOS;
                builder.with_has_shadow(false)
            };
            let window = crate::platform::overlay_builder(builder)
                .build(target)
                .map_err(|e| e.to_string())?;
            window
                .set_ignore_cursor_events(true)
                .map_err(|e| e.to_string())?;
            #[cfg(target_os = "macos")]
            unsafe {
                use tao::platform::macos::WindowExtMacOS;
                let ns = window.ns_window() as *mut objc2::runtime::AnyObject;
                let _: () = objc2::msg_send![ns,setLevel:1000isize];
                // CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenAuxiliary.
                let _: () = objc2::msg_send![ns,setCollectionBehavior:337usize];
                let _: () = objc2::msg_send![ns,setHidesOnDeactivate:false];
            }
            let id = window.id();
            let surface = Surface::new(&window)?;
            let scene = Scene::new(
                self.shared.root.clone(),
                self.shared.data.clone(),
                self.settings.clone(),
                self.bank.as_ref().unwrap().clone(),
            );
            self.windows.insert(
                id,
                Overlay {
                    surface,
                    scene,
                    window,
                    display: display.clone(),
                    ready: true,
                    visible: false,
                    pixels: None,
                    frames: Vec::new(),
                    frame_ms: Vec::new(),
                    next_frame: Instant::now(),
                },
            );
        }
        self.last_displays = displays;
        self.shared.emit(
            "lolping/overlayReady",
            json!({"count":self.windows.len(),"renderer":"native"}),
        );
        self.sync_helper();
        Ok(())
    }
    fn all(&mut self, event: &str, value: Value) {
        for w in self.windows.values_mut() {
            w.send(event, value.clone(), &self.decoders);
        }
    }
    fn stop(&mut self) {
        self.helper = None;
        self.windows.clear();
        self.bank = None;
        self.last_displays.clear();
        self.audio.clear();
        self.wheel = None;
        self.enabled = false;
        self.preview_until = None;
        self.suspend_until = None;
        self.pause_reason = None;
        self.shared.status("stopped", false, None);
    }
    fn fail(&mut self, error: String) {
        self.stop();
        self.shared.status("failed", false, Some(error));
    }
    fn start(&mut self, target: &EventLoopWindowTarget<UserEvent>) -> Result<Value, String> {
        crate::platform::global_input_available()?;
        if self.helper.is_some() {
            return Ok(self.shared.status.lock().unwrap().clone());
        }
        self.rebuild(target)?;
        self.preview_until = None;
        self.generation += 1;
        let path = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("missingExecutableDirectory")?
            .join(if cfg!(target_os = "windows") {
                "hook-helper.exe"
            } else {
                "hook-helper"
            });
        let mut cmd = Command::new(path);
        #[cfg(target_os = "linux")]
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| v.is_empty()) {
            cmd.env_remove("WAYLAND_DISPLAY");
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let input = child.stdin.take().ok_or("missingHelperInput")?;
        let output = child.stdout.take().ok_or("missingHelperOutput")?;
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("[helper] {line}");
                }
            });
        }
        let generation = self.generation;
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut line = String::new();
                match reader.by_ref().take(65537).read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    _ => {}
                }
                if line.len() > 65536 {
                    break;
                }
                if let Ok(v) = serde_json::from_str(&line) {
                    let _ = proxy.send_event(UserEvent::Helper(generation, v));
                }
            }
            let _ = proxy.send_event(UserEvent::HelperGone(generation));
        });
        self.enabled = true;
        self.helper = Some(Helper {
            child,
            input,
            generation,
            ready: false,
            config: None,
            suspended: None,
            config_updates: 0,
        });
        self.shared.status("starting", true, None);
        Ok(self.shared.status.lock().unwrap().clone())
    }
    pub fn control(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        method: &str,
        p: Value,
    ) -> Result<Value, String> {
        match method {
            "lolping/start" => {
                let result = self.start(target);
                if let Err(ref e) = result {
                    self.fail(e.clone());
                }
                result
            }
            "lolping/stop" => {
                self.stop();
                Ok(json!({"state":"stopped","enabled":false}))
            }
            "lolping/diagnostics" => Ok(
                json!({"windows":self.windows.len(),"readyWindows":self.windows.values().filter(|w|w.ready).count(),"visibleWindows":self.windows.values().filter(|w|w.visible).count(),"lowMemoryWindows":self.windows.values().filter(|w|w.pixels.is_none()).count(),"renderer":"native","systemFrames":self.windows.values().map(|w|w.surface.system_frame(&w.window)).collect::<Vec<_>>(),"surfaceBytes":self.windows.values().map(|w|w.surface.bytes()+w.pixels.as_ref().map_or(0,|p|p.data().len())).sum::<usize>(),"imageBytes":self.bank.as_ref().map_or(0,|b|b.bytes()),"frameMetric":"dispatchToNativeSubmit","frameMs":self.windows.values().flat_map(|w|w.frame_ms.iter().copied()).collect::<Vec<_>>(),"audioCacheBytes":self.audio.cached_bytes(),"helperConfigWrites":self.helper.as_ref().map(|h|h.config_updates),"helperPid":self.helper.as_ref().map(|h|h.child.id()),"enabled":self.enabled}),
            ),
            "lolping/pause" => {
                if self.helper.is_none() {
                    return Err("notRunning".into());
                }
                self.enabled = p["on"] != true;
                self.pause_reason = (!self.enabled).then_some("manual");
                if !self.enabled {
                    self.all("clear", Value::Null);
                    self.audio.clear();
                    self.wheel = None;
                }
                self.sync_helper();
                Ok(self.shared.status.lock().unwrap().clone())
            }
            "lolping/suspend" => {
                let on = p["on"] == true;
                self.suspend_until = on.then(|| Instant::now() + Duration::from_secs(20));
                if on {
                    self.all("clear", Value::Null);
                    self.wheel = None;
                }
                self.sync_helper();
                Ok(json!({"ok":true}))
            }
            "lolping/permission" => {
                #[cfg(target_os = "macos")]
                {
                    let helper = std::env::current_exe()
                        .map_err(|e| e.to_string())?
                        .parent()
                        .ok_or("missingExecutableDirectory")?
                        .join("hook-helper");
                    let _ = Command::new(helper).arg("--request-access").status();
                    Command::new("open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility").spawn().map_err(|e|e.to_string())?;
                }
                Ok(json!({"ok":true}))
            }
            "lolping/preview" => {
                crate::platform::global_input_available()?;
                let id = p["id"].as_str().ok_or("invalidPreview")?;
                let emote = p["wheel"] == "emote";
                if emote {
                    let store = self.shared.store.lock().unwrap();
                    if !store.available.contains(id) {
                        return Err("missingEmote".into());
                    }
                } else if !settings::PINGS.contains(&id) {
                    return Err("invalidPing".into());
                }
                self.rebuild(target)?;
                if self.helper.is_none() {
                    self.preview_until = Some(Instant::now() + Duration::from_secs(10));
                }
                // Native surfaces are ready synchronously, including the very first preview.
                let primary = target.primary_monitor().map(|m| m.position());
                let selected = self
                    .windows
                    .iter()
                    .find(|(_, w)| {
                        primary.is_some_and(|p| p.x == w.display.x && p.y == w.display.y)
                    })
                    .map(|(id, _)| *id)
                    .or_else(|| self.windows.keys().next().copied())
                    .ok_or("noDisplay")?;
                let window = self.windows.get_mut(&selected).unwrap();
                let message = json!({"id":id,"wheel":if emote{"emote"}else{"ping"},"x":window.display.w as f64/window.display.scale/2.,"y":window.display.h as f64/window.display.scale/2.});
                self.dispatch(selected, "preview", message);
                Ok(json!({"ok":true}))
            }
            _ => Err("unknownControl".into()),
        }
    }
    pub fn refresh(&mut self, s: Value, library_changed: bool) {
        let art_changed = library_changed
            || self.settings["emoteWheel"] != s["emoteWheel"]
            || self.settings["clickEmoteId"] != s["clickEmoteId"];
        self.settings = s;
        if art_changed && self.bank.is_some() {
            self.bank = Some(Arc::new(Bank::load(
                &self.shared.root,
                &self.shared.data,
                &self.settings,
            )));
        }
        if library_changed {
            self.audio.invalidate_custom();
            self.all("library", Value::Null);
        }
        if let Some(bank) = &self.bank {
            for w in self.windows.values_mut() {
                w.scene.settings(self.settings.clone(), bank.clone());
            }
        }
        self.sync_helper();
    }
    fn sync_helper(&mut self) {
        let ready = !self.windows.is_empty() && self.windows.values().all(|w| w.ready);
        if let Some(h) = &mut self.helper {
            if !h.ready {
                return;
            }
            let config = settings::helper_config(&self.settings, self.enabled && ready);
            let suspended = !ready || self.suspend_until.is_some();
            // setConfig cancels an in-flight gesture; appearance/audio edits must not send it.
            let result = (|| {
                if h.config.as_ref() != Some(&config) {
                    h.send(config.clone())?;
                    h.config = Some(config);
                    h.config_updates += 1;
                }
                if h.suspended != Some(suspended) {
                    h.send(json!({"type":"suspend","on":suspended}))?;
                    h.suspended = Some(suspended);
                }
                Ok::<(), String>(())
            })();
            if let Err(e) = result {
                self.fail(e);
            } else {
                self.shared.status_with_pause(
                    if ready { "running" } else { "starting" },
                    self.enabled && ready && !suspended,
                    None,
                    if self.suspend_until.is_some() {
                        Some("capture")
                    } else {
                        self.pause_reason
                    },
                );
            }
        }
    }
    fn dispatch(&mut self, id: WindowId, name: &str, p: Value) {
        let outputs = if let Some(w) = self.windows.get_mut(&id) {
            w.send(name, p, &self.decoders)
        } else {
            return;
        };
        for output in outputs {
            match output {
                Output::Sound(key) => self.renderer(id, json!({"type":"sound","key":key})),
                Output::Selected => self.renderer(id, json!({"type":"selected"})),
            }
        }
    }
    fn renderer(&mut self, id: WindowId, v: Value) {
        match v["type"].as_str().unwrap_or("") {
            "sound" => {
                let key = v["key"].as_str().unwrap_or("");
                let volume = if self.settings["muted"] == true {
                    0.
                } else {
                    self.settings["volume"].as_f64().unwrap_or(70.) as f32 / 100.
                };
                let volume = if key == "button" {
                    if self.settings["tickSound"] == true {
                        volume * 0.5
                    } else {
                        0.
                    }
                } else if key.starts_with("emote:") && self.settings["emoteSound"] != true {
                    0.
                } else {
                    volume
                };
                if let Err(e) = self
                    .audio
                    .play(&self.shared.root, &self.shared.data, key, volume)
                {
                    self.shared.emit("lolping/audioError", json!({"error":e}));
                }
            }
            "selected" => {
                for (other, w) in &mut self.windows {
                    if *other != id {
                        w.send("clear", json!(null), &self.decoders);
                    }
                }
            }
            _ => {}
        }
    }
    pub fn helper_event(&mut self, generation: u64, v: Value) {
        if self
            .helper
            .as_ref()
            .is_none_or(|h| h.generation != generation)
        {
            return;
        }
        match v["type"].as_str().unwrap_or("") {
            "ready" => {
                if let Some(h) = &mut self.helper {
                    h.ready = true;
                }
                self.sync_helper();
            }
            "error" => {
                let no_access = v["code"] == "noAccess";
                let error = v["message"].as_str().unwrap_or("helperFailed").to_string();
                self.stop();
                self.shared.status(
                    if no_access { "noAccess" } else { "failed" },
                    false,
                    Some(error),
                );
            }
            "toggled" => {
                self.enabled = v["enabled"] == true;
                self.pause_reason = (!self.enabled).then_some("shortcut");
                // The helper already changed its state; avoid a redundant config/cancel round-trip.
                if let Some(config) = self.helper.as_mut().and_then(|h| h.config.as_mut()) {
                    config["enabled"] = json!(self.enabled);
                }
                if !self.enabled {
                    self.all("clear", Value::Null);
                    self.audio.clear();
                    self.wheel = None;
                }
                self.sync_helper();
            }
            "cancel" => {
                self.all("cancel", Value::Null);
                self.wheel = None;
            }
            kind @ ("wheelOpen" | "wheelMove" | "wheelRelease" | "click") => {
                let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64()) else {
                    return;
                };
                let id = if kind == "wheelMove" || kind == "wheelRelease" {
                    self.wheel
                } else {
                    self.windows
                        .iter()
                        .find(|(_, w)| w.display.contains(x, y))
                        .map(|(id, _)| *id)
                };
                if kind == "wheelOpen" {
                    self.wheel = id;
                }
                if let Some(id) = id {
                    if let Some(w) = self.windows.get_mut(&id) {
                        let (x, y) = w.display.local(x, y, cfg!(target_os = "macos"));
                        if kind == "wheelOpen" || kind == "click" {
                            w.window.set_always_on_top(true);
                        }
                        let mut p = v.clone();
                        p["x"] = json!(x);
                        p["y"] = json!(y);
                        self.dispatch(id, kind, p);
                    }
                }
                if kind == "wheelRelease" {
                    self.wheel = None;
                }
            }
            _ => {}
        }
    }
    pub fn helper_gone(&mut self, generation: u64) {
        if self
            .helper
            .as_ref()
            .is_some_and(|h| h.generation == generation)
        {
            self.fail("helperExited".into());
        }
    }
    pub fn next_wake(&self) -> Instant {
        let now = Instant::now();
        let mut wake = now + Duration::from_millis(500);
        for w in self.windows.values() {
            if w.scene.dirty {
                return now;
            }
            if w.scene.animating(now) {
                wake = wake.min(w.next_frame);
            }
        }
        wake
    }
    pub fn tick(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        for w in self.windows.values_mut() {
            if let Err(e) = w.tick(Instant::now()) {
                self.fail(e);
                return;
            }
        }
        if self.last_poll.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_poll = Instant::now();
        if self.suspend_until.is_some_and(|t| Instant::now() >= t) {
            self.suspend_until = None;
            self.sync_helper();
        }
        if self.preview_until.is_some_and(|t| Instant::now() >= t) {
            self.preview_until = None;
            if self.helper.is_none() {
                self.windows.clear();
                self.bank = None;
                self.audio.clear();
                self.last_displays.clear();
            }
        }
        if (self.helper.is_some() || self.preview_until.is_some())
            && Self::displays(target) != self.last_displays
        {
            if let Err(e) = self.rebuild(target) {
                self.fail(e);
            }
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        self.stop();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coordinates_handle_negative_monitors_and_scaling() {
        let d = Display {
            x: -2560,
            y: 0,
            w: 2560,
            h: 1440,
            scale: 2.,
        };
        assert_eq!(d.local(-2460., 200., false), (50., 100.));
        assert_eq!(d.local(-1230., 100., true), (50., 100.));
    }
}
