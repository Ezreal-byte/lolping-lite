#!/usr/bin/env python3
"""JSON-line input/screenshot driver for the isolated native-headless GNOME test.

This is never packaged. Every command uses one persistent D-Bus sender; closing
stdin releases only this driver's held input and closes its remote-input session.
"""
import json
import math
import os
from pathlib import Path
import re
import signal
import stat
import sys

BUS = "org.freedesktop.DBus"
BUS_PATH = "/org/freedesktop/DBus"
REMOTE = "org.gnome.Mutter.RemoteDesktop"
REMOTE_PATH = "/org/gnome/Mutter/RemoteDesktop"
SESSION = REMOTE + ".Session"
SCREENSHOT = "org.gnome.Shell.Screenshot"
SCREENSHOT_PATH = "/org/gnome/Shell/Screenshot"
KEYS = {"Super_L": 125, "Alt_L": 56, "q": 16, "e": 18, "p": 25, "Escape": 1, "a": 30}
BUTTONS = {1: 272, 3: 273}  # Linux evdev BTN_LEFT / BTN_RIGHT, not X11 button codes.


def private_context():
    if (sys.platform != "linux" or os.environ.get("LOLPING_PRIVATE_GNOME") != "1"
            or os.environ.get("LOLPING_GNOME_HEADLESS") != "1" or os.environ.get("DISPLAY")
            or not os.environ.get("DBUS_SESSION_BUS_ADDRESS")):
        raise RuntimeError("Input requires the isolated headless GNOME test and its private D-Bus")
    repository = Path(__file__).resolve().parents[1]
    data = Path(os.environ.get("LOLPING_GNOME_TEST_DATA", ""))
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", ""))
    if (not data.is_absolute() or data.is_symlink() or not data.name.startswith("gnome-wayland-")
            or data.resolve().parent != (repository / ".test-data").resolve()
            or not runtime.is_absolute() or runtime.is_symlink()
            or not runtime.name.startswith("lolping-gnome-runtime-")):
        raise RuntimeError("Unexpected private GNOME test directories")
    for path in (data, runtime):
        info = path.stat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
            raise RuntimeError("GNOME test directories must belong to this user and have mode 0700")
    return data.resolve()


