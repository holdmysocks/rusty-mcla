"""The level's broad-phase worker F-84048 run offline on the generated code: the sweep sphere it hands the octree query
and the pairs its box test writes. The octree query (F-15656 first / F-45809 next) is stubbed: it records the query's
sphere and hands back the case's candidate list in order; the job counter's atomic increment F-35685 is replaced
by a plain one; the two no-op calls stubbed. Rule: docs/VEHICLE_PHYSICS.md (the broad phase).

Memory: task 0x1000 (+68 count, +72 -> the active list u16 at 0x1100, +76 -> its counter 0x1080, +80 -> the pair counter
0x1084, +84 -> the pairs at 0x1200, +88 the pair limit, 8 bytes each: u16 a, u16 b, u32 0). Level 0x1800 (global
0x828DA464, and the simulator 0x1F00's +4): +112 -> entries 0x1900 (8 bytes an index: +4 the instance | state, 0 active),
+120 -> colliders 0x1C00. Index k: instance 0x3000 + 0x80 k (+4 -> archetype, +8 u16 k, +16.. four rows), archetype
0x3800 + 0x20 k (+12 -> bound), bound 0x3C00 + 0x40 k (+8 radius, +16 max, +32 min, +48 centroid), collider
0x4000 + 0x80 k (+32.. the four rows before the move).

Writes <out-dir>/broad_ref.txt: "B n | list | k state m0..m3 p0..p3 max min centroid radius ... -> Q own cx cy cz cw r ... P a b ..."
(hex words written x + 8 hex digits). Test `physics::loose::broad_tests::broad_matches_the_original`."""
import argparse
import math
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)
import artic_reference  # noqa: F401 (its shim: the SDK helpers vcuxwfp128 and vslw128 need)

fb, addr = bs.fb, bs.addr
MAXN = 10
W = 16 + 16 + 12 + 2  # words an object: rows now, rows before, max, min, centroid, radius, state


