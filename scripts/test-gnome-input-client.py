#!/usr/bin/env python3
"""A real Wayland client used only by the private GNOME integration test."""
import json
import os

if os.environ.get("LOLPING_PRIVATE_GNOME") != "1" or os.environ.get("GDK_BACKEND") != "wayland":
    raise SystemExit("Run only inside the isolated GNOME Wayland test")
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
area.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK | Gdk.EventMask.KEY_PRESS_MASK)
window.add(area)


def on_key(_widget, event):
    emit({"type": "key", "key": Gdk.keyval_name(event.keyval), "state": int(event.state)})
    return False


def on_button(_widget, event, kind):
    area.grab_focus()
    emit({"type": kind, "button": int(event.button), "state": int(event.state)})
    return False


area.connect("key-press-event", on_key)
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
