"""The start speed's car ahead / behind (F-90496) run offline: the generated code whole and unchanged on flat guest memory,
the rail-list copy F-83161 stubbed (writes the case's list for the rail asked and its count at +256). Memory: our car
0x1000 (+452 the behaviour 0x1800 or 0: +188 the rail 0x7100, +192 the next rail 0x7200 or 0, +208 our phase), the
cars 0x2000 + 0x100 i (+452 0x4000 + 0x100 i or 0, its +208 the phase), our car itself among them as index 15.
A case: has behaviour, has next rail, our phase, the first list (count, eight indices), the second (count, eight), then
16 cars: has behaviour, phase. Writes <out-dir>/start_neighbours_ref.txt: the case's words '->' the car ahead and the
car behind (index, or - for none)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x10000)
NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, copy = addr(a.tool, 'F-90496'), addr(a.tool, 'F-83161')
    rnd = random.Random(90496)
    odd = (0.0, -0.0, 1.0, 3.4028234663852886e+38, -3.4028234663852886e+38, float('inf'), float('-inf'), float('nan'))
    cs = []
    for _ in range(4000):
        ph = rnd.choice((rnd.random(), rnd.random(), 0.0, 0.5) + odd)
        phases = [rnd.choice((rnd.random(), rnd.random(), rnd.random(), ph, 0.5, 0.0) + (odd if rnd.random() < 0.1 else ())) for _ in range(16)]
        hasb = [int(rnd.random() < 0.9) for _ in range(16)]
        l1 = [rnd.randrange(16) for _ in range(8)]
        l2 = [rnd.randrange(16) for _ in range(8)]
        c = [int(rnd.random() < 0.95), int(rnd.random() < 0.7), fb(ph), rnd.randrange(9), *l1, rnd.randrange(9), *l2]
        for i in range(16):
            c += [hasb[i], fb(phases[i])]
        cs.append(c)
    W = len(cs[0])
    stubs = {'sub_' + copy: 'const uint32_t*l=ctx.r3.u32==0x7100?K+4:ctx.r3.u32==0x7200?K+13:nullptr;'
                            'if(!l){printf("badrail ");store(ctx.r4.u32+256,0);}else{uint32_t n=K[ctx.r3.u32==0x7100?3:12];'
                            'for(uint32_t i=0;i<n;i++)store(ctx.r4.u32+4*i,l[i]==15?0x1000:0x2000+0x100*l[i]);store(ctx.r4.u32+256,n);}'}
    rows = NL.join('{%s},' % ','.join('0x%08xu' % x for x in c) for c in cs)
    body = ('static const uint32_t C[][%d]={' % W + NL + rows + NL + '};' + NL +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + NL +
            'for(auto&c:C){memset(objects,0,sizeof(objects));K=c;'
            'if(c[0]){store(0x1000+452,0x1800);store(0x1800+188,0x7100);store(0x1800+192,c[1]?0x7200:0);store(0x1800+208,c[2]);}'
            'for(int i=0;i<15;i++){const uint32_t*w=c+21+2*i;uint32_t car=0x2000+0x100*i;'
            'if(w[0]){store(car+452,0x4000+0x100*i);store(0x4000+0x100*i+208,w[1]);}}'
            'store(0x3F00,0);store(0x3F04,0);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r3.u32=0x1000;ctx.r4.u32=0x3F00;ctx.r5.u32=0x3F04;'
            'sub_FN(ctx,nullptr);printf("A%x B%xEOL",load(0x3F00),load(0x3F04));}}' + NL).replace('FN', fn).replace('EOL', chr(92) + 'n')
    recomp_closure.EXTRA += 'static const uint32_t *K;' + NL
    exe = build(index(a.recomp), ['sub_' + fn], body, a.out_dir, 'start_neighbours', stubs=stubs, scratch_end=0x10000)
    out = [x for x in run(exe, a.image).split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs))

    def name(w):
        v = int(w[1:], 16)
        return '-' if v == 0 else 'f' if v == 0x1000 else '%x' % ((v - 0x2000) // 0x100)
    lines = []
    for c, o in zip(cs, out):
        assert 'badrail' not in o, o
        w = o.split()
        lines.append(' '.join('%x' % x for x in c) + ' -> ' + name(w[-2]) + ' ' + name(w[-1]))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'start_neighbours_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
