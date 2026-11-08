"""Which of a hull's triangles a contact is on, F-48088, run offline with what it calls (the leading-corner count F-61834,
the edge neighbour F-13453, the walk round a corner F-58013, the vertex fetch) - generated code unchanged, vector mode,
the recomp's own -msse4.1 -mssse3. r3 the polyhedron bound (+128 its vertices, 16 bytes each; +140 its polygon records,
16 bytes: area, three u16 corners, three u16 neighbours; +204 the polygon count; +220 the material count, u8), r4 the
corners (u32 each), r5 how many, v1 the direction; r3 out.

Hulls: every vp_* car's (.r83) as the file has it, and each a second time with its corners moved a little (the areas
then as they were: the routine only reads them). Cases per hull: 0 .. 5 corners (repeats allowed, one past the
vertices now and then), the direction random or along a triangle's normal (equal faces), the material count as filed
and 1 / 2.

Writes <out-dir>/hull_element_ref.txt: a hull line "H nv nt mc | vertex words | polygon words" and its case lines
"C mc n c0 .. | dx dy dz -> t" (hex)."""
import argparse
import glob
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure

_vector_shim = recomp_closure.vector_shim


def _shim():
    s = _vector_shim()
    old = 'struct Condition {\n    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    return s.replace(old, '#include <simde/x86/avx2.h>\nstruct Condition {\n    bool so{}, eq{}, lt{}, gt{};\n'
                     '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)


recomp_closure.vector_shim = _shim


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(w):
    return struct.unpack('>f', struct.pack('>I', w))[0]


def hulls(data):
    out = []
    for dr in sorted(glob.glob(str(data / 'resources/vehicle/vp*'))):
        for fn in sorted(glob.glob(dr + '/*.r83')):
            raw = Path(fn).read_bytes()
            d = raw[16:16 + struct.unpack_from('<I', raw, 12)[0]]
            u32 = lambda o: struct.unpack_from('>I', d, o)[0]
            for o in range(0, len(d) - 0xe0, 16):
                if d[o + 4] == 4 and struct.unpack_from('<I', d, o)[0] == 0x0059b464:
                    nv, nt = u32(o + 200), u32(o + 204)
                    v, t = u32(o + 128) & 0xfffffff, u32(o + 140) & 0xfffffff
                    out.append((nv, nt, d[o + 220], [u32(v + 4 * i) for i in range(4 * nv)], [u32(t + 4 * i) for i in range(4 * nt)]))
                    break
            break
    return out


def normal(vs, ps, t):
    w = ps[4 * t:4 * t + 4]
    ids = [w[1] >> 16, w[1] & 0xffff, w[2] >> 16]
    p = [[bf(vs[4 * i + k]) for k in range(3)] for i in ids]
    e1 = [p[2][k] - p[1][k] for k in range(3)]
    e2 = [p[0][k] - p[1][k] for k in range(3)]
    n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
    ln = math.sqrt(sum(x * x for x in n)) or 1.0
    return [x / ln for x in n], ids


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, default=Path('D:/MCLA/extracted/cache'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--per-hull', type=int, default=300)
    a = ap.parse_args()
    fe = addr(a.tool, 'F-48088')
    rnd = random.Random(48088)
    hs = []
    for nv, nt, mc, vs, ps in hulls(a.data):
        assert 4 <= nv <= 64 and 4 <= nt <= 128  # (the guest memory's layout)
        hs.append((nv, nt, mc, vs, ps))
        moved = [fb(bf(w) + rnd.uniform(-0.01, 0.01)) if i % 4 != 3 else w for i, w in enumerate(vs)]
        hs.append((nv, nt, mc, moved, ps))
    jobs = []
    for h, (nv, nt, mc, vs, ps) in enumerate(hs):
        for _ in range(a.per_hull):
            m = rnd.choice((mc, mc, mc, 1, 2))
            t = rnd.randrange(nt)
            nrm, ids = normal(vs, ps, t)
            k = rnd.choice((1, 1, 1, 2, 2, 2, 3, 4, 0, 5))
            if rnd.random() < 0.7:
                cs = [rnd.choice(ids) if rnd.random() < 0.9 else rnd.randrange(nv) for _ in range(k)]
            else:
                cs = [rnd.randrange(nv + 1) for _ in range(k)]
            if k == 2 and rnd.random() < 0.5:
                j = rnd.randrange(3)
                cs = [ids[j], ids[(j + 1) % 3]][::rnd.choice((1, -1))]
            if rnd.random() < 0.25:
                d = nrm
            else:
                d = [rnd.gauss(0, 1) for _ in range(3)]
                ln = math.sqrt(sum(x * x for x in d)) or 1.0
                d = [x / ln for x in d]
            jobs.append((h, m, cs, [fb(x) for x in d]))
    nl = chr(10)
    NL = chr(92) + 'n'
    hflat = []
    for nv, nt, mc, vs, ps in hs:
        hflat += [nv, nt] + vs + ps
    jflat = []
    for h, m, cs, d in jobs:
        jflat += [h, m, len(cs)] + cs + d
    body = ('static const uint32_t H[]={' + ','.join('0x%x' % x for x in hflat) + '};' + nl +
            'static const uint32_t J[]={' + ','.join('0x%x' % x for x in jflat) + '};' + nl +
            'static size_t hat[%d];' % len(hs) + nl +
            'size_t at=0;for(int h=0;h<%d;h++){hat[h]=at;at+=2+4*H[at]+4*H[at+1];}' % len(hs) + nl +
            'at=0;while(at<sizeof(J)/4){uint32_t h=J[at++],m=J[at++],n=J[at++];memset(objects,0,sizeof(objects));' +
            'size_t p=hat[h];uint32_t nv=H[p],nt=H[p+1];' +
            'store(0x1000+128,0x1100);store(0x1000+140,0x1500);store(0x1000+204,nt);store(0x1000+220,m<<24);' +
            'for(uint32_t i=0;i<4*nv;i++)store(0x1100+4*i,H[p+2+i]);for(uint32_t i=0;i<4*nt;i++)store(0x1500+4*i,H[p+2+4*nv+i]);' +
            'for(uint32_t i=0;i<n;i++)store(0x1D00+4*i,J[at+i]);at+=n;' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1D00;ctx.r5.u32=n;' +
            'for(int k=0;k<3;k++)ctx.v1.u32[3-k]=J[at+k];ctx.v1.u32[0]=0;at+=3;' +
            ('sub_%s(ctx,nullptr);printf("%%u' % fe) + NL + '",ctx.r3.u32&0xFFFF);}' + nl)
    consts = 'for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);store(0x8286093C,0x3F800000);'
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + consts + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + fe], main_src, a.out_dir, 'hull_element', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(jobs), (len(out), len(jobs))
    lines = []
    last = None
    for (h, m, cs, d), o in zip(jobs, out):
        if h != last:
            nv, nt, mc, vs, ps = hs[h]
            lines.append('H %x %x %x | %s | %s' % (nv, nt, mc, ' '.join('%x' % w for w in vs), ' '.join('%x' % w for w in ps)))
            last = h
        lines.append('C %x %x %s | %s -> %s' % (m, len(cs), ' '.join('%x' % c for c in cs), ' '.join('%08x' % w for w in d), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'hull_element_ref.txt').write_text(nl.join(lines) + nl)
    print(len(hs), 'hulls,', len(jobs), 'cases,', len(set(out)), 'distinct answers,', sum(o == '0' for o in out), 'zero')


if __name__ == '__main__':
    main()
