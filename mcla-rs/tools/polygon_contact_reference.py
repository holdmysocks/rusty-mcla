"""The per-polygon contact routine F-35118 run offline on the generated code: a convex part (a polyhedron bound, type 4, as the
car's hull is) against one polygon of a fixed mesh whose matrix is the identity (the city), the pair's flag +280 set (the points
about the polygon's first corner, then the edge test) and +282 clear (the discrete detector, not the swept one). Its callees
are unchanged: the triangle shape's constructor and margin (F-35118 builds it), the detector F-35118 sets up and calls through
its table (the GJK loop with its simplex and penetration routines and the closest-result object), the moving-out test and the
edge test with F-96748 (the point-beside-edge test). Rule and evidence: docs/reference/vehicle/polygon_contact_check.md.

Stubbed: the swept routines (never reached with +282 clear: they throw), the bound-type dispatch for other types (throws),
and the two element-index helpers (0: they only name the features touched; the port's element is checked elsewhere).

Memory: the pair object 0x10000 (+16 the mesh's matrix, +80 the mesh's last, +144 the part's matrix, +208 its last - rows
and place -, +272 the polygon margin 0.005, +280/+281/+282 the flags, +704 the detector, +740 its solver 0x12000), the
manifold 0x10800 (+16 the part's bound), the bound 0x11000 (corners 0x11200, margin 0.04), the polygon's corners 0x11600,
the edge bisectors 0x11700 (r5), the edge's second vectors 0x11800 (r6), the sink 0x11900 (its table's second entry logs).

Writes <out-dir>/polygon_contact_ref.txt, one case a line (hex words x + 8 hex digits):
  "P n corners(3n) now(12) last(12) v0 v1 v2 face bis(9) ridge(9) mask -> 0" (nothing handed to the sink) or
  "... -> 1 normal(3) a(3) b(3) distance" (what F-35118 hands the sink: the normal, the part's and the polygon's points in
  the world, the distance). Test `physics::hull::tests::polygon_contact_matches_the_original`."""
import argparse
import math
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)
import artic_reference  # noqa: F401 (its shim: the SDK helpers the vector code needs)

fb, addr = bs.fb, bs.addr
import recomp_leaf
recomp_closure.shim = lambda *_: recomp_leaf.shim(0x100000)  # (the detector's frames want more stack than the shared 0x8000)

# the static initialisers of the vector constants these read (BSS in the image), as gjk_reference.py, and the edge tolerance's
INITS = ['F-50173', 'F-33402', 'F-26199', 'F-55749', 'F-97349', 'F-70580', 'F-51135', 'F-79721', 'F-64228', 'F-24192', 'F-92824']


