use crate::{assets, native::UserEvent, preferences, settings};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tao::event_loop::EventLoopProxy;

pub struct Store {
    pub settings: Value,
    pub available: HashSet<String>,
}
pub struct Shared {
    pub root: PathBuf,
    pub data: PathBuf,
    pub store: Mutex<Store>,
    pub status: Mutex<Value>,
    pub preferences: Mutex<Value>,
    sinks: Mutex<Vec<(u64, mpsc::SyncSender<Value>)>>,
    next_sink: std::sync::atomic::AtomicU64,
}
impl Shared {
    pub fn new(root: PathBuf, data: PathBuf) -> Result<Arc<Self>, String> {
        fs::create_dir_all(&data).map_err(|e| e.to_string())?;
        let raw = fs::read(data.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(settings::defaults);
        let available = assets::library_metadata(&root, &data, &raw["customEmotes"])
            .iter()
            .filter_map(|v| v["id"].as_str().map(str::to_owned))
            .collect();
        let mut s = settings::normalize(&raw, &available, cfg!(target_os = "macos"));
        s["customEmotes"] = json!(assets::library_metadata(&root, &data, &raw["customEmotes"])
            .into_iter()
            .filter(|v| v["custom"] == true)
            .map(|mut v| {
                v.as_object_mut().unwrap().remove("thumbnail");
                v.as_object_mut().unwrap().remove("custom");
                v
            })
            .collect::<Vec<_>>());
        let prefs = preferences::load(&data);
        Ok(Arc::new(Self {
            root,
            data,
            store: Mutex::new(Store {
                settings: s,
                available,
            }),
            status: Mutex::new(
                json!({"state":"stopped","enabled":false,"error":null,"pauseReason":null}),
            ),
            preferences: Mutex::new(prefs),
            sinks: Mutex::new(Vec::new()),
            next_sink: std::sync::atomic::AtomicU64::new(1),
        }))
    }
    pub fn subscribe(&self, sink: mpsc::SyncSender<Value>) -> u64 {
        let id = self
            .next_sink
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.sinks.lock().unwrap().push((id, sink));
        id
    }
    pub fn unsubscribe(&self, id: u64) {
        self.sinks.lock().unwrap().retain(|(i, _)| *i != id);
    }
    pub fn status(&self, state: &str, enabled: bool, error: Option<String>) {
        self.status_with_pause(state, enabled, error, None);
    }
    pub fn status_with_pause(
        &self,
        state: &str,
        enabled: bool,
        error: Option<String>,
        reason: Option<&str>,
    ) {
        let v = json!({"state":state,"enabled":enabled,"error":error,"pauseReason":reason});
        *self.status.lock().unwrap() = v.clone();
        self.emit("lolping/status", v);
    }
    pub fn emit(&self, method: &str, value: Value) {
        let event = json!({"event":method,"params":value});
        self.sinks.lock().unwrap().retain(|(_, s)| {
            !matches!(
                s.try_send(event.clone()),
                Err(mpsc::TrySendError::Disconnected(_))
            )
        });
    }
    pub fn snapshot(&self) -> Value {
        let store = self.store.lock().unwrap();
        json!({"status":self.status.lock().unwrap().clone(),"settings":store.settings,"emotes":assets::library(&self.root,&self.data,&store.settings["customEmotes"]),"platform":if cfg!(target_os="macos"){"mac"}else if cfg!(target_os="linux"){"linux"}else{"win"},"linuxSession":crate::platform::session_info(),"locale":sys_locale::get_locale().unwrap_or_else(||"en".into()),"version":env!("CARGO_PKG_VERSION"),"preferences":self.preferences.lock().unwrap().clone(),"extraPackInstalled":assets::pack_dir(&self.data).is_dir()})
    }
}
#[derive(Clone)]
pub struct Handler {
    pub shared: Arc<Shared>,
    pub proxy: EventLoopProxy<UserEvent>,
}
impl Handler {
    pub fn control(&self, method: &str, p: Value) -> Result<Value, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.proxy
            .send_event(UserEvent::Control {
                method: method.into(),
                params: p,
                reply: tx,
            })
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(25))
            .map_err(|e| e.to_string())?
    }
    fn refresh(&self, s: Value, library_changed: bool) {
        let _ = self
            .proxy
            .send_event(UserEvent::Settings(s.clone(), library_changed));
        self.shared.emit("lolping/settings", json!({"settings":s}));
        if library_changed {
            self.shared.emit("lolping/library", json!({"changed":true}));
        }
    }
    pub fn handle(&self, method: &str, p: Value) -> Result<Value, String> {
        let method = method.strip_prefix("lolping/").unwrap_or(method);
        match method {
            "state" => Ok(self.shared.snapshot()),
            #[cfg(target_os = "linux")]
            "wayland/install" => crate::wayland::install_extension(&self.shared.root),
            "ui/ready" => {
                self.shared.emit("ui/ready", json!({"ok":true}));
                Ok(json!({"ok":true}))
            }
            "ui/open" | "ui/close" | "diagnostics" => self.control(method, p),
            "app/quit" => {
                let _ = self.proxy.send_event(UserEvent::Quit);
                Ok(json!({"ok":true}))
            }
            "start" | "stop" | "preview" | "suspend" | "pause" | "permission" => {
                self.control(&format!("lolping/{method}"), p)
            }
            "releases" => {
                crate::links::open_releases()?;
                Ok(json!({"opened":true}))
            }
            "preferences" => {
                let mut prefs = self.shared.preferences.lock().unwrap();
                let next = preferences::update(&self.shared.data, &prefs, &p)?;
                *prefs = next.clone();
                drop(prefs);
                self.shared.emit("preferences", next.clone());
                Ok(next)
            }
            "settings" => {
                let patch = p.as_object().ok_or("invalidSettings")?;
                let mut store = self.shared.store.lock().unwrap();
                let mut raw = store.settings.clone();
                for (k, v) in patch {
                    if settings::defaults().get(k).is_some()
                        && k != "customEmotes"
                        && k != "version"
                    {
                        raw[k] = v.clone();
                    }
                }
                let next = settings::normalize(&raw, &store.available, cfg!(target_os = "macos"));
                if next == store.settings {
                    return Ok(next);
                }
                settings::atomic_json(&self.shared.data.join("settings.json"), &next)?;
                store.settings = next.clone();
                drop(store);
                self.refresh(next.clone(), false);
                Ok(next)
            }
            "import" => self.import(p),
            "emote/remove" => {
                let id = p["id"].as_str().ok_or("invalidEmote")?;
                let hash = assets::custom_id(id).ok_or("invalidEmote")?;
                let mut store = self.shared.store.lock().unwrap();
                let mut next = store.settings.clone();
                next["customEmotes"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|v| v["id"] != id);
                let available = assets::library_metadata(
                    &self.shared.root,
                    &self.shared.data,
                    &next["customEmotes"],
                )
                .iter()
                .filter_map(|v| v["id"].as_str().map(str::to_owned))
                .collect();
                next = settings::normalize(&next, &available, cfg!(target_os = "macos"));
                settings::atomic_json(&self.shared.data.join("settings.json"), &next)?;
                store.settings = next.clone();
                store.available = available;
                for ext in ["png", "anim", "sound"] {
                    let _ = fs::remove_file(
                        self.shared
                            .data
                            .join("custom")
                            .join(format!("{hash}.{ext}")),
                    );
                }
                drop(store);
                self.refresh(next, true);
                Ok(json!({"ok":true}))
            }
            _ => Err(format!("unknownMethod: {method}")),
        }
    }
    fn import(&self, p: Value) -> Result<Value, String> {
        let kind = p["kind"].as_str().ok_or("invalidImport")?;
        let limit = match kind {
            "pack" => 64 * 1024 * 1024,
            "image" => 8 * 1024 * 1024,
            "sound" => 1024 * 1024,
            _ => return Err("invalidImport".into()),
        };
        let path = PathBuf::from(p["path"].as_str().ok_or("invalidImportPath")?);
        if !path.is_absolute() {
            return Err("invalidImportPath".into());
        }
        let file = fs::File::open(&path).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("invalidImportPath".into());
        }
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("fileTooLarge".into());
        }
        let mut store = self.shared.store.lock().unwrap();
        let result = match kind {
            "pack" => {
                assets::import_pack(&bytes, &self.shared.data)?;
                json!({"ok":true})
            }
            "image" => {
                json!({"id":assets::import_image(&bytes,&path.file_name().unwrap_or_default().to_string_lossy(),&self.shared.data,&mut store.settings)?})
            }
            "sound" => {
                let id = p["id"].as_str().ok_or("invalidEmote")?;
                let hash = assets::custom_id(id).ok_or("invalidEmote")?;
                let mut next = store.settings.clone();
                let item = next["customEmotes"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|v| v["id"] == id)
                    .ok_or("missingEmote")?;
                crate::audio::validate_sound(&bytes)?;
                fs::write(
                    self.shared
                        .data
                        .join("custom")
                        .join(format!("{hash}.sound")),
                    bytes,
                )
                .map_err(|e| e.to_string())?;
                item["sound"] = json!(true);
                settings::atomic_json(&self.shared.data.join("settings.json"), &next)?;
                store.settings = next;
                json!({"ok":true})
            }
            _ => unreachable!(),
        };
        let next = store.settings.clone();
        store.available =
            assets::library_metadata(&self.shared.root, &self.shared.data, &next["customEmotes"])
                .iter()
                .filter_map(|v| v["id"].as_str().map(str::to_owned))
                .collect();
        drop(store);
        self.refresh(next, true);
        Ok(result)
    }
}
