//! GNOME Wayland transport. Rust owns rendering/audio; Shell owns only the
//! compositor actors and short-lived modal grab. No XTest, root or screencast.
use crate::{
    audio::AudioWorker,
    native::UserEvent,
    render::{
        media::{Bank, DecoderPool},
        Output, Scene,
    },
    service::Shared,
    settings,
    wayland_model::{self, Monitor, EXTENSION_ID},
};
use gtk::{
    gio,
    glib::{self, variant::ToVariant},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tao::event_loop::EventLoopProxy;
use tiny_skia::Pixmap;

const NAME: &str = "io.github.EzrealByte.LOLPingLite.Wayland";
const PATH: &str = "/io/github/EzrealByte/LOLPingLite/Wayland";
const IFACE: &str = "io.github.EzrealByte.LOLPingLite.Wayland1";
const MISSING: &str = "Enable the LOLPing Lite GNOME 46 extension, then retry. First installation may require logging out and in. / 请安装并启用 LOLPing Lite GNOME 46 扩展后重试；首次安装可能需要注销并重新登录。";

fn uid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}
fn call(
    connection: &gio::DBusConnection,
    owner: &str,
    method: &str,
    args: &glib::Variant,
) -> Result<glib::Variant, String> {
    connection
        .call_sync(
            Some(owner),
            PATH,
            IFACE,
            method,
            Some(args),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            250,
            gio::Cancellable::NONE,
        )
        .map_err(|e| format!("GNOME Wayland {method}: {e}"))
}
fn owner(connection: &gio::DBusConnection, name: &str) -> Result<String, String> {
    connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            Some(&(name,).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            250,
            gio::Cancellable::NONE,
        )
        .map_err(|_| MISSING.to_string())?
        .get::<(String,)>()
        .map(|v| v.0)
        .ok_or("Invalid GNOME bus owner".into())
}