def rot(rnd, s=math.pi):
    a, b, c = (rnd.uniform(-s, s) for _ in range(3))
    ca, sa, cb, sb, cc, sc = math.cos(a), math.sin(a), math.cos(b), math.sin(b), math.cos(c), math.sin(c)
    rz = [[ca, sa, 0], [-sa, ca, 0], [0, 0, 1]]
    ry = [[cb, 0, -sb], [0, 1, 0], [sb, 0, cb]]
    rx = [[1, 0, 0], [0, cc, sc], [0, -sc, cc]]
    mul = lambda p, q: [[sum(p[i][k] * q[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    return mul(mul(rz, ry), rx)


def mul(p, q):
    return [[sum(p[i][k] * q[k][j] for k in range(3)) for j in range(3)] for i in range(3)]


def unit(v):
    n = math.sqrt(sum(x * x for x in v))
    return [x / n for x in v]


def f32(x):
    return bs.fb(x)


def case(rnd):
    # the part: a car-sized convex point set about its own origin
    n = rnd.choice([8, 12, 17, 18, 24])
    half = [rnd.uniform(0.6, 1.1), rnd.uniform(0.4, 0.8), rnd.uniform(1.6, 2.6)]
    corners = []
    for k in range(n):
        if k < 8:
            p = [half[0] * (1 if k & 1 else -1), half[1] * (1 if k & 2 else -1), half[2] * (1 if k & 4 else -1)]
            p = [x * rnd.uniform(0.7, 1.0) for x in p]
        else:
            p = [rnd.uniform(-h, h) for h in half]
        corners.append([p[0], p[1] + 0.3, p[2]])
    # the polygon: a wall or a piece of ground somewhere in the city
    at = [rnd.uniform(-3000, 3000), rnd.uniform(-30, 60), rnd.uniform(-3000, 3000)]
    size = rnd.choice([1.0, 3.0, 8.0, 20.0])
    ground = rnd.random() < 0.5
    t = [[0, 0, 0], [size, 0, rnd.uniform(-0.3, 0.3) * size], [rnd.uniform(-0.5, 1.0) * size, 0, size]]
    r = rot(rnd, 0.2) if ground else mul(rot(rnd, 0.2), [[1, 0, 0], [0, 0, 1], [0, -1, 0]])
    yaw = rot(rnd, 0)  # identity
    tri = [[at[i] + sum(p[j] * r[j][i] for j in range(3)) for i in range(3)] for p in t]
    if rnd.random() < 0.5:
        tri = [tri[0], tri[2], tri[1]]
    tri = [[fb_round(x) for x in p] for p in tri]
    e1 = [tri[1][i] - tri[0][i] for i in range(3)]
    e2 = [tri[2][i] - tri[0][i] for i in range(3)]
    cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
    face = unit(cr)
    # the part's matrix: over a point of the polygon, pushed into it by up to 30 cm or held off by up to 6 cm
    u, v = rnd.random(), rnd.random()
    if u + v > 1:
        u, v = 1 - u, 1 - v
    if rnd.random() < 0.2:
        u, v = rnd.uniform(-0.2, 1.2), rnd.uniform(-0.2, 1.2)
    on = [tri[0][i] + e1[i] * u + e2[i] * v for i in range(3)]
    rows = rot(rnd)
    # how far the part reaches along -face from its origin
    reach = max(-sum(face[i] * sum(c[j] * rows[j][i] for j in range(3)) for i in range(3)) for c in corners)
    gap = rnd.choice([rnd.uniform(-0.3, 0.06), rnd.uniform(-0.05, 0.05), rnd.uniform(0.0, 0.08)])
    pos = [on[i] + face[i] * (reach + gap) for i in range(3)]
    now = [rows[0], rows[1], rows[2], pos]
    # the last matrix: a tick back at up to 40 m/s and a few degrees
    vel = [rnd.uniform(-40, 40) / 60 for _ in range(3)]
    if rnd.random() < 0.3:
        vel = [face[i] * rnd.uniform(-0.3, 0.3) for i in range(3)]
    lr = mul(rows, rot(rnd, 0.05))
    last = [lr[0], lr[1], lr[2], [pos[i] - vel[i] for i in range(3)]]
    # the edges: the neighbour across each, its bisector and the ridge's normal
    bis, ridge, mask = [], [], 0
    for e in range(3):
        if rnd.random() < 0.6:
            mask |= 1 << e
            other = unit([face[i] + rnd.uniform(-0.8, 0.8) for i in range(3)])
            b = unit([face[i] + other[i] for i in range(3)])
            bis.append(b)
            ridge.append(other if rnd.random() < 0.5 else face)
        else:
            bis.append(face)
            ridge.append(face)
    return corners, now, last, tri, face, bis, ridge, mask


def fb_round(x):
    import struct
    return struct.unpack('<f', struct.pack('<f', x))[0]


HARNESS = r'''
static struct { int valid; uint32_t w[10]; } res;
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    if(a==0xDETA) DETF(ctx,base);
    else if(a==0xPENA) PENF(ctx,base);
    else if(a==0xKEEPA) KEEPF(ctx,base);
    else if(a==0xDEAD0010) { res.valid++; for(int i=0;i<3;i++){ res.w[i]=ctx.v1.u32[3-i]; res.w[3+i]=ctx.v2.u32[3-i]; res.w[6+i]=ctx.v3.u32[3-i]; } res.w[9]=ctx.v4.u32[3]; }
    else { std::printf("indirect %08x\n",a); std::exit(5); }
}
enum { OBJ=0x10000, MAN=0x10800, BOUND=0x11000, VERTS=0x11200, TRIV=0x11600, BIS=0x11700, RID=0x11800, SINK=0x11900, SOLVER=0x12000, STACK=0xF0000 };
static void put3(uint32_t a,const uint32_t *v) { for(int i=0;i<3;i++) store(a+4*i,v[i]); store(a+12,0); }
int main(int argc,char **argv) {
    std::ifstream bin(argv[1],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    Context globals{}; globals.r1.u32=STACK;
    INITCALLS
    for (int k = 0; k < 3; k++) { store(0x82860950 + 4 * k, 0xFFFFFFFF); store(0x82860930 + 4 * k, 0); }
    store(0x8286095C, 0); store(0x8286093C, 0x3F800000);
    if(load(0x82084B9C)!=0xDETA || load(0x82813394)!=0x820837D4 || load(0x820837D8)!=0xPENA || load(0x82083464)!=0xKEEPA) { std::printf("tables %08x\n",load(0x82084B9C)); return 4; }
    std::printf("I %08x %08x %08x %08x\n",load(0x82081FD0),load(0x828FF860),load(0x828133A0),load(0x827D8470));
    std::ifstream cases(argv[2]); std::string line;
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        int n=int(w[0]); const uint32_t *c=&w[1], *now=c+3*n, *last=now+12, *t=last+12, *face=t+9, *bis=face+3, *rid=bis+9; uint32_t mask=rid[9];
        std::memset(objects,0,sizeof(objects)); res={};
        uint32_t one[3]={0x3F800000,0,0}, two[3]={0,0x3F800000,0}, three[3]={0,0,0x3F800000}, zero[3]={0,0,0};
        put3(OBJ+16,one); put3(OBJ+32,two); put3(OBJ+48,three); put3(OBJ+64,zero);
        put3(OBJ+80,one); put3(OBJ+96,two); put3(OBJ+112,three); put3(OBJ+128,zero);
        for(int i=0;i<4;i++) { put3(OBJ+144+16*i,now+3*i); put3(OBJ+208+16*i,last+3*i); }
        store(OBJ+272,0x3BA3D70A); *pointer(OBJ+280)=1; *pointer(OBJ+281)=0; *pointer(OBJ+282)=0;
        store(OBJ+704,0x82084B98); store(OBJ+740,SOLVER);
        store(MAN+16,BOUND); store(MAN+1260,0); store(MAN+1264,0xFFFFFFFF);
        *pointer(BOUND+4)=4; store(BOUND+128,VERTS); store(BOUND+132,0); store16(BOUND+186,uint16_t(n));
        for(int i=0;i<3;i++) store(BOUND+112+4*i,0x3D23D70A);
        for(int i=0;i<n;i++) put3(VERTS+16*i,c+3*i);
        for(int i=0;i<3;i++) { put3(TRIV+16*i,t+3*i); put3(BIS+16*i,bis+3*i); put3(RID+16*i,rid+3*i); }
        store(SINK,SINK+64); store(SINK+68,0xDEAD0010);
        Context ctx{}; ctx.r1.u32=STACK; ctx.r3.u32=OBJ; ctx.r4.u32=TRIV; ctx.r5.u32=BIS; ctx.r6.u32=RID; ctx.r7.u32=2; ctx.r8.u32=mask;
        ctx.r9.u32=SINK; ctx.r10.u32=MAN; for(int i=0;i<3;i++) ctx.v1.u32[3-i]=face[i]; ctx.v1.u32[0]=0;
        POLYF(ctx,nullptr);
        std::printf("R %d",res.valid); if(res.valid) for(uint32_t x:res.w) std::printf(" %08x",x); std::printf("\n");
    }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-35118')
    rnd = random.Random(35118)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    words = []
    for corners, now, last, tri, face, bis, ridge, mask in cases:
        w = [len(corners)] + [fb(x) for p in corners for x in p] + [fb(x) for r in now for x in r] + [fb(x) for r in last for x in r]
        w += [fb(x) for p in tri for x in p] + [fb(x) for x in face] + [fb(x) for b in bis for x in b] + [fb(x) for b in ridge for x in b] + [mask]
        words.append(w)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'polygon_contact_cases.txt'
    inp.write_text(nl.join(' '.join('%x' % x for x in w) for w in words) + nl)
    recomp_closure.EXTRA += ('\n#include <sstream>\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             'static void dispatch(Context&,const uint8_t*,uint32_t);\n')
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    inits = [sub(x) for x in INITS]
    det, pen, keep = sub('F-33272'), sub('F-25833'), sub('F-15854')  # the detector, the penetration routine, the closest result
    main_src = HARNESS.replace('INITCALLS', ' '.join(f'{x}(globals,nullptr);' for x in inits))
    for key, name in [('DET', det), ('PEN', pen), ('KEEP', keep)]:
        main_src = main_src.replace('0x' + key + 'A', '0x' + name[4:]).replace(key + 'F(', name + '(')
    main_src = main_src.replace('POLYF(', fn + '(')
    thrower = 'throw std::runtime_error("not expected");'
    # (the bound-type dispatch for other types, the two swept routines; the element-index helpers)
    stubs = {sub('F-15080'): thrower, sub('F-43958'): thrower, sub('F-98094'): thrower, sub('F-26668'): 'ctx.r3.u64=0;', sub('F-11678'): 'ctx.r3.u64=0;'}
    idx = recomp_closure.index(a.recomp)
    old = recomp_closure.closure
    recomp_closure.closure = lambda i, names, st=(), limit=60: old(i, names, st, 200)
    exe = recomp_closure.build(idx, [fn, det, pen, keep] + inits, main_src, a.out_dir, 'pref',
                               stubs=stubs, vector=True, scratch_end=0x100000, flags=('-msse4.1', '-mssse3'))
    import subprocess
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    print(out[0])
    out = out[1:]
    assert len(out) == len(cases), (len(out), len(cases))
    lines, hits = [], 0
    for w, o in zip(words, out):
        p = o.split()
        assert p[0] == 'R' and p[1] in ('0', '1'), o
        hits += p[1] == '1'
        lines.append('P ' + ' '.join('x%08x' % x for x in w[:-1]) + ' %d -> ' % w[-1] + ' '.join([p[1]] + ['x' + x for x in p[2:]]))
    (a.out_dir / 'polygon_contact_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases,', hits, 'handed to the sink')


if __name__ == '__main__':
    main()
