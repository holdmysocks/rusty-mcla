"""The sound occlusion group run offline: the original's GLOBAL_OCCLUSION reader F-43021 (with F-44963 / F-80129 and the
curve init F-57265), the group's init F-23776, set F-69008, smoothing F-80830 and latch F-43736, and the owner's update
F-87712, the generated code whole and unchanged, on random tunes and op sequences; besides, the pow F-21955 on its own
and the curve evaluation F-33490 of random Gauss curves (F-49411 / F-67749).
Stubbed, as inputs: the tune lookup F-86504 (GLOBAL_OCCLUSION from game.dat, or with random fields), the curve store
F-67395 (the curves.dat object by hash, or a random Gauss curve), the group lock F-60674 / F-60333 / F-47330 (free), the
owner's frame F-91345 (the case's frame), the world probe F-93873 (records its segment, flags and limit; writes the
case's hits: +16 position, +72 material) and the material lookup (vtable +20 of the material manager: +60 -> +0 -> +226
the factor of eight materials). The runtime vector constants (0,0,0,1) and the xyz mask, zero in the image, are filled.
Writes <out-dir>/occlusion_ref.txt (each number led by a letter, so no column reads as an address)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run
from music_play_reference import gamedat

TUNE, CRV, GB, OWN, FRAME, OUTV, COBJ, IN = 0x40100000, 0x40110000, 0x40120000, 0x40200000, 0x40210000, 0x40220000, 0x40230000, 0x40400000
MMGR, MVT, MOBJ, MPTR, MREC = 0x40300000, 0x40301000, 0x40310000, 0x40320000, 0x40330000
FM = 0x7F000010

# the vector powf of the Gauss curve (F-33229) needs the SDK's vctsxs and a per-lane shift the toolchain's SIMDe lacks
# (as solver_reference.py), and the SDK's flush mode as MXCSR FTZ | DAZ (as frame_blend_reference.py)
recomp_closure.EXTRA += r'''
static uint32_t pc;
namespace rex { namespace ppc {
static inline __m128i simde_mm_vctsxs(__m128 src1) {
    __m128 xmm2 = _mm_cmpunord_ps(src1, src1);
    __m128i xmm0 = _mm_cvttps_epi32(src1);
    __m128i xmm1 = _mm_cmpeq_epi32(xmm0, _mm_set1_epi32(INT_MIN));
    xmm1 = _mm_andnot_si128(_mm_castps_si128(src1), xmm1);
    __m128 dest = _mm_blendv_ps(_mm_castsi128_ps(xmm0), _mm_castsi128_ps(_mm_set1_epi32(INT_MAX)), _mm_castsi128_ps(xmm1));
    return _mm_andnot_si128(_mm_castps_si128(xmm2), _mm_castps_si128(dest));
} } }
#ifndef simde_mm_sllv_epi32
static inline __m128i simde_mm_sllv_epi32(__m128i a, __m128i n) {
    alignas(16) uint32_t x[4], k[4]; _mm_store_si128((__m128i*)x, a); _mm_store_si128((__m128i*)k, n);
    for (int i = 0; i < 4; i++) x[i] = k[i] > 31 ? 0 : x[i] << k[i];
    return _mm_load_si128((__m128i*)x);
}
#endif
'''
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + '#include <climits>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)
STACK = 0x40F00000
FACTORS = (1000.0, 20.0, 10.0, 2.0, 1.0, 0.0, 1000.0, 0.5)
GW = 82      # the words of one update's geometry


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def joaat(s):
    h = 0
    for c in s.lower().encode():
        h = (h + c) & 0xffffffff
        h = (h + (h << 10)) & 0xffffffff
        h ^= h >> 6
    h = (h + (h << 3)) & 0xffffffff
    h ^= h >> 11
    return (h + (h << 15)) & 0xffffffff


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--game-dat', type=Path, required=True)
    ap.add_argument('--curves-dat', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    reader, init, setf, smooth, latch, update, pow_, cinit, ceval = (addr(a.tool, f) for f in (
        'F-43021', 'F-23776', 'F-69008', 'F-80830', 'F-43736', 'F-87712', 'F-21955', 'F-57265', 'F-33490'))
    lookup, store_, lk0, lk1, lk2, frame, probe = (addr(a.tool, f) for f in ('F-86504', 'F-67395', 'F-60674', 'F-60333', 'F-47330', 'F-91345', 'F-93873'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    tune = bytearray(gamedat(a.game_dat, 'GLOBAL_OCCLUSION'))
    curves = {n: gamedat(a.curves_dat, n) for n in ('OCCLUSION_FACTOR_TO_LINEAR_VOLUME', 'OCCLUSION_DEPTH_TO_FILTER')}
    ha, hb = (joaat(n) for n in curves)
    r = random.Random(87712)

    # the pow on its own
    pw = []
    for k in range(6000):
        m = k % 6
        if m == 0:
            x, y = 2.0, r.uniform(-30, 30)
        elif m == 1:
            x, y = 10 ** r.uniform(-6, 6), r.uniform(-20, 20)
        elif m == 2:
            x, y = float(r.randint(1, 40)), float(r.randint(1, 140))
        elif m == 3:
            x, y = r.uniform(0.9, 1.1), r.uniform(-50, 50)
        elif m == 4:
            x, y = 2.0, r.randint(-400, 80) / 16.0
        else:
            x, y = 2.0 ** r.uniform(-1000, 1000), r.choice((r.uniform(-3, 3), float(r.randint(-5, 5)), 0.5))
        pw += list(struct.unpack('>II', struct.pack('>d', x))) + list(struct.unpack('>II', struct.pack('>d', y)))
    # Gauss curves
    ga, gx = [], []
    for k in range(2000):
        kk = r.choice((3.0, r.uniform(0, 10), r.uniform(0, 1), 0.0, 25.0))
        flags = r.choice((0, 1, 2, 3)) | r.choice((0, 4, 8, 12)) | 0x200
        lo, hi = sorted((r.uniform(-0.2, 0.6), r.uniform(0.4, 1.2)))
        blob = bytes([9]) + bytes(4) + struct.pack('>I', flags) + bytes(4) + struct.pack('>ff', lo, hi) + struct.pack('>f', kk) + bytes(8)
        ga.append(blob)
        gx.append(fb(r.choice((0.0, 1.0, r.uniform(-0.5, 1.5), r.uniform(0, 1)))))
    # occlusion cases
    cases, geo = [], []

    def hits(l, e):
        out = [r.choice((0, 0, 1, 1, 2, 3, 6))]
        for _ in range(6):
            t = r.uniform(-0.1, 1.1)
            p = [l[i] + (e[i] - l[i]) * t + r.uniform(-2, 2) for i in range(3)]
            out += [fb(v) for v in p] + [r.randrange(8)]
        return out

    for c in range(150):
        own = c % 3 == 0
        tw = [struct.unpack_from('>h', tune, 38)[0] & 0xffff, struct.unpack_from('>I', tune, 40)[0], struct.unpack_from('>I', tune, 48)[0],
              struct.unpack_from('>H', tune, 52)[0], struct.unpack_from('>I', tune, 54)[0], struct.unpack_from('>I', tune, 62)[0]]
        if not own:
            tw = [r.choice((0, -600, -1200, 300, r.randint(-3000, 600))) & 0xffff, fb(r.choice((100.0, 50.0, 10.0, 1.0, r.uniform(1, 200)))),
                  fb(r.choice((1.0, 0.5, 2.0, 10.0))), r.choice((4000, 1000, 200, 12000)), fb(r.choice((20.0, 5.0, 40.0, 100.0))),
                  fb(r.choice((10.0, 1.0, 30.0)))]
        ops = [(0, 0)]
        last = 0.0
        for _ in range(19):
            op = r.choice((1, 1, 2, 2, 2, 2, 3, 4, 4, 0))
            if op == 1:
                last = r.choice((last, 0.0, r.uniform(0, 150), r.uniform(0, 5), 100.0, 20.0))
                ops.append((1, fb(last)))
            elif op == 2:
                ops.append((2, fb(r.choice((1 / 30, 1 / 60, 0.1, 0.5, 0.0, 2.0)))))
            elif op == 4:
                l = [r.uniform(-500, 500) for _ in range(3)]
                e = [l[i] + r.uniform(-80, 80) for i in range(3)]
                g = [int(r.random() < 0.2), int(r.random() < 0.4), int(r.random() < 0.3)]
                g += [fb(v) for v in e] + [fb(r.choice((0.0, 1.0)))]
                g += [fb(r.uniform(-100, 100)) for _ in range(4)]
                g += [fb(r.uniform(-1, 1)) for _ in range(12)] + [fb(v) for v in (r.uniform(-300, 300), r.uniform(-30, 30), r.uniform(-300, 300))] + [fb(1.0)]
                g += [fb(v) for v in l] + [fb(r.choice((0.0, 1.0)))]
                g += [r.choice((0, 1, 0x40, 0x1234))]
                g += hits(l, e) + hits(e, l)
                assert len(g) == GW
                ops.append((4, len(geo) // GW))
                geo += g
            else:
                ops.append((op, 0))
        cases.append((tw, ops))
    cdata = ','.join('%du' % v for tw, ops in cases for v in tw + [len(ops)] + [w for o in ops for w in o])
    G = lambda hi, lo: ((hi << 16) + lo) & 0xffffffff     # the globals as the code builds them (lis / addi)
    stubs = {
        'sub_' + lookup: 'ctx.r3.u32=@TUNE@u;',
        'sub_' + store_: '{uint32_t h=ctx.r4.u32,p=0;if(h==@HA@u)p=@CRV@u;else if(h==@HB@u)p=@CRV@u+0x100;else if(h>=0x1000u&&h<0x1000u+@NG@u)p=@GB@u+(h-0x1000u)*64;'
                         'store(ctx.r5.u32,p);ctx.r3.u32=p?1:0;}',
        'sub_' + lk0: '', 'sub_' + lk2: '', 'sub_' + lk1: 'ctx.r3.u32=1;',
        'sub_' + frame: 'ctx.r3.u32=@FRAME@u;',
        'sub_' + probe: '{uint32_t s=ctx.r3.u32,q=@IN@u+pc*0x400;printf(" Q");for(int k=0;k<8;k++)printf(" p%08x",load(s+4*k));'
                        'printf(" g%u n%u",ctx.r5.u32,ctx.r6.u32);uint32_t n=load(q);for(uint32_t k=0;k<n;k++){uint32_t t=ctx.r4.u32+96*k;'
                        'store(t+16,load(q+4+16*k));store(t+20,load(q+8+16*k));store(t+24,load(q+12+16*k));store(t+72,load(q+16+16*k));}'
                        'ctx.r3.u32=n;pc++;}',
    }
    fakes = {FM: 'ctx.r3.u32=@MOBJ@u+(ctx.r4.u32&7)*256;'}
    c = r"""
