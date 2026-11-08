"""The sink's search for the kept contact a new one replaces, F-74262, run offline (generated code unchanged, on flat guest
memory): r3 the manifold (+1260 the count, contact i at +192 + 176 i with side A's point at +16 and its age, u16, at
+174), r4 the new contact (its point at +16); r3 out (the index or -1). The reach 4e-4 is read from the image.

Cases: 0 .. 6 kept contacts round the new one at distances about the reach (some exactly on it, some equal pairs, ages
0 and not), near the origin and 3 km out.

Writes <out-dir>/nearest_kept_ref.txt, one case a line: "n x y z | x y z age .. -> i" (points hex float bits)."""
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


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def cases():
    rnd = random.Random(83)
    out = []
    for i in range(20000):
        far = rnd.choice((0.0, 2.0, 3000.0))
        p = [f32(rnd.uniform(-far, far)) for _ in range(3)]
        n = rnd.choice((0, 1, 2, 3, 4, 5, 6, 6, 6))
        kept = []
        for _ in range(n):
            if kept and rnd.random() < 0.1:
                q = list(kept[rnd.randrange(len(kept))][0])
            else:
                r = rnd.choice((0.02, 0.02 * (1 + 1e-7), 0.02 * (1 - 1e-7), rnd.uniform(0, 0.03), rnd.uniform(0, 0.5)))
                d = [rnd.gauss(0, 1) for _ in range(3)]
                ln = sum(x * x for x in d) ** 0.5 or 1.0
                q = [f32(p[k] + d[k] / ln * r) for k in range(3)]
                if rnd.random() < 0.05:
                    q[rnd.randrange(3)] = f32(p[0] + 0.02)
            kept.append((q, rnd.choice((0, 1, 1, 2, 7, 65535))))
        out.append((p, kept))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-74262')
    cs = cases()
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = []
    for p, kept in cs:
        flat += [len(kept)] + [fb(x) for x in p]
        for q, age in kept:
            flat += [fb(x) for x in q] + [age]
    body = ('static const uint32_t C[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            'size_t at=0;while(at<sizeof(C)/4){uint32_t n=C[at++];memset(objects,0,sizeof(objects));' +
            'for(int k=0;k<3;k++)store(0x2000+16+4*k,C[at+k]);at+=3;store(0x1000+1260,n);' +
            'for(uint32_t i=0;i<n;i++){uint32_t c=0x1000+192+176*i;for(int k=0;k<3;k++)store(c+16+4*k,C[at+k]);' +
            'store(c+172,C[at+3]);at+=4;}' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x2000;sub_%s(ctx,nullptr);' % fn) +
            'printf("%d' + NL + '",int(ctx.r3.u32));}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'nearest_kept', scratch_end=0x3000)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = []
    for (p, kept), o in zip(cs, out):
        lines.append('%d %s | %s -> %s' % (len(kept), ' '.join('%08x' % fb(x) for x in p),
                                           ' '.join('%s %d' % (' '.join('%08x' % fb(x) for x in q), age) for q, age in kept), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'nearest_kept_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o != '-1' for o in out), 'found')


if __name__ == '__main__':
    main()
