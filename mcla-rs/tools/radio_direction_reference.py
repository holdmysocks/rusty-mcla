"""The police radio's DIRECTION special case (F-95031) run offline: unchanged generated code on flat guest memory. r3 = the
radio context (+208..+216 the suspect's heading, +252 the road under him or 0, whose +96..+104 is the road's direction),
r4 the output, r5 the scenario's name. The axes it dots with are the image's (0, 0, 1) and (1, 0, 0) globals. The
formatter F-32585 (sprintf into r3) is stubbed to print the format string's address, which names the word.
Writes <out-dir>/radio_direction_ref.txt: "hx hy hz road rx ry rz -> WORD" (floats as hex bits, road 0 / 1)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

WORDS = {0x820C4F40: 'EAST', 0x820C4F48: 'WEST', 0x820C4F50: 'SOUTH', 0x820C4F5C: 'NORTH'}


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
    ap.add_argument('--fmt', required=True, help='the formatter it calls (registry id)')
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ad, fmt = addr(a.tool, a.fid), addr(a.tool, a.fmt)
    rnd = random.Random(17)
    k = struct.unpack('>f', struct.pack('>f', 0.70710677))[0]
    edge = [0.0, -0.0, k, -k, struct.unpack('>f', struct.pack('>I', fb(k) + 1))[0], struct.unpack('>f', struct.pack('>I', fb(k) - 1))[0],
            1.0, -1.0, 1e-30, -1e-30]
    cases = []
    for _ in range(4000):
        def vec():
            if rnd.random() < 0.3:
                return [rnd.choice(edge) * rnd.choice((1, -1)) for _ in range(3)]
            t = rnd.uniform(0, 6.283185)
            v = [float(f'{__import__("math").sin(t):.7g}'), rnd.choice((0.0, 0.0, rnd.uniform(-0.3, 0.3))), float(f'{__import__("math").cos(t):.7g}')]
            return [struct.unpack('>f', struct.pack('>f', x))[0] for x in v]
        cases.append((vec(), rnd.random() < 0.5, vec()))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in [fb(c) for c in h] + [int(r)] + [fb(c) for c in d]) for h, r, d in cases)
    body = ('static const uint32_t C[][7]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));for(int k=0;k<3;k++){store(0x1000+208+4*k,c[k]);store(0x1400+96+4*k,c[4+k]);}' +
            'store(0x1000+252,c[3]?0x1400:0);' +
            ('Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r5.u32=0x1900;ctx.r1.u32=0x2F00;sub_%s(ctx,nullptr);}' % ad) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.out_dir, 'radio_direction', stubs={'sub_' + fmt: 'printf("%%08x%s",ctx.r5.u32);' % NL})
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s %d %s -> %s' % (' '.join('%08x' % fb(c) for c in h), r, ' '.join('%08x' % fb(c) for c in d), WORDS[int(o, 16)])
             for (h, r, d), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'radio_direction_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
