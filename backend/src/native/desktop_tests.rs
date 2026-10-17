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
    use tao::platform::windows::WindowExtWindows;
    use windows_sys::Win32::{
        Graphics::{Dwm::DwmFlush, Gdi::*},
        UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        },
    };
    unsafe {
        // Other system topmost windows (for example the taskbar) can reorder
        // while the test runs. Raise only our own test overlay without focus.
        assert_ne!(
            SetWindowPos(
                w.window.hwnd() as _,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER
            ),
            0
        );
        assert!(DwmFlush() >= 0);
        let dc = GetDC(std::ptr::null_mut());
        assert!(!dc.is_null());
        // Capture one coherent composited frame. Thousands of GetPixel calls
        // can take a minute and sample different desktop frames along the way.
        let capture = CreateCompatibleDC(dc);
        assert!(!capture.is_null());
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bounds.w as i32,
                biHeight: -(bounds.h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        assert!(!bitmap.is_null());
        let old = SelectObject(capture, bitmap);
        assert_ne!(
            BitBlt(
                capture,
                0,
                0,
                bounds.w as i32,
                bounds.h as i32,
                dc,
                w.display.x + bounds.x,
                w.display.y + bounds.y,
                SRCCOPY | CAPTUREBLT
            ),
            0
        );
        GdiFlush();
        ReleaseDC(std::ptr::null_mut(), dc);
        let desktop = std::slice::from_raw_parts(
            bits as *const u8,
            bounds.w as usize * bounds.h as usize * 4,
        );
        let pixels = w.pixels.as_ref().unwrap();
        let mut checked = 0;
        let mut matched = 0;
        // Small corner crops contain too few opaque points on a 24 px grid:
        // one unrelated desktop pixel would outweigh the 2% tolerance. Keep
        // the same threshold but sample densely enough to detect real clipping.
        for y in (0..bounds.h).step_by(8) {
            for x in (0..bounds.w).step_by(8) {
                let p = pixels.pixel(x, y).unwrap();
                if p.alpha() != 255 {
                    continue;
                }
                let at = (y as usize * bounds.w as usize + x as usize) * 4;
                let actual = [desktop[at + 2], desktop[at + 1], desktop[at]];
                let expected = [p.red(), p.green(), p.blue()];
                checked += 1;
                if actual
                    .into_iter()
                    .zip(expected)
                    .all(|(a, e)| a.abs_diff(e) <= 3)
                {
                    matched += 1;
                } else if checked - matched <= 8 {
                    eprintln!(
                        "{label}: pixel ({},{}) expected {expected:?}, got {actual:?}",
                        w.display.x + bounds.x + x as i32,
                        w.display.y + bounds.y + y as i32
                    );
                    if checked - matched == 1 {
                        diagnose_occlusion(
                            w,
                            w.display.x + bounds.x + x as i32,
                            w.display.y + bounds.y + y as i32,
                        );
                    }
                }
            }
        }
        SelectObject(capture, old);
        DeleteObject(bitmap);
        DeleteDC(capture);
        eprintln!("{label}: {matched}/{checked} opaque desktop pixels match submitted frame");
        assert!(checked >= 100, "{label}: insufficient opaque samples");
        assert!(
            matched * 100 >= checked * 98,
            "{label}: native surface is clipped or stale ({matched}/{checked})"
        );
    }
}

// Report rectangles and owning processes only; do not inspect other apps' text.
// This distinguishes an external always-on-top pixel from a layered-buffer bug.
fn diagnose_occlusion(overlay: &Overlay, x: i32, y: i32) {
    use tao::platform::windows::WindowExtWindows;
    use windows_sys::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{
            GetTopWindow, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
            GW_HWNDNEXT,
        },
    };
    unsafe {
        let own = overlay.window.hwnd() as _;
        let mut candidate = GetTopWindow(std::ptr::null_mut());
        while !candidate.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            if IsWindowVisible(candidate) != 0
                && GetWindowRect(candidate, &mut rect) != 0
                && rect.left <= x
                && x < rect.right
                && rect.top <= y
                && y < rect.bottom
            {
                let mut pid = 0;
                GetWindowThreadProcessId(candidate, &mut pid);
                eprintln!(
                    "pixel z-order: pid={pid} overlay={} rect=({},{})-({},{})",
                    candidate == own,
                    rect.left,
                    rect.top,
                    rect.right,
                    rect.bottom
                );
            }
            if candidate == own {
                break;
            }
            candidate = GetWindow(candidate, GW_HWNDNEXT);
        }
    }
}
