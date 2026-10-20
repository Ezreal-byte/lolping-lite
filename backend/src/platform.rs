//! Desktop capabilities shared by the native controller and overlay.

pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
            || std::env::var("WAYLAND_DISPLAY").is_ok_and(|v| !v.is_empty()))
}

pub fn session_info() -> serde_json::Value {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    serde_json::json!({"wayland":is_wayland(),"desktop":if desktop.split(':').any(|p|p=="gnome"||p=="ubuntu"){"gnome"}else{desktop.as_str()},"extensionId":"lolping-lite@ezreal-byte.github.io"})
}

/// A login session must provide desktop-wide X11 input. Running GTK through
/// XWayland is insufficient: native Wayland applications cannot be observed.
pub fn global_input_available() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux_session_check(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
        )?;
        use gtk::prelude::*;
        if let Some(display) = gtk::gdk::Display::default() {
            if display.type_().name() != "GdkX11Display" {
                return Err(WAYLAND_MESSAGE.into());
            }
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
const WAYLAND_MESSAGE: &str = "Linux global gestures require an X11 login session. Wayland and XWayland cannot capture input from all applications. / Linux 全局手势需要登录 X11 桌面会话，暂不支持 Wayland 或 XWayland。";

#[cfg(any(target_os = "linux", test))]
fn linux_session_check(
    session: Option<&str>,
    wayland_display: Option<&str>,
    display: Option<&str>,
) -> Result<(), String> {
    if session.is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
        || wayland_display.is_some_and(|value| !value.is_empty())
    {
        return Err(WAYLAND_MESSAGE.into());
    }
    if display.is_none_or(str::is_empty) {
        return Err("No X11 display is available. Start LOLPing Lite inside an X11 desktop session. / 未检测到 X11 显示器，请在 X11 桌面会话中启动。".into());
    }
    Ok(())
}

/// Cairo owns the entire transparent window; Tao must not paint over its frame.
pub fn overlay_builder(builder: tao::window::WindowBuilder) -> tao::window::WindowBuilder {
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowBuilderExtUnix;
        return builder
            .with_transparent_draw(false)
            .with_default_vbox(false)
            .with_skip_taskbar(true)
            .with_cursor_moved_event(false);
    }
    #[cfg(not(target_os = "linux"))]
    builder
}

/// AppIndicator creation succeeds even when the desktop has no tray host.
/// Check the actual host before permitting an invisible tray-only application.
#[cfg(target_os = "linux")]
pub fn tray_host_available() -> bool {
    use gtk::{gio, glib::variant::ToVariant};
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return false;
    }
    let Ok(connection) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    // This is the watcher libayatana-appindicator actually registers with.
    // NO_AUTO_START never launches a tray service just for this probe.
    let service = "org.kde.StatusNotifierWatcher";
    let params = (service, "IsStatusNotifierHostRegistered").to_variant();
    connection
        .call_sync(
            Some(service),
            "/StatusNotifierWatcher",
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&params),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            250,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|reply| reply.get::<(gtk::glib::Variant,)>())
        .and_then(|(value,)| value.get::<bool>())
        == Some(true)
}

#[cfg(target_os = "linux")]
pub fn configure_overlay(window: &tao::window::Window) -> Result<gtk::gdk::Window, String> {
    use gtk::{cairo, gdk, prelude::*};
    use tao::platform::unix::WindowExtUnix;

    global_input_available()?;
    let widget = window.gtk_window();
    let screen = GtkWindowExt::screen(widget).ok_or("No X11 screen is available")?;
    if !screen.is_composited() {
        return Err("An X11 compositor is required for transparent overlays (for example, the desktop compositor or picom). / 透明覆盖层需要启用 X11 桌面合成器。".into());
    }
    let visual = screen
        .rgba_visual()
        .ok_or("The X11 display does not offer an RGBA visual")?;
    if !widget.is_realized() {
        widget.set_visual(Some(&visual));
    }
    widget.set_app_paintable(true);
    widget.set_accept_focus(false);
    widget.set_focus_on_map(false);
    widget.set_skip_taskbar_hint(true);
    widget.set_skip_pager_hint(true);
    widget.set_keep_above(true);
    widget.set_type_hint(gdk::WindowTypeHint::Notification);
    widget.set_decorated(false);
    widget.realize();
    let native = widget.window().ok_or("Cannot realize X11 overlay")?;
    // An unmanaged overlay preserves exact screen-edge positioning, regardless
    // of the WM's placement, shadows, tiling rules or focus policy.
    native.set_override_redirect(true);
    native.set_accept_focus(false);
    native.set_focus_on_map(false);
    native.set_pass_through(true);
    native.input_shape_combine_region(&cairo::Region::create(), 0, 0);
    Ok(native)
}

#[cfg(test)]
mod tests {
    use super::linux_session_check;

    #[test]
    fn xwayland_does_not_claim_desktop_wide_input() {
        assert!(linux_session_check(Some("wayland"), None, Some(":0")).is_err());
        assert!(linux_session_check(Some("x11"), Some("wayland-0"), Some(":1")).is_err());
        assert!(linux_session_check(None, None, None).is_err());
        assert!(linux_session_check(Some("x11"), Some(""), Some(":99")).is_ok());
        assert!(linux_session_check(None, None, Some(":99")).is_ok());
    }
}