def rot(rnd, big):
    a, b, c = (rnd.uniform(-math.pi, math.pi) if big else rnd.uniform(-0.3, 0.3) for _ in range(3))
    ca, sa, cb, sb, cc, sc = math.cos(a), math.sin(a), math.cos(b), math.sin(b), math.cos(c), math.sin(c)
    rz = [[ca, sa, 0], [-sa, ca, 0], [0, 0, 1]]
    ry = [[cb, 0, -sb], [0, 1, 0], [sb, 0, cb]]
    rx = [[1, 0, 0], [0, cc, sc], [0, -sc, cc]]
    mul = lambda p, q: [[sum(p[i][k] * q[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    return mul(mul(rz, ry), rx)


def obj(rnd, at):
    r = rot(rnd, True)
    pos = [at[i] + rnd.uniform(-4, 4) for i in range(3)]
    now = [r[0] + [0.0], r[1] + [0.0], r[2] + [0.0], pos + [1.0]]
    if rnd.random() < 0.3:
        prev = [row[:] for row in now]
        if rnd.random() < 0.5:
            prev[3] = [pos[0] + rnd.uniform(-2, 2), pos[1] + rnd.uniform(-1, 1), pos[2] + rnd.uniform(-2, 2), 1.0]
    else:
        q = rot(rnd, False)
        rr = [[sum(q[i][k] * r[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
        prev = [rr[0] + [0.0], rr[1] + [0.0], rr[2] + [0.0], [pos[i] + rnd.uniform(-3, 3) for i in range(3)] + [1.0]]
    lo = [-rnd.uniform(0.05, 2.5) for _ in range(3)]
    hi = [rnd.uniform(0.05, 2.5) for _ in range(3)]
    if rnd.random() < 0.1:
        hi = [-x for x in lo]
    cg = [rnd.uniform(lo[i], hi[i]) for i in range(3)]
    rad = math.sqrt(sum(max(hi[i] - cg[i], cg[i] - lo[i]) ** 2 for i in range(3))) * rnd.choice([1.0, 1.0, 0.9, 1.3])
    state = 0 if rnd.random() < 0.6 else 1
    return now, prev, hi + [0.0], lo + [0.0], cg + [rnd.choice([0.0, 0.0, 1.0])], rad, state


def case(rnd):
    at = [rnd.uniform(-3000, 3000), rnd.uniform(-50, 50), rnd.uniform(-3000, 3000)]
    n = rnd.randrange(2, MAXN + 1)
    objs = [obj(rnd, at) for _ in range(n)]
    lst = rnd.sample(range(n), rnd.randrange(1, min(n, 3) + 1))
    for k in lst:
        objs[k] = objs[k][:6] + (0,)  # (the active list holds active objects)
    order = [rnd.sample(range(n), n) for _ in lst]
    return objs, lst, order


def words(o):
    now, prev, hi, lo, cg, rad, state = o
    return [fb(x) for row in now for x in row] + [fb(x) for row in prev for x in row] + [fb(x) for x in hi + lo + cg] + [fb(rad), state]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=1500)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-84048')
    first, nxt = 'sub_' + addr(a.tool, 'F-15656'), 'sub_' + addr(a.tool, 'F-45809')
    inc = 'sub_' + addr(a.tool, 'F-35685')
    rnd = random.Random(84048)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    flat = []
    for objs, lst, order in cases:
        w = [len(objs), len(lst)] + lst + [0] * (3 - len(lst))
        for o in order:
            w += o + [0] * (MAXN - len(o))
        w += [0] * (MAXN * (3 - len(order)))
        for o in objs:
            w += words(o)
        w += [0] * (W * (MAXN - len(objs)))
        flat.append('{' + ','.join('0x%x' % x for x in w) + '}')
    consts = ('for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);store(0x8286093C,0x3F800000);'
              'for(int k=0;k<4;k++){store(0x828608E0+4*k,0);store(0x828609E0+4*k,0x7F800000);store(0x828608C0+4*k,0);}')
    hdr = 2 + 3 + 3 * MAXN
    setup = ('for(uint32_t q=0x1000;q<0x7000;q+=4)store(q,0);uint32_t n=c[0],nl=c[1];'
             'store(0x828DA464,0x1800);store(0x828FF710,0x1F00);store(0x1F04,0x1800);store(0x1800+112,0x1900);store(0x1800+120,0x1C00);'
             'store(0x1000+68,nl);store(0x1000+72,0x1100);store(0x1000+76,0x1080);store(0x1000+80,0x1084);store(0x1000+84,0x1200);store(0x1000+88,64);'
             'for(uint32_t i=0;i<nl;i++)store16(0x1100+2*i,uint16_t(c[2+i]));'
             'for(uint32_t k=0;k<n;k++){const uint32_t*o=c+%d+%d*k;uint32_t in=0x3000+0x80*k,ar=0x3800+0x20*k,bo=0x3C00+0x40*k,co=0x4000+0x80*k;'
             'store(0x1900+8*k+4,in|o[%d]);store(0x1C00+4*k,co);store(in+4,ar);store16(in+8,uint16_t(k));store(ar+12,bo);store(ar+16,k);store(ar+20,0);'
             'for(int i=0;i<16;i++){store(in+16+4*i,o[i]);store(co+32+4*i,o[16+i]);}'
             'for(int i=0;i<4;i++){store(bo+16+4*i,o[32+i]);store(bo+32+4*i,o[36+i]);store(bo+48+4*i,o[40+i]);}store(bo+8,o[44]);}'
             ) % (hdr, W, W - 1)
    body = ('static const uint32_t C[][%d]={' % (hdr + W * MAXN) + nl + (',' + nl).join(flat) + nl + '};' + nl + consts +
            'for(auto&cc:C){const uint32_t*c=cc;G=c;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);'
            'uint32_t np=load(0x1084);printf("P %u",np);for(uint32_t i=0;i<np&&i<64;i++)printf(" %u %u %u",load16(0x1200+8*i),load16(0x1202+8*i),load(0x1204+8*i));'
            'printf("' + chr(92) + 'n");}')
    glob = ('static const uint32_t*G;static int own_at=-1,cur=0;'
            'static int pick(){const uint32_t*o=G+5+%d*own_at;uint32_t n=G[0];while(cur<(int)n){uint32_t k=o[cur++];return int(k);}return 0xFFFF;}' % MAXN)
    q = ('uint32_t qa=ctx.r4.u32;uint16_t me=load16(0x3000+0);'
         'printf("Q %08x %08x %08x %08x %08x' + chr(92) + 'n",load(qa),load(qa+4),load(qa+8),load(qa+12),load(qa+16));')
    stubs = {
        first: q.replace('uint16_t me=load16(0x3000+0);', '') + 'own_at++;cur=0;ctx.r3.u64=pick();',
        nxt: 'ctx.r3.u64=pick();',
        inc: 'uint32_t v=load(ctx.r3.u32)+1;store(ctx.r3.u32,v);ctx.r3.u64=v;',
        'sub_' + addr(a.tool, 'F-47121'): '',
        'sub_' + addr(a.tool, 'F-47330'): '',
    }
    body = body.replace('G=c;', 'G=c;own_at=-1;cur=0;')
    recomp_closure.EXTRA += glob + nl
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'broad', stubs=stubs, vector=True, scratch_end=0x10000)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    lines, i = [], 0
    for objs, lst, order in cases:
        qs = []
        while out[i].startswith('Q '):
            qs.append(out[i].split()[1:])
            i += 1
        assert out[i].startswith('P '), out[i]
        p = out[i].split()[1:]
        i += 1
        assert len(qs) == len(lst), (len(qs), lst)
        w = ['B', '%d' % len(objs), '|'] + ['%d' % k for k in lst]
        for o in order:
            w += ['|'] + ['%d' % k for k in o]
        for o in objs:
            ws = words(o)
            w += ['|'] + ['x%08x' % x for x in ws[:-1]] + ['%d' % ws[-1]]
        res = []
        for qq in qs:
            res += ['Q'] + ['x' + x for x in qq]
        res += ['P'] + p[1:]
        lines.append(' '.join(w) + ' -> ' + ' '.join(res))
    assert i == len(out)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'broad_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases,', sum(len(c[1]) for c in cases), 'queries,', sum(l.count(' P ') and len(l.split(' P ')[1].split()) // 3 for l in lines), 'pairs')


if __name__ == '__main__':
    main()
