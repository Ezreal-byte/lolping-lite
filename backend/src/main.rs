#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod assets;
mod audio;
mod cache;
mod desktop;
mod desktop_backend;
mod instance;
mod links;
#[cfg(target_os = "linux")]
mod linux_wake;
mod native;
mod platform;
mod preferences;
mod render;
mod service;
mod settings;
mod ui;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(any(target_os = "linux", test))]
mod wayland_model;

use native::UserEvent;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tao::{
    event::Event,
    event_loop::{ControlFlow, EventLoopBuilder},
};

fn main() {
    if let Err(e) = run() {
        eprintln!("LOLPing Lite: {e}");
        std::process::exit(1);
    }
}
fn argument(args: &[String], key: &str) -> Option<String> {
    args.windows(2).find(|a| a[0] == key).map(|a| a[1].clone())
}
fn resource_root(args: &[String]) -> Result<PathBuf, String> {
    if let Some(p) = argument(args, "--root")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOLPING_ROOT").map(PathBuf::from))
    {
        return Ok(p);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("missingExecutableDirectory")?;
    for candidate in [
        dir.to_owned(),
        dir.join("../Resources"),
        dir.join("../share/lolping-lite"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."),
    ] {
        if candidate.join("ui/textures").is_dir() {
            return candidate.canonicalize().map_err(|e| e.to_string());
        }
    }
    Err("Missing application resources; reinstall LOLPing Lite".into())
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--settings") {
        return ui::run();
    }
    if args.iter().any(|a| a == "--version") {
        println!("LOLPing Lite {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let data = argument(&args, "--data-dir").map(PathBuf::from).unwrap_or(
        directories::ProjectDirs::from("io.github", "EzrealByte", "LOLPingLite")
            .ok_or("missingUserDirectory")?
            .data_local_dir()
            .to_owned(),
    );
    if args.iter().any(|a| a == "--quit") {
        match instance::notify(&data, "quit") {
            Ok(()) => instance::wait_for_exit(&data).map_err(|e| e.to_string()),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }?;
        return Ok(());
    }
    let test = args.iter().any(|a| a == "--headless-test");
    let self_test = args.iter().any(|a| a == "--self-test");
    if !test && !self_test && instance::notify(&data, "open").is_ok() {
        return Ok(());
    }
    let shared = service::Shared::new(resource_root(&args)?, data)?;
    if self_test {
        let state = shared.snapshot();
        if state["emotes"].as_array().map_or(0, Vec::len) < 9 {
            return Err("missingCoreResources".into());
        }
        println!(
            "{}",
            json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"coreEmotes":9,"renderer":"native"})
        );
        return Ok(());
    }
    let listener = if test {
        None
    } else {
        match instance::listen(&shared.data) {
            Ok(listener) => Some(listener),
            Err(error) => {
                // Two launches can both miss the initial notification before
                // either process binds its listener. Hand off to the winner.
                if instance::notify_when_ready(&shared.data, "open").is_ok() {
                    return Ok(());
                }
                return Err(format!("Cannot start single instance: {error}"));
            }
        }
    };
    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        event_loop.set_dock_visibility(false);
        event_loop.set_activate_ignoring_other_apps(false);
    }
    let proxy = event_loop.create_proxy();
    if let Some(listener) = listener {
        instance::serve(listener, proxy.clone());
    }
    let handler = service::Handler {
        shared: shared.clone(),
        proxy: proxy.clone(),
    };
    // Benchmarks exercise the normal tray/startup path while keeping the same
    // isolated inherited-pipe control used by the desktop regression suite.
    if test || args.iter().any(|a| a == "--benchmark") {
        desktop::test_transport(handler.clone());
    }
    let first = shared.preferences.lock().unwrap()["onboarded"] != true;
    let background = args.iter().any(|a| a == "--background");
    let mut engine = desktop_backend::Backend::new(shared.clone(), proxy.clone());
    let mut started = false;
    let mut tray: Option<desktop::Tray> = None;
    let mut gui: Option<desktop::SettingsProcess> = None;
    let mut generation = 0u64;
    let mut quitting: Option<Instant> = None;
    let mut fallback = false;
    #[cfg(target_os = "linux")]
    let mut linux_wake = linux_wake::DeadlineWake::default();
    event_loop.run(move |event, target, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500));
        let open_gui = |gui: &mut Option<desktop::SettingsProcess>,
                        generation: &mut u64|
         -> Result<(), String> {
            if let Some(ui) = gui.as_mut() {
                if !ui.exited() {
                    ui.event("focus");
                    return Ok(());
                }
            }
            *gui = None;
            *generation += 1;
            *gui = Some(desktop::SettingsProcess::open(
                handler.clone(),
                *generation,
            )?);
            Ok(())
        };
        match event {
            Event::UserEvent(UserEvent::Control {
                method,
                params,
                reply,
            }) => {
                let result = match method.as_str() {
                    "ui/open" => open_gui(&mut gui, &mut generation).map(|_| json!({"ok":true})),
                    "ui/close" => {
                        if let Some(ui) = gui.as_ref() {
                            ui.event("quit");
                        }
                        Ok(json!({"ok":true}))
                    }
                    "diagnostics" => {
                        engine
                            .control(target, "lolping/diagnostics", params)
                            .map(|mut v| {
                                v["settingsPid"] =
                                    json!(gui.as_mut().and_then(|u| if u.exited() {
                                        None
                                    } else {
                                        Some(u.child.id())
                                    }));
                                v
                            })
                    }
                    _ if quitting.is_some() => Err("shuttingDown".into()),
                    _ => engine.control(target, &method, params),
                };
                let _ = reply.send(result);
            }
            Event::UserEvent(UserEvent::Settings(s, library)) => engine.refresh(s, library),
            Event::UserEvent(UserEvent::Helper(id, v)) => engine.helper_event(id, v),
            Event::UserEvent(UserEvent::HelperGone(id)) => engine.helper_gone(id),
            #[cfg(target_os = "linux")]
            Event::UserEvent(UserEvent::Wayland(id, v)) => engine.wayland_event(id, v),
            Event::UserEvent(UserEvent::OpenSettings) => {
                if quitting.is_none() {
                    if let Err(e) = open_gui(&mut gui, &mut generation) {
                        eprintln!("Settings: {e}");
                    }
                }
            }
            Event::UserEvent(UserEvent::SettingsGone(id)) => {
                if gui.as_ref().is_some_and(|u| u.generation == id) {
                    gui = None;
                    let _ = engine.control(target, "lolping/suspend", json!({"on":false}));
                    if fallback && quitting.is_none() {
                        let _ = proxy.send_event(UserEvent::Quit);
                    }
                }
            }
            Event::UserEvent(UserEvent::Menu(id)) => match id.as_str() {
                "settings" => {
                    let _ = proxy.send_event(UserEvent::OpenSettings);
                }
                "quit" => {
                    let _ = proxy.send_event(UserEvent::Quit);
                }
                "toggle" => {
                    let status = shared.status.lock().unwrap().clone();
                    if matches!(
                        status["state"].as_str(),
                        Some("stopped" | "failed" | "noAccess")
                    ) {
                        let _ = engine.control(target, "lolping/start", Value::Null);
                    } else {
                        let _ = engine.control(
                            target,
                            "lolping/pause",
                            json!({"on":status["enabled"]==true}),
                        );
                    }
                }
                _ => {}
            },
            Event::UserEvent(UserEvent::Quit) => {
                if quitting.is_none() {
                    quitting = Some(Instant::now() + Duration::from_secs(3));
                    let _ = engine.control(target, "lolping/stop", Value::Null);
                    if let Some(ui) = gui.as_ref() {
                        ui.event("quit");
                    }
                }
            }
            Event::MainEventsCleared => {
                if !started {
                    started = true;
                    if !test {
                        match desktop::Tray::new(proxy.clone()) {
                            Ok(t) => tray = Some(t),
                            Err(e) => {
                                eprintln!("Tray unavailable: {e}");
                                fallback = true;
                            }
                        }
                        let start = engine.control(target, "lolping/start", Value::Null);
                        if (first && !background) || fallback || start.is_err() {
                            if let Err(error) = open_gui(&mut gui, &mut generation) {
                                eprintln!("Cannot open settings: {error}");
                                // With no tray, a failed settings process would
                                // leave an invisible application without controls.
                                if fallback {
                                    let _ = proxy.send_event(UserEvent::Quit);
                                }
                            }
                        }
                        let mut prefs = shared.preferences.lock().unwrap();
                        prefs["onboarded"] = json!(true);
                        let _ = settings::atomic_json(&shared.data.join("app.json"), &prefs);
                    }
                }
                engine.tick(target);
                if let Some(tray) = tray.as_mut() {
                    tray.update(&shared);
                }
                if let Some(deadline) = quitting {
                    if gui.as_mut().is_none_or(|ui| ui.exited()) || Instant::now() >= deadline {
                        gui = None;
                        let _ = engine.control(target, "lolping/stop", Value::Null);
                        *flow = ControlFlow::Exit;
                    }
                }
            }
            _ => {}
        }
        if !matches!(*flow, ControlFlow::Exit) {
            *flow = ControlFlow::WaitUntil(if quitting.is_some() {
                Instant::now() + Duration::from_millis(20)
            } else {
                engine.next_wake()
            });
        }
        // Tao 0.35's GTK loop does not install a timeout for WaitUntil. A
        // windowless Wayland backend otherwise sleeps until unrelated input.
        #[cfg(target_os = "linux")]
        linux_wake.update(*flow, &proxy);
    })
}
