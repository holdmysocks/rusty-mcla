"""The convex part against a mesh bound's polygons, F-29567, run offline on the generated code: the car's hull (a
polyhedron bound, type 4) against a small piece of the city (a mesh bound with its polygons, quantised corners and
neighbours, whose matrix is the identity), through the pair's set-up, the culled polygon list, each polygon's box test,
its edge bisectors, the per-polygon contact F-35118 (the detector, the moving-out and edge tests), the sink F-90136 into
the manifold (F-74262, the add, the replace) and the cone of the contact's normal (F-12753 per edge the point lies on,
F-99162, F-32164). Rule and evidence: docs/reference/vehicle/polygon_loop_check.md.

Stubbed: the culled list's per-thread slot F-74289 (returns the list built here; the list itself is the broad phase's), the
swept routines, the other bound types' dispatch and the sub-manifolds' release F-36030 (throw: not reached;
the manifold's +1256 is 0), the part's element helper F-26668 (0: the
port's element of the hull is checked on its own row).

Memory: the pair 0x10000 (+0 the part's matrix, +64 the mesh's, +128 the part's last, +192 the mesh's last, +256 the part's
bound, +260 the mesh's), the part's bound 0x10200 (corners 0x10400), the mesh's bound 0x10600 (polygons 0x10800, corners
0x10C00), the culled list 0x11000 (its indices 0x11400), the sink 0x11500 (table 0x11540), the manifold 0x12000.

Writes <out-dir>/polygon_loop_ref.txt, one case a line (hex words):
  "L n corners(3n) now(12) last(12) max(3) min(3) radius | scale(3) centre(3) nv verts(3 nv, int16) np polys(np x 7: area
   v0 v1 v2 b0 b1 b2) nl list(nl) -> count records(count x 44)"
Test `physics::hull::tests::polygon_loop_matches_the_original`."""
import argparse
import math
import random
import struct
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)
import artic_reference  # noqa: F401 (its shim: the SDK helpers the vector code needs)
import polygon_contact_reference as pc

fb, addr = bs.fb, bs.addr
import recomp_leaf
recomp_closure.shim = lambda *_: recomp_leaf.shim(0x100000)

INITS = pc.INITS


