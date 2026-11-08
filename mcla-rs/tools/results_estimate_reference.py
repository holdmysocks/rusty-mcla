"""The results' time estimate for racers who did not finish runs offline: F-23843, the generated code unchanged; its
callees are stubbed and answer from the case: the online test F-24338, the checkpoint manager F-71293 (its +4 the
checkpoint array, 368-byte entries, the centre at +64), the checkpoints per lap F-67883, the grid object F-14592
(a vector), the racer's chassis state word F-32888, the race list F-24591 (+44: 160-byte entries, +136 the base time)
and the racer's performance float F-82331. The race at 0x1000 (results block +8: racer count +3080, laps +3116,
192-byte records: +8 racer, +16 type, +32 next checkpoint, +36 lap, +40 time, +172 place); the racers' +192 vector
and (+48 -> +28 -> +64) position; the career manager's generator words (holder 0x8286D8DC, +0x16C78 / +0x16C7C).
Writes <out-dir>/results_estimate_ref.txt: one case per line, "<inputs> -> <times> | <generator words>"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

SCRATCH = 0x9000
_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: _shim(SCRATCH)

UNF = 0x7FFFFFFF


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def fs(x):
    return '%.9g' % x


def cases():
    rnd = random.Random(23843)
    out = []
    pt = lambda s: [f32(rnd.uniform(-s, s)) for _ in range(3)]
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 4, 8, 12, 16, rnd.randrange(0, 17)))
        m = rnd.choice((0, 1, 2, 5, 9, 16, rnd.randrange(0, 17)))
        laps = rnd.choice((1, 1, 2, 3, 0, -2, 150, rnd.randrange(-3, 120)))
        cps = [pt(800) for _ in range(16)]
        grid = pt(800)
        nofin = rnd.random() < 0.25
        recs = []
        places = list(range(1, n + 1))
        rnd.shuffle(places)
        for i in range(n):
            kind = rnd.choice((0, 1, 2, 2, 2, 2)) if i else rnd.choice((0, 0, 0, 1, 2))
            if nofin:
                time = rnd.choice((UNF, UNF, -1))
            else:
                time = rnd.choice((UNF, UNF, -1, rnd.randrange(30000, 600000)))
            nxt = rnd.choice((0, rnd.randrange(0, m + 1) if m else 0, rnd.randrange(-2, 16)))
            lap = rnd.choice((0, 0, 1, 2, -1, rnd.randrange(0, 5)))
            state = rnd.choice((0, 0, 0, 1, 2))
            perf = f32(rnd.choice((0.0, 1.0, 0.5, rnd.random(), rnd.uniform(-1, 2))))
            place = places[i] if rnd.random() < 0.9 else rnd.randrange(-1, 999)
            p192 = pt(800)
            pos = p192 if rnd.random() < 0.5 else pt(800)
            recs.append((kind, nxt, lap, time, place, state, perf, p192, pos))
        race = rnd.choice((-1, 0, 1, 2, 3))
        if not any(r[0] != 0 for r in recs):
            race = -1
        table = [rnd.randrange(20000, 400000) for _ in range(4)]
        rng = (rnd.getrandbits(32), rnd.getrandbits(32))
        online = 1 if rnd.random() < 0.03 else 0
        out.append((online, n, m, laps, race, table, rng, grid, cps, recs))
    return out


def line(c):
    online, n, m, laps, race, table, rng, grid, cps, recs = c
    v = lambda p: ','.join(map(fs, p))
    head = '%d %d %d %d %d %s %d,%d %s %s' % (online, n, m, laps, race, ','.join(map(str, table)), rng[0], rng[1], v(grid), ';'.join(v(p) for p in cps))
    body = ' '.join('%d,%d,%d,%d,%d,%d,%s,%s,%s' % (k, nx, lp, t, pl, st, fs(pf), v(a), v(b)) for k, nx, lp, t, pl, st, pf, a, b in recs)
    return head + ' ' + (body or '-')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-23843')
    cs = cases()
    nl = chr(10)
    stubs = {
        'F-24338': 'ctx.r3.u64=g_c->on;',
        'F-71293': 'ctx.r3.u64=0x2000;',
        'F-67883': 'ctx.r3.u64=g_c->m;',
        'F-14592': 'ctx.r3.u64=0x3900;',
        'F-32888': 'ctx.r3.u64=(uint32_t)g_c->r[(ctx.r3.u32-0x3A00)/0x100].st;',
        'F-24591': 'ctx.r3.u64=0x5600;',
        'F-82331': 'ctx.f1.f64=double(g_c->r[(ctx.r3.u32-0x3A00)/0x100].pf);',
    }
    stubs = {'sub_' + addr(a.tool, k): v for k, v in stubs.items()}
    recomp_closure.EXTRA += ('struct R{int k,nx,lp,t,pl,st;float pf,a[3],b[3];};'
                             'struct K{int on,n,m,laps,race,tb[4];uint32_t g0,g1;float grid[3],cp[16][3];R r[16];};static const K*g_c;\n'
                             'static void storef(uint32_t a,float v){uint32_t u;memcpy(&u,&v,4);store(a,u);}\n')
    cf = lambda x: '(float)(%s)' % fs(x)
    fl = lambda p: ','.join(cf(x) for x in p)
    def crow(c):
        online, n, m, laps, race, table, rng, grid, cps, recs = c
        rs = ','.join('{%d,%d,%d,%d,%d,%d,%s,{%s},{%s}}' % (k, nx, lp, t, pl, st, cf(pf), fl(p1), fl(p2)) for k, nx, lp, t, pl, st, pf, p1, p2 in recs) or '{}'
        return '{%d,%d,%d,%d,%d,{%s},%uu,%uu,{%s},{%s},{%s}}' % (online, n, m, laps, race, ','.join(map(str, table)), rng[0], rng[1], fl(grid),
                                                               ','.join('{%s}' % fl(p) for p in cps), rs)
    rows = (',' + nl).join(crow(c) for c in cs)
    vec = lambda base, arr: 'for(int q=0;q<3;q++)storef(%s+4*q,%s[q]);' % (base, arr)
    body = ('static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            # (the career manager's word at 0x8286D8DC (lis -32121, -10020): its generator at +0x16C78 lands on 0x5A00)
            'store(0x8286D8DC,0x5A00u-0x16C78u);' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));g_c=&c;'
            'store(0x1008+3080,c.n);store(0x1008+3116,c.laps);store(0x1008,c.race);'
            'store(0x2004,0x2100);for(int j=0;j<16;j++){' + vec('0x2100+368*j+64', 'c.cp[j]') + '}' + vec('0x3900', 'c.grid') +
            'store(0x5600+44,0x5700);for(int j=0;j<4;j++)store(0x5700+160*j+136,c.tb[j]);store(0x5A00,c.g0);store(0x5A04,c.g1);'
            'for(int i=0;i<c.n;i++){const R&r=c.r[i];uint32_t b=0x1008+192*i,ra=0x3A00+0x100*i,o=0x4A00+0x40*i,p=0x4E00+0x80*i;'
            'store(b+8,ra);store(b+16,r.k);store(b+32,r.nx);store(b+36,r.lp);store(b+40,r.t);store(b+172,r.pl);'
            + vec('ra+192', 'r.a') + 'store(ra+48,o);store(o+28,p);' + vec('p+64', 'r.b') + '}'
            '{Context ctx{};ctx.r1.u32=0x8F00;ctx.r3.u32=0x8000;ctx.r4.u32=0x1000;sub_%s(ctx,nullptr);}' % f +
            'for(int i=0;i<c.n;i++)printf("%d ",(int)load(0x1008+192*i+40));printf("| %u %u' + chr(92) + 'n",load(0x5A00),load(0x5A04));}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'estimate', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl)][:len(cs)]
    assert len(out) == len(cs)
    lines = [line(c) + ' -> ' + o for c, o in zip(cs, out)]
    # (no word that reads as an address of the original in tracked test data: such cases are left out)
    import re
    lines = [x for x in lines if not re.search(r'\b8[0-9A-Fa-f]{7}\b', x)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'results_estimate_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
