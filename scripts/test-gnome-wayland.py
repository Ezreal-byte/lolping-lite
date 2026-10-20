#!/usr/bin/env python3
"""Launch native-headless GNOME 46 on a private D-Bus session and virtual seat.

The real Wayland compositor uses software GL and a virtual monitor, without an
outer X11 backend or access to the login session and the user's desktop.
"""
import argparse
import os
from pathlib import Path
import re
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
for program in ("dbus-run-session", "gnome-shell", "gnome-extensions", "gsettings", "gdbus", "node"):
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
components = subprocess.check_output(
    ["dpkg-query", "-W", "-f=${Package} ${Version}\n", "gnome-shell", "libgjs0g", "libmozjs-115-0t64", "libglib2.0-0t64"],
    text=True,
)
(test_root / "system-packages.txt").write_text(components, encoding="utf-8")
print(components, flush=True)
# A short socket path avoids AF_UNIX's path limit in deeply nested CI repos.
with tempfile.TemporaryDirectory(prefix="lolping-gnome-runtime-") as runtime:
    os.chmod(runtime, 0o700)
    home = test_root / "home"
    for directory in (home, home / ".config", home / ".cache", home / ".local/share"):
        directory.mkdir(parents=True, exist_ok=True)
    installed = home / ".local/share/gnome-shell/extensions" / UUID
    shutil.copytree(extension, installed)
    env = {
        **os.environ,
        "HOME": str(home),
        "XDG_RUNTIME_DIR": runtime, "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_CACHE_HOME": str(home / ".cache"), "XDG_DATA_HOME": str(home / ".local/share"),
        "XDG_SESSION_TYPE": "wayland", "XDG_CURRENT_DESKTOP": "GNOME", "XDG_SESSION_DESKTOP": "gnome",
        "GSETTINGS_BACKEND": "dconf", "LIBGL_ALWAYS_SOFTWARE": "1",
        "GNOME_SHELL_SESSION_MODE": "user",
        "NO_AT_BRIDGE": "1", "LOLPING_PRIVATE_GNOME": "1",
        "LOLPING_GNOME_HEADLESS": "1",
        "LOLPING_GNOME_TEST_DATA": str(test_root),
    }
    # Never inherit a real desktop's D-Bus, display or input settings.
    for key in ("DBUS_SESSION_BUS_ADDRESS", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "GDK_BACKEND", "DISPLAY", "LOLPING_PRIVATE_XDISPLAY",
                "WINIT_UNIX_BACKEND", "GDK_SCALE", "GDK_DPI_SCALE", "GSETTINGS_SCHEMA_DIR",
                "GIO_MODULE_DIR", "GIO_EXTRA_MODULES", "DCONF_PROFILE", "LD_LIBRARY_PATH"):
        env.pop(key, None)
    print(f"Testing {version} with native headless Wayland and a private virtual seat; evidence: {test_root}", flush=True)
    subprocess.run(
        ["dbus-run-session", "--", "node", str(REPOSITORY / "scripts/test-gnome-wayland.mjs"),
         "--root", str(root), "--executable", str(executable)],
        cwd=REPOSITORY, env=env, check=True, timeout=240,
    )
