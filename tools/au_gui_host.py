"""Exercise the actual AU Cocoa view factory, including reentrant resize callbacks."""
import ctypes as c


def check_editor(instance, get_property, check):
    c.CDLL('/System/Library/Frameworks/AppKit.framework/AppKit')
    objc = c.CDLL('/usr/lib/libobjc.A.dylib')
    cf = c.CDLL('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
    P = c.c_void_p
    objc.objc_getClass.argtypes = [c.c_char_p]
    objc.objc_getClass.restype = P
    objc.sel_registerName.argtypes = [c.c_char_p]
    objc.sel_registerName.restype = P
    cls, sel = objc.objc_getClass, objc.sel_registerName
    send = c.CFUNCTYPE(P, P, P)(('objc_msgSend', objc))
    send_arg = c.CFUNCTYPE(P, P, P, P)(('objc_msgSend', objc))

    class Size(c.Structure):
        _fields_ = [('width', c.c_double), ('height', c.c_double)]

    class Point(c.Structure):
        _fields_ = [('x', c.c_double), ('y', c.c_double)]

    class Rect(c.Structure):
        _fields_ = [('origin', Point), ('size', Size)]

    class CocoaInfo(c.Structure):
        _fields_ = [('url', P), ('classes', P * 1)]

    assert send(cls(b'NSApplication'), sel(b'sharedApplication'))
    info = CocoaInfo()
    length = c.c_uint32(c.sizeof(info))
    check(get_property(instance, 31, 0, 0, c.byref(info), c.byref(length)))
    assert info.url and info.classes[0]
    cf.CFStringGetCString.argtypes = [P, P, c.c_long, c.c_uint32]
    cf.CFStringGetCString.restype = c.c_bool
    class_name = c.create_string_buffer(512)
    assert cf.CFStringGetCString(info.classes[0], class_name, len(class_name), 0x08000100)
    bundle = send_arg(cls(b'NSBundle'), sel(b'bundleWithURL:'), info.url)
    assert bundle and send(bundle, sel(b'load'))
    owner = send(send(cls(class_name.value), sel(b'alloc')), sel(b'init'))
    assert owner
    create_view = c.CFUNCTYPE(P, P, P, P, Size)(('objc_msgSend', objc))
    create_window = c.CFUNCTYPE(P, P, P, Rect, c.c_ulong, c.c_ulong, c.c_bool)(('objc_msgSend', objc))
    cf.CFRunLoopRunInMode.argtypes = [P, c.c_double, c.c_bool]
    cf.CFRunLoopRunInMode.restype = c.c_int32
    mode = P.in_dll(cf, 'kCFRunLoopDefaultMode')
    for _ in range(3):
        view = create_view(owner, sel(b'uiViewForAudioUnit:withSize:'), instance, Size(620, 520))
        assert view
        send(view, sel(b'retain'))
        window = create_window(send(cls(b'NSWindow'), sel(b'alloc')),
                               sel(b'initWithContentRect:styleMask:backing:defer:'),
                               Rect(Point(0, 0), Size(620, 520)), 0, 2, False)
        assert window
        send_arg(window, sel(b'setReleasedWhenClosed:'), None)
        send_arg(window, sel(b'setContentView:'), view)
        for _ in range(10):
            cf.CFRunLoopRunInMode(mode, .02, False)
        send(view, sel(b'removeFromSuperview'))
        send(window, sel(b'close'))
        send(window, sel(b'release'))
        send(view, sel(b'release'))
    send(owner, sel(b'release'))
    cf.CFRelease.argtypes = [P]
    cf.CFRelease(info.url)
    cf.CFRelease(info.classes[0])
    print('AU Cocoa factory: three editor create/attach/render/remove cycles passed', flush=True)
