#!/usr/bin/env python3
"""A real Wayland client used only by the private GNOME integration test."""
import json
import os
import sys

if os.environ.get("LOLPING_PRIVATE_GNOME") != "1":
    raise SystemExit("Run only inside the isolated GNOME Wayland test")

if sys.argv[1:] == ["--outer-state"]:
    # Read only the private Xvfb keyboard. Do not alter/clear modifiers: the
    # comparison distinguishes XTest delivery from nested compositor state.
    import ctypes as c
    if not os.environ.get("DISPLAY") or os.environ["DISPLAY"] != os.environ.get("LOLPING_PRIVATE_XDISPLAY"):
        raise SystemExit("Outer keyboard inspection requires the isolated Xvfb")
    x = c.CDLL("libX11.so.6")
    x.XOpenDisplay.argtypes = [c.c_char_p]
    x.XOpenDisplay.restype = c.c_void_p
    x.XDefaultRootWindow.argtypes = [c.c_void_p]
    x.XDefaultRootWindow.restype = c.c_ulong
    x.XQueryKeymap.argtypes = [c.c_void_p, c.c_void_p]
    x.XKeysymToKeycode.argtypes = [c.c_void_p, c.c_ulong]
    x.XKeysymToKeycode.restype = c.c_ubyte
    x.XGetInputFocus.argtypes = [c.c_void_p, c.POINTER(c.c_ulong), c.POINTER(c.c_int)]
    x.XQueryPointer.argtypes = [c.c_void_p, c.c_ulong, c.POINTER(c.c_ulong), c.POINTER(c.c_ulong),
                               c.POINTER(c.c_int), c.POINTER(c.c_int), c.POINTER(c.c_int), c.POINTER(c.c_int), c.POINTER(c.c_uint)]
    x.XCloseDisplay.argtypes = [c.c_void_p]
    display = x.XOpenDisplay(None)
    if not display:
        raise SystemExit("Cannot open private Xvfb")
    try:
        keymap = (c.c_ubyte * 32)()
        x.XQueryKeymap(display, keymap)
        keys = {}
        for name, symbol in {"Super_L": 0xffeb, "Alt_L": 0xffe9, "q": 0x71, "e": 0x65, "p": 0x70}.items():
            code = x.XKeysymToKeycode(display, symbol)
            keys[name] = bool(keymap[code // 8] & (1 << (code % 8)))
        focus, revert = c.c_ulong(), c.c_int()
        x.XGetInputFocus(display, c.byref(focus), c.byref(revert))
        root, child, root_x, root_y, win_x, win_y, mask = c.c_ulong(), c.c_ulong(), c.c_int(), c.c_int(), c.c_int(), c.c_int(), c.c_uint()
        x.XQueryPointer(display, x.XDefaultRootWindow(display), c.byref(root), c.byref(child),
                        c.byref(root_x), c.byref(root_y), c.byref(win_x), c.byref(win_y), c.byref(mask))
        print(json.dumps({"keys": keys, "modifiers": mask.value, "focus": focus.value}), flush=True)
    finally:
        x.XCloseDisplay(display)
    raise SystemExit(0)

if os.environ.get("GDK_BACKEND") != "wayland":
    raise SystemExit("The input client must use native Wayland")
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk


def emit(event):
    print(json.dumps(event), flush=True)


window = Gtk.Window(title="LOLPing Wayland input acceptance")
window.set_default_size(1100, 800)
window.connect("destroy", Gtk.main_quit)
area = Gtk.DrawingArea()
area.set_can_focus(True)
area.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK |
                Gdk.EventMask.KEY_PRESS_MASK | Gdk.EventMask.KEY_RELEASE_MASK |
                Gdk.EventMask.POINTER_MOTION_MASK)
window.add(area)


def on_key(_widget, event, kind):
    emit({"type": kind, "key": Gdk.keyval_name(event.keyval), "state": int(event.state)})
    return False


def on_button(_widget, event, kind):
    area.grab_focus()
    emit({"type": kind, "button": int(event.button), "state": int(event.state)})
    return False


def on_motion(_widget, event):
    emit({"type": "motion", "x": event.x, "y": event.y, "state": int(event.state)})
    return False


area.connect("key-press-event", lambda widget, event: on_key(widget, event, "key"))
area.connect("key-release-event", lambda widget, event: on_key(widget, event, "key-release"))
area.connect("motion-notify-event", on_motion)
window.connect("focus-in-event", lambda _widget, _event: emit({"type": "focus-in"}))
window.connect("focus-out-event", lambda _widget, _event: emit({"type": "focus-out"}))
area.connect("button-press-event", lambda widget, event: on_button(widget, event, "press"))
area.connect("button-release-event", lambda widget, event: on_button(widget, event, "release"))
window.show_all()
window.maximize()
window.present()
area.grab_focus()


def ready():
    display = Gdk.Display.get_default()
    emit({"type": "ready", "displayType": display.__gtype__.name})
    return GLib.SOURCE_REMOVE


GLib.timeout_add(300, ready)
Gtk.main()
