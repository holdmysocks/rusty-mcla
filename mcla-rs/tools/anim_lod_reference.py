"""The walkers' anim LOD (F-62015) run offline: the face and hands dropped past 30 m and back at 20 m. The unchanged generated
code on flat guest memory; the matrix getter F-33765 a stub giving the case's matrix, the bone filter's remove (F-16979) and
add (F-25694) logged with their arguments (both checked to the bit elsewhere). Inputs: the walker's state byte +4896, its +500
byte (bit 0x20 takes the matrix through +52 -> +48 -> +28 + 64 when both are set, else the getter's + 48), the two matrices'
positions and the focus (the global camera block + 0x37320), placed around 20 m and 30 m (one ulp either side) and at random.
The two limits' guard word is cleared first, so the routine loads them from the image as on its first call.
Writes <out-dir>/anim_lod_ref.txt, one case per line: the inputs (ints, then floats as hex bits) "->" the calls (D = remove,
E = add, one letter per bone, - = none) and the state byte after."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('static char calls[16];static int ncalls;static const uint32_t*cur;\n'
                         'static const uint16_t IDS[3]={0x988B,0x905C,0x9002};\n')
_shim = recomp_closure.shim


def shim_low(scratch_end=0x3000):
    # (a block for the camera's at 0x81000000 .. 0x81A00000, below the image)
    return _shim(scratch_end).replace(
        'static uint8_t* pointer(uint32_t a) {',
        'static std::vector<uint8_t> LOW(0xA00000);\nstatic uint8_t* pointer(uint32_t a) {\n'
        '    if (a>=0x81000000u && a<0x81A00000u) return LOW.data()+(a-0x81000000u);', 1)


recomp_closure.shim = shim_low


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u))[0]


def near(r, rnd):
    """a distance near r: r itself, an ulp or two either side of r (as squared f32 distances land), or a few cm off."""
    k = rnd.random()
    if k < 0.5:
        return bf(fb(r) + rnd.choice((-2, -1, 0, 1, 2)))
    if k < 0.8:
        return r + rnd.uniform(-0.05, 0.05)
    return r * rnd.uniform(0.5, 1.6)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, getter, rem, add = (addr(a.tool, f) for f in ('F-62015', 'F-33765', 'F-16979', 'F-25694'))
    rnd = random.Random(62)
    cases = []
    NI = 4  # (ints: state byte, +500, +52 set, +52 -> +48 set)
    for _ in range(4000):
        st = rnd.choice((0, 0, 1, 1, 2, 0xFF))
        b500 = rnd.choice((0, 0x20, 0x20, 0x10, 0xDF, 0xFF, rnd.randrange(256)))
        p52, p48 = int(rnd.random() < 0.8), int(rnd.random() < 0.8)
        focus = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 120), rnd.uniform(-3000, 3000)]
        if rnd.random() < 0.05:
            focus = [rnd.choice((0.0, -0.0, 1e-40, float('nan'), float('inf'))) for _ in range(3)]
        mats = []
        for _m in range(2):
            r = near(rnd.choice((20.0, 30.0)), rnd)
            d = [rnd.gauss(0, 1) for _ in range(3)]
            n = sum(x * x for x in d) ** 0.5
            mats += [f + x / n * r for f, x in zip(focus, d)]
        cases.append([st, b500, p52, p48] + focus + mats)
    nl = chr(10)
    NL = chr(92) + 'n'
    NF = len(cases[0])
    rows = nl.join('{%s},' % ','.join('0x%08x' % (x if i < NI else fb(x)) for i, x in enumerate(c)) for c in cases)
    # walker 0x1000: +500 the byte, +52 0x2400 (+48 0x2480, its +28 0x2500: the matrix 0x2540), +4892 the filter 0x2700,
    # +4896 the state; the getter's block 0x2600 (matrix 0x2630); the camera block's pointer 0x8286D8E4 -> 0x81000000 (a block of its own)
    setup = ('memset(objects,0,0x2000);'
             '*pointer(0x1000+500)=c[1];store(0x1000+52,c[2]?0x2400:0);store(0x2400+48,c[3]?0x2480:0);store(0x2480+28,0x2500);'
             'store(0x1000+4892,0x2700);*pointer(0x1000+4896)=c[0];'
             'store(0x8286D8E4,0x81000000);store(0x81037320,c[4]);store(0x81037324,c[5]);store(0x81037328,c[6]);'
             'store(0x2540,c[7]);store(0x2544,c[8]);store(0x2548,c[9]);store(0x2630,c[10]);store(0x2634,c[11]);store(0x2638,c[12]);')
    body = ('static const uint32_t C[][%d]={' % NF + nl + rows + nl + '};' + nl +
            'for(auto&c:C){cur=c;ncalls=0;' + setup + 'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2F00;'
            'sub_%s(ctx,nullptr);' % fn +
            'if(!ncalls)printf("-");for(int k=0;k<ncalls;k++)printf("%c",calls[k]);' +
            ('printf(" %%d%s",*pointer(0x1000+4896));}' % NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});store(0x82909F78,0);' + nl + body + '}' + nl)
    ok = 'ctx.r3.u32==0x2700&&ctx.r5.u32==1&&ncalls<3&&ctx.r4.u32==IDS[ncalls]'
    stubs = {'sub_' + getter: 'ctx.r3.u64=0x2600;',
             'sub_' + rem: 'calls[ncalls]=(%s)?\'D\':\'d\';ncalls++;' % ok,
             'sub_' + add: 'calls[ncalls]=(%s&&ctx.f1.f64==1.0)?\'E\':\'e\';ncalls++;' % ok}
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], main_src, a.out_dir, 'anim_lod', stubs=stubs, vector=False, scratch_end=0x3000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join(('%d' % x) if i < NI else '%08x' % fb(x) for i, x in enumerate(c)), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'anim_lod_ref.txt').write_text(nl.join(lines) + nl)
    tally = {}
    for o in out:
        tally[o] = tally.get(o, 0) + 1
    print(len(lines), 'cases', tally)


if __name__ == '__main__':
    main()
