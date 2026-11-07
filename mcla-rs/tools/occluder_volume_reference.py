"""The original's volume of one occluder run offline: F-62084 (with its callees F-88707 / F-78843 the polygon plane, F-47419's
pushed plane, the plane store, F-12644 the silhouette plane, the edge end getter and F-40381 the frustum reject) on the
occluders of city.occluder, after F-45813 has set each polygon's side directions (+0) as the loader does.

Memory as the loader leaves it: an occluder of 80 bytes (sphere +0, polygons +48 (16 bytes: side bits +0, count +5, vertex
indices +8 / edge indices +12 inline up to 4, else pointers), vertices +52 (16 bytes), edges +56 (2 signed bytes), the
version 4 float +60 = 1.0, counts +65 edges / +66 vertices / +67 polygons). The view (57,792 bytes) has its eye at +57696 and
its volume list at +0 (count +52224); its eight clip planes (+53456 ..) are zero, so no vertex is outside any side and the
frustum parts (reject, silhouette edges left out) never act: the comparison is of the planes. The globals as the city's init
F-20115 sets them: the push 0x82838724 = -5.0 and the range scale 0x827D2958 = 10.0.

Writes occluder_volume_ref.txt: "V <file index> <eye x y z> <used> <count> <planes: x y z d ...>" (f32 bit patterns, hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

# (vector mode ignores build's scratch_end: the flat scratch area widened here, and the signed byte / half registers
# the generated code reads)
_plain = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda: _plain().replace('objects[8192]', 'objects[%d]' % (0x20000 - 0x1000)).replace(
    'a<12288', 'a<%d' % 0x20000).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; uint8_t u8; int16_t s16; int8_t s8;', 1)

MAN, VIEW, OCC, POLYS, IDX, VERTS, EDGES, STACK = 0x1000, 0x4000, 0x13000, 0x13100, 0x13800, 0x14000, 0x14C00, 0x1F000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def parse(text):
    """The box occluders of the file: (file index, centre, radius, vertices, edges, polygons) - the text read as the port reads it."""
    t = iter(text.split())
    nxt = lambda: next(t)
    assert nxt() == 'version:' and nxt() == '4'
    assert nxt() == 'Occluders'
    n = int(nxt())
    assert nxt() == '{'
    out = []
    for i in range(n):
        assert nxt() == 'Occluder'
        nxt(); assert nxt() == '{'
        assert nxt() == 'Type'; kind = nxt()
        assert nxt() == 'Name'; nxt()
        assert nxt() == 'BoundingSphere'
        c = [nxt() for _ in range(4)]
        assert nxt() == 'Vertices'; nv = int(nxt()); assert nxt() == '{'
        verts = [[nxt() for _ in range(3)] for _ in range(nv)]
        assert nxt() == '}'
        assert nxt() == 'Edges'; ne = int(nxt()); assert nxt() == '{'
        edges = [(int(nxt()), int(nxt())) for _ in range(ne)]
        assert nxt() == '}'
        assert nxt() == 'Polygons'; np_ = int(nxt()); assert nxt() == '{'
        polys = []
        for _ in range(np_):
            assert nxt() == 'Vertices'; k = int(nxt()); assert nxt() == '{'
            pv = [int(nxt()) for _ in range(k)]; assert nxt() == '}'
            assert nxt() == 'Edges'; k = int(nxt()); assert nxt() == '{'
            pe = [int(nxt()) for _ in range(k)]; assert nxt() == '}'
            polys.append((pv, pe))
        assert nxt() == '}' and nxt() == '}'
        if kind == 'box' and all(3 <= len(pv) == len(pe) <= 32 for pv, pe in polys):
            out.append((i, c, verts, edges, polys))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--occluders', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    volume, sides = addr(a.tool, 'F-62084'), addr(a.tool, 'F-45813')
    occs = parse(a.occluders.read_text())
    rnd = random.Random(62084)
    words = []  # per case: header, then the occluder
    for (fi, c, verts, edges, polys) in occs:
        fv = [[float(x) for x in v] for v in verts]
        cx, cy, cz = (float(x) for x in c[:3])
        eyes = [(cx + rnd.uniform(-30, 30), cy + rnd.uniform(-10, 30), cz + rnd.uniform(-30, 30))]
        v = rnd.choice(fv)  # near a corner: polygons almost edge-on, the facing test's 1e-6
        eyes.append(tuple(v[k] + rnd.choice((-1, 1)) * rnd.choice((1e-3, 0.05, 0.5, 2.0)) * rnd.random() for k in range(3)))
        if fi % 7 == 0:
            eyes.append((cx, cy, cz))  # inside: nothing faces
        if fi % 11 == 0:
            r = 400.0 + rnd.choice((-0.01, 0.01, 30.0))  # about the range (40 m x 10)
            eyes.append((cx + r * 0.6, cy + 5.0, cz + r * 0.8))
        for e in eyes:
            w = [fi, *(fb(x) for x in e), len(fv), len(edges), len(polys)]
            w += [struct.unpack('<I', struct.pack('<f', float(x)))[0] for x in c]
            for p in verts:
                w += [struct.unpack('<I', struct.pack('<f', float(x)))[0] for x in p]
            for (s, t) in edges:
                w += [s, t]
            for pv, pe in polys:
                w += [len(pv), *pv, *pe]
            words.append(w)
    blob = b''.join(struct.pack('<I', len(w)) + struct.pack('<%dI' % len(w), *w) for w in words)
    src = r'''
static float fl(uint32_t b){float f;memcpy(&f,&b,4);return f;}
static void putf(uint32_t a,uint32_t b){store(a,b);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2],std::ios::binary);std::vector<uint8_t> raw((std::istreambuf_iterator<char>(in)),{});
 auto img=[&](uint32_t a,uint32_t v){store(a,v);};
 img(0x82838724,0xC0A00000);img(0x827D2958,0x41200000);
 size_t o=0;auto rd=[&](){uint32_t v;memcpy(&v,raw.data()+o,4);o+=4;return v;};
 while(o<raw.size()){uint32_t len=rd();(void)len;
  memset(objects,0,sizeof(objects));
  uint32_t fi=rd(),ex=rd(),ey=rd(),ez=rd(),nv=rd(),ne=rd(),np=rd();
  store(MAN,VIEW);
  store(VIEW+57696,ex);store(VIEW+57700,ey);store(VIEW+57704,ez);store(VIEW+57708,0);
  for(int k=0;k<4;k++)store(OCC+4*k,rd());
  store(OCC+48,POLYS);store(OCC+52,VERTS);store(OCC+56,EDGES);store(OCC+60,0x3F800000);
  *pointer(OCC+64)=1;*pointer(OCC+65)=uint8_t(ne);*pointer(OCC+66)=uint8_t(nv);*pointer(OCC+67)=uint8_t(np);
  for(uint32_t v=0;v<nv;v++){for(int k=0;k<3;k++)store(VERTS+16*v+4*k,rd());store(VERTS+16*v+12,0x3F800000);}
  for(uint32_t e=0;e<ne;e++){*pointer(EDGES+2*e)=uint8_t(rd());*pointer(EDGES+2*e+1)=uint8_t(rd());}
  uint32_t ix=IDX;
  for(uint32_t p=0;p<np;p++){uint32_t k=rd();uint32_t r=POLYS+16*p;*pointer(r+5)=uint8_t(k);
   uint32_t va=k>4?ix:r+8;if(k>4){store(r+8,ix);ix+=k;}
   for(uint32_t j=0;j<k;j++)*pointer(va+j)=uint8_t(rd());
   uint32_t ea=k>4?ix:r+12;if(k>4){store(r+12,ix);ix+=k;}
   for(uint32_t j=0;j<k;j++)*pointer(ea+j)=uint8_t(rd());
   ix=(ix+3)&~3u;}
  {Context ctx{};ctx.r1.u32=STACK;ctx.r3.u32=OCC;SIDES(ctx,nullptr);}
  Context ctx{};ctx.r1.u32=STACK;ctx.r3.u32=MAN;ctx.r4.u32=0;ctx.r5.u32=OCC;ctx.r6.u32=1;VOLUME(ctx,nullptr);
  uint32_t n=load(VIEW+276);
  printf("V %u %08x %08x %08x %u %u",fi,ex,ey,ez,ctx.r3.u32&0xFF,n);
  for(uint32_t i=0;i<n&&i<16;i++)for(int k=0;k<4;k++)printf(" %08x",load(VIEW+16*i+4*k));
  printf("\n");}
}
'''
    for k, v in dict(MAN=MAN, VIEW=VIEW, OCC=OCC, POLYS=POLYS, IDX=IDX, VERTS=VERTS, EDGES=EDGES, STACK=STACK).items():
        src = src.replace(k, '0x%X' % v)
    src = src.replace('SIDES', 'sub_' + sides).replace('VOLUME', 'sub_' + volume)
    src = '#include <cstring>\n#include <fstream>\n#include <vector>\n' + src
    exe = build(index(a.recomp), ['sub_' + volume, 'sub_' + sides], src, a.out, 'occluder_volume', vector=True,
                scratch_end=0x20000, flags=('-msse4.1',))
    inp = a.out / 'occluder_volume_cases.bin'
    inp.write_bytes(blob)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    a.ref.write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'cases of', len(occs), 'occluders')


if __name__ == '__main__':
    main()