def f(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def case(rnd):
    # the part, as polygon_contact_reference.py makes it
    n = rnd.choice([8, 12, 17, 18, 24])
    half = [rnd.uniform(0.6, 1.1), rnd.uniform(0.4, 0.8), rnd.uniform(1.6, 2.6)]
    corners = []
    for k in range(n):
        if k < 8:
            p = [half[0] * (1 if k & 1 else -1), half[1] * (1 if k & 2 else -1), half[2] * (1 if k & 4 else -1)]
            p = [x * rnd.uniform(0.7, 1.0) for x in p]
        else:
            p = [rnd.uniform(-h, h) for h in half]
        corners.append([f(p[0]), f(p[1] + 0.3), f(p[2])])
    lo = [min(c[i] for c in corners) for i in range(3)]
    hi = [max(c[i] for c in corners) for i in range(3)]
    bmax, bmin = [f(x + 0.041) for x in hi], [f(x - 0.041) for x in lo]
    mid = [(a + b) / 2 for a, b in zip(lo, hi)]
    radius = f(max(math.dist(c, mid) for c in corners) * rnd.choice([1.0, 1.0, 0.8, 1.3, 0.05]))  # (0.05: so the test by the move decides some)
    # the piece of the city: a G x G grid of corners, bumped, sometimes with a kerb or a wall, tilted, anywhere
    g = rnd.choice([3, 4, 5])
    s = rnd.choice([1.5, 3.0, 6.0])
    h = [[rnd.uniform(-0.15, 0.15) if rnd.random() < 0.7 else 0.0 for _ in range(g)] for _ in range(g)]
    if rnd.random() < 0.4:
        row, rise = rnd.randrange(1, g), rnd.choice([0.15, 0.4, 1.5])
        for a in range(row, g):
            for b in range(g):
                h[a][b] += rise
    local = [[(a - (g - 1) / 2) * s, h[a][b], (b - (g - 1) / 2) * s] for a in range(g) for b in range(g)]
    r = pc.rot(rnd, 0.15) if rnd.random() < 0.8 else pc.mul(pc.rot(rnd, 0.15), [[1, 0, 0], [0, 0, 1], [0, -1, 0]])
    at = [rnd.uniform(-3000, 3000), rnd.uniform(-30, 60), rnd.uniform(-3000, 3000)]
    world = [[at[i] + sum(p[j] * r[j][i] for j in range(3)) for i in range(3)] for p in local]
    centre = [f(x) for x in at]
    ext = max(abs(w[i] - centre[i]) for w in world for i in range(3)) + 1.0
    scale = [f(ext / 30000 * rnd.uniform(1.0, 1.3)) for _ in range(3)]
    q = [[max(-32768, min(32767, round((w[i] - centre[i]) / scale[i]))) for i in range(3)] for w in world]
    verts = [[f(f(qi[i] * scale[i]) + centre[i]) for i in range(3)] for qi in q]
    up = [r[1][i] for i in range(3)]
    tris = []
    for a in range(g - 1):
        for b in range(g - 1):
            v00, v01, v10, v11 = a * g + b, a * g + b + 1, (a + 1) * g + b, (a + 1) * g + b + 1
            pair = [[v00, v01, v11], [v00, v11, v10]] if rnd.random() < 0.5 else [[v00, v01, v10], [v01, v11, v10]]
            for t in pair:
                c = [verts[k] for k in t]
                e1 = [c[2][i] - c[1][i] for i in range(3)]
                e2 = [c[0][i] - c[1][i] for i in range(3)]
                cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
                if sum(cr[i] * up[i] for i in range(3)) < 0:
                    t = [t[0], t[2], t[1]]
                k = rnd.randrange(3)
                tris.append(t[k:] + t[:k])
    polys = []
    for i, t in enumerate(tris):
        c = [verts[k] for k in t]
        e1 = [c[1][j] - c[0][j] for j in range(3)]
        e2 = [c[2][j] - c[0][j] for j in range(3)]
        cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
        area = bits(f(0.5 * math.sqrt(sum(x * x for x in cr)))) & ~0x300  # (byte +2's low two bits: the type, 0)
        beside = []
        for e in range(3):
            a, b = t[e], t[(e + 1) % 3]
            o = [j for j, u in enumerate(tris) if j != i and a in u and b in u]
            beside.append(o[0] if o else 0xFFFF)
        polys.append([area] + t + beside)
    # the part's matrix: over a point of one of the polygons, pushed in by up to 30 cm or held off by up to 8 cm
    t = rnd.choice(tris)
    c = [verts[k] for k in t]
    e1 = [c[1][j] - c[0][j] for j in range(3)]
    e2 = [c[2][j] - c[0][j] for j in range(3)]
    face = pc.unit([e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]])
    u, v = rnd.random(), rnd.random()
    if u + v > 1:
        u, v = 1 - u, 1 - v
    on = [c[0][j] + e1[j] * u + e2[j] * v for j in range(3)]
    rows = pc.rot(rnd)
    reach = max(-sum(face[i] * sum(cc[j] * rows[j][i] for j in range(3)) for i in range(3)) for cc in corners)
    gap = rnd.choice([rnd.uniform(-0.3, 0.08), rnd.uniform(-0.05, 0.05), rnd.uniform(-0.1, 0.0), rnd.uniform(-0.2, -0.02)])
    pos = [on[i] + face[i] * (reach + gap) for i in range(3)]
    now = [[f(x) for x in rows[0]], [f(x) for x in rows[1]], [f(x) for x in rows[2]], [f(x) for x in pos]]
    vel = [rnd.uniform(-40, 40) / 60 for _ in range(3)]
    if rnd.random() < 0.4:
        vel = [face[i] * rnd.uniform(-0.3, 0.6) for i in range(3)]
    lr = pc.mul(rows, pc.rot(rnd, rnd.choice([0.0, 0.02, 0.1, 0.3])))
    last = [[f(x) for x in lr[0]], [f(x) for x in lr[1]], [f(x) for x in lr[2]], [f(pos[i] - vel[i]) for i in range(3)]]
    order = list(range(len(polys)))
    rnd.shuffle(order)
    if rnd.random() < 0.3:
        order = order[:rnd.randrange(1, len(order) + 1)]
    return corners, now, last, bmax, bmin, radius, scale, centre, q, polys, order


