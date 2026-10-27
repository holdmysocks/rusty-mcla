"""The vector length clamp (F-38357) run offline: unchanged generated code on flat guest memory. In place on the vec4 at r3,
f1 = minimum length, f2 = maximum length (the deflection pass calls it with 0 and 25). The static image holds zero where the
game's start-up wrote the unit vector at its global (0, 1, 0, 0); main stores that first (read from the running original with
the object-dump recorder, cap u1).
Writes <out-dir>/clamp_length_ref.txt: "x y z w min max -> x y z w" as hex float bits."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--fid', required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ad = addr(a.tool, a.fid)
    rnd = random.Random(11)
    limits = [(0.0, 25.0), (0.0, 1.0), (1.0, 5.0), (0.5, 0.5), (0.0, 0.0), (2.0, 100.0), (0.0, 1e-3)]
    cases = []
    for _ in range(3000):
        lo, hi = rnd.choice(limits)
        scale = 10 ** rnd.uniform(-7, 3)
        v = [rnd.uniform(-1, 1) * scale for _ in range(3)] + [rnd.choice((0.0, 0.0, rnd.uniform(-2, 2)))]
        if rnd.random() < 0.15:
            v[rnd.randrange(3)] = 0.0
        cases.append((v, lo, hi))
    for lo, hi in limits:
        for v in ([0, 0, 0, 0], [-0.0, 0, -0.0, 0], [1e-7, 0, 0, 0], [5e-7, 5e-7, 5e-7, 0], [float('nan'), 0, 0, 0], [float('inf'), 0, 0, 0],
                  [1e30, 1e30, 0, 0], [3, 4, 0, 1], [25, 0, 0, 0], [0, 25.000002, 0, 0], [0, 0, hi, 0], [0, 0, lo, 0], [0, 0, 1e-6, 5]):
            cases.append((v, lo, hi))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in [fb(c) for c in v] + [fb(lo), fb(hi)]) for v, lo, hi in cases)
    body = ('static const uint32_t C[][6]={' + nl + rows + nl + '};' + nl +
            'store(0x828608F0,0);store(0x828608F4,0x3f800000);store(0x828608F8,0);store(0x828608FC,0);' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));for(int k=0;k<4;k++)store(0x1000+4*k,c[k]);' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x1F00;{float a,b;uint32_t u=c[4];memcpy(&a,&u,4);u=c[5];memcpy(&b,&u,4);ctx.f1.f64=a;ctx.f2.f64=b;}' +
            ('sub_%s(ctx,nullptr);printf("%%08x %%08x %%08x %%08x%s",load(0x1000),load(0x1004),load(0x1008),load(0x100c));}' % (ad, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.out_dir, 'clamp_length', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % fb(c) for c in v + [lo, hi]), o) for (v, lo, hi), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'clamp_length_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
