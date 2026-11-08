"""The racer brain's turnaround arc test F-38160 run offline (the generated code whole and unchanged, vector mode). Real
memory: the brain (+4 racer, +300 aiv, +336 backing up), the aiv (+0 -> tune +8 MaxCurvature, +25168 width, +25176
length), the chassis the code reads (racer +48 -> +8: +24 wheel count, +372 drive, +412 .. +424 the wheels, +64 each
the wheel's position), the nav mesh the code finds through the global 0x828DA3AC (+656 -> +12) with its +76. Stubs:
the racer's matrix getter F-35658 (the case's matrix: +0 right, +32 back, +48 position). The mesh's segment test (vfunc
72) is an oracle: each call logs from / to (r4 / r5), the mesh's +76 and the query width at the caller's frame +156, and
answers bit k of the case's mask on its k-th call. The query constructors / destructors and the guest sin / cos run as
generated.

Cases (seed 38160): pivot and circle, every drive 0 - 4, 2 and 4 wheels, reverse, backing up, angles over +-2 pi (0, +-0,
tiny, NaN), MaxCurvature 0.02 - 0.5, width 1.6 - 2.4, length 3.5 - 5.5, the car anywhere and turned any way, wheels about
the corners, masks 0 - 15.

Writes <out-dir>/arc_ref.txt, hex words: "case" angle reverse pivot backing drive wheels maxcurv width length pos right back
w0 w1 w2 w3 mask "->" result, then per segment call "from.xyz to.xyz m76 width"."""
import argparse
import math
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb

B, R, CR, CH, M, A, TU, GO, MP, MESH, VT = (0x40100000 + 0x1000 * k for k in range(11))
W = 0x40200000
G = 0x828DA3AC
FAKE = 0x40F00000


def case(rnd):
    angle = rnd.choice((0.0, -0.0, 1e-30, -1e-30, rnd.uniform(-math.pi, math.pi), rnd.uniform(-2 * math.pi, 2 * math.pi),
                        rnd.uniform(-0.3, 0.3)))
    if rnd.random() < 0.01:
        angle = float('nan')
    rev, piv, back, drive = rnd.choice((0, 1)), rnd.choice((0, 1)), rnd.choice((0, 1)), rnd.randrange(5)
    nw = rnd.choice((2, 4, 4))
    mc, width, length = rnd.uniform(0.02, 0.5), rnd.uniform(1.6, 2.4), rnd.uniform(3.5, 5.5)
    pos = [rnd.uniform(-900, 900), rnd.uniform(-50, 200), rnd.uniform(-900, 900)]
    h, t = rnd.uniform(-math.pi, math.pi), rnd.choice((0.0, rnd.uniform(-0.2, 0.2)))
    right = [math.cos(h), 0.0, -math.sin(h)]
    bk = [math.sin(h) * math.cos(t), math.sin(t), math.cos(h) * math.cos(t)]
    wheels = []
    for fz, fx in ((1, 1), (-1, -1), (1, -1), (-1, 1)):
        wheels.append([p + fx * width * 0.45 * r - fz * length * 0.35 * b + rnd.uniform(-0.05, 0.05) for p, r, b in zip(pos, right, bk)])
    mask = rnd.choice((0, 15, rnd.getrandbits(4), 7))
    return [fb(angle), rev, piv, back, drive, nw, fb(mc), fb(width), fb(length)] + [fb(x) for x in pos + right + bk + sum(wheels, [])] + [mask]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-38160')
    rnd = random.Random(38160)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    main_src = (
        'static const uint32_t CS[][31]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(auto&s:CS){rt_reset();memset(pointer(%du),0,0xB000);memset(pointer(%du),0,0x4000);' % (B, W) +
        'store(%du+4,%du);store(%du+300,%du);store(%du+48,%du);store(%du+8,%du);store(%du,%du);' % (B, R, B, A, R, CR, CR, CH, A, TU) +
        'store(%du,%du);store(%du+656,%du);store(%du+12,%du);store(%du,%du);store(%du+72,%du);store(%du+76,0x40400000u);' % (G, GO, GO, MP, MP, MESH, MESH, VT, VT, FAKE, MESH) +
        '*pointer(%du+336)=(uint8_t)s[3];store(%du+372,s[4]);store(%du+24,s[5]);store(%du+8,s[6]);store(%du+25168,s[7]);store(%du+25176,s[8]);'
        % (B, CH, CH, TU, A, A) +
        'for(int i=0;i<3;i++){store(%du+48+4*i,s[9+i]);store(%du+4*i,s[12+i]);store(%du+32+4*i,s[15+i]);}' % (M, M, M) +
        'store(%du+60,0x3f800000u);for(int k=0;k<4;k++){store(%du+412+4*k,%du+0x100*k);for(int i=0;i<3;i++)store(%du+0x100*k+64+4*i,s[18+3*k+i]);store(%du+0x100*k+76,0x3f800000u);}'
        % (M, CH, W, W, W) +
        'oracle_mask()=s[30];oracle_nc()=0;Context ctx=fresh();ctx.r3.u32=%du;ctx.f1.f64=(double)[&]{float f;memcpy(&f,&s[0],4);return f;}();' % B +
        'ctx.r5.u32=s[1];ctx.r6.u32=s[2];sub_%s(ctx,nullptr);' % fn +
        'printf("case");for(int i=0;i<31;i++)printf(" %08x",s[i]);printf(" -> %u",ctx.r3.u32&0xff);' +
        'for(uint32_t k=0;k<oracle_nc();k++)for(int i=0;i<8;i++)printf(" %08x",oracle_log()[k][i]);' +
        'if(load(%du+76)!=0x40400000u)printf(" m76-not-restored");printf("\\n");}}' % MESH + nl)
    seg = ('{uint32_t &n=oracle_nc();if(n>=8)throw std::runtime_error("seg calls");auto&L=oracle_log()[n];'
           'for(int i=0;i<3;i++){L[i]=load(ctx.r4.u32+4*i);L[3+i]=load(ctx.r5.u32+4*i);}L[6]=load(%du+76);L[7]=load(ctx.r1.u32+156);'
           'ctx.r3.u32=(oracle_mask()>>n)&1;n++;}' % MESH)
    pre = ('static uint32_t &oracle_mask(){static uint32_t v;return v;}static uint32_t &oracle_nc(){static uint32_t v;return v;}'
           'static uint32_t (&oracle_log())[8][8]{static uint32_t v[8][8];return v;}' + nl)
    stubs = {'sub_' + addr(a.tool, 'F-35658'): 'ctx.r3.u32=%du;' % M}
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + pre
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'arc', stubs=stubs, image=rd.load_image(a.image),
                       fakes={FAKE: seg})
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    assert not any('m76' in x for x in lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'arc_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    from collections import Counter
    res = [x.split('->')[1].split() for x in lines]
    print(len(res), 'cases; clear', Counter(r[0] for r in res), 'segment calls', Counter((len(r) - 1) // 8 for r in res))


if __name__ == '__main__':
    main()
