use crate::{
    native::UserEvent,
    service::{Handler, Shared},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use tao::event_loop::EventLoopProxy;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

pub struct Tray {
    _tray: TrayIcon,
    open: MenuItem,
    toggle: MenuItem,
    quit: MenuItem,
    last: String,
}
impl Tray {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        if !crate::platform::tray_host_available() {
            return Err("No system-tray host is available; keep the settings window open. / 未检测到可用托盘，使用设置窗口控制应用。".into());
        }
        let menu = Menu::new();
        let open = MenuItem::with_id("settings", "Settings", true, None);
        let toggle = MenuItem::with_id("toggle", "Pause", true, None);
        let quit = MenuItem::with_id("quit", "Quit", true, None);
        menu.append_items(&[&open, &toggle, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| e.to_string())?;
        let image = image::load_from_memory(include_bytes!("../../assets/plugin.png"))
            .map_err(|e| e.to_string())?
            .resize(32, 32, image::imageops::FilterType::Lanczos3)
            .into_rgba8();
        let icon =
            tray_icon::Icon::from_rgba(image.into_raw(), 32, 32).map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("LOLPing Lite")
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;
        let menu_proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = menu_proxy.send_event(UserEvent::Menu(e.id.0));
        }));
        TrayIconEvent::set_event_handler(Some(move |e| {
            if matches!(
                e,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = proxy.send_event(UserEvent::OpenSettings);
            }
        }));
        Ok(Self {
            _tray: tray,
            open,
            toggle,
            quit,
            last: String::new(),
        })
    }
    pub fn update(&mut self, shared: &Shared) {
        let status = shared.status.lock().unwrap().clone();
        let lang = shared.store.lock().unwrap().settings["language"]
            .as_str()
            .unwrap_or("auto")
            .to_owned();
        let zh = lang == "zh-CN"
            || lang == "auto" && sys_locale::get_locale().is_some_and(|s| s.starts_with("zh"));
        let enabled = status["enabled"] == true;
        let stopped = matches!(
            status["state"].as_str(),
            Some("stopped" | "failed" | "noAccess")
        );
        let label = if stopped {
            if zh {
                "启动"
            } else {
                "Start"
            }
        } else if enabled {
            if zh {
                "暂停"
            } else {
                "Pause"
            }
        } else if zh {
            "恢复"
        } else {
            "Resume"
        };
        let state = if stopped {
            if zh {
                "已停止"
            } else {
                "Stopped"
            }
        } else if enabled {
            if zh {
                "运行中"
            } else {
                "Running"
            }
        } else if zh {
            "已暂停"
        } else {
            "Paused"
        };
        let signature = format!("{zh}:{label}");
        if self.last == signature {
            return;
        }
        self.last = signature;
        self.open.set_text(if zh { "设置" } else { "Settings" });
        self.toggle.set_text(label);
        self.quit.set_text(if zh { "退出" } else { "Quit" });
        let _ = self
            ._tray
            .set_tooltip(Some(format!("LOLPing Lite · {state}")));
    }
}
pub struct SettingsProcess {
    pub child: Child,
    pub generation: u64,
    pub tx: mpsc::SyncSender<Value>,
    shared: Arc<Shared>,
    subscription: u64,
}
impl SettingsProcess {
    pub fn open(handler: Handler, generation: u64) -> Result<Self, String> {
        let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
        command
            .arg("--settings")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let input = child.stdin.take().ok_or("missingSettingsInput")?;
        let output = child.stdout.take().ok_or("missingSettingsOutput")?;
        let (tx, rx) = mpsc::sync_channel(32);
        let subscription = handler.shared.subscribe(tx.clone());
        std::thread::spawn(move || write_messages(input, rx));
        let shared = handler.shared.clone();
        let reply = tx.clone();
        std::thread::spawn(move || {
            read_requests(output, &handler, &reply);
            handler.shared.unsubscribe(subscription);
            let _ = handler
                .proxy
                .send_event(UserEvent::SettingsGone(generation));
        });
        Ok(Self {
            child,
            generation,
            tx,
            shared,
            subscription,
        })
    }
    pub fn event(&self, event: &str) {
        let _ = self.tx.try_send(json!({"event":event,"params":{}}));
    }
    pub fn exited(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_some()
    }
}
impl Drop for SettingsProcess {
    fn drop(&mut self) {
        self.shared.unsubscribe(self.subscription);
        self.event("quit");
        let deadline = Instant::now() + Duration::from_millis(350);
        while Instant::now() < deadline {
            if self.exited() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn write_messages(mut writer: impl Write, rx: mpsc::Receiver<Value>) {
    for value in rx {
        if writeln!(writer, "{value}")
            .and_then(|_| writer.flush())
            .is_err()
        {
            break;
        }
    }
}
fn read_requests(reader: impl Read, handler: &Handler, tx: &mpsc::SyncSender<Value>) {
    let mut reader = BufReader::new(reader);
    loop {
        let mut line = String::new();
        match reader.by_ref().take(1024 * 1024 + 1).read_line(&mut line) {
            Ok(0) | Err(_) => break,
            _ => {}
        }
        if line.len() > 1024 * 1024 {
            break;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or("");
        let result = handler.handle(method, request["params"].clone());
        let response = match result {
            Ok(v) => json!({"id":id,"result":v}),
            Err(e) => json!({"id":id,"error":e}),
        };
        if tx.send(response).is_err() {
            break;
        }
    }
}
pub fn test_transport(handler: Handler) {
    let (tx, rx) = mpsc::sync_channel(64);
    let subscription = handler.shared.subscribe(tx.clone());
    std::thread::spawn(move || write_messages(std::io::stdout(), rx));
    std::thread::spawn(move || {
        read_requests(std::io::stdin(), &handler, &tx);
        handler.shared.unsubscribe(subscription);
        let _ = handler.proxy.send_event(UserEvent::Quit);
    });
}
