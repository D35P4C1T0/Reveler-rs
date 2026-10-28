#!/usr/bin/env python3
"""Exercise the Linux VST3 editor callback crash in an independent Carla host.

Requires Carla's Python backend, PyQt6, X11/Xwayland, and a running JACK server.
Run after `cargo xtask bundle reveler-rs --release --locked`:
  NICE_LOG=/tmp/reveler-carla.log python tools/carla_gui_smoke.py
  NICE_LOG=/tmp/reveler-carla-direct.log python tools/carla_gui_smoke.py --unbridged

No installed plugins or existing Carla sessions are modified. The child editor
window is resized directly to exercise baseview's host-notification path.
"""
import argparse
import ctypes as c
import os
from pathlib import Path
import sys
import time


class XWindows:
    def __init__(self):
        self.x = x = c.CDLL('libX11.so.6')
        window = c.c_ulong
        x.XOpenDisplay.argtypes = [c.c_char_p]
        x.XOpenDisplay.restype = c.c_void_p
        x.XDefaultRootWindow.argtypes = [c.c_void_p]
        x.XDefaultRootWindow.restype = window
        x.XQueryTree.argtypes = [c.c_void_p, window, c.POINTER(window),
                                c.POINTER(window), c.POINTER(c.POINTER(window)),
                                c.POINTER(c.c_uint)]
        x.XFetchName.argtypes = [c.c_void_p, window, c.POINTER(c.c_char_p)]
        x.XFree.argtypes = [c.c_void_p]
        x.XGetGeometry.argtypes = [c.c_void_p, window, c.POINTER(window),
                                  c.POINTER(c.c_int), c.POINTER(c.c_int),
                                  *([c.POINTER(c.c_uint)] * 4)]
        x.XResizeWindow.argtypes = [c.c_void_p, window, c.c_uint, c.c_uint]
        x.XSync.argtypes = [c.c_void_p, c.c_int]
        x.XCloseDisplay.argtypes = [c.c_void_p]
        self.display = x.XOpenDisplay(None)
        assert self.display, 'Cannot open X11 display'

    def children(self, window):
        root, parent, count = c.c_ulong(), c.c_ulong(), c.c_uint()
        children = c.POINTER(c.c_ulong)()
        assert self.x.XQueryTree(self.display, window, c.byref(root),
                                c.byref(parent), c.byref(children), c.byref(count))
        try:
            return [children[i] for i in range(count.value)]
        finally:
            if children:
                self.x.XFree(children)

    def name(self, window):
        name = c.c_char_p()
        self.x.XFetchName(self.display, window, c.byref(name))
        try:
            return name.value or b''
        finally:
            if name:
                self.x.XFree(name)

    def resize_editor(self, title):
        root = self.x.XDefaultRootWindow(self.display)
        parents = [w for w in self.children(root) if title.encode() in self.name(w)]
        assert len(parents) == 1, f'Expected one probe window, found {parents}'
        children = self.children(parents[0])
        assert len(children) == 1, f'Expected one embedded editor, found {children}'
        child = children[0]
        root, left, top = c.c_ulong(), c.c_int(), c.c_int()
        width, height, border, depth = (c.c_uint() for _ in range(4))
        assert self.x.XGetGeometry(self.display, child, c.byref(root), c.byref(left),
                                  c.byref(top), c.byref(width), c.byref(height),
                                  c.byref(border), c.byref(depth))
        assert width.value > 100 and height.value > 100
        self.x.XResizeWindow(self.display, child, width.value, height.value - 2)
        self.x.XSync(self.display, 0)
        print(f'Resized editor {width.value}x{height.value} by two pixels', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--unbridged', action='store_true')
    parser.add_argument('--bundle', type=Path, default=Path(__file__).resolve().parents[1]
                        / 'target/bundled/Reveler-rs.vst3')
    args = parser.parse_args()
    assert args.bundle.is_dir(), f'Missing VST3 bundle: {args.bundle}'
    sys.path.insert(0, '/usr/share/carla')
    import carla_backend as backend
    from PyQt6.QtWidgets import QApplication
    app = QApplication([])
    host = backend.CarlaHostDLL('/usr/lib/carla/libcarla_standalone2.so', True)
    errors = []

    def event(_, action, plugin_id, value1, value2, value3, valuef, text):
        if (action in (backend.ENGINE_CALLBACK_ERROR, backend.ENGINE_CALLBACK_PLUGIN_UNAVAILABLE)
                or (action == backend.ENGINE_CALLBACK_UI_STATE_CHANGED and value1 < 0)):
            errors.append((action, value1, text))

    host.set_engine_callback(event)
    host.set_engine_option(backend.ENGINE_OPTION_PATH_BINARIES, 0, '/usr/lib/carla')
    host.set_engine_option(backend.ENGINE_OPTION_PROCESS_MODE,
                           backend.ENGINE_PROCESS_MODE_CONTINUOUS_RACK, '')
    host.set_engine_option(backend.ENGINE_OPTION_PREFER_PLUGIN_BRIDGES,
                           int(not args.unbridged), '')
    host.set_engine_option(backend.ENGINE_OPTION_OSC_ENABLED, 0, '')
    title = f'RevelerCarlaSmoke-{os.getpid()}'
    assert host.engine_init('JACK', title), host.get_last_error()
    windows = None

    def idle(seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            app.processEvents()
            host.engine_idle()
            time.sleep(0.01)
            assert not errors, errors

    try:
        assert host.add_plugin(backend.BINARY_NATIVE, backend.PLUGIN_VST3,
                               str(args.bundle.resolve()), title,
                               '0x52657665,0x6C657252,0x53323032,0x36763031',
                               0, None, 0), host.get_last_error()
        windows = XWindows()
        for cycle in range(3):
            host.show_custom_ui(0, True)
            idle(2)
            windows.resize_editor(title)
            idle(2)
            assert host.get_parameter_count(0) == 10
            before = host.get_current_parameter_value(0, 0)
            ranges = host.get_parameter_ranges(0, 0)
            target = ranges['min'] if before != ranges['min'] else ranges['max']
            host.set_parameter_value(0, 0, target)
            idle(0.5)
            assert abs(host.get_current_parameter_value(0, 0) - target) < .001
            host.show_custom_ui(0, False)
            idle(0.5)
            print(f'PASS open/resize/parameter change/close cycle {cycle + 1}', flush=True)
    finally:
        host.remove_all_plugins()
        assert host.engine_close()
        if windows:
            windows.x.XCloseDisplay(windows.display)
    print('PASS clean shutdown; no engine errors or UI failures', flush=True)


if __name__ == '__main__':
    main()
