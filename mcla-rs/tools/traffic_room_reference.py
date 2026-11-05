"""The traffic spawner's room test (F-43944, called by the traffic spawner F-92144 with the type's bound word +0x0C and
the tuning's fMinSpawnTrafficSpacing) run offline: the generated code whole and unchanged on flat guest memory, with
the object grid's cell code (F-88455 -> F-59709, F-99153), the motion test F-24126 and the segment-sphere test F-47913
running as they are. The only calls out are each object's slots: 1 (its matrix, the place at +48), 2 (its velocity)
and 7 (its extent, f1).

Guest memory: the grid 0x1B00 (as tools/people_room_reference.py; the image's grid pointer word points at it), the objects 0x2000 +
0x40 k (vtable 0x2E00, +6 kind, +12 next, +32 the extent), their matrices 0x4000 + 0x40 k (+48 the place) and
velocities 0x6000 + 0x10 k, inserted in object order at the tail of the cell F-88455 gives for their place; the point
0x1A00 (its w a junk word: the code replaces it).

Writes <out-dir>/traffic_room_ref.txt, a case a line: "w h minx minz px py pz bound spacing n | kind x y z vx vy vz
ext ... -> r" (floats as bits; r the code's answer, 1 = no room)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

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


def jig(rnd, x):
    """x or a float step or two away."""
    if x != 0 and rnd.random() < 0.5:
        return bf(fb(x) + rnd.randint(-2, 2))
    return x


def cases(rnd, count):
    out = []
    for c in range(count):
        w, h = rnd.randint(1, 8), rnd.randint(1, 8)
        minx, minz = f32(rnd.uniform(-400, 0)), f32(rnd.uniform(-400, 0))
        if rnd.random() < 0.05:
            p = [f32(minx - 1.0), 0.0, f32(minz + 5.0)]
        elif rnd.random() < 0.08:
            p = [f32(minx + 40.0 * rnd.randint(0, w)), f32(rnd.uniform(-5, 5)), f32(minz + 40.0 * rnd.randint(0, h - 1) + 3)]
        else:
            p = [f32(minx + rnd.uniform(0, 40.0 * w)), f32(rnd.uniform(-20, 40)), f32(minz + rnd.uniform(0, 40.0 * h))]
        bound = f32(rnd.choice((2.8193, 3.3495, 8.1164, rnd.uniform(0.5, 9.0), 0.0)))
        spacing = f32(rnd.choice((15.0, 15.0, rnd.uniform(1.0, 30.0), 0.0)))
        objs = []
        for _ in range(rnd.choice((0, 1, 2, 4, 8, 16))):
            kind = rnd.choice((0, 1, 1, 2, 3, 4, 7))
            ext = f32(rnd.choice((2.4, 2.6, rnd.uniform(0.0, 6.0))))
            r = ext + bound
            a = rnd.uniform(0, 6.283)
            mode = rnd.random()
            if mode < 0.2:
                # about the flat spacing's edge
                d = spacing * rnd.choice((1.0, 1.0, 0.5, 1.5))
                q = [jig(rnd, f32(p[0] + d * math.cos(a))), f32(p[1] + rnd.uniform(-3, 3)), jig(rnd, f32(p[2] + d * math.sin(a)))]
                v = [f32(rnd.uniform(-20, 20)) for _ in range(3)]
            elif mode < 0.85:
                # out of the spacing, its motion about the sphere: aimed at it, past it, short of it, or starting on its edge
                d = max(spacing, r) * rnd.uniform(1.0, 2.5) + rnd.choice((0.0, 0.0, 5.0, 30.0))
                q = [f32(p[0] + d * math.cos(a)), f32(p[1] + rnd.uniform(-r, r) * rnd.choice((0.0, 0.5, 1.0))), f32(p[2] + d * math.sin(a))]
                to = [p[k] - q[k] for k in range(3)]
                k = rnd.choice((1.0, (d - r) / d, (d - r) / d, (d + r) / d, 2.0, 0.5, rnd.uniform(0, 2.5)))
                side = rnd.choice((0.0, 0.0, r, r * 0.99, r * 1.01, rnd.uniform(0, 2 * r)))
                v = [f32(to[0] * k - math.sin(a) * side), f32(to[1] * k), f32(to[2] * k + math.cos(a) * side)]
                v = [jig(rnd, x) for x in v]
                if rnd.random() < 0.05:
                    v = [0.0, 0.0, 0.0]
            else:
                d = rnd.uniform(0, 90)
                q = [f32(p[0] + d * math.cos(a)), f32(p[1] + rnd.uniform(-30, 30)), f32(p[2] + d * math.sin(a))]
                v = [f32(rnd.uniform(-60, 60)) for _ in range(3)]
            objs.append([kind] + [fb(x) for x in q] + [fb(x) for x in v] + [fb(ext)])
        out.append([w, h, fb(minx), fb(minz)] + [fb(x) for x in p] + [fb(bound), fb(spacing), len(objs)] + [x for o in objs for x in o])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    nm = {f: addr(a.tool, f) for f in ('F-43944', 'F-88455')}
    rows = cases(random.Random(43944), a.cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'traffic_room_inputs.bin'
    inp.write_bytes(b''.join(struct.pack('<%dI' % len(r), *r) for r in rows))
    nl = chr(10)
    NL = chr(92) + 'n'
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){' + nl +
            ' uint32_t k=(ctx.r3.u32-0x2000)/0x40;' + nl +
            ' if(a==0x82FF0001){ctx.r3.u64=0x4000+0x40*k;return;}' + nl +
            ' if(a==0x82FF0002){ctx.r3.u64=0x6000+0x10*k;return;}' + nl +
            ' if(a==0x82FF0007){ctx.f1.f64=getf(ctx.r3.u32+32);return;}' + nl +
            ' throw std::runtime_error("unexpected indirect call");}' + nl)
    body = ('FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);' + nl +
            'size_t i=0;while(i<in.size()){const uint32_t*c=in.data()+i;int n=int(c[9]);i+=10+8*n;' + nl +
            ' memset(objects,0,0xF000);const uint32_t G=0x1B00;' + nl +
            ' store(G,c[0]);store(G+4,c[1]);store(G+8,c[0]*c[1]);store(G+12,c[2]);store(G+16,c[3]);' + nl +
            ' putf(G+20,getf(G+12)+40.0f*float(c[0]));putf(G+24,getf(G+16)+40.0f*float(c[1]));store(G+28,0x1C00);' + nl +
            ' put((0x8287<<16)-3660,G);store(0x2E00+4,0x82FF0001);store(0x2E00+8,0x82FF0002);store(0x2E00+28,0x82FF0007);' + nl +
            ' for(int j=0;j<3;j++)store(0x1A00+4*j,c[4+j]);store(0x1A00+12,0x7F800001u);' + nl +
            ' for(int k=0;k<n;k++){const uint32_t*d=c+10+8*k;uint32_t o=0x2000+0x40*k,m=0x4000+0x40*k,v=0x6000+0x10*k;' + nl +
            '  store(o,0x2E00);*pointer(o+6)=uint8_t(d[0]);store(o+32,d[7]);' + nl +
            '  for(int j=0;j<3;j++){store(m+48+4*j,d[1+j]);store(v+4*j,d[4+j]);}' + nl +
            '  Context g{};g.r1.u32=0xFF00;g.r3.u32=G;g.r4.u32=m+48;sub_' + nm['F-88455'] + '(g,nullptr);uint32_t cell=g.r3.u32&0xFFFF;' + nl +
            '  if(cell!=0xFFFF&&cell<load(G+8)){uint32_t hp=0x1C00+4*cell;while(load(hp))hp=load(hp)+12;store(hp,o);}}' + nl +
            ' Context ctx{};ctx.r1.u32=0xFF00;ctx.r3.u32=0x1A00;' + nl +
            ' {float s;uint32_t b=c[7];memcpy(&s,&b,4);ctx.f1.f64=s;b=c[8];memcpy(&s,&b,4);ctx.f2.f64=s;}' + nl +
            ' sub_' + nm['F-43944'] + '(ctx,nullptr);printf("%d' + NL + '",int(ctx.r3.u32&0xFF));}' + nl)
    main_src = (disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + nm['F-43944'], 'sub_' + nm['F-88455']], main_src, a.out_dir, 'traffic_room', vector=True)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    lines = []
    for row, o in zip(rows, out):
        n = row[9]
        head = ' '.join('%x' % x for x in row[:10])
        objs = ' '.join(' '.join('%x' % x for x in row[10 + 8 * k:18 + 8 * k]) for k in range(n))
        lines.append('%s | %s -> %s' % (head, objs, o))
    (a.out_dir / 'traffic_room_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o == '1' for o in out), 'no room,', sum(r[9] == 0 for r in rows), 'without objects')


if __name__ == '__main__':
    main()