struct FrameDirectory {
    path: PathBuf,
    files: HashMap<u32, File>,
}
impl FrameDirectory {
    fn new() -> Result<Self, String> {
        let runtime = glib::user_runtime_dir();
        let meta = runtime.symlink_metadata().map_err(|e| e.to_string())?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || meta.uid() != uid()
            || meta.mode() & 0o077 != 0
        {
            return Err(
                "Wayland requires a private XDG_RUNTIME_DIR owned by the current user".into(),
            );
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = runtime.join(format!(
            "lolping-lite-frames-{}-{nonce:x}",
            std::process::id()
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            path,
            files: HashMap::new(),
        })
    }
    fn write(&mut self, monitor: u32, pixels: &Pixmap) -> Result<String, String> {
        let len = wayland_model::frame_bytes(pixels.width(), pixels.height())?;
        let name = format!("frame-{monitor}.rgba");
        if let std::collections::hash_map::Entry::Vacant(entry) = self.files.entry(monitor) {
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(self.path.join(&name))
                .map_err(|e| e.to_string())?;
            entry.insert(file);
        }
        let file = self.files.get_mut(&monitor).unwrap();
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.set_len(len as u64))
            .and_then(|_| file.write_all(pixels.data()))
            .map_err(|e| e.to_string())?;
        Ok(name)
    }
    fn release(&mut self, monitor: u32) {
        if self.files.remove(&monitor).is_some() {
            let _ = fs::remove_file(self.path.join(format!("frame-{monitor}.rgba")));
        }
    }
}
impl Drop for FrameDirectory {
    fn drop(&mut self) {
        let ids: Vec<_> = self.files.keys().copied().collect();
        for id in ids {
            self.release(id);
        }
        // Only remove our now-empty private directory, never a recursive path.
        let _ = fs::remove_dir(&self.path);
    }
}
struct Bridge {
    connection: gio::DBusConnection,
    owner: String,
    subscriptions: Vec<gio::SignalSubscriptionId>,
    frames: FrameDirectory,
    connected: bool,
}
impl Bridge {
    fn connect(
        proxy: EventLoopProxy<UserEvent>,
        generation: u64,
    ) -> Result<(Self, Vec<Monitor>, bool), String> {
        let desktop = crate::platform::session_info();
        if desktop["desktop"]
            .as_str()
            .is_some_and(|d| !d.is_empty() && d != "gnome")
        {
            return Err("Wayland support currently requires Ubuntu 24.04 / GNOME 46. KDE and other compositors are not supported yet. / 当前 Wayland 路径支持 Ubuntu 24.04 / GNOME 46，暂不支持 KDE 等桌面。".into());
        }
        let address =
            gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)
                .map_err(|e| format!("{MISSING} ({e})"))?;
        // A dedicated unique sender lets Drop force Shell cleanup even if a
        // method times out; the tray's shared session connection cannot do so.
        let connection = gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .map_err(|e| format!("{MISSING} ({e})"))?;
        let shell = owner(&connection, "org.gnome.Shell")?;
        let extension = owner(&connection, NAME)?;
        if shell != extension {
            return Err("The Wayland bridge is not owned by GNOME Shell".into());
        }
        let mut bridge = Self {
            connection,
            owner: extension,
            subscriptions: vec![],
            frames: FrameDirectory::new()?,
            connected: false,
        };
        let events = proxy.clone();
        bridge
            .subscriptions
            .push(bridge.connection.signal_subscribe(
                Some(&bridge.owner),
                Some(IFACE),
                Some("Event"),
                Some(PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, args| {
                    if let Some((message,)) = args.get::<(String,)>() {
                        if message.len() <= 65536 {
                            if let Ok(event) = serde_json::from_str(&message) {
                                let _ = events.send_event(UserEvent::Wayland(generation, event));
                            }
                        }
                    }
                },
            ));
        bridge
            .subscriptions
            .push(bridge.connection.signal_subscribe(
                Some("org.freedesktop.DBus"),
                Some("org.freedesktop.DBus"),
                Some("NameOwnerChanged"),
                Some("/org/freedesktop/DBus"),
                Some(NAME),
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, _| {
                    let _ =
                        proxy.send_event(UserEvent::Wayland(generation, json!({"type":"gone"})));
                },
            ));
        let response = bridge.call(
            "Connect",
            &(bridge.frames.path.to_string_lossy().as_ref(),).to_variant(),
        )?;
        bridge.connected = true;
        let (message,) = response
            .get::<(String,)>()
            .ok_or("Invalid GNOME handshake")?;
        if message.len() > 65536 {
            return Err("GNOME handshake too large".into());
        }
        let hello: Value = serde_json::from_str(&message).map_err(|e| e.to_string())?;
        if hello["protocol"] != 1 {
            return Err("Update the LOLPing Lite GNOME extension: incompatible protocol".into());
        }
        let monitors = wayland_model::monitors(&hello["monitors"])?;
        Ok((bridge, monitors, hello["available"] != false))
    }
    fn call(&self, method: &str, args: &glib::Variant) -> Result<glib::Variant, String> {
        call(&self.connection, &self.owner, method, args)
    }
    fn config(&self, enabled: bool, suspended: bool, emotes: bool) -> Result<(), String> {
        self.call(
            "Configure",
            &(json!({"enabled":enabled,"suspended":suspended,"emotesEnabled":emotes}).to_string(),)
                .to_variant(),
        )
        .map(|_| ())
    }
    fn clear(&mut self, monitor: u32) -> Result<(), String> {
        self.call("Clear", &(monitor as i32,).to_variant())?;
        self.frames.release(monitor);
        Ok(())
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        if self.connected {
            let _ = self.call("Disconnect", &().to_variant());
        }
        for id in self.subscriptions.drain(..) {
            self.connection.signal_unsubscribe(id);
        }
        let _ = self.connection.close_sync(gio::Cancellable::NONE);
    }
}
struct Overlay {
    monitor: Monitor,
    scene: Scene,
    pixels: Option<Pixmap>,
    visible: bool,
    next_frame: Instant,
}
pub struct Engine {
    shared: Arc<Shared>,
    proxy: EventLoopProxy<UserEvent>,
    settings: Value,
    bridge: Option<Bridge>,
    overlays: HashMap<u32, Overlay>,
    bank: Option<Arc<Bank>>,
    decoders: DecoderPool,
    audio: AudioWorker,
    generation: u64,
    running: bool,
    enabled: bool,
    wheel: Option<u32>,
    suspend_until: Option<Instant>,
    preview_until: Option<Instant>,
    pause_reason: Option<&'static str>,
    desktop_available: bool,
    presented_frames: u64,
    monitor_wait: Option<Instant>,
}
impl Engine {
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
            bridge: None,
            overlays: HashMap::new(),
            bank: None,
            decoders: DecoderPool::new(),
            audio,
            generation: 0,
            running: false,
            enabled: false,
            wheel: None,
            suspend_until: None,
            preview_until: None,
            pause_reason: None,
            desktop_available: true,
            presented_frames: 0,
            monitor_wait: None,
        }
    }
    fn ensure(&mut self) -> Result<(), String> {
        if self.bridge.is_some() {
            return Ok(());
        }
        self.generation += 1;
        let (bridge, monitors, available) = Bridge::connect(self.proxy.clone(), self.generation)?;
        self.desktop_available = available;
        self.bridge = Some(bridge);
        self.rebuild(monitors);
        Ok(())
    }
    fn rebuild(&mut self, monitors: Vec<Monitor>) {
        self.monitor_wait = None;
        self.wheel = None;
        if let Some(bridge) = self.bridge.as_mut() {
            for id in self.overlays.keys() {
                bridge.frames.release(*id);
            }
        }
        self.overlays.clear();
        if self.bank.is_none() {
            self.bank = Some(Arc::new(Bank::load(
                &self.shared.root,
                &self.shared.data,
                &self.settings,
            )));
        }
        for monitor in monitors {
            let scene = Scene::new(
                self.shared.root.clone(),
                self.shared.data.clone(),
                self.settings.clone(),
                self.bank.as_ref().unwrap().clone(),
            );
            self.overlays.insert(
                monitor.id,
                Overlay {
                    monitor,
                    scene,
                    pixels: None,
                    visible: false,
                    next_frame: Instant::now(),
                },
            );
        }
    }
    fn sync(&self) -> Result<(), String> {
        if let Some(bridge) = &self.bridge {
            bridge.config(
                self.running && self.enabled,
                self.suspend_until.is_some(),
                self.settings["emoteTrigger"] != "off",
            )?;
        }
        if self.running {
            self.shared.status_with_pause(
                "running",
                self.enabled && self.suspend_until.is_none() && self.desktop_available,
                None,
                if !self.desktop_available {
                    Some("desktop")
                } else if self.suspend_until.is_some() {
                    Some("capture")
                } else {
                    self.pause_reason
                },
            );
        }
        Ok(())
    }
    fn all(&mut self, event: &str) {
        for overlay in self.overlays.values_mut() {
            overlay
                .scene
                .event(event, &Value::Null, &self.decoders, Instant::now());
        }
        self.wheel = None;
    }
    fn stop(&mut self) {
        self.bridge = None;
        self.overlays.clear();
        self.bank = None;
        self.audio.clear();
        self.wheel = None;
        self.running = false;
        self.enabled = false;
        self.suspend_until = None;
        self.preview_until = None;
        self.pause_reason = None;
        self.monitor_wait = None;
        self.shared.status("stopped", false, None);
    }
    fn fail(&mut self, error: String) {
        self.stop();
        self.shared.status("failed", false, Some(error));
    }
    pub fn control(&mut self, method: &str, p: Value) -> Result<Value, String> {
        let result = self.control_inner(method, p);
        if let Err(error) = &result {
            if matches!(
                method,
                "lolping/start" | "lolping/pause" | "lolping/suspend"
            ) {
                self.fail(error.clone());
            }
        }
        result
    }
    fn control_inner(&mut self, method: &str, p: Value) -> Result<Value, String> {
        match method {
            "lolping/start" => {
                self.ensure()?;
                if !self.desktop_available {
                    return Err("desktopUnavailable: close the GNOME overview or unlock the desktop before previewing".into());
                }
                self.running = true;
                self.enabled = true;
                self.preview_until = None;
                self.pause_reason = None;
                self.sync()?;
                Ok(self.shared.status.lock().unwrap().clone())
            }
            "lolping/stop" => {
                self.stop();
                Ok(json!({"state":"stopped","enabled":false}))
            }
            "lolping/pause" => {
                if !self.running {
                    return Err("notRunning".into());
                }
                self.enabled = p["on"] != true;
                self.pause_reason = (!self.enabled).then_some("manual");
                if !self.enabled {
                    self.all("clear");
                    self.audio.clear();
                }
                self.sync()?;
                Ok(self.shared.status.lock().unwrap().clone())
            }
            "lolping/suspend" => {
                self.suspend_until =
                    (p["on"] == true).then(|| Instant::now() + Duration::from_secs(20));
                if self.suspend_until.is_some() {
                    self.all("clear");
                }
                self.sync()?;
                Ok(json!({"ok":true}))
            }
            "lolping/preview" => {
                let id = p["id"].as_str().ok_or("invalidPreview")?;
                let emote = p["wheel"] == "emote";
                if emote {
                    if !self.shared.store.lock().unwrap().available.contains(id) {
                        return Err("missingEmote".into());
                    }
                } else if !settings::PINGS.contains(&id) {
                    return Err("invalidPing".into());
                }
                self.ensure()?;
                if !self.running {
                    self.preview_until = Some(Instant::now() + Duration::from_secs(10));
                }
                let overlay = self
                    .overlays
                    .values()
                    .min_by_key(|v| v.monitor.id)
                    .ok_or("noDisplay")?;
                let monitor = &overlay.monitor;
                let event = json!({"id":id,"wheel":if emote{"emote"}else{"ping"},"x":monitor.width as f64/2.,"y":monitor.height as f64/2.});
                self.dispatch(monitor.id, "preview", event);
                Ok(json!({"ok":true}))
            }
            "lolping/permission" => Err(MISSING.into()),
            "lolping/diagnostics" => Ok(
                json!({"renderer":"native-gnome-wayland","backend":"gnome-wayland","wayland":true,"wheelOpen":self.wheel.is_some(),"presentedFrames":self.presented_frames,"desktopAvailable":self.desktop_available,"transport":"private-rgba-files+session-dbus","windows":self.overlays.len(),"readyWindows":self.overlays.len(),"visibleWindows":self.overlays.values().filter(|o|o.visible).count(),"surfaceBytes":self.overlays.values().map(|o|o.pixels.as_ref().map_or(0,|p|p.data().len())).sum::<usize>(),"imageBytes":self.bank.as_ref().map_or(0,|b|b.bytes()),"helperPid":null,"helperConfigWrites":0,"enabled":self.enabled,"connected":self.bridge.is_some(),"shortcuts":{"ping":"Super+Alt+Q","emote":"Super+Alt+E","pause":"Super+Alt+P"}}),
            ),
            _ => Err("unknownControl".into()),
        }
    }
    pub fn refresh(&mut self, settings: Value, library: bool) {
        let art_changed = library
            || self.settings["emoteWheel"] != settings["emoteWheel"]
            || self.settings["clickEmoteId"] != settings["clickEmoteId"];
        self.settings = settings;
        if art_changed && self.bank.is_some() {
            self.bank = Some(Arc::new(Bank::load(
                &self.shared.root,
                &self.shared.data,
                &self.settings,
            )));
        }
        if library {
            self.audio.invalidate_custom();
            self.all("library");
        }
        if let Some(bank) = &self.bank {
            for overlay in self.overlays.values_mut() {
                overlay.scene.settings(self.settings.clone(), bank.clone());
            }
        }
        if let Err(error) = self.sync() {
            self.fail(error);
        }
    }
    fn dispatch(&mut self, id: u32, name: &str, event: Value) {
        let Some(overlay) = self.overlays.get_mut(&id) else {
            return;
        };
        let outputs = overlay
            .scene
            .event(name, &event, &self.decoders, Instant::now());
        for output in outputs {
            match output {
                Output::Selected => {
                    for (other, overlay) in &mut self.overlays {
                        if *other != id {
                            overlay.scene.event(
                                "clear",
                                &Value::Null,
                                &self.decoders,
                                Instant::now(),
                            );
                        }
                    }
                }
                Output::Sound(key) => {
                    let mut volume = if self.settings["muted"] == true {
                        0.
                    } else {
                        self.settings["volume"].as_f64().unwrap_or(70.) as f32 / 100.
                    };
                    if key == "button" {
                        volume = if self.settings["tickSound"] == true {
                            volume * 0.5
                        } else {
                            0.
                        };
                    } else if key.starts_with("emote:") && self.settings["emoteSound"] != true {
                        volume = 0.;
                    }
                    if let Err(error) =
                        self.audio
                            .play(&self.shared.root, &self.shared.data, &key, volume)
                    {
                        self.shared
                            .emit("lolping/audioError", json!({"error":error}));
                    }
                }
            }
        }
    }
    pub fn event(&mut self, generation: u64, event: Value) {
        if generation != self.generation || self.bridge.is_none() {
            return;
        }
        match event["type"].as_str().unwrap_or("") {
            "gone" => self.fail(MISSING.into()),
            "error" => self.fail(
                event
                    .get("message")
                    .or_else(|| event.get("error"))
                    .unwrap_or(&Value::Null)
                    .as_str()
                    .unwrap_or("GNOME Wayland bridge failed")
                    .to_owned(),
            ),
            "cancel" => self.all("cancel"),
            "session" => {
                self.desktop_available = event["available"] == true;
                if !self.desktop_available {
                    self.all("clear");
                    self.audio.clear();
                }
                if let Err(e) = self.sync() {
                    self.fail(e);
                }
            }
            "monitors" => match wayland_model::monitors(&event["monitors"]) {
                Ok(m) => self.rebuild(m),
                Err(e) => self.fail(e),
            },
            "toggled" if self.running && self.suspend_until.is_none() => {
                self.enabled = event["enabled"] == true;
                self.pause_reason = (!self.enabled).then_some("shortcut");
                if !self.enabled {
                    self.all("clear");
                    self.audio.clear();
                }
                if let Err(e) = self.sync() {
                    self.fail(e);
                }
            }
            kind @ ("wheelOpen" | "wheelMove" | "wheelRelease" | "click")
                if self.running && self.enabled && self.suspend_until.is_none() =>
            {
                let Some((x, y)) = wayland_model::pointer(&event) else {
                    return;
                };
                let id = if matches!(kind, "wheelMove" | "wheelRelease") {
                    self.wheel
                } else {
                    self.overlays
                        .values()
                        .find(|o| o.monitor.contains(x, y))
                        .map(|o| o.monitor.id)
                };
                if !self.desktop_available {
                    return;
                }
                if kind == "wheelOpen" {
                    self.wheel = id;
                }
                if let Some(id) = id {
                    if let Some(overlay) = self.overlays.get(&id) {
                        let mut local = event.clone();
                        local["x"] = json!(x - overlay.monitor.x as f64);
                        local["y"] = json!(y - overlay.monitor.y as f64);
                        self.dispatch(id, kind, local);
                    }
                }
                if kind == "wheelRelease" {
                    self.wheel = None;
                }
            }
            _ => {}
        }
    }
    pub fn next_wake(&self) -> Instant {
        let now = Instant::now();
        let mut wake = now + Duration::from_millis(500);
        for overlay in self.overlays.values() {
            if overlay.scene.dirty {
                return now;
            }
            if overlay.scene.animating(now) {
                wake = wake.min(overlay.next_frame);
            }
        }
        wake
    }
    fn paint(&mut self) -> Result<(), String> {
        let Some(bridge) = self.bridge.as_mut() else {
            return Ok(());
        };
        let now = Instant::now();
        for overlay in self.overlays.values_mut() {
            if !overlay.scene.dirty && (!overlay.scene.animating(now) || now < overlay.next_frame) {
                continue;
            }
            overlay.scene.advance(now);
            let monitor = &overlay.monitor;
            let (w, h) = monitor.pixels();
            let Some(bounds) = overlay.scene.bounds(monitor.scale, w, h) else {
                if overlay.visible {
                    bridge.clear(monitor.id)?;
                    overlay.visible = false;
                }
                overlay.pixels = None;
                overlay.scene.release_buffers();
                overlay.scene.dirty = false;
                continue;
            };
            wayland_model::frame_bytes(bounds.w, bounds.h)?;
            if overlay
                .pixels
                .as_ref()
                .is_none_or(|p| p.width() != bounds.w || p.height() != bounds.h)
            {
                overlay.pixels =
                    Some(Pixmap::new(bounds.w, bounds.h).ok_or("Wayland frame allocation failed")?);
            }
            let pixels = overlay.pixels.as_mut().unwrap();
            overlay.scene.draw(pixels, bounds, monitor.scale, now);
            let file = bridge.frames.write(monitor.id, pixels)?;
            let message = json!({"monitor":monitor.id,"x":monitor.x as f64+bounds.x as f64/monitor.scale,"y":monitor.y as f64+bounds.y as f64/monitor.scale,"width":bounds.w,"height":bounds.h,"scale":monitor.scale,"file":file});
            // Shell only uploads the mapped bytes and replies; it never waits for
            // Rust. One frame in flight and a bounded timeout prevent queues or
            // reuse of the mapping before the compositor has copied its pixels.
            bridge.call("Present", &(message.to_string(),).to_variant())?;
            self.presented_frames += 1;
            overlay.visible = true;
            overlay.next_frame = now + Duration::from_micros(16_667);
        }
        Ok(())
    }
    pub fn tick(&mut self) {
        if let Err(error) = self.paint() {
            if error.contains("desktopUnavailable") {
                self.desktop_available = false;
                self.all("clear");
                self.audio.clear();
                let _ = self.sync();
            } else if [
                "Unknown monitor",
                "Frame scale does not match",
                "Frame is outside its monitor",
            ]
            .iter()
            .any(|kind| error.contains(kind))
            {
                // A monitors-changed event can be queued while a synchronous
                // upload is in flight. Drop that old geometry and let the queued
                // event rebuild it, rather than stopping the entire app.
                self.all("clear");
                if let Some(bridge) = self.bridge.as_mut() {
                    for id in self.overlays.keys() {
                        bridge.frames.release(*id);
                    }
                }
                self.overlays.clear();
                self.monitor_wait = Some(Instant::now() + Duration::from_secs(2));
            } else {
                self.fail(error);
                return;
            }
        }
        if self
            .monitor_wait
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.fail("GNOME did not provide updated monitor geometry; retry startup".into());
            return;
        }
        if self.suspend_until.is_some_and(|t| Instant::now() >= t) {
            self.suspend_until = None;
            if let Err(e) = self.sync() {
                self.fail(e);
            }
        }
        if self.preview_until.is_some_and(|t| Instant::now() >= t) && !self.running {
            self.stop();
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

/// This action is invoked by the explicit settings button, never at startup.
pub fn install_extension(root: &Path) -> Result<Value, String> {
    if !crate::platform::is_wayland() {
        return Err("This setup is only needed in GNOME Wayland".into());
    }
    if crate::platform::session_info()["desktop"] != "gnome" {
        return Err("This extension requires Ubuntu 24.04 / GNOME 46".into());
    }
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|e| e.to_string())?;
    let shell_version = connection
        .call_sync(
            Some("org.gnome.Shell"),
            "/org/gnome/Shell",
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&("org.gnome.Shell", "ShellVersion").to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            1500,
            gio::Cancellable::NONE,
        )
        .map_err(|e| format!("Cannot check GNOME Shell version: {e}"))?
        .get::<(glib::Variant,)>()
        .and_then(|(v,)| v.get::<String>())
        .ok_or("Cannot read GNOME Shell version")?;
    if shell_version.split('.').next() != Some("46") {
        return Err(format!("This bridge currently supports GNOME 46 (Ubuntu 24.04), found {shell_version}. / 当前扩展支持 GNOME 46。"));
    }
    let source = [
        root.join("gnome-extension").join(EXTENSION_ID),
        root.join("native/gnome-extension").join(EXTENSION_ID),
    ]
    .into_iter()
    .find(|p| p.join("metadata.json").is_file())
    .ok_or("The GNOME extension is missing from this application package")?;
    let base = directories::BaseDirs::new().ok_or("Cannot locate user data directory")?;
    let destination = base
        .data_dir()
        .join("gnome-shell/extensions")
        .join(EXTENSION_ID);
    let replacing = destination.join("metadata.json").exists();
    // Do not follow an existing user-created symlink when replacing our files.
    if destination
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("The extension installation directory is a symbolic link".into());
    }
    fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
    for name in ["metadata.json", "extension.js", "protocol.js"] {
        let to = destination.join(name);
        if to
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(format!("Refusing to overwrite symbolic link: {name}"));
        }
        let bytes = fs::read(source.join(name)).map_err(|e| e.to_string())?;
        fs::write(to, bytes).map_err(|e| e.to_string())?;
    }
    let enabled = connection
        .call_sync(
            Some("org.gnome.Shell"),
            "/org/gnome/Shell",
            "org.gnome.Shell.Extensions",
            "EnableExtension",
            Some(&(EXTENSION_ID,).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            1500,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|v| v.get::<(bool,)>())
        .is_some_and(|v| v.0);
    // An already-loaded older module also requires a new login to use new code.
    Ok(
        json!({"ok":true,"reloginRequired":!enabled || replacing,"path":destination,"extensionId":EXTENSION_ID}),
    )
}