class PrivateSeat:
    def __init__(self, data):
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio, GLib
        self.Gio, self.GLib, self.data = Gio, GLib, data
        self.keys, self.buttons = {}, set()
        self.session = None
        self.connection = Gio.DBusConnection.new_for_address_sync(
            os.environ["DBUS_SESSION_BUS_ADDRESS"],
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None, None)
        try:
            self.shell_pid = self.call(BUS, BUS_PATH, BUS, "GetConnectionUnixProcessID", "(s)", ("org.gnome.Shell",))[0]
            process = Path(f"/proc/{self.shell_pid}")
            arguments = (process / "cmdline").read_bytes().split(b"\0")
            if (process.stat().st_uid != os.getuid() or b"--headless" not in arguments
                    or b"--wayland-display=lolping-test-wayland" not in arguments):
                raise RuntimeError("The D-Bus Shell must be this test's native headless compositor")
            for name in (REMOTE, SCREENSHOT):
                if self.call(BUS, BUS_PATH, BUS, "GetConnectionUnixProcessID", "(s)", (name,))[0] != self.shell_pid:
                    raise RuntimeError(f"{name} is not owned by the private GNOME Shell")
            devices = self.call(REMOTE, REMOTE_PATH, "org.freedesktop.DBus.Properties", "Get",
                                "(ss)", (REMOTE, "SupportedDeviceTypes"))[0]
            if hasattr(devices, "unpack"):
                devices = devices.unpack()
            if devices & 3 != 3:
                raise RuntimeError("The private compositor must support virtual keyboard and pointer input")
            # The screenshot service checks the unique owner of this allowlisted
            # name. Never replace or queue behind an existing owner.
            owned = self.call(BUS, BUS_PATH, BUS, "RequestName", "(su)", ("org.gnome.Screenshot", 4))[0]
            if owned not in (1, 4):
                raise RuntimeError("The private screenshot test name is already in use")
            self.session = self.call(REMOTE, REMOTE_PATH, REMOTE, "CreateSession")[0]
            self.call(REMOTE, self.session, SESSION, "Start")
        except Exception:
            self.close()
            raise

    def call(self, destination, path, interface, method, signature="()", arguments=(), timeout=5000):
        result = self.connection.call_sync(destination, path, interface, method,
                                           self.GLib.Variant(signature, arguments), None,
                                           self.Gio.DBusCallFlags.NO_AUTO_START, timeout, None)
        return result.unpack()

    def key(self, name, pressed):
        if name not in KEYS or type(pressed) is not bool:
            raise ValueError("Expected a supported test key and boolean pressed state")
        if pressed != (name in self.keys):
            self.call(REMOTE, self.session, SESSION, "NotifyKeyboardKeycode", "(ub)", (KEYS[name], pressed))
            if pressed:
                self.keys[name] = KEYS[name]
            else:
                self.keys.pop(name)

    def button(self, number, pressed):
        if type(number) is not int or number not in BUTTONS or type(pressed) is not bool:
            raise ValueError("Expected test button 1 or 3 and boolean pressed state")
        if pressed != (number in self.buttons):
            self.call(REMOTE, self.session, SESSION, "NotifyPointerButton", "(ib)", (BUTTONS[number], pressed))
            if pressed:
                self.buttons.add(number)
            else:
                self.buttons.remove(number)

    def command(self, request):
        command = request.get("command")
        if command == "key":
            self.key(request.get("key"), request.get("pressed"))
        elif command == "button":
            self.button(request.get("button"), request.get("pressed"))
        elif command == "move":
            delta = (request.get("dx"), request.get("dy"))
            if any(type(value) not in (int, float) or not math.isfinite(value) or abs(value) > 32768 for value in delta):
                raise ValueError("Relative test movement must be finite and within 32768 pixels")
            self.call(REMOTE, self.session, SESSION, "NotifyPointerMotionRelative", "(dd)", delta)
        elif command == "screenshot":
            name = request.get("name")
            if not isinstance(name, str) or not re.fullmatch(r"[a-z][a-z0-9-]{0,47}", name):
                raise ValueError("Expected a short screenshot name, not a path")
            destination = self.data / (name + ".png")
            if destination.exists() or destination.is_symlink():
                raise ValueError("A test screenshot must use a new filename")
            success, filename = self.call(SCREENSHOT, SCREENSHOT_PATH, SCREENSHOT, "Screenshot",
                                          "(bbs)", (False, False, str(destination)), timeout=10000)
            if (not success or Path(filename) != destination or destination.is_symlink()
                    or not destination.is_file()):
                raise RuntimeError("The compositor did not save the expected private screenshot")
            return {"path": str(destination)}
        elif command not in ("state", "quit"):
            raise ValueError("Unknown private seat command")
        # This is the driver's bookkeeping, not a replacement for the GTK
        # client's actual modifier-state assertions. Never mask or reset them.
        return {"heldKeys": list(self.keys), "heldButtons": sorted(self.buttons)}

    def close(self):
        if self.session is not None:
            for name in reversed(list(self.keys)):
                try:
                    self.key(name, False)
                except Exception:
                    pass
            for number in list(self.buttons):
                try:
                    self.button(number, False)
                except Exception:
                    pass
            try:
                self.call(REMOTE, self.session, SESSION, "Stop")
            except Exception:
                pass
            self.session = None
        self.connection.close_sync(None)


def emit(value):
    print(json.dumps(value), flush=True)


def main():
    seat = PrivateSeat(private_context())
    try:
        emit({"event": "ready", "backend": "native-headless", "shellPid": seat.shell_pid})
        while line := sys.stdin.readline(8193):
            if len(line) > 8192:
                raise ValueError("Private seat command exceeds 8192 characters")
            request = None
            try:
                request = json.loads(line)
                if not isinstance(request, dict):
                    raise ValueError("Expected a JSON command object")
                result = seat.command(request)
                emit({"id": request.get("id"), "ok": True, "result": result})
                if request.get("command") == "quit":
                    break
            except (ValueError, TypeError, seat.GLib.Error) as error:
                emit({"id": request.get("id") if isinstance(request, dict) else None,
                      "ok": False, "error": str(error)})
    finally:
        seat.close()


def interrupted(_signal, _frame):
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
    except Exception as error:
        emit({"event": "fatal", "error": str(error)})
        sys.exit(1)
