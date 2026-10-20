#!/usr/bin/env python3
"""Report host GPU-driver loading against system or AppImage dependencies."""
import ctypes
import glob
import json
import os

if os.environ.get("LOLPING_PRIVATE_GNOME") != "1":
    raise SystemExit("Use only the isolated GNOME test launcher")

results = []
loaded = []
libraries = ["libEGL.so.1"]
for pattern in ("/usr/lib/*/libEGL_mesa.so.0", "/usr/lib/*/dri/swrast_dri.so"):
    libraries.extend(glob.glob(pattern))
for library in libraries:
    try:
        handle = ctypes.CDLL(library, mode=os.RTLD_NOW | os.RTLD_LOCAL)
        loaded.append(handle)
        result = {"library": library, "loaded": True}
        if library == "libEGL.so.1":
            handle.eglQueryString.argtypes = [ctypes.c_void_p, ctypes.c_int]
            handle.eglQueryString.restype = ctypes.c_char_p
            extensions = handle.eglQueryString(None, 0x3055)
            result["clientExtensions"] = extensions.decode() if extensions else None
        results.append(result)
    except OSError as error:
        results.append({"library": library, "loaded": False, "error": str(error)})
print(json.dumps({"libraryPath": os.environ.get("LD_LIBRARY_PATH", "system"), "results": results}, indent=2))
