"""F-35029 (the undirected capsule's dispatch) and F-75667 (the sphere's) on the city's grid bound (type 8) and its cells
(type 10), with the result list they fill - run offline on the original's code whole: the list's reset F-16848; the
grid loop (F-22767, F-30918, the cell box F-78191, the list's box per cell, F-19693, F-15315, the box reset, F-67153);
each cell's polygon walk (the capsule's F-54631 / F-74304 with F-32549, F-94830, F-26111, F-93593, F-95864, F-88692,
F-67335, F-81407, F-44462; the sphere's F-20956 with F-33149, F-95142, F-54895, F-33493, F-63013); each triangle by
F-92114 and F-43031 / F-82650 (with F-54719 for the neighbours); the slot allocator F-89028; after each dispatch that
answers, the commit F-77105 with r5 = 0 by the city's identity matrix. The cells' material call is the real F-10817
(table +208 [number]) - nothing is faked.

The city's collision file (resources/city/sc/0x3e5fd91d.r1) is laid into a 32 MB arena as it is, its pointers that the
dispatch follows (the grid's cell list, each cell's polygons, vertices, material table and numbers and BVH, the BVH's
nodes and subtree headers) moved to where it lies and each cell's vtable word a fake whose +48 is F-10817: so the
original reads the very bytes the port's reader (`bound::CityBound::parse`) reads. The query's polygon list is the
drivers' 4096-entry scratch (+324 / +292), as F-91312 and the sphere's driver give a query without one.

A case: the mode (0 capsule, 1 sphere), the list's number of records, the caller's f30 (two words), then one to three
dispatches on the same list: the ends (the sphere: the centre and three unused words) and radius, the material mask.
Out: per dispatch the answers of the dispatch and of F-77105, then the query's ends / centre and boxes, the list's box,
counts, bits and scores, and every record. Writes <out-dir>/city_dispatch_ref.txt. Test
`camprobe::tests::city_dispatch_matches_the_original`."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


_vs = rd.vector_shim
_run = subprocess.run
SDK_INTRINSICS = 'D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h'


def _shim():
    header = Path(SDK_INTRINSICS).read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    s = '#include <simde/x86/avx2.h>' + chr(10) + _vs()
    old = 'struct Condition {' + chr(10) + '    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    s = s.replace(old, 'struct Condition {' + chr(10) + '    bool so{}, eq{}, lt{}, gt{};' + chr(10) +
                  '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)
    # (a 32 MB arena: the city's file is 14.5 MB)
    for a, b in (('a<0x40000000u+0x1000000u', 'a<0x40000000u+0x2000000u'), ('arena[0x1000000]', 'arena[0x2000000]')):
        assert s.count(a) == 1, a
        s = s.replace(a, b)
    return s + mask.replace('inline uint8_t', 'alignas(16) static uint8_t') + chr(10)


def _gpp(cmd, *a, **k):
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


import recomp_closure
recomp_closure.EXTRA += (
    'namespace rex { namespace ppc {' + chr(10) +
    'inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {' + chr(10) +
    '  simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));' + chr(10) +
    '  simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8);' + chr(10) +
    '  xmm0 = simde_mm_srli_epi32(xmm0, 31);' + chr(10) +
    '  xmm0 = simde_mm_add_epi32(xmm0, xmm1);' + chr(10) +
    '  xmm0 = simde_mm_srai_epi32(xmm0, 8);' + chr(10) +
    '  xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000));' + chr(10) +
    '  simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);' + chr(10) +
    '  return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1));' + chr(10) +
    '}' + chr(10) +
    'inline simde__m128i simde_mm_vctsxs(simde__m128 src1) {' + chr(10) +  # the SDK's intrinsics.h
    '  simde__m128 xmm2 = simde_mm_cmpunord_ps(src1, src1);' + chr(10) +
    '  simde__m128i xmm0 = simde_mm_cvttps_epi32(src1);' + chr(10) +
    '  simde__m128i xmm1 = simde_mm_cmpeq_epi32(xmm0, simde_mm_set1_epi32(INT32_MIN));' + chr(10) +
    '  xmm1 = simde_mm_andnot_si128(simde_mm_castps_si128(src1), xmm1);' + chr(10) +
    '  simde__m128 dest = simde_mm_blendv_ps(simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(simde_mm_set1_epi32(INT32_MAX)), simde_mm_castsi128_ps(xmm1));' + chr(10) +
    '  return simde_mm_andnot_si128(simde_mm_castps_si128(xmm2), simde_mm_castps_si128(dest));' + chr(10) +
    '} } }' + chr(10))
rd.vector_shim = _shim
subprocess.run = _gpp
RES = 0x41000000
MATERIAL_ID = "F-10817"      # the cells' material call (vtable +48)
MAIN = r'''
static const uint32_t RES=RES_, Q=0x40E00000, REC=0x40E04000, VT=0x40E05000, MX=0x40E05100, SCR=0x40E10000;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static std::vector<uint8_t> blob;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 for(int k=0;k<4;k++) put(0x828609E0+4*k,0x7F800000);
 {std::ifstream b(argv[3],std::ios::binary); blob.assign(std::istreambuf_iterator<char>(b),{});}
 memcpy(pointer(RES),blob.data(),blob.size());
 uint32_t GRID=strtoul(argv[4],nullptr,16);
 FILE*f=fopen(argv[2],"r"); unsigned head[4];
 while(fscanf(f,"%x %x %x %x",&head[0],&head[1],&head[2],&head[3])==4){
  bool ball=head[0]!=0;
  unsigned nq; if(fscanf(f,"%x",&nq)!=1)break; std::vector<std::vector<unsigned>> qs(nq,std::vector<unsigned>(8));
  for(auto&q:qs) for(int i=0;i<8;i++) if(fscanf(f,"%x",&q[i])!=1) return 1;
  rt_reset();
  for(uint32_t a=Q;a<VT+0x200;a+=4) store(a,0);
  store(VT+48,MATERIAL_);
  if(ball){ store(Q+292,SCR); store16(Q+296,4096); } else { store(Q+324,SCR); store16(Q+328,4096); }
  store(Q+32,head[1]?REC:0); store(Q+36,head[1]); for(uint32_t a=REC;a<REC+96*16;a+=4) store(a,0xDEADBEEF);
  { Context c=fresh(); c.r3.u64=Q; sub_RESET_(c,nullptr); }
  store(MX,0x3F800000); store(MX+20,0x3F800000); store(MX+40,0x3F800000);
  bool bad=false;
  for(auto&q:qs){
   float A[3],B[3],r; for(int j=0;j<3;j++){memcpy(&A[j],&q[j],4);memcpy(&B[j],&q[3+j],4);} memcpy(&r,&q[6],4);
   auto put3=[&](uint32_t a,const float*v){for(int j=0;j<3;j++){uint32_t w;memcpy(&w,&v[j],4);store(a+4*j,w);} store(a+12,0);};
   float lo[3],hi[3];
   if(ball){ for(int j=0;j<3;j++){lo[j]=A[j]-r; hi[j]=r+A[j];} put3(Q+160,A); store(Q+176,q[6]); }
   else { for(int j=0;j<3;j++){float mn=A[j]<B[j]?A[j]:B[j], mx=A[j]>B[j]?A[j]:B[j]; lo[j]=mn-r; hi[j]=mx+r;}
          put3(Q+176,A); put3(Q+192,B); store(Q+208,q[6]); *pointer(Q+212)=0; }
   put3(Q+96,lo); put3(Q+112,hi); store(Q+88,q[7]);
   Context c=fresh(); c.r3.u64=Q; c.r4.u64=GRID; c.r5.u64=0x1234; c.r6.u64=0xFFFFFFFF; c.r7.u64=0xFFFFFFFF;
   { uint64_t b=(uint64_t(head[2])<<32)|head[3]; memcpy(&c.f30.f64,&b,8); }
   try{ if(ball) sub_BALL_(c,nullptr); else sub_DISP_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); bad=true; break; }
   int r1=c.r3.u32&0xFF, r2=0;
   if(r1){ Context d=fresh(); d.r3.u64=Q; d.r4.u64=MX; d.r5.u64=0; try{ sub_COMMIT_(d,nullptr);}catch(std::exception&e){ printf("EXC %s\n",e.what()); bad=true; break; } r2=d.r3.u32&0xFF; }
   printf("%d %d ",r1,r2);
  }
  if(bad) continue;
  if(ball){ for(int o: {160,164,168,160,164,168}) printf(" %08x",load(Q+o)); }
  else { for(int o: {176,180,184,192,196,200}) printf(" %08x",load(Q+o)); }
  for(int o: {0,4,8,16,20,24,96,100,104,112,116,120,44,48,52,64,68,72,76,80,84}) printf(" %08x",load(Q+o));
  for(unsigned k=0;k<head[1];k++){ uint32_t r=REC+96*k;
   for(int o: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",load(r+o));
   printf(" %04x %04x %02x %04x",load16(r+76),load16(r+78),*pointer(r+80),load16(r+82)); }
  printf("\n"); } }
'''


def load_city(path):
    """The city file with the pointers the dispatch follows moved to RES + offset; also the cells' triangles (for the
    cases) and the grid's address."""
    d = Path(path).read_bytes()
    assert d[:4] == b'MCLR'
    vs = min(struct.unpack('<I', d[12:16])[0], len(d) - 16)
    data = bytearray(d[16:])
    U32 = lambda o: struct.unpack('>I', data[o:o + 4])[0]
    U16 = lambda o: struct.unpack('>H', data[o:o + 2])[0]
    I16 = lambda o: struct.unpack('>h', data[o:o + 2])[0]
    F = lambda o: struct.unpack('>f', data[o:o + 4])[0]

    def ptr(p):
        if p == 0:
            return None
        off = p & 0x0FFFFFFF
        if p >> 28 == 5:
            return off if off < vs else None
        if p >> 28 == 6:
            return vs + off if vs + off < len(data) else None
        return None

    def P(o):
        return ptr(U32(o))

    def reloc(o):
        q = P(o)
        data[o:o + 4] = struct.pack('>I', RES + q if q is not None else 0)
        return q

    vt = 0x40E05000
    grid = P(P(0x28) + 0x28)
    assert data[grid + 4] == 8
    lst = P(grid + 0xa0)
    cells = []
    for i in range(U16(grid + 0xa4)):
        c = P(lst + 4 * i)
        if c is None:
            continue
        assert data[c + 4] == 10
        nv, nt = U32(c + 0xc8), U32(c + 0xcc)
        sc = [F(c + 0x90 + 4 * k) for k in range(3)]
        ce = [F(c + 0xa0 + 4 * k) for k in range(3)]
        polys, verts = P(c + 0x8c), P(c + 0xb0)
        vx = [[f32(f32(I16(verts + 6 * v + 2 * k) * sc[k]) + ce[k]) for k in range(3)] for v in range(nv)]
        tris = [[U16(polys + 16 * t + 4 + 2 * k) for k in range(3)] for t in range(nt)]
        nums = [data[P(c + 0xd8) + t] for t in range(nt)]
        ids = [U32(P(c + 0xd0) + 4 * k) for k in range(data[c + 0xdc])]
        cells.append((i, vx, tris, [ids[n] if n < len(ids) else 0 for n in nums]))
        b = P(c + 0xe0)
        for o in (0x8c, 0xb0, 0xd0, 0xd8, 0xe0):
            reloc(c + o)
        assert U32(c + 0x80) == 0
        reloc(b)
        reloc(b + 0x60)
        data[c:c + 4] = struct.pack('>I', vt)
    for i in range(U16(grid + 0xa4)):
        reloc(lst + 4 * i)
    reloc(grid + 0xa0)
    data[grid:grid + 4] = struct.pack('>I', vt)
    return bytes(data), RES + grid, cells


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--city', default='D:/MCLA/extracted/cache/resources/city/sc/0x3e5fd91d.r1')
    ap.add_argument('--sdk', default=SDK_INTRINSICS)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    ap.add_argument('--seed', type=int, default=22767)
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    blob, grid, cells = load_city(a.city)
    rnd = random.Random(a.seed)
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    # (walls - what the sweep's mask 0x28000 lets through - come up more often)
    walls = [(ci, t) for ci, c in enumerate(cells) for t in range(len(c[2])) if not c[3][t] & 0x28000]
    cases = []
    while len(cases) < a.cases:
        ball = rnd.random() < 0.5
        cap = rnd.choice([0, 1, 1, 1, 3, 12] if ball else [0, 1, 1, 1, 3])
        f30 = struct.unpack('>Q', struct.pack('>d', f32(rnd.uniform(-50, 50))))[0]
        qs = []
        for _ in range(rnd.choice([1, 1, 2, 3])):
            if rnd.random() < 0.6:
                ci, t = rnd.choice(walls)
            else:
                ci = rnd.choice([k for k, c in enumerate(cells) if c[2]])
                t = rnd.randrange(len(cells[ci][2]))
            _, vx, tris, _ = cells[ci]
            w = [rnd.random() for _ in range(3)]
            s = sum(w)
            on = [sum(w[k] / s * vx[tris[t][k]][j] for k in range(3)) for j in range(3)]
            if rnd.random() < 0.1:
                # near a cell's edge (700 m squares)
                k = rnd.choice([0, 2])
                on[k] = round(on[k] / 700.0) * 700.0 + rnd.uniform(-3, 3)
            on = [on[j] + rnd.gauss(0, 0.6) for j in range(3)]
            R = f32(rnd.choice([0.25, 0.5, 1.0, rnd.uniform(0.05, 2.0)]))
            dv = unit([rnd.gauss(0, 1) for _ in range(3)])
            if rnd.random() < 0.5:
                dv = unit([dv[0], dv[1] * 0.1, dv[2]])
            L = rnd.choice([rnd.uniform(0.0, 1.0), rnd.uniform(1.0, 8.0), rnd.uniform(5.0, 40.0)])
            u0 = rnd.uniform(0, 1)
            A = [f32(on[j] - dv[j] * L * u0) for j in range(3)]
            B = [f32(A[j] + dv[j] * L) for j in range(3)]
            if ball:
                B = [0.0, 0.0, 0.0]
            mask = rnd.choice([0x28000] * 6 + [0, 0x8000, 0x10000])
            qs.append([fb(v) for v in A + B] + [fb(R), mask])
        cases.append((int(ball), cap, f30, qs))
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'city.bin').write_bytes(blob)
    inp = work / 'cases.txt'
    inp.write_text(''.join('%x %x %x %x %x ' % (c[0], c[1], c[2] >> 32, c[2] & 0xFFFFFFFF, len(c[3])) +
                           ' '.join(' '.join('%x' % w for w in q) for q in c[3]) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    disp, ball, reset, commit = (addr(a.tool, x) for x in ('F-35029', 'F-75667', 'F-16848', 'F-77105'))
    material = int(addr(a.tool, MATERIAL_ID), 16)
    main_src = (MAIN.replace('RES_', '0x%08Xu' % RES).replace('MATERIAL_', '0x%08Xu' % material).replace('DISP_', disp)
                .replace('BALL_', ball).replace('RESET_', reset).replace('COMMIT_', commit))
    exe = rd.build(idx, ['sub_' + disp, 'sub_' + ball, 'sub_' + reset, 'sub_' + commit], main_src, work, 'city', image=img,
                   targets=[material], limit=1200)
    r = _run([str(exe), str(a.image), str(inp), str(work / 'city.bin'), '%x' % grid], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = []
    for (ball, cap, f30, qs), o in zip(cases, out):
        lines.append('%x %x %x %x %x | ' % (ball, cap, f30 >> 32, f30 & 0xFFFFFFFF, len(qs)) +
                     ' | '.join(' '.join('%x' % w for w in q) for q in qs) + ' -> ' + o.strip())
    nl = chr(10)
    (a.out_dir / 'city_dispatch_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split() for o in out]
    print(len(cases), 'cases;', sum(x[0] == '1' for x in res), 'first dispatch answered 1')


if __name__ == '__main__':
    main()
