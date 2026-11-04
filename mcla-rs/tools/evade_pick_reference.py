"""The walkers' evade clip pick (F-37443) run offline: the generated code whole and unchanged in vector mode, with the
xz angle F-32060 and the C library's acos in the closure, on flat guest memory. Stubbed, as inputs of the case: the
clip's mover sample F-21043 (the case's displacement of each entry: its first call for an entry the one at the pick's
time, its second the one at the clip's end). The list is the manager's avoid list (+8 the entries, 20 bytes: +4
AvoidType, +8 ApproachDir, +16 the clip; +12 the count), here up to 10 entries with random types, sectors, clips
present or not; the shared MWC (0x82860890) is the case's.
Writes <out-dir>/evade_pick_ref.txt: the case's words, then '->', the picked entry (-1 none), the angle written (hex)
and the MWC after (lo carry; the MWC words as two 16-bit hex halves, hi_lo)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

N = 10

_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic const uint32_t *K;static int calls[16];\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
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
    pick, sample = (addr(a.tool, f) for f in ('F-37443', 'F-21043'))
    rnd = random.Random(37443)
    q = math.pi / 4

    def near(v):
        # an angle at a sector edge, a float step either side, or anywhere
        if rnd.random() < 0.4:
            e = rnd.choice((q, 3 * q, -q, -3 * q, 2 * q, -2 * q, math.pi, -math.pi))
            return struct.unpack('>f', struct.pack('>I', fb(e) + rnd.choice((-1, 0, 1))))[0]
        return rnd.uniform(-v, v)
    cases = []
    for _ in range(4000):
        n = rnd.choice((0, 1, 3, 6, 10, 10))
        fem = int(rnd.random() < 0.5)
        f1 = near(4.0)
        f2 = near(4.0)
        f3 = rnd.choice((0.25, 0.2, 0.0, 0.5, rnd.uniform(0, 2)))
        f4 = rnd.uniform(0, 2)
        f5 = rnd.uniform(0, 9)
        lo, carry = rnd.getrandbits(32), rnd.getrandbits(32)
        c = [n, fem, fb(f1), fb(f2), fb(f3), fb(f4), fb(f5), lo, carry]
        for i in range(N):
            ty = rnd.choice((0, 1, 1, 2, 2, 3, 4, 5))
            dr = rnd.choice((0, 0, 0, 1, 2, 3, 4))
            pres = int(rnd.random() < 0.9)
            r = rnd.uniform(0, 4)
            if rnd.random() < 0.3:
                # the direction about 50 degrees off the threat angle
                ang = f1 + rnd.choice((1, -1)) * 0.8726646 + rnd.uniform(-1e-6, 1e-6)
            elif rnd.random() < 0.6:
                ang = f1 + rnd.uniform(-0.9, 0.9)
            else:
                ang = rnd.uniform(-math.pi, math.pi)
            # xz_angle(d, -z): d = (-sin a, y, -cos a) r
            dt = (-math.sin(ang) * r, rnd.uniform(-0.1, 0.1), -math.cos(ang) * r)
            re_ = rnd.uniform(0, 5)
            de = (rnd.uniform(-1, 1) * re_, 0.0, rnd.uniform(-1, 1) * re_)
            c += [ty, dr, pres] + [fb(x) for x in dt + de]
        cases.append(c)
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    # the mover sample: r3 = the clip (0x1800 + 16 i), r7 = the out block (+16 the point)
    stub = ('int i=(ctx.r3.u32-0x1800)/16;int k=9+i*9+3+(calls[i]++?3:0);'
            'store(ctx.r7.u32+16,K[k]);store(ctx.r7.u32+20,K[k+1]);store(ctx.r7.u32+24,K[k+2]);store(ctx.r7.u32+28,0);')
    setup = ('memset(objects,0,sizeof(objects));memset(calls,0,sizeof(calls));K=c;'
             'store(0x1000,0x1080);store(0x1080+8,0x1100);objects[0x1080-0x1000+12]=0;objects[0x1080-0x1000+13]=c[0];'
             'for(int i=0;i<10;i++){store(0x1100+20*i+4,c[9+9*i]);store(0x1100+20*i+8,c[10+9*i]);'
             'store(0x1100+20*i+16,c[11+9*i]?0x1800+16*i:0);}'
             'put(0x82860890,c[7]);put(0x82860894,c[8]);')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'auto get=[](uint32_t a){const uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|p[1]<<16|p[2]<<8|p[3];};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=c[1];store(0x2F00+84,0x2FC0);store(0x2FC0,0xFFFFFFFF);'
            'float v;memcpy(&v,&c[2],4);ctx.f1.f64=v;memcpy(&v,&c[3],4);ctx.f2.f64=v;memcpy(&v,&c[4],4);ctx.f3.f64=v;'
            'memcpy(&v,&c[5],4);ctx.f4.f64=v;memcpy(&v,&c[6],4);ctx.f5.f64=v;'
            'sub_%s(ctx,nullptr);int e=ctx.r3.u32?int((ctx.r3.u32-0x1100)/20):-1;' % pick +
            ('uint32_t lo=get(0x82860890),cy=get(0x82860894);printf("%%d %%08x %%04x_%%04x %%04x_%%04x%s",e,load(0x2FC0),lo>>16,lo&0xFFFF,cy>>16,cy&0xFFFF);}' % NL) + nl)
    # (the list header: r3 -> +0 the list object, whose +8 the entries and u16 +12 the count)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + pick], main_src, a.out_dir, 'evade_pick', stubs={'sub_' + sample: stub}, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    # (the MWC words as two 16-bit halves: whole, some read like the original's addresses)
    lines = [' '.join(('%04x_%04x' % (x >> 16, x & 0xFFFF)) if i in (7, 8) else '%x' % x for i, x in enumerate(c)) + ' -> ' + o
             for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'evade_pick_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] != '-1' for o in out), 'with a pick')


if __name__ == '__main__':
    main()
