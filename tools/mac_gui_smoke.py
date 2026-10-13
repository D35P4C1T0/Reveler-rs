#!/usr/bin/env python3
"""macOS CLAP editor regression: early AppKit callbacks, parenting and reopening.

The initialization hook exists only in this test process. It reproduces AppKit
re-entering updateTrackingAreas before NSView's initializer has returned.
Tests the native CLAP and the engine embedded in the AU bundle.
"""
import ctypes as c
from pathlib import Path

P, B, U = c.c_void_p, c.c_bool, c.c_uint32
F = c.CFUNCTYPE
ROOT = Path(__file__).resolve().parents[1]
c.CDLL('/System/Library/Frameworks/AppKit.framework/AppKit')
objc = c.CDLL('/usr/lib/libobjc.A.dylib')

def function(lib, name, result, *args):
    fn = getattr(lib, name)
    fn.restype, fn.argtypes = result, list(args)
    return fn

get_class = function(objc, 'objc_getClass', P, c.c_char_p)
selector = function(objc, 'sel_registerName', P, c.c_char_p)
get_method = function(objc, 'class_getInstanceMethod', P, P, P)
get_imp = function(objc, 'method_getImplementation', P, P)
set_imp = function(objc, 'method_setImplementation', P, P, P)
class_name = function(objc, 'object_getClassName', c.c_char_p, P)
# Separate prototypes avoid changing a shared objc_msgSend signature.
send = F(P, P, P)(('objc_msgSend', objc))
send_arg = F(P, P, P, P)(('objc_msgSend', objc))
app = send(get_class(b'NSApplication'), selector(b'sharedApplication'))
assert app

class Point(c.Structure):
    _fields_ = [('x', c.c_double), ('y', c.c_double)]
class Size(c.Structure):
    _fields_ = [('width', c.c_double), ('height', c.c_double)]
class Rect(c.Structure):
    _fields_ = [('origin', Point), ('size', Size)]
class Version(c.Structure):
    _fields_ = [('major', U), ('minor', U), ('revision', U)]
class Entry(c.Structure):
    _fields_ = [('version', Version), ('init', F(B, c.c_char_p)),
                ('deinit', F(None)), ('factory', F(P, c.c_char_p))]
class Factory(c.Structure):
    _fields_ = [('count', F(U, P)), ('description', F(P, P, U)),
                ('create', F(P, P, P, c.c_char_p))]
class Host(c.Structure):
    _fields_ = [('version', Version), ('data', P), ('name', c.c_char_p),
                ('vendor', c.c_char_p), ('url', c.c_char_p), ('release', c.c_char_p),
                ('extension', F(P, P, c.c_char_p)), ('restart', F(None, P)),
                ('process', F(None, P)), ('callback', F(None, P))]
class Plugin(c.Structure):
    _fields_ = [('description', P), ('data', P), ('init', F(B, P)),
                ('destroy', F(None, P)), ('activate', P), ('deactivate', P),
                ('start', P), ('stop', P), ('reset', P), ('process', P),
                ('extension', F(P, P, c.c_char_p)), ('main', F(None, P))]
class HostGui(c.Structure):
    _fields_ = [('hints', F(None, P)), ('resize', F(B, P, U, U)),
                ('show', F(B, P)), ('hide', F(B, P)), ('closed', F(None, P, B))]
class Window(c.Structure):
    _fields_ = [('api', c.c_char_p), ('handle', P)]
class Gui(c.Structure):
    _fields_ = [('supports', F(B, P, c.c_char_p, B)), ('preferred', P),
                ('create', F(B, P, c.c_char_p, B)), ('destroy', F(None, P)),
                ('scale', F(B, P, c.c_double)), ('size', F(B, P, P, P)),
                ('can_resize', P), ('resize_hints', P), ('adjust_size', P),
                ('set_size', F(B, P, U, U)), ('parent', F(B, P, P)),
                ('transient', P), ('title', P), ('show', F(B, P)), ('hide', F(B, P))]

host_gui = HostGui(F(None, P)(lambda _: None), F(B, P, U, U)(lambda *_: True),
                   F(B, P)(lambda _: True), F(B, P)(lambda _: True),
                   F(None, P, B)(lambda *_: None))
callback_pending = False

def request_callback(_):
    global callback_pending
    callback_pending = True

host = Host(Version(1, 2, 6), None, b'Editor regression', b'Reveler-rs', b'', b'1',
            F(P, P, c.c_char_p)(lambda _, name: c.addressof(host_gui) if name == b'clap.gui' else None),
            F(None, P)(lambda _: None), F(None, P)(lambda _: None),
            F(None, P)(request_callback))

init_method = get_method(get_class(b'NSView'), selector(b'initWithFrame:'))
Init = F(P, P, P, Rect)
original_pointer = get_imp(init_method)
original_init = Init(original_pointer)
early_callbacks = 0

@Init
def reentrant_init(view, sel, rect):
    global early_callbacks
    if class_name(view).startswith(b'BaseviewNSView_'):
        early_callbacks += 1
        send(view, selector(b'updateTrackingAreas'))
    return original_init(view, sel, rect)

cf = c.CDLL('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
run_loop = function(cf, 'CFRunLoopRunInMode', c.c_int32, P, c.c_double, B)
default_mode = P.in_dll(cf, 'kCFRunLoopDefaultMode')

# Real initialized parent, retained for the entire editor lifecycle.
parent = send(get_class(b'NSView'), selector(b'alloc'))
parent = original_init(parent, selector(b'initWithFrame:'), Rect(Point(0, 0), Size(620, 520)))
assert parent
window = Window(b'cocoa', parent)
set_imp(init_method, c.cast(reentrant_init, P))
try:
    for relative in ['Reveler-rs.clap/Contents/MacOS/Reveler-rs',
                     'Reveler-rs.component/Contents/PlugIns/Reveler-rs.clap/Contents/MacOS/Reveler-rs']:
        path = ROOT / 'target/bundled' / relative
        lib = c.CDLL(str(path))
        entry = Entry.in_dll(lib, 'clap_entry')
        assert entry.init(str(path).encode())
        fp = entry.factory(b'clap.plugin-factory')
        factory = c.cast(fp, c.POINTER(Factory)).contents
        pp = factory.create(fp, c.byref(host), b'org.reveler-rs.loudness-leveler')
        assert pp
        plugin = c.cast(pp, c.POINTER(Plugin)).contents
        assert plugin.init(pp)
        gui = c.cast(plugin.extension(pp, b'clap.gui'), c.POINTER(Gui)).contents
        assert gui.supports(pp, b'cocoa', False)
        for _ in range(3):
            assert gui.create(pp, b'cocoa', False)
            assert gui.parent(pp, c.byref(window))
            width, height = U(), U()
            assert gui.size(pp, c.byref(width), c.byref(height))
            assert width.value > 0 and height.value > 0
            assert gui.show(pp)
            for _ in range(10):
                run_loop(default_mode, .02, False)
                if callback_pending:
                    callback_pending = False
                    plugin.main(pp)
            assert gui.hide(pp)
            gui.destroy(pp)
            assert send(send(parent, selector(b'subviews')), selector(b'count')) in (None, 0)
        plugin.destroy(pp)
        entry.deinit()
        print(f'{relative}: three editor create/parent/show/render/hide/destroy cycles passed', flush=True)
    assert early_callbacks >= 6
    print(f'Handled {early_callbacks} initialization callbacks safely')
finally:
    set_imp(init_method, original_pointer)
    send(parent, selector(b'release'))
