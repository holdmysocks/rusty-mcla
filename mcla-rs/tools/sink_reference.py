"""The contact sink F-90136, run offline on the generated code: a new contact (the part's and the mesh's points in the
world, the normal, the distance, the two element numbers) into a manifold that already holds 0 .. 6 contacts, through the
record's constructor F-34223, the nearest kept contact F-74262, the replace F-96165, the add F-28977 and, for a seventh
contact, F-55965. Rule and evidence: docs/reference/vehicle/sink_check.md.

Stubbed: the sub-manifolds' release F-36030 (throw: not reached, the manifold's +1256 is 0), the library's memmove F-44611.

Memory: the part's matrix 0x10000, the mesh's 0x10040 (the identity: the city), the sink 0x11500 (table 0x11540), the
manifold 0x12000.

Writes <out-dir>/sink_ref.txt, one case a line (hex words):
  "S now(12) a b normal(3) world_a(3) world_b(3) distance n records(n x 44) -> fresh(44) count records(count x 44)"
`fresh` is the record the sink makes of the new contact (the same call into an empty manifold).
Test `physics::hull::tests::sink_matches_the_original`."""
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


def f(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def record(rnd, now, local, depth, man, age):
    """A kept record as the sink and a refresh leave it: the part's point (local, world), the mesh's (the world: the
    mesh's frame is the identity), the normal and its negative, the multipliers, the flags, the element numbers, the age."""
    w = [0] * 44
    rows, pos = now[:3], now[3]
    world_a = [f(pos[i] + sum(local[j] * rows[j][i] for j in range(3))) for i in range(3)]
    normal = pc.unit([rnd.uniform(-1, 1) for _ in range(3)])
    normal = [f(x) for x in normal]
    world_b = [f(world_a[i] + normal[i] * depth) for i in range(3)]
    pushed = [f(rnd.uniform(-2, 2)) for _ in range(3)] if rnd.random() < 0.7 else [0.0] * 3
    kept = [f(rnd.uniform(-2, 2)) for _ in range(4)] if rnd.random() < 0.7 else [0.0] * 4
    put = lambda at, v: w.__setitem__(slice(at, at + len(v)), [bits(x) for x in v])
    put(0, pushed + [f(depth)])
    put(4, [f(x) for x in local])
    put(8, world_b)
    put(12, world_a)
    put(16, world_b)
    put(20, normal)
    put(24, [f(0.0 - x) for x in normal])
    put(32, kept[:3])
    # (what the solver leaves in a kept record: friction +28, elasticity +44, the impulse +112, the velocity asked +144)
    if rnd.random() < 0.7:
        put(7, [f(rnd.uniform(0, 1.5))])
        put(11, [f(rnd.uniform(0, 0.5))])
        put(28, [f(rnd.uniform(-3, 3)) for _ in range(3)])
        put(36, [f(rnd.uniform(-3, 3)) for _ in range(3)])
    w[40] = man
    enabled = rnd.random() < 0.8
    w[41] = (0x40 | (0x80 if enabled else 0)) << 16
    w[42] = (rnd.randrange(0, 40) << 16) | rnd.randrange(0, 600)
    w[43] = age
    return w


def case(rnd, man):
    rows = pc.rot(rnd)
    pos = [rnd.uniform(-3000, 3000), rnd.uniform(-30, 60), rnd.uniform(-3000, 3000)]
    now = [[f(x) for x in rows[0]], [f(x) for x in rows[1]], [f(x) for x in rows[2]], [f(x) for x in pos]]
    n = rnd.choice([0, 1, 3, 5, 6, 6, 6, 6])
    # the kept contacts' points: on a plane of the part (a face resting on the city) or anywhere on it
    flat = rnd.random() < 0.7
    axis = rnd.randrange(3)
    side = rnd.choice([-1, 1]) * rnd.uniform(0.3, 1.0)
    def point():
        p = [rnd.uniform(-1.0, 1.0), rnd.uniform(-0.5, 0.5), rnd.uniform(-2.2, 2.2)]
        if flat:
            p[axis] = side + (rnd.uniform(-0.002, 0.002) if rnd.random() < 0.5 else 0.0)
        return [f(x) for x in p]
    locs = [point() for _ in range(n)]
    if n >= 2 and rnd.random() < 0.2:
        locs[1] = list(locs[0])  # (two equal points: the plane from the first three fails)
    recs = [record(rnd, now, locs[k], rnd.uniform(-0.02, 0.3), man, rnd.choice([0, 1, 1, 2, 7]) if rnd.random() < 0.85 else 0)
            for k in range(n)]
    # the new contact: near a kept one (within 2 cm or just beyond), or anywhere on the part
    if n and rnd.random() < 0.45:
        base = rnd.choice(locs)
        d = rnd.choice([0.0, 0.005, 0.015, 0.0199, 0.02, 0.03])
        dirn = pc.unit([rnd.uniform(-1, 1) for _ in range(3)])
        local = [base[i] + dirn[i] * d for i in range(3)]
    elif n == 6 and rnd.random() < 0.5:
        # inside the kept ones' outline, or just outside it
        a, b = rnd.sample(locs, 2)
        t = rnd.uniform(-0.3, 1.3)
        local = [a[i] + (b[i] - a[i]) * t for i in range(3)]
    else:
        local = point()
    rows_, p = now[:3], now[3]
    world_a = [f(p[i] + sum(local[j] * rows_[j][i] for j in range(3))) for i in range(3)]
    normal = [f(x) for x in pc.unit([rnd.uniform(-1, 1) for _ in range(3)])]
    distance = f(rnd.choice([rnd.uniform(-0.3, 0.02), rnd.uniform(-0.05, 0.0), 0.02, f(0.02) * 0.99999, rnd.uniform(0.02, 0.05)]))
    world_b = [f(world_a[i] - normal[i] * distance) for i in range(3)]
    a, b = rnd.randrange(0, 40), rnd.randrange(0, 600)
    return now, a, b, normal, world_a, world_b, distance, recs


HARNESS = r'''
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    if(a==0xSINKA) SINKF(ctx,base);
    else { std::printf("indirect %08x\n",a); std::exit(5); }
}
enum { MA=0x10000, MB=0x10040, SO=0x11500, SV=0x11540, MAN=0x12000, STACK=0xF0000 };
static void vec(VReg &v,const uint32_t *w) { for(int k=0;k<3;k++) v.u32[3-k]=w[k]; v.u32[0]=0; }
static void call(const uint32_t *now,uint32_t a,uint32_t b,const uint32_t *nrm,const uint32_t *wa,const uint32_t *wb,uint32_t dist) {
    for(int i=0;i<4;i++) { for(int k=0;k<3;k++) store(MA+16*i+4*k,now[3*i+k]); store(MA+16*i+12,0); }
    for(int i=0;i<16;i++) store(MB+4*i,(i%5==0&&i<15)?0x3F800000:0);
    store(SO,SV); store(SO+4,MAN); store(SO+8,MA); store(SO+12,MB); store(SV+4,0xSINKA);
    Context ctx{}; ctx.r1.u32=STACK; ctx.r3.u32=SO; ctx.r4.u64=a; ctx.r5.u64=b;
    vec(ctx.v1,nrm); vec(ctx.v2,wa); vec(ctx.v3,wb); for(int k=0;k<4;k++) ctx.v4.u32[k]=dist;
    SINKF(ctx,nullptr);
}
int main(int argc,char **argv) {
    std::ifstream bin(argv[1],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    Context globals{}; globals.r1.u32=STACK;
    INITCALLS
    std::printf("I %08x %08x\n",load(0x82812A90),load(0x82081BFC));
    std::ifstream cases(argv[2]); std::string line;
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        size_t p=0; const uint32_t *now=&w[p]; p+=12; uint32_t a=w[p++], b=w[p++];
        const uint32_t *nrm=&w[p]; p+=3; const uint32_t *wa=&w[p]; p+=3; const uint32_t *wb=&w[p]; p+=3; uint32_t dist=w[p++];
        int n=int(w[p++]); const uint32_t *rec=&w[p];
        // (the fresh record: the same contact into an empty manifold)
        std::memset(objects,0,sizeof(objects));
        store(MAN+1260,0); store(MAN+1264,0xFFFFFFFF);
        call(now,a,b,nrm,wa,wb,dist);
        std::printf("R");
        if(load(MAN+1260)==1) for(int i=0;i<44;i++) std::printf(" %08x",load(MAN+192+4*i));
        else std::printf(" none");
        std::memset(objects,0,sizeof(objects));
        store(MAN+1260,uint32_t(n)); store(MAN+1264,0xFFFFFFFF);
        for(int k=0;k<n;k++) for(int i=0;i<44;i++) store(MAN+192+176*k+4*i,rec[44*k+i]);
        call(now,a,b,nrm,wa,wb,dist);
        int count=int(load(MAN+1260)); std::printf(" | %d %08x",count,load(MAN+1264));
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
    fn = sub('F-90136')
    rnd = random.Random(90136)
    man = 0x12000
    cases = [case(rnd, man) for _ in range(a.cases)]
    nl = chr(10)
    words = []
    for now, ea, eb, normal, world_a, world_b, distance, recs in cases:
        w = [bits(x) for r in now for x in r] + [ea, eb] + [bits(x) for x in normal + world_a + world_b] + [bits(distance)]
        w += [len(recs)] + [x for r in recs for x in r]
        words.append(w)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'sink_cases.txt'
    inp.write_text(nl.join(' '.join('%x' % x for x in w) for w in words) + nl)
    recomp_closure.EXTRA += ('\n#include <sstream>\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             'static void dispatch(Context&,const uint8_t*,uint32_t);\n'
                             'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                             '#define REX_QUERY_TIMEBASE() 1\n')
    inits = [sub(x) for x in pc.INITS]
    main_src = HARNESS.replace('INITCALLS', ' '.join(f'{x}(globals,nullptr);' for x in inits))
    main_src = main_src.replace('0xSINKA', '0x' + fn[4:]).replace('SINKF(', fn + '(')
    stubs = {sub('F-36030'): 'throw std::runtime_error("not expected");',
             sub('F-44611'): 'memmove(pointer(ctx.r3.u32),pointer(ctx.r4.u32),ctx.r5.u32);'}  # (the library's memmove)
    idx = recomp_closure.index(a.recomp)
    old = recomp_closure.closure
    recomp_closure.closure = lambda i, names, st=(), limit=60: old(i, names, st, 300)
    exe = recomp_closure.build(idx, [fn] + inits, main_src, a.out_dir, 'sref',
                               stubs=stubs, vector=True, scratch_end=0x100000, flags=('-msse4.1', '-mssse3'))
    import subprocess
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    print(out[0])
    out = out[1:]
    assert len(out) == len(cases), (len(out), len(cases))
    lines, kinds = [], {}
    for w, o, c in zip(words, out, cases):
        assert o.startswith('R '), o
        fresh, rest = o[2:].split(' | ')
        n, count = len(c[7]), int(rest.split()[0])
        kind = 'refused' if fresh == 'none' else 'added' if count > n else 'six'
        kinds[kind] = kinds.get(kind, 0) + 1
        lines.append('S ' + ' '.join('%x' % x for x in w) + ' -> ' + fresh + ' | ' + rest)
    (a.out_dir / 'sink_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases;', kinds)


if __name__ == '__main__':
    main()
