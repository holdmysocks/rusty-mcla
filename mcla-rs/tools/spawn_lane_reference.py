"""The traffic spawner's lane pick F-10273 run offline: the generated code whole and unchanged (with the record walks
F-95725 / F-12622 in its closure) on flat guest memory, r5 = 0 (no direction) as the spawner passes it. The road record
and its tree are the case's: a record 0x1000 + 0x40 k (+12 its child, +16 its sibling, +20 its lanes, u16 +24 their
count), the root's sibling filled too (its lanes are not the root's). The lanes' slot 18 (+72, the race clearing's hold:
the lane's byte) and slot 20 (+80, its length) answer from the case through their vtable. The shared MWC (0x82860890)
is the case's. The words above the stack's start are FLT_MAX (the pick's loop reads past the weights when the rounding
leaves u above them all; the result is clamped to the last lane either way).

Writes <out-dir>/spawn_lane_ref.txt, a case a line: the MWC (16-bit halves), n, then per candidate lane in the walk's
order (own, then the child's subtree: own, sibling's, child's) its id, held, length bits; '->' the lane id picked
(ffffffff none), the phase's bits and the MWC after (16-bit halves)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x80000)
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')

NL = chr(10)
A = 0x5CDCFAA7


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u & 0xFFFFFFFF))[0]


def f32(x):
    return bf(fb(x))


def gen(rnd):
    # the tree: up to 8 records, each with 0..6 lanes; record 0 the root
    nrec = rnd.choice((1, 1, 2, 3, 4, 6, 8))
    recs = [{'lanes': [], 'child': None, 'sib': None} for _ in range(nrec)]
    for k in range(1, nrec):
        # hang record k under an earlier one: as its child or its sibling (the root's sibling: outside the road)
        p = rnd.randrange(k)
        slot = rnd.choice(('child', 'sib'))
        while recs[p][slot] is not None:
            p = recs[p][slot]
        recs[p][slot] = k
    nl = 0
    lanes = []
    for r in recs:
        for _ in range(rnd.choice((0, 1, 1, 2, 3, 4, 6))):
            held = int(rnd.random() < 0.12)
            ln = rnd.choice((f32(rnd.uniform(5, 400)), f32(rnd.uniform(5, 400)), f32(rnd.uniform(0.5, 30)), 0.0, f32(1e-3),
                             f32(rnd.uniform(1e5, 1e7)), bf(1), f32(100.0)))
            lanes.append((held, fb(ln)))
            r['lanes'].append(nl)
            nl += 1
    order = []

    def walk(k, top):
        order.extend(recs[k]['lanes'])
        if not top and recs[k]['sib'] is not None:
            walk(recs[k]['sib'], False)
        if recs[k]['child'] is not None:
            walk(recs[k]['child'], False)
    walk(0, True)
    lo = rnd.getrandbits(32)
    new = rnd.getrandbits(32)
    if order and rnd.random() < 0.5:
        # U23 at a weight boundary: (U23 x total) x 2^-23 on a running sum, a step either side
        w = [0.0 if lanes[l][0] else bf(lanes[l][1]) for l in order]
        tot = 0.0
        for x in w:
            tot = f32(x + tot)
        if tot > 0 and tot == tot and tot < 3e38:
            cum = 0.0
            cut = rnd.randrange(len(w) + 1)
            for x in w[:cut]:
                cum += x
            u = int(cum / tot * 8388608) + rnd.choice((-1, 0, 0, 1, 2))
            u = max(0, min(0x7FFFFF, u))
            new = (rnd.getrandbits(9) << 23) | u
    elif rnd.random() < 0.1:
        new = (rnd.getrandbits(9) << 23) | rnd.choice((0, 0x7FFFFF, 0x7FFFFE))
    carry = (new - A * lo) & 0xFFFFFFFF
    return recs, lanes, order, lo, carry


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=5000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-10273')
    rnd = random.Random(10273)
    cs = [gen(rnd) for _ in range(a.cases)]
    # the input: lo carry nrec, per record child sib n lanes..., nl, per lane held len
    rows = []
    for recs, lanes, order, lo, carry in cs:
        w = [lo, carry, len(recs)]
        for r in recs:
            w += [0xFFFFFFFF if r['child'] is None else r['child'], 0xFFFFFFFF if r['sib'] is None else r['sib'], len(r['lanes'])] + r['lanes']
        w += [len(lanes)]
        for h, ln in lanes:
            w += [h, ln]
        rows.append(' '.join('%x' % x for x in w))
    src = r'''
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static uint32_t get(uint32_t a){const uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|p[1]<<16|p[2]<<8|p[3];}
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){uint32_t t=ctx.r3.u32;switch(a){
case 0x9018: ctx.r3.u64=*pointer(t+4);return;
case 0x9020: ctx.f1.f64=double(getf(t+8));return;}
printf("bad call %%08x\n",a);throw std::runtime_error("call");}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));size_t k=0;
for(uint32_t q=0x7F000;q<0x80000;q+=4)store(q,0x7F7FFFFFu);
uint32_t lo=w[k++],cy=w[k++],nrec=w[k++];put(0x82860890,lo);put(0x82860894,cy);
store(0x3000+72,0x9018);store(0x3000+80,0x9020);
for(uint32_t r=0;r<nrec;r++){uint32_t o=0x1000+0x40*r;uint32_t ch=w[k++],sb=w[k++],n=w[k++];
  store(o+12,ch==0xFFFFFFFFu?0:0x1000+0x40*ch);store(o+16,sb==0xFFFFFFFFu?0:0x1000+0x40*sb);store(o+20,0x8000+0x80*r);store16(o+24,n);
  for(uint32_t j=0;j<n;j++)store(0x8000+0x80*r+4*j,0x10000+0x40*w[k++]);}
uint32_t nl=w[k++];for(uint32_t l=0;l<nl;l++){uint32_t o=0x10000+0x40*l;store(o,0x3000);*pointer(o+4)=uint8_t(w[k++]);store(o+8,w[k++]);}
store(0x7000,0xDEADBEEFu);
Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=0x7000;ctx.r5.u32=0;
sub_%s(ctx,nullptr);
uint32_t l=ctx.r3.u32?(ctx.r3.u32-0x10000)/0x40:0xFFFFFFFFu;uint32_t a2=get(0x82860890),c2=get(0x82860894);
printf("%%x %%x %%04x_%%04x %%04x_%%04x\n",l,load(0x7000),a2>>16,a2&0xFFFF,c2>>16,c2&0xFFFF);}
}
''' % fn
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], src, a.out_dir, 'spawn_lane', scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'spawn_lane_in.txt'
    inp.write_text(NL.join(rows) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    lines = []
    for (recs, lanes, order, lo, carry), o in zip(cs, out):
        ws = ['%04x_%04x' % (lo >> 16, lo & 0xFFFF), '%04x_%04x' % (carry >> 16, carry & 0xFFFF), '%x' % len(order)]
        for l in order:
            ws += ['%x' % l, '%x' % lanes[l][0], '%x' % lanes[l][1]]
        line = ' '.join(ws) + ' -> ' + o
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'spawn_lane_ref.txt').write_text(NL.join(lines) + NL)
    picks = [o.split()[0] for o in out]
    print(len(lines), 'cases,', sum(p == 'ffffffff' for p in picks), 'without lanes')


if __name__ == '__main__':
    main()