HARNESS = r'''
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    if(a==0xDETA) DETF(ctx,base);
    else if(a==0xPENA) PENF(ctx,base);
    else if(a==0xKEEPA) KEEPF(ctx,base);
    else if(a==0xSINKA) SINKF(ctx,base);
    else { std::printf("indirect %08x\n",a); std::exit(5); }
}
enum { PAIR=0x10000, PB=0x10200, PV=0x10400, MB=0x10600, MP=0x10800, MV=0x10C00, CULL=0x11000, CL=0x11400, SO=0x11500, SV=0x11540,
       MAN=0x12000, STACK=0xF0000 };
static void put3(uint32_t a,const uint32_t *v) { for(int i=0;i<3;i++) store(a+4*i,v[i]); store(a+12,0); }
int main(int argc,char **argv) {
    std::ifstream bin(argv[1],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    Context globals{}; globals.r1.u32=STACK;
    INITCALLS
    for (int k = 0; k < 3; k++) { store(0x82860950 + 4 * k, 0xFFFFFFFF); store(0x82860930 + 4 * k, 0); }
    store(0x8286095C, 0); store(0x8286093C, 0x3F800000);
    std::printf("I %02x %02x %08x %08x %08x %08x\n",*pointer(0x828127D7),*pointer(0x828127D8),load(0x82812810),load(0x82812A90),load(0x82001D14),load(0x828FF860));
    std::ifstream cases(argv[2]); std::string line;
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        size_t p=0; int n=int(w[p++]); const uint32_t *c=&w[p]; p+=3*n; const uint32_t *now=&w[p]; p+=12; const uint32_t *last=&w[p]; p+=12;
        const uint32_t *bmax=&w[p]; p+=3; const uint32_t *bmin=&w[p]; p+=3; uint32_t radius=w[p++];
        const uint32_t *scale=&w[p]; p+=3; const uint32_t *centre=&w[p]; p+=3; int nv=int(w[p++]); const uint32_t *q=&w[p]; p+=3*nv;
        int np=int(w[p++]); const uint32_t *poly=&w[p]; p+=7*np; int nl=int(w[p++]); const uint32_t *list=&w[p]; p+=nl;
        std::memset(objects,0,sizeof(objects));
        uint32_t one[3]={0x3F800000,0,0}, two[3]={0,0x3F800000,0}, three[3]={0,0,0x3F800000}, zero[3]={0,0,0};
        for(int i=0;i<4;i++) { put3(PAIR+16*i,now+3*i); put3(PAIR+128+16*i,last+3*i); }
        put3(PAIR+64,one); put3(PAIR+80,two); put3(PAIR+96,three); put3(PAIR+112,zero);
        put3(PAIR+192,one); put3(PAIR+208,two); put3(PAIR+224,three); put3(PAIR+240,zero);
        store(PAIR+256,PB); store(PAIR+260,MB); *pointer(PAIR+280)=0; *pointer(PAIR+281)=0;
        *pointer(PB+4)=4; store(PB+8,radius); put3(PB+16,bmax); put3(PB+32,bmin); store(PB+128,PV); store16(PB+186,uint16_t(n));
        for(int i=0;i<4;i++) store(PB+112+4*i,0x3D23D70A);
        for(int i=0;i<n;i++) put3(PV+16*i,c+3*i);
        *pointer(MB+4)=4; for(int i=0;i<4;i++) store(MB+112+4*i,0x3BA3D70A);
        store(MB+128,0); store(MB+140,MP); put3(MB+144,scale); put3(MB+160,centre); store(MB+176,MV); store(MB+204,uint32_t(np));
        for(int i=0;i<nv;i++) for(int k=0;k<3;k++) store16(MV+6*i+2*k,uint16_t(q[3*i+k]));
        for(int i=0;i<np;i++) { const uint32_t *r=poly+7*i; store(MP+16*i,r[0]); for(int k=0;k<6;k++) store16(MP+16*i+4+2*k,uint16_t(r[1+k])); }
        store(CULL+756,0); store16(CULL+106,uint16_t(nl)); store16(CULL+628,0); store(CULL+100,CL);
        for(int i=0;i<nl;i++) store16(CL+2*i,uint16_t(list[i]));
        store(SO,SV); store(SO+4,MAN); store(SO+8,PAIR); store(SO+12,PAIR+64); store(SV+4,0xSINKA);
        store(MAN+16,PB); store(MAN+1260,0); store(MAN+1264,0xFFFFFFFF);
        Context ctx{}; ctx.r1.u32=STACK; ctx.r3.u32=0; ctx.r4.u32=PAIR; ctx.r5.u32=MAN; ctx.r6.u32=SO;
        LOOPF(ctx,nullptr);
        int count=int(load(MAN+1260)); std::printf("R %d",count);
        for(int k=0;k<count;k++) for(int i=0;i<44;i++) std::printf(" %08x",load(MAN+192+176*k+4*i));
        std::printf("\n");
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
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    fn = sub('F-29567')
    rnd = random.Random(29567)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    words = []
    for corners, now, last, bmax, bmin, radius, scale, centre, q, polys, order in cases:
        w = [len(corners)] + [bits(x) for p in corners for x in p] + [bits(x) for r in now for x in r] + [bits(x) for r in last for x in r]
        w += [bits(x) for x in bmax] + [bits(x) for x in bmin] + [bits(radius)] + [bits(x) for x in scale] + [bits(x) for x in centre]
        w += [len(q)] + [x & 0xFFFF for p in q for x in p] + [len(polys)] + [x for p in polys for x in p] + [len(order)] + order
        words.append(w)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'polygon_loop_cases.txt'
    inp.write_text(nl.join(' '.join('%x' % x for x in w) for w in words) + nl)
    recomp_closure.EXTRA += ('\n#include <sstream>\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             'static void dispatch(Context&,const uint8_t*,uint32_t);\n'
                             'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                             '#define REX_QUERY_TIMEBASE() 1\n')  # (F-99162 waits for a time base that is not 0)
    inits = [sub(x) for x in INITS]
    det, pen, keep, sink = sub('F-33272'), sub('F-25833'), sub('F-15854'), sub('F-90136')
    main_src = HARNESS.replace('INITCALLS', ' '.join(f'{x}(globals,nullptr);' for x in inits))
    for key, name in [('DET', det), ('PEN', pen), ('KEEP', keep), ('SINK', sink)]:
        main_src = main_src.replace('0x' + key + 'A', '0x' + name[4:]).replace(key + 'F(', name + '(')
    main_src = main_src.replace('LOOPF(', fn + '(')
    thrower = 'throw std::runtime_error("not expected");'
    # (the culled list's slot: the list built here; the swept routines and other types' dispatch; the hull's element)
    stubs = {sub('F-15080'): thrower, sub('F-43958'): thrower, sub('F-98094'): thrower, sub('F-26668'): 'ctx.r3.u64=0;',
             sub('F-74289'): 'ctx.r3.u64=0x11000;', sub('F-36030'): thrower,
             sub('F-44611'): 'memmove(pointer(ctx.r3.u32),pointer(ctx.r4.u32),ctx.r5.u32);'}  # (the library's memmove)
    idx = recomp_closure.index(a.recomp)
    old = recomp_closure.closure
    recomp_closure.closure = lambda i, names, st=(), limit=60: old(i, names, st, 300)
    exe = recomp_closure.build(idx, [fn, det, pen, keep, sink] + inits, main_src, a.out_dir, 'lref',
                               stubs=stubs, vector=True, scratch_end=0x100000, flags=('-msse4.1', '-mssse3'))
    import subprocess
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    print(out[0])
    out = out[1:]
    assert len(out) == len(cases), (len(out), len(cases))
    lines, total, hist = [], 0, {}
    for w, o in zip(words, out):
        p = o.split()
        assert p[0] == 'R', o
        k = int(p[1])
        total += k
        hist[k] = hist.get(k, 0) + 1
        lines.append('L ' + ' '.join('%x' % x for x in w) + ' -> ' + ' '.join(p[1:]))
    (a.out_dir / 'polygon_loop_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases,', total, 'contacts kept; per case', sorted(hist.items()))


if __name__ == '__main__':
    main()
