"""Three island routines run offline: generated code unchanged (vector mode, the recomp's own -msse4.1 -mssse3), what they call
answered by host bodies that record their arguments or give a case's numbers.

  apply   F-71376  a contact's update: the impulse (+112) at A's point (+48) and its opposite (taken from 0) at B's point
                   (+64), each handed to F-78198 (stubbed: records r4 r5 r6 and the vectors v1 v2) when the manifold (+160)
                   has that side's collider (non-null, its id u16 +8 not 0xffff)
  pend    F-43928  a side's pending velocity: zero stored; the collider's slot 22 (stub: records the step, the part, the point and
                   gives the case's vector) times 'less' unless the flag byte is non-zero; else the instance's slot 6 (stub)
  sweeps  F-50131  an island's velocity solve: F-67704 (stub: records r3 r4 r5 f1) with the sweeps from the island's bytes and the
                   three globals when the island's +52 or +16 is above zero; byte +78 set to 1 either way

Writes <out-dir>/island_small_ref.txt, one case per line "<tag> <input words> -> <output words>" (hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure
from cite_safe import cite_safe

_vector_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda scratch_end=0x3000: '#include <simde/x86/avx2.h>' + chr(10) + _vector_shim(scratch_end)
recomp_closure.EXTRA += ('#define ctr r2' + chr(10) + 'static void dispatch(Context&,const uint8_t*,uint32_t);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10) +
                         'static uint32_t REC[8][12]; static int ncalls; static uint32_t PEND_RES[4], OTHER_RES[4], PEND_SEEN[12];' + chr(10))


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def is_denormal(w):
    return (w >> 23) & 0xFF == 0 and (w & 0x7FFFFF) != 0


def word(rnd, scale=10.0):
    r = rnd.random()
    if r < 0.12:
        w = rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x00800000))
    elif r < 0.9:
        w = fb(rnd.uniform(-scale, scale))
    else:
        w = rnd.getrandbits(32)
    return 0 if is_denormal(w) else w


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fapply, fpend, fsweeps, fimpulse, fsolve = (addr(a.tool, f) for f in ('F-71376', 'F-43928', 'F-50131', 'F-78198', 'F-67704'))
    rnd = random.Random(71376)
    secs = []

    # apply: J(3) PA(3) PB(3) cA idA cB idB m8 m12 m28 m36
    cs = []
    for _ in range(3000):
        cs.append([word(rnd) for _ in range(9)] + [rnd.choice((0, 1, 1, 1)), rnd.choice((0xFFFF, 0, 5, 0x1FFFF, 0xFFFE)), rnd.choice((0, 1, 1, 1)),
                                                  rnd.choice((0xFFFF, 0, 7, 0x2FFFF, 0xFFFE))] + [rnd.getrandbits(32) for _ in range(4)])
    setup = ('for(int k=0;k<3;k++){store(0x1000+112+4*k,c[k]);store(0x1000+48+4*k,c[3+k]);store(0x1000+64+4*k,c[6+k]);}' +
             'store(0x1000+160,0x1200);store(0x1200,c[9]?0x1300:0);store(0x1204,c[11]?0x1340:0);store(0x1208,c[13]);store(0x120C,c[14]);store(0x121C,c[15]);store(0x1224,c[16]);' +
             'store16(0x1300+8,c[10]);store16(0x1340+8,c[12]);ctx.r3.u32=0x1000;ncalls=0;memset(REC,0,sizeof(REC));')
    out = ('printf("%d ",ncalls);for(int j=0;j<2;j++)for(int i=0;i<9;i++)printf("%08x ",REC[j][i]);')
    secs.append(('apply', cs, setup, fapply, out))

    # pend: cA cB flag dt less point(3) part RES(4) OTHER(4)
    cs = []
    for _ in range(4000):
        cs.append([rnd.choice((0, 1, 1, 1)), rnd.choice((0, 1)), rnd.choice((0, 0, 1, 2, 0x100, 0xFF)), word(rnd, 0.1), word(rnd, 2)] +
                  [word(rnd, 20) for _ in range(3)] + [rnd.getrandbits(32)] + [word(rnd, 50) for _ in range(4)] + [word(rnd, 50) for _ in range(4)])
    setup = ('for(int i=0;i<4;i++){store(0x1100+4*i,0xA5A5A5A5);PEND_RES[i]=c[9+i];OTHER_RES[i]=c[13+i];}for(int k=0;k<3;k++)store(0x1600+4*k,c[5+k]);store(0x1600+12,0x11112222);' +
             'store(0x1200,0x1300);store(0x1300+88,0x7F000001);store(0x1400,0x1500);store(0x1500+24,0x7F000002);' +
             'ctx.r3.u32=0x1100;ctx.r4.u32=c[0]?0x1200:0;ctx.r5.u32=c[1]?0x1400:0;ctx.r6.u32=0x1600;ctx.r7.u32=c[8];ctx.r10.u32=c[2];' +
             'float dt,ls;memcpy(&dt,&c[3],4);memcpy(&ls,&c[4],4);ctx.f1.f64=dt;ctx.f2.f64=ls;memset(PEND_SEEN,0,sizeof(PEND_SEEN));')
    out = ('printf("%u ",(unsigned)(ctx.r3.u32==0x1100));for(int i=0;i<4;i++)printf("%08x ",load(0x1100+4*i));for(int i=0;i<9;i++)printf("%08x ",PEND_SEEN[i]);')
    secs.append(('pend', cs, setup, fpend, out))

    # sweeps: g358C g3590 g3594 isl52 isl16 isl20 byte80 rows count
    cs = []
    small = (0, 1, -1, 2, 4, 8, 32, 5, -7, 100)
    for _ in range(3000):
        cs.append([rnd.choice(small) & 0xFFFFFFFF for _ in range(3)] + [rnd.choice((0, 1, -1, 5, 0x7FFFFFFF, -0x80000000)) & 0xFFFFFFFF for _ in range(2)] +
                  [rnd.choice((0, 0, 1, 3, -1)) & 0xFFFFFFFF, rnd.choice((0, 0, 1, 2, 0x80)), rnd.getrandbits(32), rnd.getrandbits(32)])
    setup = ('store(0x8281358C,c[0]);store(0x82813590,c[1]);store(0x82813594,c[2]);store(0x1000+4,0x1100);' +
             'store(0x1100+52,c[3]);store(0x1100+16,c[4]);store(0x1100+20,c[5]);*pointer(0x1100+80)=c[6];*pointer(0x1100+78)=0x55;store(0x1100+88,c[7]);store(0x1100+92,c[8]);' +
             'ctx.r3.u32=0x1000;ncalls=0;memset(REC,0,sizeof(REC));')
    out = ('printf("%d %08x %08x %08x %08x %02x ",ncalls,REC[0][0],REC[0][1],REC[0][2],REC[0][3],(unsigned)*pointer(0x1100+78));')
    secs.append(('sweeps', cs, setup, fsweeps, out))

    stubs = {
        'sub_' + fimpulse: 'if(ncalls<2){REC[ncalls][0]=ctx.r4.u32;REC[ncalls][1]=ctx.r5.u32;REC[ncalls][2]=ctx.r6.u32;'
                           'for(int k=0;k<3;k++){REC[ncalls][3+k]=ctx.v1.u32[3-k];REC[ncalls][6+k]=ctx.v2.u32[3-k];}}ncalls++;',
        'sub_' + fsolve: 'REC[0][0]=ctx.r3.u32;REC[0][1]=ctx.r4.u32;REC[0][2]=ctx.r5.u32;{float f=float(ctx.f1.f64);memcpy(&REC[0][3],&f,4);}ncalls++;',
        # slot 22: r3 = where to write, r4 = the collider, r5 = &dt, r6 = the part, v1 = the point; gives its case's vector at r3
        'fk_pend': 'PEND_SEEN[0]=load(ctx.r5.u32);PEND_SEEN[1]=ctx.r6.u32;for(int k=0;k<3;k++)PEND_SEEN[2+k]=ctx.v1.u32[3-k];'
                   'for(int i=0;i<4;i++)store(ctx.r3.u32+4*i,PEND_RES[i]);',
        # slot 6: r3 = the instance, r4 = where to write, r5 = the part, v1 = the point
        'fk_other': 'PEND_SEEN[5]=ctx.r5.u32;for(int k=0;k<3;k++)PEND_SEEN[6+k]=ctx.v1.u32[3-k];'
                    'for(int i=0;i<4;i++)store(ctx.r4.u32+4*i,OTHER_RES[i]);',
    }
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x7F000001)fk_pend(ctx,base);else if(a==0x7F000002)fk_other(ctx,base);else throw std::runtime_error("unexpected indirect call");}')
    nl = chr(10)
    NL = chr(92) + 'n'
    body = ''
    for tag, cs, setup, fn, outc in secs:
        w = len(cs[0])
        rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cs)
        body += ('{static const uint32_t C[][%d]={%s%s%s};' % (w, nl, rows, nl) +
                 'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;' + setup +
                 'sub_%s(ctx,nullptr);' % fn + outc + 'printf("%s");}}' % NL + nl)
    consts = ('for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);' +
              'store(0x8286093C,0x3F800000);')
    main_src = disp + nl + ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                            nl + consts + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + fapply, 'sub_' + fpend, 'sub_' + fsweeps], main_src, a.out_dir, 'island_small', stubs=stubs, vector=True,
                flags=('-msse4.1', '-mssse3'))
    res = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(res) == sum(len(s[1]) for s in secs), (len(res), sum(len(s[1]) for s in secs))
    lines = []
    k = 0
    for tag, cs, *_ in secs:
        for c in cs:
            lines.append('%s %s -> %s' % (tag, ' '.join('%08x' % x for x in c), res[k]))
            k += 1
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'island_small_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print({s[0]: len(s[1]) for s in secs})


if __name__ == '__main__':
    main()
