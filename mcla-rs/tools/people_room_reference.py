"""The people spawner's room test (F-70932, called by the people spawner with the tuning's fMinSpawnPedSpacing and
fMinSpawnPedTypeSpacing) run offline: the generated code whole and unchanged on flat guest memory, with the object
grid's cell code (F-88455 -> F-59709, F-99153) running as it is. The only call out is each object's slot 1 (its
matrix), answered with the object's place at +48.

Guest memory: the grid 0x1B00 (+0 w, +4 h, +8 count, +12 / +16 min, +20 / +24 max = min + 40 n, +28 the cell heads
0x1C00; [0x8286F1B4] points at it), the objects 0x2000 + 0x40 k (vtable 0x2E00, +6 kind, +12 next, +16 owner
0x6000 + 0x200 k whose +448 is the type 0x9F00 or another), their matrices 0x4000 + 0x40 k (+48 the place), inserted
in object order at the tail of the cell F-88455 gives for their place (none when off the grid); the point 0x1A00.

Writes <out-dir>/people_room_ref.txt, a case a line: "w h minx minz px py pz s1 s2 n | kind x y z same ... -> r"
(floats as bits; same 1: the owner's type is the spawner's; r the code's answer, 1 = no room)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

# vector mode (the cell code converts with simde); its shim takes the scratch size from here
recomp_closure.shim = lambda: recomp_leaf.shim(0x10000)

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                         'static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}\n')


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u & 0xFFFFFFFF))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def near(rnd, p, r):
    """A place about r away from p (flat), often a float step or two around the fused squared distance's edge."""
    import math
    a = rnd.uniform(0, 6.283)
    d = r * rnd.choice((1.0, 1.0, 0.5, 1.5, rnd.uniform(0, 2)))
    x, z = f32(p[0] + d * math.cos(a)), f32(p[2] + d * math.sin(a))
    if rnd.random() < 0.5:
        x = bf(fb(x) + rnd.randint(-2, 2)) if x != 0 else x
    return [x, f32(p[1] + rnd.choice((0.0, rnd.uniform(-30, 30)))), z]


def cases(rnd, count):
    out = []
    for c in range(count):
        w, h = rnd.randint(1, 8), rnd.randint(1, 8)
        minx, minz = f32(rnd.uniform(-400, 0)), f32(rnd.uniform(-400, 0))
        if rnd.random() < 0.05:
            p = [f32(minx - 1.0), 0.0, f32(minz + 5.0)]
        elif rnd.random() < 0.08:
            # on a cell's edge
            p = [f32(minx + 40.0 * rnd.randint(0, w)), f32(rnd.uniform(-5, 5)), f32(minz + 40.0 * rnd.randint(0, h - 1) + 3)]
        else:
            p = [f32(minx + rnd.uniform(0, 40.0 * w)), f32(rnd.uniform(-20, 40)), f32(minz + rnd.uniform(0, 40.0 * h))]
        s1 = f32(rnd.choice((1.0, 1.0, rnd.uniform(0.2, 5.0))))
        s2 = f32(rnd.choice((20.0, 20.0, rnd.uniform(5.0, 50.0), 0.0)))
        objs = []
        for _ in range(rnd.choice((0, 1, 2, 4, 8, 16, 24))):
            kind = rnd.choice((0, 1, 2, 3, 4, 4, 4, 5, 7))
            same = rnd.random() < 0.5
            r = s2 if (kind == 4 and same and rnd.random() < 0.7) else s1
            q = near(rnd, p, r) if rnd.random() < 0.8 else near(rnd, p, rnd.uniform(0, 90))
            objs.append([kind] + [fb(x) for x in q] + [int(same)])
        out.append([w, h, fb(minx), fb(minz)] + [fb(x) for x in p] + [fb(s1), fb(s2), len(objs)] + [x for o in objs for x in o])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    nm = {f: addr(a.tool, f) for f in ('F-70932', 'F-88455')}
    rows = cases(random.Random(70932), a.cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'people_room_inputs.bin'
    inp.write_bytes(b''.join(struct.pack('<%dI' % len(r), *r) for r in rows))
    nl = chr(10)
    NL = chr(92) + 'n'
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){' + nl +
            ' if(a==0x82FF0001){ctx.r3.u64=0x4000+0x40*((ctx.r3.u32-0x2000)/0x40);return;}' + nl +
            ' throw std::runtime_error("unexpected indirect call");}' + nl)
    body = ('FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);' + nl +
            'size_t i=0;while(i<in.size()){const uint32_t*c=in.data()+i;int n=int(c[9]);i+=10+5*n;' + nl +
            ' memset(objects,0,0xF000);const uint32_t G=0x1B00;' + nl +
            ' store(G,c[0]);store(G+4,c[1]);store(G+8,c[0]*c[1]);store(G+12,c[2]);store(G+16,c[3]);' + nl +
            ' putf(G+20,getf(G+12)+40.0f*float(c[0]));putf(G+24,getf(G+16)+40.0f*float(c[1]));store(G+28,0x1C00);' + nl +
            ' put((0x8287<<16)-3660,G);store(0x2E00+4,0x82FF0001);' + nl +
            ' for(int j=0;j<3;j++)store(0x1A00+4*j,c[4+j]);' + nl +
            ' for(int k=0;k<n;k++){const uint32_t*d=c+10+5*k;uint32_t o=0x2000+0x40*k,m=0x4000+0x40*k,ow=0x6000+0x200*k;' + nl +
            '  store(o,0x2E00);*pointer(o+6)=uint8_t(d[0]);store(o+16,ow);store(ow+448,d[4]?0x9F00:0x9E00);' + nl +
            '  for(int j=0;j<3;j++)store(m+48+4*j,d[1+j]);' + nl +
            '  Context g{};g.r1.u32=0xFF00;g.r3.u32=G;g.r4.u32=m+48;sub_' + nm['F-88455'] + '(g,nullptr);uint32_t cell=g.r3.u32&0xFFFF;' + nl +
            '  if(cell!=0xFFFF&&cell<load(G+8)){uint32_t hp=0x1C00+4*cell;while(load(hp))hp=load(hp)+12;store(hp,o);}}' + nl +
            ' Context ctx{};ctx.r1.u32=0xFF00;ctx.r3.u32=0x9F00;ctx.r4.u32=0x1A00;ctx.f1.f64=1.5;' + nl +
            ' {float s;uint32_t b=c[7];memcpy(&s,&b,4);ctx.f2.f64=s;b=c[8];memcpy(&s,&b,4);ctx.f3.f64=s;}' + nl +
            ' sub_' + nm['F-70932'] + '(ctx,nullptr);printf("%d' + NL + '",int(ctx.r3.u32&0xFF));}' + nl)
    main_src = (disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + nm['F-70932'], 'sub_' + nm['F-88455']], main_src, a.out_dir, 'people_room', vector=True)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    lines = []
    for row, o in zip(rows, out):
        n = row[9]
        head = ' '.join('%x' % x for x in row[:10])
        objs = ' '.join(' '.join('%x' % x for x in row[10 + 5 * k:15 + 5 * k]) for k in range(n))
        lines.append('%s | %s -> %s' % (head, objs, o))
    (a.out_dir / 'people_room_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o == '1' for o in out), 'no room,', sum(r[9] == 0 for r in rows), 'without objects')


if __name__ == '__main__':
    main()
