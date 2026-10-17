#!/usr/bin/env python3
"""Native overlay regressions in a private Xvfb; never send input to the real desktop.

Requires Xvfb, xcompmgr and the regular GTK/X11 build dependencies. Runs at both
GTK scales 1 and 2, including composited pixel, input-pass-through, focus, screen
edge geometry and idle-release assertions. The helper's X11 test is separate.
"""
import os
from pathlib import Path
import select
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
for program in ("Xvfb", "xcompmgr", "cargo"):
    if not shutil.which(program):
        raise SystemExit(f"Missing test dependency: {program}")


def stop(process):
    if process is None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


for scale in (1, 2):
    server = compositor = None
    try:
        server = subprocess.Popen(
            ["Xvfb", "-displayfd", "1", "-screen", "0", "1024x768x24",
             "+extension", "Composite", "+extension", "RENDER", "-nolisten", "tcp"],
            stdout=subprocess.PIPE, text=True,
        )
        if not select.select([server.stdout], [], [], 10)[0]:
            raise RuntimeError("Xvfb did not start within 10 seconds")
        number = server.stdout.readline().strip()
        if not number.isdigit():
            raise RuntimeError(f"Invalid Xvfb display: {number!r}")
        env = {
            **os.environ, "DISPLAY": ":" + number, "XDG_SESSION_TYPE": "x11",
            "GDK_BACKEND": "x11", "GDK_SCALE": str(scale), "GDK_DPI_SCALE": "1",
            "LOLPING_PRIVATE_XVFB": "1",
        }
        env.pop("WAYLAND_DISPLAY", None)
        # Normal client-side alpha compositing, without drop shadows or fades.
        # -a uses the X server's PictOpSrc copy, which discards window alpha.
        compositor = subprocess.Popen(["xcompmgr", "-n"], env=env)
        time.sleep(0.8)
        if compositor.poll() is not None:
            raise RuntimeError("The X11 compositor exited before the test")
        print(f"Testing native overlay at GDK_SCALE={scale}", flush=True)
        subprocess.run(
            ["cargo", "test", "--locked", "--manifest-path", "backend/Cargo.toml",
             "x11_surface_composites_edges_and_passes_clicks", "--", "--ignored", "--nocapture"],
            cwd=ROOT, env=env, check=True, timeout=900,
        )
        if scale == 1:
            print("Testing the native import file chooser without an XDG portal", flush=True)
            subprocess.run(
                ["cargo", "test", "--locked", "--manifest-path", "backend/Cargo.toml",
                 "linux_native_import_dialog_selects_and_cancels_without_portal",
                 "--", "--ignored", "--nocapture", "--test-threads=1"],
                cwd=ROOT, env={**env, "GTK_USE_PORTAL": "0"}, check=True, timeout=60,
            )
    finally:
        stop(compositor)
        stop(server)
