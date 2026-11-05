"""The ambient rider's obstacle gather (F-47175's head and gather, with F-14781, F-25797 and F-44673) run offline: the
generated code whole and unchanged in vector mode (the VMX flush mode as the host's DAZ / FTZ), on flat guest memory.

F-47175 runs from its start; the selection loop after the gather is not run: the asker's slot 3 (called right after the
gather with r4 = stack + 144) prints the gather's count (stack + 96) and its obstacles (stack + 240 ..) and sets the
count to 0, so the function returns. Stubbed: the grid's cell F-88455 (0) and its 3 x 3 heads F-99153 (the case's nine
lists). Everything else is the case's objects read through their vtables: obstacles (slot 1 the matrix: row 2 +32,
row 3 +48; slot 2 the velocity; kind byte +6; the grid's next +12), lanes (word +4 flags, 0x1800 for the run-time rail;
slot 12 the start, 13 the end, 15 the curve: +28 the last index, +32 the vertex indices; +96 the car list: node +0 the
entry (its +12 the obstacle), +4 the next node), the railnet (+8 the root, +56 the vertex table: 8 bytes a vertex,
+0 the lanes, u16 +4 their count). The shared MWC (0x82860890) is the case's; the mask words 0x82860950 (zero in the
image, filled at run time) are set to x / y / z.

Writes <out-dir>/traffic_gather_ref.txt, a case a line (layout in `traffic.rs` test `gather_matches_the_original`):
words '->' then 'k' (the last search kept) or 'g' n and the n obstacles, then the MWC after (16-bit halves) and the
result's two words after (obstacle index, ffffffff none)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x80000)
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    reg = 'uint16_t u16; uint8_t u8;'
    assert s.count(reg) == 1
    s = s.replace(reg, 'uint16_t u16; int16_t s16; uint8_t u8;', 1)
    fl = 'void disableFlushMode() {} void enableFlushMode() {}'
    assert s.count(fl) == 1
    # (the VMX flush mode: denormal inputs read as zero, denormal results written as zero)
    return s.replace(fl, 'void disableFlushMode() {_mm_setcsr(_mm_getcsr()&~0x8040u);} void enableFlushMode() {_mm_setcsr(_mm_getcsr()|0x8040u);}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                         'static uint32_t G[9];\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

NL = chr(10)
A = 0x5CDCFAA7
NONE, RUNTIME = 0xFFFFFFFF, 0xFFFFFFFE


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u & 0xFFFFFFFF))[0]


def f32(x):
    return bf(fb(x))


def gen(rnd):
    cap = rnd.random() < 0.06
    nobs = rnd.randint(60, 90) if cap else rnd.choice((1, 2, 3, 5, 8, 10, 12, 16, 20))
    asker = rnd.randrange(nobs)
    # the asker's back axis: axis aligned (exact zero dots) or any unit vector
    ax = rnd.choice(((0.0, 0.0, 1.0), (0.0, 0.0, -1.0), (1.0, 0.0, 0.0), (-0.0, 0.0, 1.0), None, None))
    if ax is None:
        import math
        a = rnd.uniform(-math.pi, math.pi)
        ax = (math.sin(a), rnd.choice((0.0, rnd.uniform(-0.1, 0.1))), math.cos(a))
    r2 = tuple(f32(x) for x in ax)
    base = (f32(rnd.choice((0.0, rnd.uniform(-900, 900)))), f32(rnd.choice((0.0, rnd.uniform(-20, 40)))), f32(rnd.choice((0.0, rnd.uniform(-900, 900)))))
    den = (bf(1), bf(0x80000001), bf(0x7FFFFF), bf(0x80400000))
    obs = []
    for i in range(nobs):
        kind = rnd.choice((0, 2, 2, 2, 1)) if cap else rnd.choice((0, 1, 2, 2, 2, 2, 3, 4, 7))
        k = rnd.random()
        if i == asker:
            p = base
        elif k < 0.08:
            p = base
        elif k < 0.18:
            # beside: no offset along an axis-aligned back axis
            t = rnd.uniform(-30, 30)
            p = (f32(base[0] + t * (1 - abs(r2[0]))), base[1], f32(base[2] + t * (1 - abs(r2[2]))))
        elif k < 0.24:
            # a denormal off (flushed)
            p = tuple(f32(base[j] + rnd.choice(den)) if base[j] == 0.0 else base[j] for j in range(3))
            if p == base:
                p = (rnd.choice(den), base[1], base[2]) if base[0] == 0.0 else (base[0], base[1], f32(base[2] + rnd.choice((-1, 1)) * 1e-30))
        elif k < 0.26:
            p = (float('nan'), base[1], base[2])
        else:
            t = rnd.uniform(-60, 60)
            s = rnd.uniform(-8, 8)
            p = (f32(base[0] - r2[0] * t + s * r2[2]), f32(base[1] + rnd.uniform(-2, 2)), f32(base[2] - r2[2] * t - s * r2[0]))
        rr = r2 if i == asker else tuple(f32(rnd.uniform(-1, 1)) for _ in range(3))
        v = rnd.choice(((0.0, 0.0, 0.0), (0.0, 0.0, 0.0), (-0.0, 0.0, -0.0), (den[0], 0.0, 0.0), (0.0, f32(rnd.uniform(-5, 5)), 0.0),
                        tuple(f32(rnd.uniform(-20, 20)) for _ in range(3))))
        obs.append([kind] + [fb(x) for x in rr + p + v])
    # vertices and lanes
    nv = rnd.randint(1, 9)
    vpos = [tuple(f32(rnd.choice((0.0, rnd.uniform(-900, 900)))) for _ in range(3)) for _ in range(nv)]
    nl = rnd.choice((0, 1, 2, 3, 5, 8, 12))
    lanes = []
    for l in range(nl):
        a, b = rnd.randrange(nv), rnd.randrange(nv)
        s, e = list(vpos[a]), list(vpos[b])
        for q in (s, e):
            if rnd.random() < 0.1:
                j = rnd.randrange(3)
                q[j] = rnd.choice((bf(fb(q[j]) + 1), -q[j], q[j] + den[0] if q[j] == 0.0 else bf(fb(q[j]) - 1), float('nan')))
        nc = rnd.randint(0, 30 if cap else 6)
        cars = [rnd.randrange(nobs) for _ in range(nc)]
        lanes.append([a, b] + [fb(x) for x in s + e] + [nc] + cars)
    vl = [[] for _ in range(nv)]
    for l, ln in enumerate(lanes):
        vl[ln[0]].append(l)
        if ln[1] != ln[0]:
            vl[ln[1]].append(l)
    for v in vl:
        rnd.shuffle(v)
        if nl and rnd.random() < 0.15:
            v.insert(rnd.randint(0, len(v)), rnd.randrange(nl))
    # the grid: nine lists, each object at most once
    pool = list(range(nobs))
    rnd.shuffle(pool)
    pool = pool[:rnd.randint(nobs // 2, nobs) if cap and rnd.random() < 0.5 else rnd.randint(0, nobs)]
    grid = [[] for _ in range(9)]
    for o in pool:
        grid[rnd.randrange(9)].append(o)

    def rail():
        k = rnd.random()
        if k < 0.15 or nl == 0:
            return rnd.choice((NONE, RUNTIME))
        return rnd.randrange(nl)
    cur, nxt, oth = rail(), rail(), rail()
    if cap and rnd.random() < 0.5:
        cur = RUNTIME
    if nl and isinstance(cur, int) and cur < nl and rnd.random() < 0.5:
        # the next lane starts at the current lane's end vertex
        fol = [l for l in range(nl) if lanes[l][0] == lanes[cur][1]]
        if fol:
            nxt = rnd.choice(fol)
    prev = rnd.choice((NONE, NONE, rnd.randrange(nobs), rnd.randrange(nobs)))
    prevb = rnd.choice((NONE, rnd.randrange(nobs)))
    lo = rnd.getrandbits(32)
    if rnd.random() < 0.3:
        # the draw on 0.025's edge: U23 = 209715 (0.024999976, kept? no) or 209716 (yes)
        new = (rnd.getrandbits(9) << 23) | rnd.choice((209714, 209715, 209716, 209717))
        carry = (new - A * lo) & 0xFFFFFFFF
    else:
        carry = rnd.getrandbits(32)
    words = [lo, carry, nobs, asker, prev, prevb, cur, nxt, oth, nl, nv]
    for o in obs:
        words += o
    for ln in lanes:
        words += ln
    for v in vl:
        words += [len(v)] + v
    for g in grid:
        words += [len(g)] + g
    return words


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn, cell, heads = (addr(a.tool, f) for f in ('F-47175', 'F-88455', 'F-99153'))
    rnd = random.Random(47175)
    cs = [gen(rnd) for _ in range(a.cases)]
    # memory: result 0x1000, context 0x1100, railnet 0x1200 (+8 -> root 0x1300, +56 -> table 0x1400), vertex lists
    # 0x1800 + 0x80 v, vtables 0x2000 (obstacle) / 0x2100 (rail), obstacles 0x10000 + 0x100 i (matrix +0x40, velocity
    # +0xC0), lanes 0x20000 + 0x100 l (start +0x20, end +0x30, car list +96, curve +0xA0: +28 = 1, +32 -> +0xD0 the two
    # vertex indices), the run-time rail 0x30000, list nodes 0x40000 + 16 k, entries 0x60000 + 16 k; stack below 0x7F000
    setup = r'''
static uint32_t OB(uint32_t i){return 0x10000+0x100*i;}
static uint32_t LA(uint32_t l){return 0x20000+0x100*l;}
static uint32_t RA(uint32_t r){return r==0xFFFFFFFFu?0:r==0xFFFFFFFEu?0x30000:LA(r);}
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static uint32_t get(uint32_t a){const uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|p[1]<<16|p[2]<<8|p[3];}
static uint32_t ix(uint32_t a){return a==0?0xFFFFFFFFu:(a-0x10000)/0x100;}
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){uint32_t t=ctx.r3.u32;switch(a){
case 0x9001: ctx.r3.u64=t+0x40;return; case 0x9002: ctx.r3.u64=t+0xC0;return;
case 0x9003: {uint32_t r1=ctx.r1.u32;int n=(int)load(r1+96);printf("g %d",n);for(int i=0;i<n;i++)printf(" %x",ix(load(r1+240+4*i)));
  store(r1+96,0);return;}
case 0x9012: ctx.r3.u64=t+0x20;return; case 0x9013: ctx.r3.u64=t+0x30;return; case 0x9015: ctx.r3.u64=t+0xA0;return;}
printf("bad call %08x\n",a);throw std::runtime_error("call");}
'''
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
put(0x82860950,0xFFFFFFFFu);put(0x82860954,0xFFFFFFFFu);put(0x82860958,0xFFFFFFFFu);put(0x8286095C,0);put(0x8286F1B4,0x5000);
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));size_t k=0;
uint32_t lo=w[k++],cy=w[k++],nobs=w[k++],asker=w[k++],prev=w[k++],prevb=w[k++],cur=w[k++],nxt=w[k++],oth=w[k++],nl=w[k++],nv=w[k++];
put(0x82860890,lo);put(0x82860894,cy);
store(0x2000+4,0x9001);store(0x2000+8,0x9002);store(0x2000+12,0x9003);
store(0x2100+48,0x9012);store(0x2100+52,0x9013);store(0x2100+60,0x9015);
for(uint32_t i=0;i<nobs;i++){uint32_t o=OB(i);store(o,0x2000);*pointer(o+6)=uint8_t(w[k++]);
  for(int j=0;j<3;j++)store(o+0x60+4*j,w[k++]);for(int j=0;j<3;j++)store(o+0x70+4*j,w[k++]);for(int j=0;j<3;j++)store(o+0xC0+4*j,w[k++]);}
uint32_t node=0x40000,ent=0x60000;
for(uint32_t l=0;l<nl;l++){uint32_t r=LA(l);store(r,0x2100);store(r+4,0);uint32_t a=w[k++],b=w[k++];
  for(int j=0;j<3;j++)store(r+0x20+4*j,w[k++]);for(int j=0;j<3;j++)store(r+0x30+4*j,w[k++]);
  store16(r+0xA0+28,1);store(r+0xA0+32,r+0xD0);store16(r+0xD0,a);store16(r+0xD2,b);
  uint32_t nc=w[k++],*link=nullptr;uint32_t prevn=0;
  for(uint32_t c=0;c<nc;c++){uint32_t o=OB(w[k++]);store(ent+12,o);store(node,ent);store(node+4,0);
    if(prevn)store(prevn+4,node);else store(r+96,node);prevn=node;node+=16;ent+=16;}}
store(0x30000,0x2100);store(0x30004,0x1800);
store(0x1200+8,0x1300);store(0x1300+56,0x1400);
for(uint32_t v=0;v<nv;v++){uint32_t n=w[k++];store(0x1400+8*v,0x1800+0x80*v);store16(0x1400+8*v+4,n);for(uint32_t j=0;j<n;j++)store(0x1800+0x80*v+4*j,LA(w[k++]));}
for(int g=0;g<9;g++){uint32_t n=w[k++];G[g]=0;uint32_t last=0;for(uint32_t j=0;j<n;j++){uint32_t o=OB(w[k++]);if(last)store(last+12,o);else G[g]=o;last=o;}}
store(0x1000,prev==0xFFFFFFFFu?0:OB(prev));store(0x1004,prevb==0xFFFFFFFFu?0:OB(prevb));
store(0x1100,0x1200);store(0x1104,OB(asker));store(0x1100+12,RA(cur));store(0x1100+16,RA(nxt));store(0x1100+20,RA(oth));
Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;bool g=false;
long pos=ftell(stdout);
sub_%s(ctx,nullptr);
uint32_t a2=get(0x82860890),c2=get(0x82860894);
printf(" %%04x_%%04x %%04x_%%04x %%x %%x\n",a2>>16,a2&0xFFFF,c2>>16,c2&0xFFFF,ix(load(0x1000)),ix(load(0x1004)));}
}
''' % fn
    stubs = {'sub_' + cell: 'ctx.r3.u64=0;', 'sub_' + heads: 'for(int i=0;i<9;i++)store(ctx.r5.u32+4*i,G[i]);'}
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], setup + body, a.out_dir, 'traffic_gather', stubs=stubs, vector=True, scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'traffic_gather_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    lines = []
    for c, o in zip(cs, out):
        o = o.strip()
        if not o.startswith('g'):
            o = 'k ' + o
        ws = ['%04x_%04x' % (x >> 16, x & 0xFFFF) if i < 2 else '%x' % x for i, x in enumerate(c)]
        line = ' '.join(ws) + ' -> ' + o
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'traffic_gather_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases,', sum(o.startswith('k') for o in (l.split(' -> ')[1] for l in lines)), 'kept,',
          sum(int(l.split(' -> ')[1].split()[1]) if l.split(' -> ')[1].startswith('g') else 0 for l in lines), 'candidates')


if __name__ == '__main__':
    main()
