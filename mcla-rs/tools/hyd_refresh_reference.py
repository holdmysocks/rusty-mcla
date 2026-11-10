"""The hydraulics' wheel weights at the car's refresh and the drag numbers' square, run offline: the generated code
unchanged (vector mode, the recomp's own -msse4.1 -mssse3) on flat guest memory. Each case line of
<out-dir>/hyd_refresh_ref.txt is "<tag> <input words> -> <output words>" (hex words; a word that reads 82xxxxxx is
written with an h in front). Sections:

  hydinit  F-60656 with the wheels' own slot 1 real (F-73903: the wheel's set-up, its weighing F-11389 and the setter
           F-51861, the C library pow F-21955 inside it), then F-17224 and F-51861 real; only the bound-slice builder
           F-36757 is a stub (it writes the slice records, nothing compared). Out: each wheel's +428 and +432, the
           number of slot-1 calls.
  drag     F-31519 with the C library pow real, on rounding ties of the square (the exact square half way between two
           floats), on every HighGearMPH of the disc's vehicle tunes (--mph, one hex word a line) and on random speeds.

Run: PYTHONPATH=mcla-rs/tools python mcla-rs/tools/hyd_refresh_reference.py --recomp D:/MCLA-static
--image <original-image.bin> --out-dir <dir> --tool <mcla-tool.exe> [--mph <file>] (every harness run under the
memcap job of recomp_closure)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure
from cite_safe import cite_safe

_vector_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda scratch_end=0x3000: '#include <simde/x86/avx2.h>' + chr(10) + _vector_shim(scratch_end).replace('int32_t s32;', 'int32_t s32; int16_t s16; int8_t s8;', 1)
recomp_closure.EXTRA += ('namespace rex { namespace ppc { static simde__m128 simde_mm_cvtepu32_ps_(simde__m128i v) { alignas(16) uint32_t a[4]; alignas(16) float f[4];' +
                         ' simde_mm_store_si128((simde__m128i*)a, v); for (int i = 0; i < 4; i++) f[i] = float(a[i]); return simde_mm_load_ps(f); } } }' + chr(10) +
                         '#define ctr r2' + chr(10) + 'static void dispatch(Context&,const uint8_t*,uint32_t);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10) +
                         'static uint32_t ncalls;' + chr(10) +
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10))

NL = chr(92) + 'n'
nl = chr(10)
# the wheel tune words the set-up and the setter read
TUNE = (4, 8, 12, 16, 24, 28, 56, 60, 64, 76, 80, 84, 88, 96, 100, 112, 116, 120)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(w):
    return struct.unpack('>f', struct.pack('>I', w))[0]


def word(rnd, scale=10.0):
    r = rnd.random()
    if r < 0.1:
        w = rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000))
    elif r < 0.9:
        w = fb(rnd.uniform(-scale, scale))
    else:
        w = rnd.getrandbits(32)
    return 0 if (w >> 23) & 0xFF == 0 and w & 0x7FFFFF else w


def tie(mph):
    """True where the exact square of the top speed (mph x 0.447 in single) lies half way between two floats."""
    top = bf(fb(bf(mph) * bf(0x3ee4dd2f)))
    sq = top * top
    near = bf(fb(sq)) if abs(sq) < 3.4e38 else 0.0
    if near == 0.0 or near != near:
        return False
    up, dn = bf(fb(near) + 1), bf(fb(near) - 1)
    return abs(near - sq) * 2 == abs(up - near) or abs(near - sq) * 2 == abs(near - dn)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--mph', type=Path)
    a = ap.parse_args()
    f = {k: addr(a.tool, v) for k, v in dict(hyd='F-60656', init='F-73903', slices='F-36757', drag='F-31519').items()}
    rnd = random.Random(60656)
    secs = []

    # hydinit: fitted byte, on, pattern, stiffness, tune mass, wheel count, gravity (+124), body mass (+192), the centre of
    # mass (4 words, chassis +112), chassis +104; per wheel x (+384), z (+392), +428 and +432 before; the front and back
    # wheel tunes (the words of TUNE)
    cs = []
    for _ in range(3000):
        c = [rnd.choice((0, 1, 1, 1, 2)), rnd.choice((0, 1, 1, 1, 2)), rnd.choice((0, 0, 0, 1, 3)),
             rnd.choice((fb(rnd.uniform(0, 14)), fb(rnd.uniform(0.5, 1.5)), 0x3f800000, 0x41200000, 0x41300000, 0x7fc00000, word(rnd, 20.0))),
             rnd.choice((fb(rnd.uniform(500, 3000)), fb(rnd.uniform(500, 3000)), word(rnd, 3000.0))), rnd.choice((4, 4, 4, 4, 2, 0, 1, 3)),
             rnd.choice((fb(1.4), fb(rnd.uniform(0.5, 2.5)), word(rnd, 3.0))), rnd.choice((fb(rnd.uniform(500, 3000)), word(rnd, 3000.0)))]
        c += [rnd.choice((fb(rnd.uniform(-0.2, 0.2)), word(rnd, 0.5))) for _ in range(4)] + [word(rnd, 1.0)]
        for k in range(4):
            sx = -1 if k % 2 == 0 else 1
            sz = -1 if k < 2 else 1
            c += [rnd.choice((fb(sx * rnd.uniform(0.6, 1.0)), word(rnd, 1.5))), rnd.choice((fb(sz * rnd.uniform(1.0, 1.8)), fb(sz * rnd.uniform(1.0, 1.8)), word(rnd, 2.0))),
                  word(rnd, 8000.0), word(rnd, 0.3)]
        for _ in range(2):
            vals = dict(zip(TUNE, [fb(rnd.uniform(0.28, 0.4)), fb(rnd.uniform(0.15, 1.2)), fb(rnd.uniform(0.5, 2)), fb(rnd.uniform(0.5, 2)),
                                   fb(rnd.uniform(0.5, 2)), fb(rnd.uniform(0.5, 2)), rnd.choice((0, fb(rnd.uniform(0, 1)))), fb(rnd.uniform(0, 3)),
                                   fb(rnd.uniform(0, 3)), fb(rnd.uniform(0.05, 0.3)), rnd.choice((0, 0, fb(rnd.uniform(-0.1, 0.1)))), fb(rnd.uniform(0.05, 0.3)),
                                   fb(rnd.uniform(0.05, 0.3)), fb(rnd.uniform(0.5, 3)), fb(rnd.uniform(0.1, 1)), fb(rnd.uniform(0.05, 0.3)),
                                   fb(rnd.uniform(0.8, 2)), fb(rnd.uniform(0.8, 2))]))
            if rnd.random() < 0.1:
                vals[rnd.choice(TUNE[:6] + TUNE[9:15])] = word(rnd, 2.0)
            c += [vals[o] for o in TUNE]
        cs.append(c)
    tune_set = ''.join('store(0x4000+%d,c[%d]);store(0x4100+%d,c[%d]);' % (o, 29 + j, o, 29 + len(TUNE) + j) for j, o in enumerate(TUNE))
    setup = ('store8(0x1000+12,c[0]);store(0x1000+16,c[1]);store(0x1000+20,c[2]);store(0x1000+4,0x1100);store(0x1100+36,c[3]);' +
             'store(0x1000+8,0x1200);store(0x1200+8,0x1300);store(0x1300+368,0x2000);store(0x2000+112,c[4]);store(0x2000+124,c[6]);' +
             'store(0x1300+24,c[5]);store(0x1300+28,0x1500);store(0x1300+128,0x1600);store(0x1600+192,c[7]);' +
             'for(int k=0;k<4;k++)store(0x1300+112+4*k,c[8+k]);store(0x1300+104,c[12]);' +
             'for(int k=0;k<64;k++)store(0x2800+4*k,0x7F000000+k);' +
             'for(int k=0;k<4;k++){uint32_t w=0x3000+0x200*k;store(0x1500+4*k,w);store(w,0x2800);store(w+4,k<2?0x4000:0x4100);store(w+8,0x1300);' +
             'store(w+384,c[13+4*k]);store(w+392,c[14+4*k]);store(w+428,c[15+4*k]);store(w+432,c[16+4*k]);}' + tune_set + 'ctx.r3.u32=0x1000;')
    secs.append(('hydinit', cs, setup, f['hyd'], 9, '(i<8?load(0x3000+0x200*(i/2)+(i%2?432:428)):ncalls)'))

    # drag: high gear mph, DragMin, DragMax, Down - the ties, the disc's speeds, random speeds
    mphs = []
    while len(mphs) < 600:
        m = fb(rnd.uniform(20, 400)) if rnd.random() < 0.7 else fb(rnd.uniform(0, 1000))
        if tie(m):
            mphs.append(m)
    disc = [int(x, 16) for x in a.mph.read_text().split()] if a.mph else []
    mphs += disc
    mphs += [fb(rnd.uniform(60, 260)) for _ in range(1000)]
    cs = [[m, fb(rnd.uniform(0, 5)), fb(rnd.uniform(0, 5)), fb(rnd.uniform(0, 5))] for m in mphs]
    secs.append(('drag', cs, 'store(0x1000+12,0x1100);store(0x1100+368,0x1200);store(0x1200+148,0x1300);store(0x1300+12,c[0]);store(0x1000+8,0x1400);' +
                 'store(0x1400+88,c[1]);store(0x1400+92,c[2]);store(0x1400+68,c[3]);ctx.r3.u32=0x1000;', f['drag'], 3, 'load(0x1000+(i==0?24:i==1?28:32))'))

    stubs = {'sub_' + f['slices']: ''}
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){' +
            'case 0x7F000001:ncalls++;sub_%s(ctx,base);return;' % f['init'] +
            'default:throw std::runtime_error("unexpected indirect call");}}')
    body = ''
    for tag, cs, setup, fn, nout, outc in secs:
        w = max(len(c) for c in cs)
        rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c + [0] * (w - len(c))) for c in cs)
        body += ('{static const uint32_t C[][%d]={%s%s%s};' % (w, nl, rows, nl) +
                 'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x7F00;ncalls=0;' + setup +
                 'sub_%s(ctx,nullptr);' % fn +
                 'for(int i=0;i<%d;i++)printf("%%08x ",uint32_t(%s));printf("%s");fflush(stdout);}}' % (nout, outc, NL) + nl)
    names = {'sub_' + s[3] for s in secs} | {'sub_' + f['init']}
    main_src = ('static void store8(uint32_t a,uint8_t v){*pointer(a)=v;}' + nl + disp + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), sorted(names), main_src, a.out_dir, 'hyd_refresh', stubs=stubs, vector=True, scratch_end=0x8000, flags=('-msse4.1', '-mssse3'))
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == sum(len(s[1]) for s in secs), (len(out), sum(len(s[1]) for s in secs))
    lines, k = [], 0
    for tag, cs, *_ in secs:
        for c in cs:
            lines.append('%s %s -> %s' % (tag, ' '.join('%08x' % x for x in c), out[k].strip()))
            k += 1
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'hyd_refresh_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print({s[0]: len(s[1]) for s in secs}, 'disc speeds', len(disc))


if __name__ == '__main__':
    main()
