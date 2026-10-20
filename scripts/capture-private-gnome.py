#!/usr/bin/env python3
"""Capture only the owned Xvfb output containing the nested GNOME compositor."""
import os
from pathlib import Path
import sys

if (os.environ.get("LOLPING_PRIVATE_GNOME") != "1"
        or os.environ.get("GDK_BACKEND") != "x11"
        or not os.environ.get("LOLPING_PRIVATE_XDISPLAY")
        or os.environ.get("DISPLAY") != os.environ["LOLPING_PRIVATE_XDISPLAY"]):
    raise SystemExit("Capture requires the isolated test's own Xvfb display")
if len(sys.argv) != 2 or sys.argv[1] not in ("signal", "wheel"):
    raise SystemExit("Expected signal or wheel capture")
import gi

gi.require_version("Gdk", "3.0")
from gi.repository import Gdk

Gdk.init([])
display = Gdk.Display.get_default()
if display is None or "X11" not in display.__gtype__.name:
    raise SystemExit("Capture must use the private outer X11 display")
window = Gdk.get_default_root_window()
width, height = window.get_width(), window.get_height()
if width != 1600 or height != 1200:
    raise SystemExit(f"Unexpected private Xvfb dimensions: {width}x{height}")
pixels = Gdk.pixbuf_get_from_window(window, 0, 0, width, height)
if pixels is None:
    raise SystemExit("Could not capture the nested compositor output")
destination = Path(os.environ["LOLPING_GNOME_TEST_DATA"]) / f"{sys.argv[1]}.png"
pixels.savev(str(destination), "png", [], [])
print(f"Saved private GNOME composited output: {destination}")
