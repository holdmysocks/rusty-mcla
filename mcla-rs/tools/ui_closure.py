"""recomp_closure with the few shim additions the UI functions need (signed byte/half register views, the count register a
jump table writes before its switch, all eight condition fields, a trap that throws, the vector flush-mode call) and the critical-section imports
replaced by empty bodies (a lock around a C library call: no effect on the values compared)."""
import recomp_closure as rc
from recomp_leaf import shim as _shim

LOCKS = ('__imp__RtlEnterCriticalSection', '__imp__RtlLeaveCriticalSection')


def shim(scratch_end=0x3000):
    s = _shim(scratch_end)
    s = s.replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; uint8_t u8; int16_t s16; int8_t s8;', 1)
    s = s.replace('struct Context { ', 'struct Context { PPCRegister ctr; ', 1)
    s = s.replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr2,cr3,cr4,cr5,cr6,cr7;', 1)
    s = s.replace('struct { void disableFlushMode() {} } fpscr;',
                  'struct { void disableFlushMode() {} void enableFlushMode() {} void enableFlushModeUnconditional() {} } fpscr;')
    return s + 'struct Context; static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'


rc.shim = shim
index, run = rc.index, rc.run


def build(idx, names, main, out, tag, stubs=(), vector=False, scratch_end=0x3000):
    st = dict.fromkeys(stubs, '') if not isinstance(stubs, dict) else dict(stubs)
    for k in LOCKS:
        st.setdefault(k, '')
    return rc.build(idx, names, main, out, tag, st, vector, scratch_end)


def closure(idx, names, stubs=(), limit=60):
    return rc.closure(idx, names, tuple(stubs) + LOCKS, limit)
