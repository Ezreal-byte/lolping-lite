//! Real layered-window regression. Run explicitly on an interactive Windows desktop.
use super::*;
use std::path::PathBuf;
use tao::{
    event::Event,
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::{run_return::EventLoopExtRunReturn, windows::EventLoopBuilderExtWindows},
};

#[test]
#[ignore = "requires an interactive Windows desktop"]
fn edge_then_center_keeps_native_surface_geometry() {
    let mut events = EventLoopBuilder::<UserEvent>::with_user_event()
        .with_any_thread(true)
        .build();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.stage/windows-x64");
    let data = tempfile::tempdir().unwrap();
    let shared = Shared::new(root, data.path().into()).unwrap();
    let mut native = Native::new(shared, events.create_proxy());
    native.settings["keepLastPing"] = json!(true);
    native.settings["muted"] = json!(true);
    native.rebuild(&events).unwrap();
    let primary = events.primary_monitor().unwrap().position();
    let id = *native
        .windows
        .iter()
        .find(|(_, w)| w.display.x == primary.x && w.display.y == primary.y)
        .unwrap()
        .0;
    let display = native.windows[&id].display.clone();
    let width = display.w as f64 / display.scale;
    let height = display.h as f64 / display.scale;
    let mut pump = || {
        events.run_return(|event, _, flow| {
            *flow = if matches!(event, Event::MainEventsCleared) {
                ControlFlow::Exit
            } else {
                ControlFlow::Poll
            };
        })
    };
    let mut cases = Vec::new();
    for (label, x, y) in [
        ("left", 0., height / 2.),
        ("top-left", 0., 0.),
        ("right", width - 1., height / 2.),
        ("top", width / 2., 0.),
        ("bottom", width / 2., height - 1.),
        ("top-right", width - 1., 0.),
        ("bottom-left", 0., height - 1.),
        ("bottom-right", width - 1., height - 1.),
    ] {
        cases.push((label, x, y));
        cases.push(("center-after-edge", width / 2., height / 2.));
    }
    cases.push(("idle-wake", width / 2., height / 2.));
    for (step, (label, x, y)) in cases.into_iter().enumerate() {
        if label == "idle-wake" {
            native.dispatch(id, "clear", Value::Null);
            native
                .windows
                .get_mut(&id)
                .unwrap()
                .tick(Instant::now())
                .unwrap();
            assert!(!native.windows[&id].visible);
            assert_eq!(native.windows[&id].surface.bytes(), 0);
        }
        native.dispatch(id, "wheelOpen", json!({"x":x,"y":y}));
        pump();
        native
            .windows
            .get_mut(&id)
            .unwrap()
            .tick(Instant::now() + Duration::from_millis(160))
            .unwrap();
        pump();
        let w = &native.windows[&id];
        let bounds = w.scene.bounds(display.scale, display.w, display.h).unwrap();
        let pos = w.window.outer_position().unwrap();
        let size = w.window.inner_size();
        eprintln!("{label}: expected {bounds:?}, actual {pos:?} {size:?}");
        assert_eq!(
            (pos.x, pos.y),
            (display.x + bounds.x, display.y + bounds.y),
            "{label}"
        );
        assert_eq!((size.width, size.height), (bounds.w, bounds.h), "{label}");
        assert_eq!(w.surface.system_frame(&w.window), Some(false));
        assert_visible_pixels(w, bounds, label);
        if std::env::var_os("LOLPING_PROBE_HOLD").is_some() && step == 1 {
            for _ in 0..2000 {
                pump();
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        // Keep a signal at the edge while opening the next wheel, as in the report.
        native.dispatch(id, "wheelRelease", json!({"x":x+150.,"y":y}));
        native
            .windows
            .get_mut(&id)
            .unwrap()
            .tick(Instant::now())
            .unwrap();
    }
}

// Geometry alone passed before this fix: the layered HWND reported the correct
// enlarged rectangle while the actual desktop still showed the old clipped region.
// Compare opaque pixels only, so the desktop wallpaper cannot affect the result.
fn assert_visible_pixels(w: &Overlay, bounds: crate::render::Bounds, label: &str) {
    use windows_sys::Win32::Graphics::{
        Dwm::DwmFlush,
        Gdi::{GetDC, GetPixel, ReleaseDC},
    };
    unsafe {
        assert!(DwmFlush() >= 0);
        let dc = GetDC(std::ptr::null_mut());
        assert!(!dc.is_null());
        let pixels = w.pixels.as_ref().unwrap();
        let mut checked = 0;
        let mut matched = 0;
        for y in (0..bounds.h).step_by(24) {
            for x in (0..bounds.w).step_by(24) {
                let p = pixels.pixel(x, y).unwrap();
                if p.alpha() != 255 {
                    continue;
                }
                let c = GetPixel(
                    dc,
                    w.display.x + bounds.x + x as i32,
                    w.display.y + bounds.y + y as i32,
                );
                let actual = [c as u8, (c >> 8) as u8, (c >> 16) as u8];
                let expected = [p.red(), p.green(), p.blue()];
                checked += 1;
                if actual
                    .into_iter()
                    .zip(expected)
                    .all(|(a, e)| a.abs_diff(e) <= 3)
                {
                    matched += 1;
                }
            }
        }
        ReleaseDC(std::ptr::null_mut(), dc);
        eprintln!("{label}: {matched}/{checked} opaque desktop pixels match submitted frame");
        assert!(checked >= 10, "{label}: insufficient opaque samples");
        assert!(
            matched * 100 >= checked * 98,
            "{label}: native surface is clipped or stale ({matched}/{checked})"
        );
    }
}
