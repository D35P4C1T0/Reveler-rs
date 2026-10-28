#!/usr/bin/env python3
"""Minimal native VST3 host smoke test (macOS/Linux ABI, no editor).

Structs and vtable slots follow Steinberg's VST3 interfaces as generated in
vst3 crate 0.3.0. Tests native format instantiation and audio processing.
"""
import ctypes as c
import math
import sys
from pathlib import Path

P, I, U, B = c.c_void_p, c.c_int32, c.c_uint32, c.c_uint8

def method(obj, slot, result, *args):
    table = c.cast(c.cast(obj, c.POINTER(P))[0], c.POINTER(P))
    return c.CFUNCTYPE(result, P, *args)(table[slot])

def uid(hex_string):
    return (c.c_uint8 * 16).from_buffer_copy(bytes.fromhex(hex_string))

class ClassInfo(c.Structure):
    _fields_ = [('cid', c.c_uint8 * 16), ('cardinality', I),
                ('category', c.c_char * 32), ('name', c.c_char * 64)]

class Setup(c.Structure):
    _fields_ = [('mode', I), ('sample_size', I), ('max_block', I), ('rate', c.c_double)]

class Audio(c.Structure):
    _fields_ = [('channels', I), ('silence', c.c_uint64),
                ('data', c.POINTER(c.POINTER(c.c_float)))]

class Process(c.Structure):
    _fields_ = [('mode', I), ('sample_size', I), ('samples', I),
                ('num_inputs', I), ('num_outputs', I),
                ('inputs', c.POINTER(Audio)), ('outputs', c.POINTER(Audio)),
                ('input_params', P), ('output_params', P),
                ('input_events', P), ('output_events', P), ('context', P)]

path = Path(__file__).resolve().parent.parent / 'target/bundled/Reveler-rs.vst3/Contents/MacOS/Reveler-rs'
if sys.platform == 'linux':
    path = path.parent.parent / 'x86_64-linux/Reveler-rs.so'
lib = c.CDLL(str(path))
lib.GetPluginFactory.restype = P
factory = lib.GetPluginFactory()
assert factory and method(factory, 4, I)(factory) >= 1
info = ClassInfo()
assert method(factory, 5, I, I, P)(factory, 0, c.byref(info)) == 0
assert info.name == b'Reveler-rs'
component_iid = uid('E831FF31F2D54301928EBBEE25697802')
audio_iid = uid('42043F99B7DA453CA569E79D9AAEC33D')
for channels in (1, 2, 6):
    component = P()
    assert method(factory, 6, I, P, P, P)(factory, info.cid, component_iid, c.byref(component)) == 0
    assert component.value
    assert method(component, 3, I, P)(component, None) == 0
    processor = P()
    assert method(component, 0, I, P, P)(component, audio_iid, c.byref(processor)) == 0
    arrangement = c.c_uint64({1: 1 << 19, 2: 3, 6: 63}[channels])
    assert method(processor, 3, I, P, I, P, I)(processor, c.byref(arrangement), 1, c.byref(arrangement), 1) == 0
    setup = Setup(0, 0, 64, 48000.0)
    assert method(processor, 7, I, P)(processor, c.byref(setup)) == 0
    assert method(component, 10, I, I, I, I, B)(component, 0, 0, 0, 1) == 0
    assert method(component, 10, I, I, I, I, B)(component, 0, 1, 0, 1) == 0
    assert method(component, 11, I, B)(component, 1) == 0
    latency = method(processor, 6, U)(processor)
    assert latency > 0
    assert method(processor, 8, I, B)(processor, 1) == 0
    ins = [(c.c_float * 64)() for _ in range(channels)]
    outs = [(c.c_float * 64)() for _ in range(channels)]
    ip = (c.POINTER(c.c_float) * channels)(*ins)
    op = (c.POINTER(c.c_float) * channels)(*outs)
    ib, ob = Audio(channels, 0, ip), Audio(channels, 0, op)
    data = Process(0, 0, 64, 1, 1, c.pointer(ib), c.pointer(ob), None, None, None, None, None)
    energy = 0.0
    for block in range(200):
        for n in range(64):
            for channel in ins:
                channel[n] = .05 * math.sin(2 * math.pi * 1000 * (block * 64 + n) / 48000)
        assert method(processor, 9, I, P)(processor, c.byref(data)) == 0
        assert all(math.isfinite(v) for channel in outs for v in channel)
        energy += sum(v * v for v in outs[0])
    assert energy > 0
    assert method(processor, 8, I, B)(processor, 0) == 0
    assert method(component, 11, I, B)(component, 0) == 0
    assert method(component, 4, I)(component) == 0
    method(processor, 2, U)(processor)
    method(component, 2, U)(component)
    print(f'VST3 {channels} channels: processed 12800 finite frames; latency={latency}')
method(factory, 2, U)(factory)
