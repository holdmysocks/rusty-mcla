"""Three small vector routines run offline (generated code unchanged, vector mode; a register's lane k is the host's
f32[3 - k]): a triangle's normal over twice its area F-13958 (v1..v3 the corners, r4 the area's word, r3 out), a body's
velocity at a point F-67516 (r3 the collider: +256 place, +272 velocity, +288 turning; v1 the point; r4 out) and the
inverse mass rows F-42768 (r3 the collider: +196 the inverse mass; r4 three rows out).
Writes <out-dir>/vec_small_ref.txt: "normal a0 a1 a2 b.. c.. area -> n", "pointvel at v w p -> u", "mass m -> r0 r1 r2"
(xyz each, hex bits)."""
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


def tri_area(p, q, r):
    u = [q[i] - p[i] for i in range(3)]
    v = [r[i] - p[i] for i in range(3)]
    c = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
    return 0.5 * sum(x * x for x in c) ** 0.5


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, fp, fm = addr(a.tool, 'F-13958'), addr(a.tool, 'F-67516'), addr(a.tool, 'F-42768')
    rnd = random.Random(47)
    jobs = []
    for _ in range(3000):
        o = [rnd.uniform(-3000, 3000) for _ in range(3)]
        t = [[o[i] + rnd.uniform(-8, 8) for i in range(3)] for _ in range(3)]
        ar = tri_area(*t)
        if rnd.random() < 0.2:
            ar *= rnd.uniform(0.98, 1.02)
        if rnd.random() < 0.02:
            ar = 0.0
        jobs.append(('normal', [c for p in t for c in p] + [ar]))
    for _ in range(3000):
        at = [rnd.uniform(-3000, 3000) for _ in range(3)]
        v = [rnd.uniform(-60, 60) for _ in range(3)]
        w = [rnd.uniform(-8, 8) for _ in range(3)]
        p = [at[i] + rnd.uniform(-3, 3) for i in range(3)]
        if rnd.random() < 0.05:
            p = list(at)
        jobs.append(('pointvel', at + v + w + p))
    for m in [rnd.uniform(0, 1) for _ in range(500)] + [0.0, 0.25, 1 / 1500, 1e-30]:
        jobs.append(('mass', [m]))
    nl = chr(10)
    NL = chr(92) + 'n'
    kind = {'normal': 0, 'pointvel': 1, 'mass': 2}
    rows = nl.join('{%d,%s},' % (kind[k], ','.join('0x%08x' % fb(x) for x in (c + [0.0] * 12)[:12])) for k, c in jobs)
    setv = 'for(int k=0;k<3;k++){ctx.v%d.u32[3-k]=c[%d+k];}ctx.v%d.u32[0]=0;'
    pr3 = 'for(int k=0;k<3;k++)printf(" %08x",load(OUT+4*k));'
    body = ('static const uint32_t C[][13]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;' +
            'if(c[0]==0){' + setv % (1, 1, 1) + setv % (2, 4, 2) + setv % (3, 7, 3) + 'store(0x1200,c[10]);ctx.r4.u32=0x1200;ctx.r3.u32=0x1100;' +
            ('sub_%s(ctx,nullptr);' % fn) + pr3.replace('OUT', '0x1100') + '}' +
            'else if(c[0]==1){for(int i=0;i<3;i++)for(int k=0;k<3;k++)store(0x1000+256+16*i+4*k,c[1+3*i+k]);' + setv % (1, 10, 1) +
            ('ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;sub_%s(ctx,nullptr);' % fp) + pr3.replace('OUT', '0x1400') + '}' +
            ('else{store(0x1000+196,c[1]);ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;sub_%s(ctx,nullptr);' % fm) +
            'for(int i=0;i<3;i++)' + pr3.replace('OUT', '0x1400+16*i') + '}' +
            'printf("' + NL + '");}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fn, 'sub_' + fp, 'sub_' + fm], main_src, a.out_dir, 'vec_small', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(jobs), (len(out), len(jobs))
    lines = ['%s %s -> %s' % (k, ' '.join('%08x' % fb(x) for x in c), o) for (k, c), o in zip(jobs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'vec_small_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
