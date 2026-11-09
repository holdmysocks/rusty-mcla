"""The walker's "something ahead" F-10083 with its detour request F-79428 run offline: both functions' generated code whole
and unchanged in vector mode on flat guest memory (copy of the pattern of amb_wheel_reference.py), with the getter F-88657
in the closure. Stubbed and logged: the passing rail's builder F-92334 (already verified whole: traffic_rails_reference.py
--mode pass; here it prints the head record r3 and the request r4 it is handed and answers the case's result), the stop
timer F-84843 (verified: stop_time_reference.py; prints T), the dynamic cast F-75805 (answers the case's object or 0) and
the follower's rail reset F-78813 (prints F). The obstacle's slot 1 (vt+4) answers its matrix.
Guest memory: the walker's behaviour M 0x1000 (+260 the flags, +224 the type 0x1200 with +492, +96 the stop point out),
the holder 0x1400 (r4; +168 / +172 its two words), the sphere-2 slot r5 0x1600 (+0 the instance), the instance 0x2000
(+4 its archetype 0x2100 with +16 the type bits; +80 the object 0x2600; +128 the entity 0x2400 with +500 / +52 / +496),
the obstacle 0x2800 (vtable 0x2900; its matrix 0x2A00, the place at +48). Per case: M flags, the slot's state, the type
bits, the chain's switches, the type's +492, the holder's words, the builder's answer.
Writes <out-dir>/ped_ahead_ref.txt: the case's words, '->', the log (P head+request words, F, T) and M+260, M+96..+108."""

import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    return s


_shim = recomp_closure.shim
recomp_closure.shim = lambda scratch_end=0x3000: _shim(0x8000)
recomp_closure.vector_shim = vector_shim
NL = chr(92) + 'n'
recomp_closure.EXTRA += ('static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10) +
                         'static const uint32_t *K;' + chr(10) +
                         'static void ind(Context&ctx,uint32_t a){if(a==0x0F000004){ctx.r3.u64=0x2A00;return;}'
                         'printf(" I%08x",a);fflush(stdout);throw std::runtime_error("unexpected indirect call");}' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) ind(ctx,a)' + chr(10))
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nm = {k: addr(a.tool, k) for k in ('F-10083', 'F-79428', 'F-92334', 'F-84843', 'F-75805', 'F-78813', 'F-48384', 'F-29352')}
    rnd = random.Random(10083)
    cases = []
    bits = (0x20, 0x1000, 0x4, 0x8, 0x2, 0x10, 0x1, 0x40, 0x100, 0x800, 0x80, 0x1020, 0x22, 0x0C, 0x12, 0x2C, 0x185, 0x189, 0x1100, 0)
    for _ in range(4000):
        fl = rnd.choice((1, 1, 1, 0x9, 0x11, 0x19, 0, 3, 5, 7, 0x21, rnd.getrandbits(32)))
        slot = rnd.choice((1, 1, 1, 1, 0, 2))           # 0 empty slot, 1 an instance, 2 an instance with no archetype
        ty = rnd.choice(bits) if rnd.random() < 0.85 else rnd.getrandbits(16)
        e500 = rnd.choice((0, 0, 0x20, rnd.getrandbits(8)))
        e52 = rnd.random() < 0.5                        # the cab proxy set
        obst = rnd.random() < 0.9                       # the obstacle word (+496, +128 or +140) set
        x80 = rnd.random() < 0.9                        # the object at +80 set
        cast = rnd.random() < 0.85                      # the dynamic cast answers an object
        r492 = rnd.choice((0.3, 0.0, 0.6, rnd.uniform(0, 2)))
        res = rnd.choice((0, 1, 1, 0x100, 0x101))       # the builder's answer (only its low byte is read)
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        cases.append([fl, slot, ty, e500, int(e52), int(obst), int(x80), int(cast), fb(r492), res,
                      rnd.getrandbits(32), rnd.getrandbits(32)] + [fb(v) for v in place])
    W = len(cases[0])
    nl = chr(10)
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    stubs = {
        'sub_' + nm['F-92334']: ('printf(" P");for(int k=0;k<32;k+=4)printf(" %08x",load(ctx.r3.u32+k));'
                                 'for(int k=0;k<48;k+=4)printf(" %08x",load(ctx.r4.u32+k));ctx.r3.u64=K[9];'),
        'sub_' + nm['F-84843']: 'printf(" T");',
        'sub_' + nm['F-75805']: ('printf(" C %d %08x %08x %08x",ctx.r3.u32==0x2000,ctx.r4.u32,ctx.r5.u32,ctx.r6.u32);'
                                 'ctx.r3.u64=K[7]?0x2700:0;'),
        'sub_' + nm['F-78813']: 'printf(" F %d",ctx.r3.u32==0x1400);',
        'sub_' + nm['F-48384']: 'printf(" S");', 'sub_' + nm['F-29352']: 'printf(" L");',
    }
    setup = ('memset(objects,0,sizeof(objects));K=c;'
             'store(0x1000+260,c[0]);store(0x1000+224,0x1200);store(0x1200+492,c[8]);'
             'store(0x1400+168,c[10]);store(0x1400+172,c[11]);'
             'if(c[1])store(0x1600,0x2000);if(c[1]==1)store(0x2000+4,0x2100);store(0x2100+16,c[2]);'
             'store(0x2000+128,0x2400);*pointer(0x2400+500)=uint8_t(c[3]);if(c[4])store(0x2400+52,0x2500);'
             'if(c[5]){store(0x2400+496,0x2800);store(0x2600+128,0x2800);store(0x2700+140,0x2800);}'
             'if(c[6])store(0x2000+80,0x2600);'
             'store(0x2800,0x2900);store(0x2500,0x2900);store(0x2900+4,0x0F000004);'
             'for(int k=0;k<3;k++)store(0x2A00+48+4*k,c[12+k]);store(0x2A00+60,0x3F800000);')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;ctx.r5.u32=0x1600;'
            'sub_%s(ctx,nullptr);printf(" M %%08x %%08x %%08x %%08x %%08x",load(0x1000+260),load(0x1000+96),load(0x1000+100),'
            'load(0x1000+104),load(0x1000+108));printf("%s");}' % (nm['F-10083'], NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + nm['F-10083']], main_src, a.out_dir, 'ped_ahead', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that would read as addresses of the original are split by '_'; the test removes it)
    (a.out_dir / 'ped_ahead_ref.txt').write_text(re.sub(r'(82[0-9a-f]{2})([0-9a-f]{4})', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
