"""Run the original's mcRaceArray_FindBeatMeThereRaceIndex (F-73402) offline and record it, for the port's `mc4::bmt_pick`
(crates/game/src/testdata/bmt_ref.txt; test mc4::tests::bmt_pick_matches_the_original).

F-73402 (r3 the race array: +0 entries of 8 bytes, the table index at +4, +4 the count as a half word; r4 the district, 6 = any;
r5 the racer, f1 the distance): the matrix of the racer (F-35658: row c +32, the place +48); a buffer of 12-byte candidates
(F-34831); each entry of the array whose record (F-24591's table +44, stride 160: +8 the district, +48 the start) is in the
district (or 6 asked): its position in the array, its distance on the map (x / z) and the dot product of row c and the direction
to the race, both normalised by the vector unit's 1 / sqrt with one Newton step. Sorted on the dot (F-79639 with F-33494, run
as they are). Then, of those further than the distance, the first whose history place (F-97924) is -1, else the one with the
lowest place (the first such). None: (lo & 0x7FFFFFFF) mod the candidates' count (or, with no candidate, the array's) of one
draw of the career's generator (career +0x16C78), used as an index into the whole array. Stubbed: the matrix, the buffer, the
table, the history and the log.

  d<district> r<dist> p<x,y,z> c<x,y,z> <entries: table:district:x,y,z:history ...> g<lo> g<carry> -> <table index> g<lo> g<carry>
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
CAREERG = IMG + 0x86D8DC        # the career manager (-10020 off 0x82870000)

MAIN = r'''
static uint32_t fb(float f){uint32_t u; memcpy(&u,&f,4); return u;}
static float bf(uint32_t u){float f; memcpy(&f,&u,4); return f;}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t A=0x40800000u, T=0x40810000u, R=0x40820000u, M=0x40900000u, C=0x40980000u, H=0x40A00000u;
 while(in>>tok){
  if(tok!="case") break; uint32_t d,rd,n,lo,ca; float px,py,pz,cx,cy,cz; in>>d>>rd>>px>>py>>pz>>cx>>cy>>cz>>n;
  rt_reset(); for(uint32_t a=A;a<A+0x400;a+=4) store(a,0);
  store(A,A+0x100); store(A+4,n<<16); store(T+44,R);
  store(M+32,fb(cx)); store(M+36,fb(cy)); store(M+40,fb(cz)); store(M+44,0); store(M+48,fb(px)); store(M+52,fb(py)); store(M+56,fb(pz)); store(M+60,fb(1.0f));
  for(uint32_t k=0;k<n;k++){ uint32_t t,dd,h; float x,y,z; in>>t>>dd>>x>>y>>z>>h; uint32_t r=R+t*160u;
   store(A+0x100+8*k,0); store(A+0x100+8*k+4,t); store(r+8,dd); store(r+48,fb(x)); store(r+52,fb(y)); store(r+56,fb(z)); store(r+60,fb(1.0f)); store(H+4*t,h); }
  in>>lo>>ca; store(CAREERG,C); store(C+0x16C78,lo); store(C+0x16C7C,ca);
  Context c=fresh(); c.r3.u32=A; c.r4.u32=d; c.r5.u32=0x40990000u; c.f1.f64=(double)bf(rd); BMT(c,nullptr);
  printf("%u | %u %u\n",c.r3.u32,load(C+0x16C78),load(C+0x16C7C)); }
 printf("end\n");
}
'''


def f32(v):
    return struct.unpack('<f', struct.pack('<f', v))[0]


def fbits(v):
    return struct.unpack('<I', struct.pack('<f', v))[0]


def cases():
    g = random.Random(73402)
    out = []
    for _ in range(3000):
        n = g.choice((1, 1, 2, 3, 4, 6, 8, 12, 20, 30))
        d = g.choice((0, 1, 2, 3, 4, 6, 6))
        rd = g.choice((400.0, 400.0, 0.0, 150.0, f32(g.uniform(0, 1500))))
        p = [f32(g.uniform(-2500, 2500)), f32(g.uniform(-20, 120)), f32(g.uniform(-2500, 2500))]
        c = [f32(g.uniform(-1, 1)), f32(g.uniform(-0.2, 0.2)), f32(g.uniform(-1, 1))]
        if g.random() < 0.3:
            c = [f32(v * g.choice((0.01, 3.0, 250.0))) for v in c]
        if all(v == 0 for v in c):
            c[0] = 1.0
        ents, used, places = [], set(), []
        for _ in range(n):
            t = g.randrange(60)
            while t in used:
                t = g.randrange(60)
            used.add(t)
            if places and g.random() < 0.1:                 # (a race at the same start as another: equal dots)
                x = g.choice(places)
            else:
                x = [f32(p[0] + g.uniform(-1800, 1800)), f32(g.uniform(-20, 120)), f32(p[2] + g.uniform(-1800, 1800))]
                if g.random() < 0.05:                       # (straight along row c / behind: dots at +-1)
                    k = g.choice((-900.0, 600.0))
                    x = [f32(p[0] + c[0] * k), f32(p[1] + c[1] * k), f32(p[2] + c[2] * k)]
            places.append(x)
            if x == p:
                x = [f32(x[0] + 1.0), x[1], x[2]]
            dd = g.choice((0, 1, 2, 3, 4, d, d, -1)) & 0xFFFFFFFF
            h = g.choice((-1, -1, -1, 0, 1, 2, 3, 5, 9, 2)) & 0xFFFFFFFF
            ents.append((t, dd, x, h))
        out.append((d, rd, p, c, ents, g.getrandbits(32), g.getrandbits(32) % 0x5CDCFAA7))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-73402', 'F-34831', 'F-35658', 'F-24591', 'F-79639', 'F-33494', 'F-97924', 'F-47121', 'F-13813']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {
        name('F-34831'): 'ctx.r3.u64=0x40880000u;',
        name('F-35658'): 'ctx.r3.u64=0x40900000u;',
        name('F-24591'): 'ctx.r3.u64=0x40810000u;',
        name('F-97924'): 'ctx.r3.u64=(uint64_t)(int64_t)(int32_t)load(0x40A00000u+4u*ctx.r4.u32);',
        name('F-47121'): '',
        name('F-13813'): '',
    }
    src = MAIN.replace('BMT', name('F-73402')).replace('CAREERG', '0x%08Xu' % CAREERG)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-73402')], src, a.out, 'bmt', image=img, stubs=stubs, targets=[addr['F-33494']])
    cs = cases()
    cf = a.out / 'bmt_cases.txt'
    with cf.open('w') as f:
        for d, rd, p, c, ents, lo, ca in cs:
            f.write('case %d %d %r %r %r %r %r %r %d %s %d %d\n' % (d, fbits(rd), *p, *c, len(ents), ' '.join(
                '%d %d %r %r %r %d' % (t, dd, *x, h) for t, dd, x, h in ents), lo, ca))
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.split('\n')[:-2]
    assert len(lines) == len(cs)
    s32 = lambda v: v - (1 << 32) if v >= 0x80000000 else v
    xs = lambda v: ','.join('%08x' % fbits(q) for q in v)
    rows = []
    for (d, rd, p, c, ents, lo, ca), l in zip(cs, lines):
        got, gen = l.split(' | ')
        lo2, ca2 = gen.split()
        es = ' '.join('%d:%d:%s:%d' % (t, s32(dd), xs(x), s32(h)) for t, dd, x, h in ents)
        rows.append('d%d r%08x p%s c%s %s g%08x g%08x -> %s g%08x g%08x' % (d, fbits(rd), xs(p), xs(c), es, lo, ca, got, int(lo2), int(ca2)))
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
