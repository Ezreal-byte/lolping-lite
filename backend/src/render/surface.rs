//! OS-owned transparent surfaces; there is no browser, GPU device, or swapchain.
//! Pixels are premultiplied RGBA. Windows DIBs use BGRA, CoreGraphics uses RGBA,
//! and X11/Cairo uses native-endian ARGB32.
use super::Bounds;
use tao::window::Window;
use tiny_skia::Pixmap;

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::{ffi::c_void, mem::size_of, ptr};
    use tao::platform::windows::WindowExtWindows;
    use windows_sys::Win32::{
        Foundation::{POINT, SIZE},
        Graphics::{Dwm::*, Gdi::*},
        UI::WindowsAndMessaging::*,
    };
    pub struct Surface {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
        bits: *mut c_void,
        size: (u32, u32),
        geometry: Option<(i32, i32, u32, u32)>,
    }
    impl Surface {
        pub fn new(window: &Window) -> Result<Self, String> {
            unsafe {
                let hwnd = window.hwnd() as _;
                // Tao hides the title bar via WM_NCCALCSIZE but retains WS_CAPTION.
                // DWM can still draw a rounded border/shadow around the rectangular
                // surface, even when all edge pixels have zero alpha. Disable that
                // independently of the styles Tao reapplies on show/hide.
                let policy = DWMNCRP_DISABLED;
                let result = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_NCRENDERING_POLICY as u32,
                    &policy as *const _ as _,
                    size_of::<i32>() as u32,
                );
                if result < 0 {
                    return Err(format!("disableOverlayFrame: {result:#x}"));
                }
                // These Windows 11 attributes are optional on older Windows.
                let corners = DWMWCP_DONOTROUND;
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    &corners as *const _ as _,
                    size_of::<i32>() as u32,
                );
                let border: u32 = 0xfffffffe; // DWMWA_COLOR_NONE
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR as u32,
                    &border as *const _ as _,
                    size_of::<u32>() as u32,
                );
                // Per-pixel layered composition supplies transparency; Tao's DWM
                // blur-behind setup is unnecessary for this rendering path.
                let blur = DWM_BLURBEHIND {
                    dwFlags: DWM_BB_ENABLE,
                    ..std::mem::zeroed()
                };
                DwmEnableBlurBehindWindow(hwnd, &blur);
                // Tao's cursor-ignore path can set a constant-alpha layered window.
                // Reset WS_EX_LAYERED before using per-pixel UpdateLayeredWindow.
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_LAYERED as isize));
                SetWindowLongPtrW(
                    hwnd,
                    GWL_EXSTYLE,
                    style
                        | (WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW)
                            as isize,
                );
                let dc = CreateCompatibleDC(ptr::null_mut());
                if dc.is_null() {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                Ok(Self {
                    dc,
                    bitmap: ptr::null_mut(),
                    old: ptr::null_mut(),
                    bits: ptr::null_mut(),
                    size: (0, 0),
                    geometry: None,
                })
            }
        }
        fn release_bitmap(&mut self) {
            unsafe {
                if !self.bitmap.is_null() {
                    SelectObject(self.dc, self.old);
                    DeleteObject(self.bitmap);
                }
            }
            self.bitmap = ptr::null_mut();
            self.bits = ptr::null_mut();
            self.size = (0, 0);
        }
        pub fn clear(&mut self, _window: &Window) {
            self.release_bitmap();
            self.geometry = None;
        }
        pub fn bytes(&self) -> usize {
            self.size.0 as usize * self.size.1 as usize * 4
        }
        pub fn system_frame(&self, window: &Window) -> Option<bool> {
            let mut enabled: i32 = 0;
            let result = unsafe {
                DwmGetWindowAttribute(
                    window.hwnd() as _,
                    DWMWA_NCRENDERING_ENABLED as u32,
                    &mut enabled as *mut _ as _,
                    size_of::<i32>() as u32,
                )
            };
            (result >= 0).then_some(enabled != 0)
        }
        pub fn present(
            &mut self,
            window: &Window,
            pixels: &Pixmap,
            b: Bounds,
            origin: (i32, i32),
        ) -> Result<(), String> {
            unsafe {
                if self.size != (b.w, b.h) {
                    self.release_bitmap();
                    let info = BITMAPINFO {
                        bmiHeader: BITMAPINFOHEADER {
                            biSize: size_of::<BITMAPINFOHEADER>() as u32,
                            biWidth: b.w as i32,
                            biHeight: -(b.h as i32),
                            biPlanes: 1,
                            biBitCount: 32,
                            biCompression: BI_RGB,
                            ..std::mem::zeroed()
                        },
                        ..std::mem::zeroed()
                    };
                    let bitmap = CreateDIBSection(
                        self.dc,
                        &info,
                        DIB_RGB_COLORS,
                        &mut self.bits,
                        ptr::null_mut(),
                        0,
                    );
                    if bitmap.is_null() {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                    self.bitmap = bitmap;
                    self.old = SelectObject(self.dc, bitmap);
                    self.size = (b.w, b.h);
                }
                let output = std::slice::from_raw_parts_mut(self.bits as *mut u8, self.bytes());
                for (src, dst) in pixels
                    .data()
                    .chunks_exact(4)
                    .zip(output.chunks_exact_mut(4))
                {
                    dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
                }
                let pos = POINT {
                    x: origin.0 + b.x,
                    y: origin.1 + b.y,
                };
                let size = SIZE {
                    cx: b.w as i32,
                    cy: b.h as i32,
                };
                let from = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let geometry = (pos.x, pos.y, b.w, b.h);
                if self.geometry != Some(geometry) {
                    // Establish the complete destination before uploading pixels. Resizing/moving
                    // only through UpdateLayeredWindow can retain the previous clipped visible
                    // region (e.g. a half-wheel at the monitor edge), even though GetWindowRect
                    // reports the new size. Do not preserve old client pixels across this change.
                    if SetWindowPos(
                        window.hwnd() as _,
                        ptr::null_mut(),
                        pos.x,
                        pos.y,
                        size.cx,
                        size.cy,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOCOPYBITS,
                    ) == 0
                    {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                }
                if UpdateLayeredWindow(
                    window.hwnd() as _,
                    ptr::null_mut(),
                    &pos,
                    &size,
                    self.dc,
                    &from,
                    0,
                    &blend,
                    ULW_ALPHA,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                self.geometry = Some(geometry);
                Ok(())
            }
        }
    }
    impl Drop for Surface {
        fn drop(&mut self) {
            self.release_bitmap();
            unsafe {
                DeleteDC(self.dc);
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use gtk::{cairo, glib, prelude::*};
    use std::{cell::RefCell, rc::Rc};
    use tao::platform::unix::WindowExtUnix;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Geometry {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        offset_x: f64,
        offset_y: f64,
        scale: f64,
    }

    impl Geometry {
        fn new(bounds: Bounds, origin: (i32, i32), scale: f64) -> Self {
            let physical_x = f64::from(origin.0) + f64::from(bounds.x);
            let physical_y = f64::from(origin.1) + f64::from(bounds.y);
            let x = (physical_x / scale).floor();
            let y = (physical_y / scale).floor();
            // GTK window geometry is integral logical pixels. Expand outwards
            // and keep the sub-logical-pixel offset in the image placement, so
            // an odd physical coordinate at 2x scaling does not move the wheel.
            Self {
                x: x as i32,
                y: y as i32,
                width: (((physical_x + f64::from(bounds.w)) / scale).ceil() - x).max(1.) as i32,
                height: (((physical_y + f64::from(bounds.h)) / scale).ceil() - y).max(1.) as i32,
                offset_x: physical_x / scale - x,
                offset_y: physical_y / scale - y,
                scale,
            }
        }
    }

    #[derive(Default)]
    struct Frame {
        image: Option<cairo::ImageSurface>,
        geometry: Option<Geometry>,
        error: Option<String>,
    }

    fn paint(context: &cairo::Context, frame: &Frame) -> Result<(), cairo::Error> {
        context.save()?;
        let result = (|| {
            // SOURCE replaces alpha as well as RGB: transparent pixels erase
            // the previous frame, including any outward-rounded DPI margins.
            context.set_operator(cairo::Operator::Source);
            context.set_source_rgba(0., 0., 0., 0.);
            context.paint()?;
            if let (Some(image), Some(geometry)) = (&frame.image, frame.geometry) {
                context.set_source_surface(image, geometry.offset_x, geometry.offset_y)?;
                context.source().set_filter(cairo::Filter::Nearest);
                context.paint()?;
            }
            Ok(())
        })();
        // Drop the Cairo source's image reference before the next CPU upload.
        let restore = context.restore();
        result.and(restore)
    }

    fn copy_argb32(image: &mut cairo::ImageSurface, pixels: &Pixmap) -> Result<(), String> {
        let stride = image.stride() as usize;
        let row_bytes = pixels.width() as usize * 4;
        let mut bytes = image.data().map_err(|error| error.to_string())?;
        for (source, target) in pixels
            .data()
            .chunks_exact(row_bytes)
            .zip(bytes.chunks_exact_mut(stride))
        {
            for (rgba, argb) in source.chunks_exact(4).zip(target.chunks_exact_mut(4)) {
                // tiny-skia already premultiplies alpha. Cairo expects the same
                // values in the native byte order of the 0xAARRGGBB word.
                let word = u32::from_be_bytes([rgba[3], rgba[0], rgba[1], rgba[2]]);
                argb.copy_from_slice(&word.to_ne_bytes());
            }
        }
        Ok(())
    }

    pub struct Surface {
        widget: gtk::ApplicationWindow,
        native: gtk::gdk::Window,
        frame: Rc<RefCell<Frame>>,
        draw_handler: Option<glib::SignalHandlerId>,
        map_handler: Option<glib::SignalHandlerId>,
    }

    impl Surface {
        pub fn new(window: &Window) -> Result<Self, String> {
            let native = crate::platform::configure_overlay(window)?;
            let widget = window.gtk_window().clone();
            let frame = Rc::new(RefCell::new(Frame::default()));
            let drawing = frame.clone();
            let draw_handler = widget.connect_draw(move |_, context| {
                let mut frame = drawing.borrow_mut();
                if let Err(error) = paint(context, &frame) {
                    frame.error = Some(format!("Cairo overlay draw failed: {error}"));
                }
                // No themed window background or default child may cover the
                // per-pixel image after this draw handler.
                glib::Propagation::Stop
            });
            let map_handler = widget.connect_map(|widget| {
                if let Some(native) = widget.window() {
                    native.raise();
                }
            });
            Ok(Self {
                widget,
                native,
                frame,
                draw_handler: Some(draw_handler),
                map_handler: Some(map_handler),
            })
        }

        pub fn clear(&mut self, _window: &Window) {
            *self.frame.borrow_mut() = Frame::default();
            self.widget.queue_draw();
        }

        pub fn bytes(&self) -> usize {
            self.frame
                .borrow()
                .image
                .as_ref()
                .map_or(0, |image| image.stride() as usize * image.height() as usize)
        }

        pub fn system_frame(&self, _window: &Window) -> Option<bool> {
            None
        }

        pub fn present(
            &mut self,
            _window: &Window,
            pixels: &Pixmap,
            bounds: Bounds,
            origin: (i32, i32),
        ) -> Result<(), String> {
            if pixels.width() != bounds.w || pixels.height() != bounds.h {
                return Err("Native pixel buffer does not match overlay bounds".into());
            }
            let scale = f64::from(self.widget.scale_factor().max(1));
            let geometry = Geometry::new(bounds, origin, scale);
            let mut frame = self.frame.borrow_mut();
            if let Some(error) = frame.error.take() {
                return Err(error);
            }
            if frame.geometry != Some(geometry) {
                // Update the actual X11 destination before queueing pixels.
                // Tao's setters are asynchronous GTK requests, so using them
                // alone can draw one frame into the previous window's clip.
                self.widget.move_(geometry.x, geometry.y);
                self.widget.resize(geometry.width, geometry.height);
                self.native
                    .move_resize(geometry.x, geometry.y, geometry.width, geometry.height);
                self.native
                    .input_shape_combine_region(Some(&cairo::Region::create()), 0, 0);
            }
            if frame.image.as_ref().is_none_or(|image| {
                image.width() != bounds.w as i32 || image.height() != bounds.h as i32
            }) {
                frame.image = Some(
                    cairo::ImageSurface::create(
                        cairo::Format::ARgb32,
                        bounds.w as i32,
                        bounds.h as i32,
                    )
                    .map_err(|error| error.to_string())?,
                );
            }
            let image = frame.image.as_mut().unwrap();
            image.set_device_scale(scale, scale);
            copy_argb32(image, pixels)?;
            frame.geometry = Some(geometry);
            drop(frame);
            self.widget.queue_draw();
            Ok(())
        }
    }

    impl Drop for Surface {
        fn drop(&mut self) {
            if let Some(handler) = self.draw_handler.take() {
                self.widget.disconnect(handler);
            }
            if let Some(handler) = self.map_handler.take() {
                self.widget.disconnect(handler);
            }
            *self.frame.borrow_mut() = Frame::default();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn physical_edges_keep_their_anchor_at_double_scale() {
            let bounds = Bounds {
                x: 1,
                y: 3,
                w: 101,
                h: 107,
            };
            let g = Geometry::new(bounds, (-1920, -1080), 2.);
            assert_eq!((g.x, g.y, g.width, g.height), (-960, -539, 51, 54));
            assert_eq!((g.offset_x, g.offset_y), (0.5, 0.5));
            assert_eq!((f64::from(g.x) + g.offset_x) * g.scale, -1919.);
            assert_eq!((f64::from(g.y) + g.offset_y) * g.scale, -1077.);
        }

        #[test]
        fn cairo_preserves_alpha_and_clears_previous_frame() {
            let mut pixels = Pixmap::new(8, 8).unwrap();
            pixels.fill(tiny_skia::Color::from_rgba8(200, 40, 10, 128));
            let mut source = cairo::ImageSurface::create(cairo::Format::ARgb32, 8, 8).unwrap();
            copy_argb32(&mut source, &pixels).unwrap();
            let mut target = cairo::ImageSurface::create(cairo::Format::ARgb32, 8, 8).unwrap();
            let context = cairo::Context::new(&target).unwrap();
            let frame = Frame {
                image: Some(source),
                geometry: Some(Geometry::new(
                    Bounds {
                        x: 0,
                        y: 0,
                        w: 8,
                        h: 8,
                    },
                    (0, 0),
                    1.,
                )),
                error: None,
            };
            paint(&context, &frame).unwrap();
            target
                .with_data(|bytes| {
                    assert_eq!(
                        u32::from_ne_bytes(bytes[0..4].try_into().unwrap()),
                        0x80641405
                    );
                })
                .unwrap();
            paint(&context, &Frame::default()).unwrap();
            drop(context);
            assert!(target.data().unwrap().iter().all(|byte| *byte == 0));
        }

        #[test]
        #[ignore = "requires the private Xvfb and compositor started by scripts/test-linux-surface.py"]
        fn x11_surface_composites_edges_and_passes_clicks() {
            use std::{
                cell::Cell,
                time::{Duration, Instant},
            };
            use tao::{
                dpi::PhysicalSize,
                event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
                platform::{
                    run_return::EventLoopExtRunReturn,
                    unix::{EventLoopBuilderExtUnix, EventLoopWindowTargetExtUnix},
                },
                window::WindowBuilder,
            };

            assert_eq!(std::env::var("LOLPING_PRIVATE_XVFB").as_deref(), Ok("1"),
                "run scripts/test-linux-surface.py; this test must not inject input into a user's desktop");
            let mut event_loop = EventLoopBuilder::<()>::new().with_any_thread(true).build();
            let connection = event_loop.xlib_xconnection().unwrap();
            let xlib = &connection.xlib;
            let display = connection.display;
            let root = unsafe { (xlib.XDefaultRootWindow)(display) };

            fn pump(event_loop: &mut EventLoop<()>) {
                let until = Instant::now() + Duration::from_millis(200);
                event_loop.run_return(|_, _, control| {
                    *control = if Instant::now() >= until {
                        ControlFlow::Exit
                    } else {
                        ControlFlow::Poll
                    };
                });
            }

            let background = gtk::Window::new(gtk::WindowType::Toplevel);
            background.set_decorated(false);
            background.set_app_paintable(true);
            background.move_(0, 0);
            background.resize(1024, 768);
            background.connect_draw(|_, context| {
                context.set_operator(cairo::Operator::Source);
                context.set_source_rgb(32. / 255., 64. / 255., 96. / 255.);
                context.paint().unwrap();
                glib::Propagation::Stop
            });
            background.add_events(gtk::gdk::EventMask::BUTTON_PRESS_MASK);
            let clicks = Rc::new(Cell::new(0));
            let clicked = clicks.clone();
            background.connect_button_press_event(move |_, _| {
                clicked.set(clicked.get() + 1);
                glib::Propagation::Stop
            });
            background.show_all();
            pump(&mut event_loop);

            let sample = |x, y| unsafe {
                (xlib.XSync)(display, 0);
                let image = (xlib.XGetImage)(display, root, x, y, 1, 1, !0, 2);
                assert!(!image.is_null());
                let pixel = (xlib.XGetPixel)(image, 0, 0) as u32 & 0xffffff;
                (xlib.XDestroyImage)(image);
                pixel
            };
            assert_eq!(
                sample(500, 300),
                0x204060,
                "test background must be visible"
            );

            let window = crate::platform::overlay_builder(
                WindowBuilder::new()
                    .with_title("LOLPing isolated X11 surface test")
                    .with_visible(false)
                    .with_transparent(true)
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_focusable(false)
                    .with_focused(false)
                    .with_inner_size(PhysicalSize::new(2, 2)),
            )
            .build(&event_loop)
            .unwrap();
            let mut surface = Surface::new(&window).unwrap();
            assert!(surface.native.is_pass_through());
            assert!(!surface.native.accepts_focus());
            assert!(!surface.native.gets_focus_on_map());
            let mut focus = 0;
            let mut revert = 0;
            unsafe {
                (xlib.XGetInputFocus)(display, &mut focus, &mut revert);
            }

            // Move from each clipped edge/corner back to the centre. Check
            // composited desktop pixels, not merely reported HWND/GDK geometry.
            for (x, y, width, height) in [
                (0, 180, 80, 200),
                (210, 140, 240, 240),
                (0, 0, 60, 60),
                (210, 140, 240, 240),
                (950, 180, 74, 200),
                (210, 140, 240, 240),
                (400, 0, 200, 60),
                (210, 140, 240, 240),
                (400, 710, 200, 58),
                (210, 140, 240, 240),
                (960, 0, 64, 60),
                (210, 140, 240, 240),
                (0, 710, 60, 58),
                (210, 140, 240, 240),
                (960, 710, 64, 58),
                (210, 140, 240, 240),
            ] {
                let mut pixels = Pixmap::new(width, height).unwrap();
                pixels.fill(tiny_skia::Color::from_rgba8(200, 40, 10, 255));
                // A transparent corner must show the desktop, not stale pixels.
                for row in 0..20usize {
                    pixels.data_mut()[row * width as usize * 4..row * width as usize * 4 + 80]
                        .fill(0);
                }
                let bounds = Bounds {
                    x,
                    y,
                    w: width,
                    h: height,
                };
                surface.present(&window, &pixels, bounds, (0, 0)).unwrap();
                window.set_visible(true);
                pump(&mut event_loop);
                assert_eq!(
                    sample(x + width as i32 / 2, y + height as i32 / 2),
                    0xc8280a,
                    "missing opaque frame at {x},{y} ({width}x{height})"
                );
                assert_eq!(
                    sample(x + 8, y + 8),
                    0x204060,
                    "transparent pixel at {x},{y} was not cleared"
                );
            }
            let mut new_focus = 0;
            unsafe {
                (xlib.XGetInputFocus)(display, &mut new_focus, &mut revert);
            }
            assert_eq!(new_focus, focus, "overlay must not take keyboard focus");

            #[link(name = "Xtst")]
            extern "C" {
                fn XTestFakeButtonEvent(
                    display: *mut std::ffi::c_void,
                    button: u32,
                    press: i32,
                    delay: u64,
                ) -> i32;
            }
            unsafe {
                (xlib.XWarpPointer)(display, 0, root, 0, 0, 0, 0, 300, 230);
                XTestFakeButtonEvent(display.cast(), 1, 1, 0);
                XTestFakeButtonEvent(display.cast(), 1, 0, 0);
                (xlib.XFlush)(display);
            }
            pump(&mut event_loop);
            assert_eq!(
                clicks.get(),
                1,
                "an opaque overlay pixel must pass clicks to the application below"
            );

            surface.clear(&window);
            window.set_visible(false);
            pump(&mut event_loop);
            assert_eq!(
                surface.bytes(),
                0,
                "idle overlay must release its pixel buffer"
            );
            assert_eq!(sample(300, 230), 0x204060);
            let mut pixels = Pixmap::new(100, 100).unwrap();
            pixels.fill(tiny_skia::Color::from_rgba8(200, 40, 10, 255));
            surface
                .present(
                    &window,
                    &pixels,
                    Bounds {
                        x: 400,
                        y: 300,
                        w: 100,
                        h: 100,
                    },
                    (0, 0),
                )
                .unwrap();
            window.set_visible(true);
            pump(&mut event_loop);
            assert_eq!(
                sample(450, 350),
                0xc8280a,
                "idle wake must display its complete new frame"
            );
            drop(surface);
            drop(window);
            background.close();
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use objc2::{msg_send, runtime::AnyObject};
    use std::{ffi::c_void, ptr};
    use tao::{
        dpi::{PhysicalPosition, PhysicalSize},
        platform::macos::WindowExtMacOS,
    };
    type Ref = *const c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGColorSpaceCreateDeviceRGB() -> Ref;
        fn CGColorSpaceRelease(space: Ref);
        fn CGDataProviderCreateWithData(
            info: *mut c_void,
            data: Ref,
            size: usize,
            release: Option<unsafe extern "C" fn(*mut c_void, Ref, usize)>,
        ) -> Ref;
        fn CGDataProviderRelease(provider: Ref);
        fn CGImageCreate(
            w: usize,
            h: usize,
            bits: usize,
            pixel_bits: usize,
            row: usize,
            space: Ref,
            bitmap_info: u32,
            provider: Ref,
            decode: Ref,
            interpolate: bool,
            intent: u32,
        ) -> Ref;
        fn CGImageRelease(image: Ref);
    }
    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {}
    // The provider owns the boxed byte slice until CALayer releases the image.
    unsafe extern "C" fn release_pixels(_info: *mut c_void, data: Ref, len: usize) {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(
            data as *mut u8,
            len,
        )));
    }
    pub struct Surface {
        layer: *mut AnyObject,
        size: (u32, u32),
    }
    impl Surface {
        pub fn system_frame(&self, _window: &Window) -> Option<bool> {
            None // DWM diagnostic only applies to Windows.
        }
        pub fn new(window: &Window) -> Result<Self, String> {
            unsafe {
                let view = window.ns_view() as *mut AnyObject;
                let _: () = msg_send![view,setWantsLayer:true];
                let layer: *mut AnyObject = msg_send![view, layer];
                if layer.is_null() {
                    return Err("missingNativeLayer".into());
                }
                let _: () = msg_send![layer,setOpaque:false];
                let _: () = msg_send![layer,setContentsScale:window.scale_factor()];
                Ok(Self {
                    layer,
                    size: (0, 0),
                })
            }
        }
        pub fn clear(&mut self, _window: &Window) {
            unsafe {
                let _: () = msg_send![self.layer,setContents:ptr::null::<AnyObject>()];
            }
            self.size = (0, 0);
        }
        pub fn bytes(&self) -> usize {
            self.size.0 as usize * self.size.1 as usize * 4
        }
        pub fn present(
            &mut self,
            window: &Window,
            pixels: &Pixmap,
            b: Bounds,
            origin: (i32, i32),
        ) -> Result<(), String> {
            // Tao converts top-left global physical pixels to AppKit points, including negative monitors.
            window.set_outer_position(PhysicalPosition::new(origin.0 + b.x, origin.1 + b.y));
            if self.size != (b.w, b.h) {
                window.set_inner_size(PhysicalSize::new(b.w, b.h));
            }
            unsafe {
                let space = CGColorSpaceCreateDeviceRGB();
                let bytes = pixels.data().to_vec().into_boxed_slice();
                let len = bytes.len();
                let data = Box::into_raw(bytes) as *const u8;
                let provider = CGDataProviderCreateWithData(
                    ptr::null_mut(),
                    data as Ref,
                    len,
                    Some(release_pixels),
                );
                if provider.is_null() {
                    release_pixels(ptr::null_mut(), data as Ref, len);
                    CGColorSpaceRelease(space);
                    return Err("nativeImageProvider".into());
                }
                // kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big = RGBA byte order.
                let image = CGImageCreate(
                    b.w as usize,
                    b.h as usize,
                    8,
                    32,
                    b.w as usize * 4,
                    space,
                    1 | (4 << 12),
                    provider,
                    ptr::null(),
                    true,
                    0,
                );
                CGDataProviderRelease(provider);
                CGColorSpaceRelease(space);
                if image.is_null() {
                    return Err("nativeImage".into());
                }
                let transaction = objc2::class!(CATransaction);
                let _: () = msg_send![transaction, begin];
                let _: () = msg_send![transaction,setDisableActions:true];
                let _: () = msg_send![self.layer,setContentsScale:window.scale_factor()];
                let _: () = msg_send![self.layer,setContents:image as *const AnyObject];
                let _: () = msg_send![transaction, commit];
                CGImageRelease(image);
                self.size = (b.w, b.h);
            }
            Ok(())
        }
    }
}
pub use platform::Surface;
