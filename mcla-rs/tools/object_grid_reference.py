"""The object grid's cell lists run offline: the insert / move F-66511 (with the cell F-59709) and the unlink F-60332, as
the ambient cars' F-27899, the racers' F-80793 and the props call them. The unchanged generated code on flat guest memory:
a grid (+0 cells across, +8 cell count, +12 / +16 min x / z, +20 / +24 max x / z, +28 the cell heads) and 8 obstacles
(+0 vtable, its slot 1 a stub giving the object's matrix, +4 the u16 cell, 0xFFFF as the ctor leaves it, +8 prev,
+12 next). Each case: a grid (small, or 300 x 300 so that cell numbers wrap at 16 bits), then 60 operations, each a filing
of object k at (x, z) or its unlink; places on cell borders (exact multiples of 40 m from the min), in cell 0, off the grid
and at random. Writes <out-dir>/object_grid_ref.txt: per case a line "G nx nz minx minz" (floats as hex bits), the
operations "F k x z" / "U k", and "-> " the lists after the last one: "cell:k,k,.." from each head, by cell number."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n')

NOBJ, NOPS = 8, 60


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
    ap.add_argument('--cases', type=int, default=600)
    a = ap.parse_args()
    ins, unl, cel = (addr(a.tool, f) for f in ('F-66511', 'F-60332', 'F-59709'))
    rnd = random.Random(66)
    cases = []
    for c in range(a.cases):
        if c % 20 == 19:
            nx, nz = 300, 300
        else:
            nx, nz = rnd.randint(1, 6), rnd.randint(1, 6)
        mx, mz = rnd.choice((0.0, -40.0, -120.0, -3000.0, 80.0)), rnd.choice((0.0, -40.0, -200.0, -2960.0, 40.0))
        ops = []
        for _ in range(NOPS):
            k = rnd.randrange(NOBJ)
            if rnd.random() < 0.08:
                ops.append(('U', k))
                continue

            def coord(m, n):
                r = rnd.random()
                if r < 0.3:
                    return m + 40.0 * rnd.randint(-1, n + 1)  # a border
                if r < 0.4:
                    return m + rnd.uniform(0, 40)  # the first row / column (cell 0 when both)
                if r < 0.5:
                    return rnd.choice((m - 0.001, m + 40.0 * n, m + 40.0 * n - 0.001, m - 500.0))
                return m + rnd.uniform(-20, 40.0 * n + 20)
            if nx == 300 and rnd.random() < 0.5:
                # (past cell 65,536: z rows 219 and up)
                x, z = mx + rnd.uniform(0, 40.0 * nx), mz + 40.0 * rnd.uniform(218, 300)
            else:
                x, z = coord(mx, nx), coord(mz, nz)
            ops.append(('F', k, x, z))
        cases.append((nx, nz, mx, mz, ops))
    nl = chr(10)
    NL = chr(92) + 'n'
    # one row per case: nx nz minx minz, then NOPS x (op, k, x, z) with op 1 = file, 2 = unlink
    rows = []
    for nx, nz, mx, mz, ops in cases:
        w = [nx, nz, fb(mx), fb(mz)]
        for o in ops:
            w += [1, o[1], fb(o[2]), fb(o[3])] if o[0] == 'F' else [2, o[1], 0, 0]
        rows.append('{%s},' % ','.join('0x%08x' % x for x in w))
    NF = 4 + 4 * NOPS
    s_vf = 0x82FF0100
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x%x){ctx.r3.u64=0x1800+((ctx.r3.u32-0x1100)/32)*64;}'
            'else throw std::runtime_error("unexpected indirect call");}' % s_vf)
    # grid 0x1000 (heads at 0x4000), obstacles 0x1100 + 32 k (vtable 0x1080), matrices 0x1800 + 64 k (+48 the place)
    body = ('static const uint32_t C[][%d]={' % NF + nl + nl.join(rows) + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));uint32_t nx=c[0],nz=c[1];float mx,mz;memcpy(&mx,&c[2],4);memcpy(&mz,&c[3],4);'
            'store(0x1000,nx);store(0x1008,nx*nz);putf(0x100C,mx);putf(0x1010,mz);putf(0x1014,mx+40.0f*nx);putf(0x1018,mz+40.0f*nz);'
            'store(0x101C,0x4000);store(0x1084,0x%x);' % s_vf +
            'for(int k=0;k<%d;k++){store(0x1100+32*k,0x1080);store16(0x1100+32*k+4,0xFFFF);}' % NOBJ +
            'for(int i=0;i<%d;i++){const uint32_t*o=c+4+4*i;uint32_t ob=0x1100+32*o[1];' % NOPS +
            'if(o[0]==1){store(0x1800+64*o[1]+48,o[2]);store(0x1800+64*o[1]+52,0);store(0x1800+64*o[1]+56,o[3]);'
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=ob;ctx.r1.u32=0x3F00;sub_%s(ctx,nullptr);}' % ins +
            'else{Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=ob;ctx.r1.u32=0x3F00;sub_%s(ctx,nullptr);}}' % unl +
            'printf("->");for(uint32_t q=0;q<nx*nz;q++){uint32_t h=load(0x4000+4*q);if(!h)continue;printf(" %u:",q);int first=1;'
            'int guard=0;while(h&&guard++<64){printf(first?"%u":",%u",(h-0x1100)/32);first=0;h=load(h+12);}}' +
            ('printf("%s");}' % NL) + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ins, 'sub_' + unl, 'sub_' + cel], main_src, a.out_dir, 'object_grid', vector=True, scratch_end=0x4000 + 4 * 90000 + 0x100)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (nx, nz, mx, mz, ops), o in zip(cases, out):
        lines.append('G %d %d %08x %08x' % (nx, nz, fb(mx), fb(mz)))
        lines += ['F %d %08x %08x' % (op[1], fb(op[2]), fb(op[3])) if op[0] == 'F' else 'U %d' % op[1] for op in ops]
        lines.append(o)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'object_grid_ref.txt').write_text(nl.join(lines) + nl)
    linked = sum(o.count(':') for o in out)
    print(len(out), 'cases', linked, 'non-empty cells at the end')


if __name__ == '__main__':
    main()
