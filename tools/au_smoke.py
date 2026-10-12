#!/usr/bin/env python3
"""Load an uninstalled AUv2 bundle, instantiate it and render the default stereo layout."""
import ctypes as C
import math
import plistlib
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
bundle=ROOT/'target/bundled/Reveler-rs.component'
cf=C.CDLL('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
au=C.CDLL('/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox')
def signature(lib,name,args,result):
    f=getattr(lib,name);f.argtypes=args;f.restype=result;return f
void=C.c_void_p;u32=C.c_uint32;status=C.c_int32
string=signature(cf,'CFStringCreateWithCString',[void,C.c_char_p,u32],void)
url=signature(cf,'CFURLCreateFromFileSystemRepresentation',[void,C.c_char_p,C.c_long,C.c_bool],void)
create=signature(cf,'CFBundleCreate',[void,void],void)
load=signature(cf,'CFBundleLoadExecutable',[void],C.c_bool)
function=signature(cf,'CFBundleGetFunctionPointerForName',[void,void],void)
release=signature(cf,'CFRelease',[void],None)
class Description(C.Structure):
    _fields_=[('type',u32),('subtype',u32),('manufacturer',u32),('flags',u32),('mask',u32)]
class Format(C.Structure):
    _fields_=[('rate',C.c_double),('format',u32),('flags',u32),('bytes_packet',u32),('frames_packet',u32),('bytes_frame',u32),('channels',u32),('bits',u32),('reserved',u32)]
class SMPTE(C.Structure):
    _fields_=[('subframes',C.c_int16),('divisor',C.c_int16),('counter',u32),('type',u32),('flags',u32),('hours',C.c_int16),('minutes',C.c_int16),('seconds',C.c_int16),('frames',C.c_int16)]
class Timestamp(C.Structure):
    _fields_=[('sample',C.c_double),('host',C.c_uint64),('rate',C.c_double),('word',C.c_uint64),('smpte',SMPTE),('flags',u32),('reserved',u32)]
class Buffer(C.Structure):
    _fields_=[('channels',u32),('size',u32),('data',void)]
class Buffers(C.Structure):
    _fields_=[('count',u32),('buffers',Buffer*6)]
Callback=C.CFUNCTYPE(status,void,C.POINTER(u32),C.POINTER(Timestamp),u32,u32,void)
class CallbackInfo(C.Structure):
    _fields_=[('proc',Callback),('ref',void)]
register=signature(au,'AudioComponentRegister',[C.POINTER(Description),void,u32,void],void)
instantiate=signature(au,'AudioComponentInstanceNew',[void,C.POINTER(void)],status)
dispose=signature(au,'AudioComponentInstanceDispose',[void],status)
set_property=signature(au,'AudioUnitSetProperty',[void,u32,u32,u32,void,u32],status)
get_property=signature(au,'AudioUnitGetProperty',[void,u32,u32,u32,void,C.POINTER(u32)],status)
initialize=signature(au,'AudioUnitInitialize',[void],status)
uninitialize=signature(au,'AudioUnitUninitialize',[void],status)
render=signature(au,'AudioUnitRender',[void,C.POINTER(u32),C.POINTER(Timestamp),u32,u32,void],status)
def check(code):
    if code!=0:raise RuntimeError(f'Audio Unit status {code}')
def fourcc(s):return int.from_bytes(s.encode(),'big')
info=plistlib.loads((bundle/'Contents/Info.plist').read_bytes())['AudioComponents'][0]
bundle_path=str(bundle).encode();bundle_url=url(None,bundle_path,len(bundle_path),True)
handle=create(None,bundle_url);assert handle and load(handle)
factory_name=string(None,info['factoryFunction'].encode(),0x08000100);factory=function(handle,factory_name);assert factory
name=string(None,info['name'].encode(),0x08000100)
desc=Description(*[fourcc(info[k]) for k in ['type','subtype','manufacturer']],0,0)
component=register(C.byref(desc),name,info['version'],factory);assert component
for channels in [2]:
    instance=void();check(instantiate(component,C.byref(instance)))
    fmt=Format(48000.,fourcc('lpcm'),41,4,1,4,channels,32,0)
    for scope in [1,2]:check(set_property(instance,8,scope,0,C.byref(fmt),C.sizeof(fmt)))
    @Callback
    def input_audio(_ref,_flags,time,_bus,frames,data):
        buffers=C.cast(data,C.POINTER(Buffers)).contents
        for channel in range(buffers.count):
            values=C.cast(buffers.buffers[channel].data,C.POINTER(C.c_float))
            for i in range(frames):values[i]=.04*math.sin((time.contents.sample+i)*2*math.pi*997/48000)/(channel+1)
        return 0
    callback=CallbackInfo(input_audio,None)
    check(set_property(instance,23,1,0,C.byref(callback),C.sizeof(callback)))
    check(initialize(instance))
    latency=C.c_double();size=u32(C.sizeof(latency));check(get_property(instance,12,0,0,C.byref(latency),C.byref(size)))
    assert round(latency.value*48000)==1698,latency.value
    storage=[(C.c_float*128)() for _ in range(channels)];output=Buffers();output.count=channels
    for i,values in enumerate(storage):output.buffers[i]=Buffer(1,C.sizeof(values),C.cast(values,void))
    flags=u32();peak=0.
    for block in range(100):
        timestamp=Timestamp();timestamp.sample=block*128;timestamp.flags=1
        check(render(instance,C.byref(flags),C.byref(timestamp),0,128,C.byref(output)))
        for values in storage:
            assert all(math.isfinite(v) for v in values)
            peak=max(peak,max(abs(v) for v in values))
    assert peak>0,'AU rendered only silence'
    check(uninitialize(instance));check(dispose(instance))
    print(f'AUv2 {channels} channels: processed 12800 finite frames; latency=1698; peak={peak:.6f}')
for value in [name,factory_name,handle,bundle_url]:release(value)