static const uint32_t PW[]={@PW@};
static const uint32_t GX[]={@GX@};
static const uint8_t GA[]={@GA@};
static const uint32_t CD[]={@CD@};
static const uint32_t GEO[]={@GEO@};
static const uint8_t TB[]={@TB@};
static const uint8_t CA[]={@CA@};
static const uint8_t CB[]={@CB@};
static const uint32_t FAC[]={@FAC@};
static double dbl(uint32_t h,uint32_t l){uint64_t b=((uint64_t)h<<32)|l;double d;memcpy(&d,&b,8);return d;}
static uint64_t dbits(double d){uint64_t b;memcpy(&b,&d,8);return b;}
static void grp(){printf(" a%08x v%08x h%08x l%08x w%08x z%08x",load(@OWN@u+52),load(@OWN@u+56),load(@OWN@u+60),load(@OWN@u+64),load(@OWN@u+68),load(@OWN@u+72));}
static void outs(){printf(" o%08x o%08x",load(@OUTV@u),load(@OUTV@u+4));}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@GV1@u,0);store(@GV1@u+4,0);store(@GV1@u+8,0);store(@GV1@u+12,0x3f800000u);
store(@GVM@u,0xffffffffu);store(@GVM@u+4,0xffffffffu);store(@GVM@u+8,0xffffffffu);store(@GVM@u+12,0);
memcpy(pointer(@CRV@u),CA,sizeof(CA));memcpy(pointer(@CRV@u+0x100),CB,sizeof(CB));memcpy(pointer(@GB@u),GA,sizeof(GA));
store(@GMAT@u,@MMGR@u);store(@MMGR@u,@MVT@u);store(@MVT@u+20,@FM@u);
for(int k=0;k<8;k++){store(@MOBJ@u+k*256+60,@MPTR@u+k*16);store(@MPTR@u+k*16,@MREC@u+k*512);store(@MREC@u+k*512+226,FAC[k]);}
printf("F");for(int k=0;k<8;k++)printf(" f%08x",FAC[k]);printf("\n");
for(size_t i=0;i<sizeof(PW)/4;i+=4){Context ctx{};ctx.r1.u32=@STACK@u;ctx.f1.f64=dbl(PW[i],PW[i+1]);ctx.f2.f64=dbl(PW[i+2],PW[i+3]);
 sub_@POW@(ctx,nullptr);printf("P x%08x%08x y%08x%08x r%016llx\n",PW[i],PW[i+1],PW[i+2],PW[i+3],(unsigned long long)dbits(ctx.f1.f64));}
