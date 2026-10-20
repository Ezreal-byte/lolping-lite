#!/usr/bin/env python3
"""Launch GNOME 46 integration tests on a private Xvfb and D-Bus session.

The virtual X server is only the *outer* nested compositor host. The application
and its settings connect to the inner GNOME Wayland display, with a real Shell
extension. No input is sent to the login session or the user's desktop.
"""
import argparse
import os
from pathlib import Path
import re
import select
import shutil
import subprocess
import tempfile

REPOSITORY = Path(__file__).resolve().parents[1]
UUID = "lolping-lite@ezreal-byte.github.io"
parser = argparse.ArgumentParser()
parser.add_argument("--root", type=Path, required=True)
parser.add_argument("--executable", type=Path, required=True)
args = parser.parse_args()
root, executable = args.root.resolve(), args.executable.resolve()
for program in ("Xvfb", "dbus-run-session", "gnome-shell", "gnome-extensions", "gsettings", "gdbus", "xdotool", "node"):
    if not shutil.which(program):
        raise SystemExit(f"Missing GNOME Wayland test dependency: {program}")
version = subprocess.check_output(["gnome-shell", "--version"], text=True).strip()
if not re.search(r"\b46(?:\.|\b)", version):
    raise SystemExit(f"The extension currently targets GNOME 46; found {version}")
if not executable.is_file():
    raise SystemExit(f"Missing application: {executable}")
extension = root / "gnome-extension" / UUID
if not extension.is_dir():
    raise SystemExit(f"Missing bundled extension: {extension}")

reports = REPOSITORY / ".test-data"
reports.mkdir(exist_ok=True)
test_root = Path(tempfile.mkdtemp(prefix="gnome-wayland-", dir=reports))
server = None


def stop(process):
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


try:
    # A short socket path avoids AF_UNIX's path limit in deeply nested CI repos.
    with tempfile.TemporaryDirectory(prefix="lolping-gnome-runtime-") as runtime:
        os.chmod(runtime, 0o700)
        home = test_root / "home"
        for directory in (home, home / ".config", home / ".cache", home / ".local/share"):
            directory.mkdir(parents=True, exist_ok=True)
        installed = home / ".local/share/gnome-shell/extensions" / UUID
        shutil.copytree(extension, installed)
        server = subprocess.Popen(
            ["Xvfb", "-displayfd", "1", "-screen", "0", "1600x1200x24",
             "+extension", "Composite", "+extension", "RENDER", "-nolisten", "tcp"],
            stdout=subprocess.PIPE, text=True,
        )
        if not select.select([server.stdout], [], [], 10)[0]:
            raise RuntimeError("Private Xvfb did not start in 10 seconds")
        number = server.stdout.readline().strip()
        if not number.isdigit():
            raise RuntimeError(f"Invalid private Xvfb display: {number!r}")
        env = {
            **os.environ,
            "HOME": str(home), "DISPLAY": ":" + number,
            "XDG_RUNTIME_DIR": runtime, "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_CACHE_HOME": str(home / ".cache"), "XDG_DATA_HOME": str(home / ".local/share"),
            "XDG_SESSION_TYPE": "wayland", "XDG_CURRENT_DESKTOP": "GNOME", "XDG_SESSION_DESKTOP": "gnome",
            "GSETTINGS_BACKEND": "dconf", "LIBGL_ALWAYS_SOFTWARE": "1",
            "GNOME_SHELL_SESSION_MODE": "user",
            "MUTTER_DEBUG_DUMMY_MODE_SPECS": "1280x960@60.0",
            "NO_AT_BRIDGE": "1", "LOLPING_PRIVATE_GNOME": "1",
            "LOLPING_GNOME_TEST_DATA": str(test_root),
        }
        # Never inherit a real desktop's D-Bus, display or input settings.
        for key in ("DBUS_SESSION_BUS_ADDRESS", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "GDK_BACKEND",
                    "WINIT_UNIX_BACKEND", "GDK_SCALE", "GDK_DPI_SCALE", "GSETTINGS_SCHEMA_DIR",
                    "GIO_MODULE_DIR", "GIO_EXTRA_MODULES", "DCONF_PROFILE", "LD_LIBRARY_PATH"):
            env.pop(key, None)
        print(f"Testing {version} Wayland inside private Xvfb :{number}; evidence: {test_root}", flush=True)
        subprocess.run(
            ["dbus-run-session", "--", "node", str(REPOSITORY / "scripts/test-gnome-wayland.mjs"),
             "--root", str(root), "--executable", str(executable)],
            cwd=REPOSITORY, env=env, check=True, timeout=240,
        )
finally:
    stop(server)
