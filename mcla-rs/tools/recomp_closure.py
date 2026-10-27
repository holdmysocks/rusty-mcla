"""Run unmodified recomp functions plus the closure of their direct callees on a flat guest memory.

Builds on the scalar host shim of recomp_leaf.py (64-bit loads/stores added). The generated game code is read
from the user's local recomp and compiled natively; instructions the shim does not know (vector/simde, indirect
calls) fail compilation and the row is simply not verifiable this way. Callees named in `stubs` are replaced by
empty functions (use only for callees that touch fields the comparison ignores, and say so in the evidence).
"""
import re
import subprocess
from pathlib import Path
from recomp_leaf import shim, extract

EXTRA = r'''
static uint64_t load64(uint32_t a){auto p=pointer(a);uint64_t v=0;for(int i=0;i<8;i++)v=(v<<8)|p[i];return v;}
static void store64(uint32_t a,uint64_t v){auto p=pointer(a);for(int i=7;i>=0;i--){p[i]=uint8_t(v);v>>=8;}}
#define REX_LOAD_U64(a) load64(a)
#define REX_STORE_U64(a,v) store64(a,v)
'''


def index(recomp: Path) -> dict:
    idx = {}
    for p in (recomp / 'generated/default').glob('mcla_recomp.*.cpp'):
        for m in re.finditer(r'^DEFINE_REX_FUNC\((\w+)\) \{', p.read_text(errors='replace'), re.M):
            idx[m.group(1)] = p
    return idx


def closure(idx, names, stubs=(), limit=60):
    seen, todo = {}, list(names)
    while todo:
        n = todo.pop()
        if n in seen or n in stubs:
            continue
        seen[n] = extract(idx[n], n)
        todo += re.findall(r'\b(sub_[0-9A-F]{8}|__\w+)\(ctx, base\)', seen[n])
        if len(seen) > limit:
            raise RuntimeError('closure too big')
    return seen


# Vector mode (build(..., vector=True)): the SDK's own simde headers, 128 vector registers and the load/store mask table,
# so functions with lvx128 / stvx128 / lvlx128 / vspltw128 / vmulfp128 ... compile. Each register holds what the
# generated code leaves in it (the PowerPC vector with its bytes reversed, as lvx128's shuffle writes it).
SIMDE = Path('D:/MCLA-static/third_party/rexglue-sdk/thirdparty/simde')


def vector_shim() -> str:
    # VectorMaskL as the SDK's intrinsics.h: row k = bytes 15..0 shifted right by k places, 0xFF filled in front
    rows = ',\n'.join(', '.join('0x%02X' % b for b in [0xFF] * k + list(range(15, k - 1, -1))) for k in range(16))
    prelude = (
        '#include <simde/x86/sse4.1.h>\n#include <simde/x86/ssse3.h>\n'
        'struct alignas(16) VReg { union { uint8_t u8[16]; int8_t s8[16]; uint16_t u16[8]; int16_t s16[8]; uint32_t u32[4];'
        ' int32_t s32[4]; float f32[4]; uint64_t u64[2]; double f64[2]; }; };\n'
        'alignas(16) static uint8_t VectorMaskL[]={' + rows + '};\n')
    s = shim()
    s = s.replace('struct Context {', prelude + 'struct Context { VReg ' + ', '.join(f'v{i}' for i in range(128)) + ';', 1)
    s = s.replace('struct { void disableFlushMode() {} } fpscr;',
                  'struct { void disableFlushMode() {} void enableFlushMode() {} } fpscr;')
    s = s.replace('static uint8_t objects[', 'alignas(16) static uint8_t objects[', 1)
    return s + '#define REX_RAW_ADDR(a) pointer(a)\n'


def build(idx, names, main, out: Path, tag, stubs=(), vector=False, scratch_end=0x3000):
    """`stubs`: names (empty bodies) or a dict name -> C++ body (e.g. to capture an argument); `vector`: see above."""
    fs = closure(idx, names, stubs)
    protos = ''.join(f'void {n}(Context &ctx,const uint8_t *base);\n' for n in list(fs) + list(stubs))
    stubdefs = ''.join('void %s(Context &ctx,const uint8_t *base){%s}\n' % (n, stubs[n] if isinstance(stubs, dict) else '')
                       for n in stubs)
    out.mkdir(parents=True, exist_ok=True)
    cpp = out / f'{tag}.cpp'
    cpp.write_text((vector_shim() if vector else shim(scratch_end)) + EXTRA + protos + stubdefs + ''.join(fs.values()) + main)
    exe = out / f'{tag}.exe'
    r = subprocess.run(['g++', '-std=c++17', '-O2', '-ffp-contract=off', '-static', '-I' + str(SIMDE), str(cpp), '-o', str(exe)],
                       capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr[:800])
    return exe


def run(exe, image: Path) -> str:
    r = subprocess.run([str(exe), str(image), ''], capture_output=True, text=True, check=True)
    return r.stdout
