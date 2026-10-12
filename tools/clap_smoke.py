"""Minimal macOS CLAP mini-host smoke: metadata, stereo DSP, latency and state.
Loads the CLAP plugin and checks audio, latency and state persistence.
"""
import ctypes as c, pathlib, math
P=c.c_void_p; U=c.c_uint32; B=c.c_bool; D=c.c_double; I=c.c_int32
F=lambda result,*args:c.CFUNCTYPE(result,*args)
class Version(c.Structure): _fields_=[('major',U),('minor',U),('revision',U)]
class Entry(c.Structure): _fields_=[('version',Version),('init',F(B,c.c_char_p)),('deinit',F(None)),('get_factory',F(P,c.c_char_p))]
class Host(c.Structure): _fields_=[('version',Version),('data',P),('name',c.c_char_p),('vendor',c.c_char_p),('url',c.c_char_p),('release',c.c_char_p),('ext',F(P,P,c.c_char_p)),('restart',F(None,P)),('process',F(None,P)),('callback',F(None,P))]
class Plugin(c.Structure): _fields_=[('desc',P),('data',P),('init',F(B,P)),('destroy',F(None,P)),('activate',F(B,P,D,U,U)),('deactivate',F(None,P)),('start',F(B,P)),('stop',F(None,P)),('reset',F(None,P)),('process',F(I,P,P)),('ext',F(P,P,c.c_char_p)),('main',F(None,P))]
class Factory(c.Structure): _fields_=[('count',F(U,P)),('desc',F(P,P,U)),('create',F(P,P,P,c.c_char_p))]
class ParamInfo(c.Structure): _fields_=[('id',U),('flags',U),('cookie',P),('name',c.c_char*256),('module',c.c_char*1024),('min',D),('max',D),('default',D)]
class Params(c.Structure): _fields_=[('count',F(U,P)),('info',F(B,P,U,P)),('value',F(B,P,U,P)),('vtext',P),('tvalue',P),('flush',P)]
class Audio(c.Structure): _fields_=[('data32',c.POINTER(c.POINTER(c.c_float))),('data64',P),('channels',U),('latency',U),('constant',c.c_uint64)]
class InputEvents(c.Structure): _fields_=[('ctx',P),('size',F(U,P)),('get',F(P,P,U))]
class OutputEvents(c.Structure): _fields_=[('ctx',P),('push',F(B,P,P))]
class Process(c.Structure): _fields_=[('time',c.c_int64),('frames',U),('transport',P),('inputs',c.POINTER(Audio)),('outputs',c.POINTER(Audio)),('input_count',U),('output_count',U),('input_events',c.POINTER(InputEvents)),('output_events',c.POINTER(OutputEvents))]
class Latency(c.Structure): _fields_=[('get',F(U,P))]
class State(c.Structure): _fields_=[('save',F(B,P,P)),('load',F(B,P,P))]
class OStream(c.Structure): _fields_=[('ctx',P),('write',F(c.c_int64,P,P,c.c_uint64))]
class IStream(c.Structure): _fields_=[('ctx',P),('read',F(c.c_int64,P,P,c.c_uint64))]
p=pathlib.Path(__file__).resolve().parent.parent / 'target/bundled/Reveler-rs.clap/Contents/MacOS/Reveler-rs'; lib=c.CDLL(str(p)); e=Entry.in_dll(lib,'clap_entry'); assert e.init(str(p).encode())
fp=e.get_factory(b'clap.plugin-factory'); factory=c.cast(fp,c.POINTER(Factory)).contents
callbacks=[F(P,P,c.c_char_p)(lambda *_:None),F(None,P)(lambda *_:None),F(None,P)(lambda *_:None),F(None,P)(lambda *_:None)]
host=Host(Version(1,2,6),None,b'Smoke Test',b'Local',b'',b'0.1',*callbacks)
pp=factory.create(fp,c.byref(host),b'org.reveler-rs.loudness-leveler'); assert pp
plugin=c.cast(pp,c.POINTER(Plugin)).contents; assert plugin.init(pp)
params=c.cast(plugin.ext(pp,b'clap.params'),c.POINTER(Params)).contents; assert params.count(pp)==10
names=[]
for i in range(10):
 info=ParamInfo(); assert params.info(pp,i,c.byref(info)); names.append(info.name.decode()); assert info.flags&32
print('10 automatable parameters:',', '.join(names))
assert plugin.activate(pp,48000,64,64); assert plugin.start(pp)
latency=c.cast(plugin.ext(pp,b'clap.latency'),c.POINTER(Latency)).contents.get(pp); assert latency>0
ins=[(c.c_float*64)(),(c.c_float*64)()]; outs=[(c.c_float*64)(),(c.c_float*64)()]
inptr=(c.POINTER(c.c_float)*2)(*ins); outptr=(c.POINTER(c.c_float)*2)(*outs)
inbuf=Audio(inptr,None,2,0,0); outbuf=Audio(outptr,None,2,0,0)
ie=InputEvents(None,F(U,P)(lambda *_:0),F(P,P,U)(lambda *_:None)); oe=OutputEvents(None,F(B,P,P)(lambda *_:True))
process=Process(0,64,None,c.pointer(inbuf),c.pointer(outbuf),1,1,c.pointer(ie),c.pointer(oe))
energy=0
for block in range(200):
 for n in range(64):
  value=.05*math.sin(2*math.pi*1000*(block*64+n)/48000); ins[0][n]=ins[1][n]=value
 process.time=block*64; assert plugin.process(pp,c.byref(process))>0
 assert all(math.isfinite(v) for channel in outs for v in channel)
 energy+=sum(v*v for v in outs[0])
assert energy>0
plugin.stop(pp); plugin.deactivate(pp)
state=c.cast(plugin.ext(pp,b'clap.state'),c.POINTER(State)).contents
saved=bytearray()
def write(_,ptr,size):saved.extend(c.string_at(ptr,size));return size
outstream=OStream(None,F(c.c_int64,P,P,c.c_uint64)(write)); assert state.save(pp,c.byref(outstream))
position=0
def read(_,ptr,size):
 global position
 data=saved[position:position+size];c.memmove(ptr,bytes(data),len(data));position+=len(data);return len(data)
instream=IStream(None,F(c.c_int64,P,P,c.c_uint64)(read)); assert state.load(pp,c.byref(instream))
assert state.save(pp,c.byref(outstream))
plugin.destroy(pp);e.deinit()
print(f'Processed 12800 stereo frames without nonfinite output; latency={latency}; state save/load succeeds ({position} bytes)')
