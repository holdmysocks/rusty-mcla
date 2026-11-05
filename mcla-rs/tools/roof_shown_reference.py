"""The convertible roof's three draw tests run offline: the original's F-36938 (the raised roof: top time < 0.01),
F-80163 (the folded roof: time > the skinned_top clip's length - 0.01) and F-91671 (the skinned roof: either player
between its ends), the generated code unchanged (all three are leaves), on random and edge times.
The car-parts object at 0x1000: the top's time +640 and length +660, the hatch's +544 / +564; D+84 the skinned_top
holder (0x2000), its +76 the clip (0x2100), the clip's +12 its length (the top's length, as the binder copies it).
Writes <out-dir>/roof_shown_ref.txt, one case a line (f32 bits, hex): top_time top_len hatch_time hatch_len | up down moving
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def nudge(x, k):
    b = fbits(x)
    if x == 0.0:
        return struct.unpack('>f', struct.pack('>I', 1 if k > 0 else 0x80000001))[0]
    return struct.unpack('>f', struct.pack('>I', b + k if x > 0 else b - k))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    up, down, moving = (addr(a.tool, f) for f in ['F-36938', 'F-80163', 'F-91671'])
    rnd = random.Random(91671)
    cases = []
    for _ in range(4000):
        lt = rnd.choice([5.0, 7.3333335, 1.0, rnd.uniform(0.05, 9.0)])
        lh = rnd.choice([lt, 5.0, 7.3333335, rnd.uniform(0.05, 9.0)])

        def pick(l):
            e = rnd.choice([0.0, -0.0, 0.01, l, f32(l - 0.01), rnd.uniform(0, l), rnd.uniform(-1, l + 1)])
            return nudge(e, rnd.choice([-1, 0, 0, 1])) if rnd.random() < 0.5 else e
        tt, th = pick(lt), pick(lh)
        cases.append([fbits(f32(x)) for x in (tt, lt, th, lh)])
    n = len(cases)
    nl = chr(10)
    main_src = ('static const uint32_t CS[][4]={' + ','.join('{0x%08x,0x%08x,0x%08x,0x%08x}' % tuple(c) for c in cases) + '};' + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
                ('for(size_t i=0;i<%d;i++){const uint32_t*v=CS[i];memset(objects,0,sizeof(objects));store(0x1000+640,v[0]);store(0x1000+660,v[1]);'
                 'store(0x1000+544,v[2]);store(0x1000+564,v[3]);store(0x1000+84,0x2000);store(0x2000+76,0x2100);store(0x2100+12,v[1]);int r[3];'
                 'Context a{};a.r3.u32=0x1000;a.r1.u32=0x2800;sub_%s(a,nullptr);r[0]=a.r3.u32&0xff;'
                 'Context b{};b.r3.u32=0x1000;b.r1.u32=0x2800;sub_%s(b,nullptr);r[1]=b.r3.u32&0xff;'
                 'Context c{};c.r3.u32=0x1000;c.r1.u32=0x2800;sub_%s(c,nullptr);r[2]=c.r3.u32&0xff;'
                 'printf("%%08x %%08x %%08x %%08x | %%d %%d %%d\\n",v[0],v[1],v[2],v[3],r[0],r[1],r[2]);}}' % (n, up, down, moving)) + nl)
    exe = build(idx, [f'sub_{x}' for x in (up, down, moving)], main_src, a.out, 'roof_shown')
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'roof_shown_ref.txt').write_text(out)
    print(n, 'cases;', sum(1 for l in out.splitlines() if l.endswith('0 0 1')), 'moving only')


if __name__ == '__main__':
    main()