for(size_t i=0;i<sizeof(GX)/4;i++){memset(pointer(@COBJ@u),0,64);Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@COBJ@u;ctx.r4.u32=0x1000u+(uint32_t)i;
 sub_@CINIT@(ctx,nullptr);ctx.r3.u32=@COBJ@u;ctx.f1.f64=(double)fbits2(GX[i]);sub_@CEVAL@(ctx,nullptr);const uint8_t*b=GA+64*i;
 printf("G k%02x%02x%02x%02x f%02x%02x%02x%02x n%02x%02x%02x%02x m%02x%02x%02x%02x x%08x r%08x\n",b[21],b[22],b[23],b[24],b[5],b[6],b[7],b[8],
  b[13],b[14],b[15],b[16],b[17],b[18],b[19],b[20],GX[i],fbits((float)ctx.f1.f64));}
size_t p=0;while(p<sizeof(CD)/4){const uint32_t*tw=CD+p;uint32_t n=CD[p+6];const uint32_t*ops=CD+p+7;p+=7+2*n;
 memcpy(pointer(@TUNE@u),TB,sizeof(TB));store16(@TUNE@u+38,tw[0]);store(@TUNE@u+40,tw[1]);store(@TUNE@u+48,tw[2]);store16(@TUNE@u+52,tw[3]);
 store(@TUNE@u+54,tw[4]);store(@TUNE@u+62,tw[5]);memset(pointer(@OWN@u),0,256);
 Context ctx{};ctx.r1.u32=@STACK@u;sub_@READER@(ctx,nullptr);
 printf("T m%u a%08x b%08x c%u d%08x e%08x",tw[0],tw[1],tw[2],tw[3],tw[4],tw[5]);
 for(int k=0;k<6;k++)printf(" s%08x",load(@GSET@u+4*k));printf("\n");
 for(uint32_t j=0;j<n;j++){uint32_t op=ops[2*j],arg=ops[2*j+1];memset(pointer(@OUTV@u),0,8);Context c2{};c2.r1.u32=@STACK@u;
  if(op==0){c2.r3.u32=@OWN@u+20;c2.r4.u32=@OUTV@u;sub_@INIT@(c2,nullptr);printf("I");grp();outs();}
  else if(op==1){c2.r3.u32=@OWN@u+20;c2.f1.f64=(double)fbits2(arg);sub_@SET@(c2,nullptr);printf("S x%08x",arg);grp();}
  else if(op==2){c2.r3.u32=@OWN@u+20;c2.f1.f64=(double)fbits2(arg);c2.r5.u32=@OUTV@u;sub_@SMOOTH@(c2,nullptr);printf("M t%08x",arg);grp();outs();}
  else if(op==3){c2.r3.u32=@OWN@u+20;c2.r4.u32=@OUTV@u;sub_@LATCH@(c2,nullptr);printf("L");grp();outs();}
  else{const uint32_t*g=GEO+@GW@*arg;printf("U");for(int k=0;k<@GW@;k++)printf(" i%08x",g[k]);
   *pointer(@OWN@u+116)=(uint8_t)g[0];*pointer(@OWN@u+117)=(uint8_t)g[1];*pointer(@OWN@u+118)=(uint8_t)g[2];
   for(int k=0;k<4;k++){store(@OWN@u+80+4*k,g[3+k]);store(@OWN@u+96+4*k,g[7+k]);}
   for(int k=0;k<16;k++)store(@FRAME@u+16+4*k,g[11+k]);for(int k=0;k<4;k++)store(@GLIS@u+48+4*k,g[27+k]);store(@OWN@u+76,g[31]);
   for(int s=0;s<2;s++){const uint32_t*h=g+32+25*s;store(@IN@u+s*0x400,h[0]);for(int k=0;k<24;k++)store(@IN@u+s*0x400+4+4*k,h[1+k]);}
   pc=0;c2.r3.u32=@OWN@u;sub_@UPDATE@(c2,nullptr);
   printf(" W%08x W%08x W%08x W%08x b%u",load(@OWN@u+96),load(@OWN@u+100),load(@OWN@u+104),load(@OWN@u+108),(unsigned)*pointer(@OWN@u+118));grp();}
  printf("\n");}}}
