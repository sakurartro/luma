# -*- mode: python ; coding: utf-8 -*-

import sys
import subprocess

sys.path.insert(0, SPECPATH)
from frontend.window_backend import _find_layer_shell
from PySide6 import QtCore


system_qt_version = subprocess.check_output(
    ["pkg-config", "--modversion", "Qt6Core"], text=True
).strip()
if system_qt_version != QtCore.qVersion():
    raise RuntimeError(
        "layer-shell-qt and PySide6 must use the same Qt version "
        f"({system_qt_version} != {QtCore.qVersion()})"
    )


layer_shell_files = _find_layer_shell()
if layer_shell_files is None:
    raise RuntimeError("layer-shell-qt is required to build the Wayland launcher")
plugin_root, interface_library = layer_shell_files
layer_shell_binaries = [
    (str(plugin_root / "wayland-shell-integration/liblayer-shell.so"),
     "PySide6/Qt/plugins/wayland-shell-integration"),
    (str(interface_library), "."),
]


a = Analysis(
    ['frontend/app.py'],
    pathex=[],
    binaries=layer_shell_binaries,
    datas=[],
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name='app',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=True,
    upx_exclude=[],
    name='app',
)
