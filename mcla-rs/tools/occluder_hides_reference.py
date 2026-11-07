"""The original's box test against the occluder volumes, run offline: for an eye and a few occluders of city.occluder,
F-45813 (side directions) and F-62084 (the volume) build the view's volume list as in occluder_volume_reference.py, then
F-54648 builds the plane sets the sector tests use (set 0 the frustum, from the view's clip planes, all zero here, so it
never rejects; then F-26279's copy of each volume still switched on, F-32005's corner indices, F-79439's padding), and for
each box F-96918 makes the eight corners and F-60582 gives 0 (hidden or outside) or 255 (seen);
F-92840, the draw's test, given the same sets as a list (no frustum part), must give the same for every box.
vmaxfp is run as the console does it (NaN in, NaN out), not as the recomp emits it (SSE max_ps): see VMAXFP below.

Writes occluder_hides_ref.txt, one line per case:
"H <eye x y z> <file indices, comma separated> <volumes> <sets> <boxes: min x y z max x y z ...> <results: 0/1 per box>",
eye as f32 bit patterns (hex), boxes as decimals (multiples of 1/16, exact in f32), result 1 = hidden."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build
from occluder_volume_reference import parse, addr, fb

# (the widened scratch and the signed byte / half registers: set by occluder_volume_reference on import)
# and the condition register's vector compare (the SDK's Condition::setFromMask) with its summary-overflow bit, as
# instance_cull_reference.py; vslw128 (F-32005) is the AVX2 per-lane shift in the recomp's intrinsics
import recomp_closure
_vs = recomp_closure.vector_shim

# vmaxfp as the console's VMX unit does it: NaN when either input is NaN. The recomp emits SSE max_ps, which gives the
# second input then; that is the recomp's, not the original's, and decides F-60582's result for a volume with a NaN plane
# (a polygon of no area): on the console a NaN in a group of four planes makes the whole group count as behind.
VMAXFP = chr(10) + (
    'static inline simde__m128 vmx_max_ps(simde__m128 a, simde__m128 b) { simde__m128 u = simde_mm_cmpunord_ps(a, b);'
    ' return simde_mm_or_ps(simde_mm_andnot_ps(u, simde_mm_max_ps(a, b)), simde_mm_and_ps(u, simde_mm_add_ps(a, b))); }'
    + chr(10) + '#define simde_mm_max_ps vmx_max_ps' + chr(10))


def _shim():
    s = '#include <simde/x86/avx2.h>' + chr(10) + _vs()
    old = 'struct Condition {' + chr(10) + '    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    return s.replace(old, 'struct Condition {' + chr(10) + '    bool so{}, eq{}, lt{}, gt{};' + chr(10) +
                     '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1) + VMAXFP


recomp_closure.vector_shim = _shim

MAN, VIEW, OCC, POLYS, IDX, VERTS, EDGES, SETS, CORNERS, STACK = (0x1000, 0x4000, 0x13000, 0x13100, 0x13800, 0x14000,
                                                                   0x14C00, 0x15000, 0x18400, 0x1F000)


def q(x):
    """x rounded to 1/16 (exact in f32 for the city's coordinates)."""
    return round(x * 16.0) / 16.0


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--occluders', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--every', type=int, default=4)
    a = ap.parse_args()
    fns = {k: addr(a.tool, k) for k in ('F-62084', 'F-45813', 'F-54648', 'F-96918', 'F-60582', 'F-92840')}
    occs = parse(a.occluders.read_text())
    rnd = random.Random(60582)
    centres = [tuple(float(x) for x in c[:3]) for (_, c, _, _, _) in occs]
    words, heads = [], []
    for n, (fi, c, verts, edges, polys) in enumerate(occs):
        if n % a.every:
            continue
        cx, cy, cz = centres[n]
        d2 = lambda m: sum((centres[m][k] - centres[n][k]) ** 2 for k in range(3))
        group = [n] + sorted((m for m in range(len(occs)) if m != n), key=d2)[:rnd.choice((0, 1, 3, 5))]
        fv = [[float(x) for x in v] for v in verts]
        v = rnd.choice(fv)
        eye = rnd.choice([(cx + rnd.uniform(-40, 40), cy + rnd.uniform(-10, 30), cz + rnd.uniform(-40, 40)),
                          tuple(v[k] + rnd.choice((-1, 1)) * rnd.choice((0.05, 0.5, 2.0, 8.0)) * rnd.random() for k in range(3))])
        boxes = []
        for _ in range(16):
            # behind the occluder seen from the eye, about it, or straddling one of its corners
            kind = rnd.random()
            if kind < 0.6:
                t = rnd.uniform(1.0, 4.0)
                m = [eye[k] + (centres[n][k] - eye[k]) * t + rnd.uniform(-8, 8) for k in range(3)]
            else:
                w = rnd.choice(fv)
                m = [w[k] + (w[k] - eye[k]) * rnd.uniform(0.0, 1.5) + rnd.uniform(-2, 2) for k in range(3)]
            h = [rnd.choice((0.25, 1.0, 4.0, 15.0)) * rnd.random() for _ in range(3)]
            lo = [q(m[k] - h[k]) for k in range(3)]
            hi = [q(m[k] + h[k]) for k in range(3)]
            hi = [max(hi[k], lo[k]) for k in range(3)]
            boxes.append(lo + hi)
        w = [fb(eye[0]), fb(eye[1]), fb(eye[2]), len(group)]
        for m in group:
            fi2, c2, verts2, edges2, polys2 = occs[m]
            w += [len(verts2), len(edges2), len(polys2)]
            w += [fb(float(x)) for x in c2]
            for p in verts2:
                w += [fb(float(x)) for x in p]
            for (s, t) in edges2:
                w += [s & 0xFF, t & 0xFF]
            for pv, pe in polys2:
                w += [len(pv), *pv, *pe]
        w += [len(boxes)]
        for b in boxes:
            w += [fb(x) for x in b]
        words.append(w)
        heads.append((eye, [occs[m][0] for m in group], boxes))
    blob = b''.join(struct.pack('<I', len(w)) + struct.pack('<%dI' % len(w), *w) for w in words)
    src = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2],std::ios::binary);std::vector<uint8_t> raw((std::istreambuf_iterator<char>(in)),{});
 store(0x82838724,0xC0A00000);store(0x827D2958,0x41200000);
 size_t o=0;auto rd=[&](){uint32_t v;memcpy(&v,raw.data()+o,4);o+=4;return v;};
 while(o<raw.size()){rd();
  memset(objects,0,sizeof(objects));
  uint32_t ex=rd(),ey=rd(),ez=rd(),ng=rd();
  store(MAN,VIEW);
  store(VIEW+57696,ex);store(VIEW+57700,ey);store(VIEW+57704,ez);store(VIEW+57708,0);
  for(uint32_t g=0;g<ng;g++){
   for(uint32_t a=OCC;a<SETS;a+=4)store(a,0);
   uint32_t nv=rd(),ne=rd(),np=rd();
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
   {Context ctx{};ctx.r1.u32=STACK;ctx.r3.u32=MAN;ctx.r4.u32=0;ctx.r5.u32=OCC;ctx.r6.u32=1;VOLUME(ctx,nullptr);}
  }
  uint32_t nvol=load(VIEW+52224);
  uint32_t nsets;{Context ctx{};ctx.r1.u32=STACK;ctx.r3.u32=VIEW;ctx.r4.u32=SETS;ctx.r5.u32=32;BUILD(ctx,nullptr);nsets=ctx.r3.u32;}
  uint32_t nb=rd();
  printf("H %u %u ",nvol,nsets);
  for(uint32_t b=0;b<nb;b++){uint32_t bx[6];for(int k=0;k<6;k++)bx[k]=rd();
   Context ctx{};ctx.r1.u32=STACK;ctx.r3.u32=CORNERS;
   for(int k=0;k<3;k++){ctx.v1.u32[3-k]=bx[k];ctx.v2.u32[3-k]=bx[3+k];}ctx.v1.u32[0]=0;ctx.v2.u32[0]=0;  // (lanes reversed: element 0 is u32[3])
   CORNERS_FN(ctx,nullptr);
   Context t{};t.r1.u32=STACK;t.r3.u32=CORNERS;t.r4.u32=SETS;t.r5.u32=nsets;TEST(t,nullptr);
   printf("%c",(t.r3.u32&0xFF)==0?'1':'0');
   // F-92840 on the same sets, given as the list 1 .. sets - 1 (r8 = 0: no frustum part)
   for(uint32_t k=1;k<nsets;k++)store(0x18500+4*(k-1),k);
   Context u{};u.r1.u32=STACK;u.r3.u32=0x18500;u.r4.u32=nsets-1;u.r5.u32=CORNERS;u.r6.u32=SETS;u.r8.u32=0;DRAW(u,nullptr);
   if((u.r3.u32&0xFF)!=(t.r3.u32&0xFF)){fputs("F-92840 differs from F-60582",stderr);return 1;}}
  printf("\n");}
}
'''
    for k, v in dict(MAN=MAN, VIEW=VIEW, OCC=OCC, POLYS=POLYS, IDX=IDX, VERTS=VERTS, EDGES=EDGES, SETS=SETS,
                     STACK=STACK).items():
        src = src.replace(k, '0x%X' % v)
    src = src.replace('CORNERS_FN', 'sub_' + fns['F-96918']).replace('CORNERS', '0x%X' % CORNERS)
    src = src.replace('SIDES', 'sub_' + fns['F-45813']).replace('VOLUME', 'sub_' + fns['F-62084'])
    src = src.replace('BUILD', 'sub_' + fns['F-54648']).replace('TEST', 'sub_' + fns['F-60582']).replace('DRAW', 'sub_' + fns['F-92840'])
    src = '#include <cstring>\n#include <fstream>\n#include <vector>\n' + src
    exe = build(index(a.recomp), ['sub_' + fns[k] for k in fns], src, a.out, 'occluder_hides', vector=True,
                scratch_end=0x20000, flags=('-msse4.1',))
    inp = a.out / 'occluder_hides_cases.bin'
    inp.write_bytes(blob)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = []
    for (eye, fis, boxes), line in zip(heads, r.stdout.splitlines()):
        _, nvol, nsets, bits = line.split()
        out.append('H %08x %08x %08x %s %s %s %s %s' % (fb(eye[0]), fb(eye[1]), fb(eye[2]), ','.join(map(str, fis)),
                                                       nvol, nsets, ' '.join(repr(x) for b in boxes for x in b), bits))
    a.ref.write_text('\n'.join(out) + '\n')
    hidden = sum(l.split()[-1].count('1') for l in out)
    print(len(out), 'cases,', hidden, 'boxes hidden of', 16 * len(out))


if __name__ == '__main__':
    main()