"""
    recomp_closure.EXTRA += ('static uint32_t fbits(float f){uint32_t b;memcpy(&b,&f,4);return b;}' + chr(10) +
                             'static float fbits2(uint32_t b){float f;memcpy(&f,&b,4);return f;}' + chr(10))
    ga_bytes = b''.join(g.ljust(64, b'\0') for g in ga)
    glob = (('GV1', G(-32122, 2352)), ('GVM', G(-32122, 2384)), ('GMAT', G(-32114, -23464)), ('GSET', G(-32121, -11976)),
            ('GLIS', G(-32121, -12352)))
    for k, v in glob + (('PW', ','.join('%du' % w for w in pw)), ('GX', ','.join('%du' % w for w in gx)), ('GA', ','.join(str(b) for b in ga_bytes)),
                 ('CD', cdata), ('GEO', ','.join('%du' % w for w in geo) or '0'), ('TB', ','.join(str(b) for b in tune)),
                 ('CA', ','.join(str(b) for b in curves['OCCLUSION_FACTOR_TO_LINEAR_VOLUME'])), ('CB', ','.join(str(b) for b in curves['OCCLUSION_DEPTH_TO_FILTER'])),
                 ('FAC', ','.join('%du' % fb(f) for f in FACTORS)), ('NG', len(ga)), ('GW', GW), ('HA', ha), ('HB', hb),
                 ('TUNE', TUNE), ('CRV', CRV), ('GB', GB), ('OWN', OWN), ('FRAME', FRAME), ('OUTV', OUTV), ('COBJ', COBJ), ('IN', IN),
                 ('MMGR', MMGR), ('MVT', MVT), ('MOBJ', MOBJ), ('MPTR', MPTR), ('MREC', MREC), ('FM', FM), ('STACK', STACK),
                 ('READER', reader), ('INIT', init), ('SET', setf), ('SMOOTH', smooth), ('LATCH', latch), ('UPDATE', update),
                 ('POW', pow_), ('CINIT', cinit), ('CEVAL', ceval)):
        c = c.replace('@%s@' % k, str(v))
        stubs = {n: b.replace('@%s@' % k, str(v)) for n, b in stubs.items()}
        fakes = {n: b.replace('@%s@' % k, str(v)) for n, b in fakes.items()}
    roots = ['sub_' + f for f in (reader, init, setf, smooth, latch, update, pow_, cinit, ceval)]
    exe = rd.build(idx, roots, c, a.out_dir, 'occlusion', stubs=stubs, image=img, fakes=fakes)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    cl = 'C h%08x c%s h%08x c%s' % (ha, curves['OCCLUSION_FACTOR_TO_LINEAR_VOLUME'].hex(), hb, curves['OCCLUSION_DEPTH_TO_FILTER'].hex())
    out = cl + chr(10) + out
    (a.out_dir / 'occlusion_ref.txt').write_text(out)
    ls = out.split(chr(10))
    print(sum(l.startswith('P') for l in ls), 'pows,', sum(l.startswith('G') for l in ls), 'curves,', sum(l.startswith('T') for l in ls), 'tunes,',
          sum(l[:1] in 'ISML' for l in ls if l), 'group ops,', sum(l.startswith('U') for l in ls), 'updates')


if __name__ == '__main__':
    main()
