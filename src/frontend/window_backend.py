"""Window-system setup for the launcher surface."""

import ctypes
import os
from pathlib import Path
import socket
import struct
import sys

import shiboken6
from PySide6 import QtCore


_INTERFACE = "zwlr_layer_shell_v1"
_PLUGIN = "wayland-shell-integration/liblayer-shell.so"
_LIBRARY = "libLayerShellQtInterface.so.6"
_SYMBOLS = {
    "get": "_ZN12LayerShellQt6Window3getEP7QWindow",
    "anchors": "_ZN12LayerShellQt6Window10setAnchorsE6QFlagsINS0_6AnchorEE",
    "keyboard": "_ZN12LayerShellQt6Window24setKeyboardInteractivityENS0_21KeyboardInteractivityE",
    "layer": "_ZN12LayerShellQt6Window8setLayerENS0_5LayerE",
    "active_screen": "_ZN12LayerShellQt6Window26setWantsToBeOnActiveScreenEb",
    "activate": "_ZN12LayerShellQt6Window17setActivateOnShowEb",
}
_library = None


def _supports_layer_shell() -> bool:
    """Read the Wayland registry before Qt connects to the compositor."""
    if os.environ.get("WAYLAND_SOCKET"):
        return False

    display = os.environ.get("WAYLAND_DISPLAY", "wayland-0")
    path = Path(display)
    if not path.is_absolute():
        runtime_dir = os.environ.get("XDG_RUNTIME_DIR")
        if not runtime_dir:
            return False
        path = Path(runtime_dir) / path

    try:
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(1)
            connection.connect(str(path))
            # wl_display.get_registry(new_id=2), then sync(new_id=3).
            connection.sendall(struct.pack("III", 1, (12 << 16) | 1, 2))
            connection.sendall(struct.pack("III", 1, 12 << 16, 3))
            pending = bytearray()
            found = False
            while True:
                while len(pending) < 8:
                    chunk = connection.recv(4096)
                    if not chunk:
                        return False
                    pending.extend(chunk)
                object_id, header = struct.unpack_from("II", pending)
                size, opcode = header >> 16, header & 0xFFFF
                if size < 8:
                    return False
                while len(pending) < size:
                    chunk = connection.recv(4096)
                    if not chunk:
                        return False
                    pending.extend(chunk)
                body = pending[8:size]
                if object_id == 2 and opcode == 0 and len(body) >= 8:
                    name_length = struct.unpack_from("I", body, 4)[0]
                    if bytes(body[8:8 + name_length]).rstrip(b"\0").decode("utf-8", "replace") == _INTERFACE:
                        found = True
                elif object_id == 3 and opcode == 0:
                    return found
                elif object_id == 1 and opcode == 0:
                    return False
                del pending[:size]
    except (OSError, ValueError, struct.error):
        return False


def _find_layer_shell():
    roots = []
    if root := os.environ.get("LUMA_LAYER_SHELL_ROOT"):
        roots.append(Path(root))
    if bundle := getattr(sys, "_MEIPASS", None):
        roots.append(Path(bundle))
    roots.extend((Path("/usr"), Path("/usr/local")))

    for root in roots:
        plugin_roots = (
            root / "PySide6/Qt/plugins",
            root / "lib/qt6/plugins",
            root / "lib64/qt6/plugins",
            root / "lib/x86_64-linux-gnu/qt6/plugins",
        )
        libraries = (
            root / _LIBRARY,
            root / "lib" / _LIBRARY,
            root / "lib64" / _LIBRARY,
            root / "lib/x86_64-linux-gnu" / _LIBRARY,
        )
        for plugin_root in plugin_roots:
            if (plugin_root / _PLUGIN).is_file():
                for library in libraries:
                    if library.is_file():
                        return plugin_root, library
    return None


def prepare_layer_shell() -> bool:
    """Select layer shell only when the compositor and Qt plugin support it."""
    global _library
    if os.environ.get("XDG_SESSION_TYPE") == "x11":
        return False
    if not (os.environ.get("WAYLAND_DISPLAY") or os.environ.get("XDG_SESSION_TYPE") == "wayland"):
        return False
    if os.environ.get("QT_QPA_PLATFORM", "wayland").split(";")[0] not in ("wayland", "wayland-egl"):
        return False
    if os.environ.get("QT_WAYLAND_SHELL_INTEGRATION", "layer-shell") != "layer-shell":
        return False
    if not _supports_layer_shell():
        return False
    locations = _find_layer_shell()
    if locations is None:
        return False

    plugin_root, library_path = locations
    try:
        library = ctypes.CDLL(str(library_path), mode=ctypes.RTLD_GLOBAL)
        for symbol in _SYMBOLS.values():
            getattr(library, symbol)
    except (OSError, AttributeError):
        return False

    _library = library
    existing = os.environ.get("QT_PLUGIN_PATH", "")
    pyside_plugins = QtCore.QLibraryInfo.path(QtCore.QLibraryInfo.PluginsPath)
    paths = [pyside_plugins, str(plugin_root), *existing.split(os.pathsep)]
    os.environ["QT_PLUGIN_PATH"] = os.pathsep.join(
        dict.fromkeys(path for path in paths if path)
    )
    os.environ["QT_WAYLAND_SHELL_INTEGRATION"] = "layer-shell"
    os.environ.setdefault("QT_QPA_PLATFORM", "wayland")
    return True


def configure_layer_window(widget) -> None:
    """Configure the public LayerShellQt::Window API before first show."""
    if _library is None:
        return

    widget.winId()
    qwindow = shiboken6.getCppPointer(widget.windowHandle())[0]
    get = getattr(_library, _SYMBOLS["get"])
    get.argtypes = [ctypes.c_void_p]
    get.restype = ctypes.c_void_p
    layer_window = get(qwindow)
    if not layer_window:
        raise RuntimeError("LayerShellQt could not configure the launcher window")

    for key, value, argument in (
        ("anchors", 0, ctypes.c_int),
        ("keyboard", 1, ctypes.c_int),
        ("layer", 3, ctypes.c_int),
        ("active_screen", True, ctypes.c_bool),
        ("activate", True, ctypes.c_bool),
    ):
        method = getattr(_library, _SYMBOLS[key])
        method.argtypes = [ctypes.c_void_p, argument]
        method(layer_window, value)
    widget.uses_layer_shell = True
