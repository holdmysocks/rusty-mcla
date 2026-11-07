"""The rail sweep's two level queries run whole offline on the original's code, from F-23371's parameter block to the
answer it reads: the sphere query F-66549 (its query object's ctor F-61157, the set-up F-38096 with 12 records, the
material mask, the excluded list with its callback F-19966, the sphere driver F-49353) and the swept capsule F-27454 (ctor F-93477 with the
excluded-list callback F-53082, the set-up, the driver F-91312), each driver with the culler's ctor F-33380 and volume
(F-36470 / F-21683), the list's reset F-16848, the culler's walk over a level (F-70638 / F-71552 with F-51402,
F-64932, the node test F-38446, the object chain and the object test F-62425 -> F-92940), the frame steps F-51913 /
F-56283, the dispatches F-75667 / F-35029 with everything below them, the commit F-77105 with r5 = 0, F-72810.

Only the material manager's cast is answered (an object whose flag shift +16 is 8, as rail_sweep_query_reference.py)
and the props' material call faked ((index + 1) << 8); the city cells' material call is the real F-10817.

The level is built in the arena as the walk reads it: +112 the object table (8 bytes: +0 the next object's index,
+4 the object's address | its state 0 .. 3), +132 the octree's nodes (48 bytes) - here one root (+0 its half size 1e7,
+4 the states 7, +24 no children, +25 objects, +26 the first object, +32 its centre 0, +44 0), so every object is
walked, in the case's order. An object: +4 its archetype (+12 the bound, +16 the type flags, +20 the include flags), +16
its matrix (rows, place). Bounds: the city's grid (resources/city/sc/0x3e5fd91d.r1, laid as city_dispatch_reference.py
lays it), the prop types' composites (resources/prop/0x0e66973c.r131, as capsule_dispatch_reference.py) and walker
capsules (type 1, made here: +8 the bounding radius L / 2 + R, +16 / +32 the box, +48 the centroid, +128 R, +144 L).

A case: the query (0 capsule, 1 sphere), the caller's f30 (the capsule's stack word), the block's words (from / centre,
to, radius, the include mask +20, the states +24, the excluded count +4 and words +8 / +12 - as objects' indices or a
foreign word), then the objects (kind 0 city / 1 prop (its bound's address, its type index) / 2 walker (R, L), state, type flags,
include flags, matrix).
Out: the sphere's answer (+52), the query object's list (+36, +44 .. +84, read from its stack frame after the call) and
every record of the block (12 at +64 for the sphere, 1 at +112 for the capsule). Writes <out-dir>/level_query_ref.txt.
Test `camprobe::tests::level_query_matches_the_original`."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import memcap  # (every harness process capped at 4 GB)
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
    # (a 32 MB arena: the city's file is 14.5 MB, the props' 11.3 MB)
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
FAKE_MAT = 0x7F000010
RES_P = 0x40100000            # the props' file
RES_C = 0x41000000            # the city's file
VT_P, VT_C = 0x40D04000, 0x40D04200
MAIN = r'''
static const uint32_t LV=0x40D00000, ENT=0x40D00100, NODE=0x40D00200, INST=0x40D01000, ARCH=0x40D02000, WB=0x40D03000,
 VTP=VTP_, VTC=VTC_, BLK=0x40D05000, CASTO=0x40D06000, STK=0x40DF0000;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static std::vector<uint8_t> blobp, blobc;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 for(int k=0;k<4;k++) put(0x828609E0+4*k,0x7F800000);
 put(LEVEL_GLOBAL_, LV);
 {std::ifstream b(argv[3],std::ios::binary); blobp.assign(std::istreambuf_iterator<char>(b),{});}
 {std::ifstream b(argv[4],std::ios::binary); blobc.assign(std::istreambuf_iterator<char>(b),{});}
 memcpy(pointer(RESP_),blobp.data(),blobp.size()); memcpy(pointer(RESC_),blobc.data(),blobc.size());
 uint32_t GRID=strtoul(argv[5],nullptr,16); const char*fe=getenv("MCLA_LQ_FILL"); uint32_t FILL=fe?strtoul(fe,nullptr,16):0;
 FILE*f=fopen(argv[2],"r"); unsigned h[16];
 while(fscanf(f,"%x %x %x",&h[0],&h[1],&h[2])==3){
  bool ball=h[0]!=0;
  for(int i=3;i<15;i++) if(fscanf(f,"%x",&h[i])!=1) return 1;  // (15 header words, as the writer puts them)
  unsigned n; if(fscanf(f,"%x",&n)!=1) return 1;
  if(n>4096){fprintf(stderr,"bad case file: %u objects\n",n); return 2;}  // (a layout slip once asked for 3e9 x 18 words)
  std::vector<std::vector<unsigned>> ob(n,std::vector<unsigned>(18));
  for(auto&o:ob) for(int i=0;i<18;i++) if(fscanf(f,"%x",&o[i])!=1) return 1;
  rt_reset();
  for(uint32_t a=LV;a<CASTO+0x100;a+=4) store(a,0);
  for(uint32_t a=STK-0x20000;a<STK+0x100;a+=4) store(a,FILL);  // (the stack as the caller leaves it: 0, or MCLA_LQ_FILL to see what depends on it)
  store(VTP+48,FAKE_MAT_); store(VTC+48,MATERIAL_);
  store(CASTO+16,8);
  // the level: one root node holding every object, in the case's order
  store(LV+112,ENT); store(LV+132,NODE);
  store(NODE+0,0x4B189680); *pointer(NODE+4)=7; *pointer(NODE+24)=0; *pointer(NODE+25)=n?1:0; store16(NODE+26,0);
  for(int k=0;k<8;k++) store16(NODE+6+2*k,0xFFFF);
  for(unsigned i=0;i<n;i++){ auto&o=ob[i]; uint32_t in=INST+0x100*i, ar=ARCH+0x40*i;
   store16(ENT+8*i,i+1<n?i+1:0xFFFF); store(ENT+8*i+4,in|(o[3]&3));
   store(in+4,ar); for(int j=0;j<12;j++) store(in+16+(j/3)*16+(j%3)*4,o[6+j]);
   for(int j=0;j<3;j++) store(in+16+16*j+12,0); store(in+16+48+12,0x3F800000);
   uint32_t b;
   if(o[0]==0) b=GRID; else if(o[0]==1) b=o[1]; else { b=WB+0x100*i; float R,L; memcpy(&R,&o[1],4); memcpy(&L,&o[2],4);
    float br=L*0.5f+R, hl=L*0.5f+R; uint32_t w;
    store(b,VTP); *pointer(b+4)=1; memcpy(&w,&br,4); store(b+8,w);
    float mx[3]={R,hl,R}, mn[3]={-R,-hl,-R};
    for(int j=0;j<3;j++){ memcpy(&w,&mx[j],4); store(b+16+4*j,w); memcpy(&w,&mn[j],4); store(b+32+4*j,w); store(b+48+4*j,0);}
    for(int j=0;j<4;j++){ store(b+128+4*j,o[1]); store(b+144+4*j,o[2]); } }
   store(ar+12,b); store(ar+16,o[4]); store(ar+20,o[5]); }
  // F-23371's block
  for(int j=0;j<3;j++){ store(BLK+32+4*j,h[3+j]); store(BLK+48+4*j,h[6+j]); } store(BLK+44,0); store(BLK+60,0);
  if(ball) store(BLK+48,h[9]); else store(BLK+64,h[9]);
  store(BLK+20,h[10]); *pointer(BLK+24)=h[11]; store(BLK+16,640); store(BLK+4,h[12]);
  for(int j=0;j<2;j++){ unsigned w=h[13+j]; store(BLK+8+4*j, w<0x100? INST+0x100*w : w); }
  uint32_t recs=ball?BLK+64:BLK+112, nrec=ball?12:1;
  for(uint32_t a=recs;a<recs+96*nrec;a+=4) store(a,0xDEADBEEF);
  Context c=fresh(); c.r1.u64=STK; c.r3.u64=BLK; c.r4.u64=1;
  { uint64_t b=(uint64_t(h[1])<<32)|h[2]; memcpy(&c.f30.f64,&b,8); }
  try{ if(ball) sub_SPH_(c,nullptr); else sub_CAP_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  uint32_t q=ball? STK-2624+80 : STK-2656+80;
  printf("%08x",ball?load(BLK+52):0);
  for(int o2: {36,44,48,52,64,68,72,76,80,84}) printf(" %08x",load(q+o2));
  for(unsigned k=0;k<nrec;k++){ uint32_t r=recs+96*k;
   for(int o2: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",load(r+o2));
   printf(" %04x %04x %02x %04x",load16(r+76),load16(r+78),*pointer(r+80),load16(r+82)); }
  printf("\n"); } }
'''


def _resource(path):
    d = Path(path).read_bytes()
    assert d[:4] == b'MCLR'
    vs = min(struct.unpack('<I', d[12:16])[0], len(d) - 16)
    return d, vs, bytearray(d[16:])


def _ptr(vs, n):
    def ptr(p):
        if p == 0:
            return None
        off = p & 0x0FFFFFFF
        if p >> 28 == 5:
            return off if off < vs else None
        if p >> 28 == 6:
            return vs + off if vs + off < n else None
        return None
    return ptr


def load_props(path):
    """capsule_dispatch_reference.py's layout at RES_P with the vtable VT_P: (blob, [(address, centroid, radius) or None])."""
    d, vs, data = _resource(path)
    U32 = lambda o: struct.unpack('>I', data[o:o + 4])[0]
    U16 = lambda o: struct.unpack('>H', data[o:o + 2])[0]
    F = lambda o: struct.unpack('>f', data[o:o + 4])[0]
    ptr = _ptr(vs, len(data))
    seen = set()

    def reloc(o):
        if o in seen:
            return
        seen.add(o)
        q = ptr(U32(o))
        data[o:o + 4] = struct.pack('>I', RES_P + q if q is not None else 0)

    tp = ptr(U32(8))
    arr, n = ptr(U32(tp + 0x18)), U16(tp + 0x1c)
    types = []
    for i in range(n):
        fr = ptr(U32(arr + 4 * i))
        b = ptr(U32(fr + 0xec))
        if b is None or data[b + 4] != 12:
            types.append(None)
            continue
        parts, cnt = ptr(U32(b + 0x80)), U16(b + 0x90)
        for k in range(cnt):
            p = ptr(U32(parts + 4 * k))
            if p is None:
                continue
            data[p:p + 4] = struct.pack('>I', VT_P)
            if data[p + 4] == 4:
                for o in (0x80, 0x8c, 0xd8, 0xd0):
                    reloc(p + o)
        for o in (0x80, 0x84, 0x94):
            reloc(b + o)
        for k in range(cnt):
            reloc(parts + 4 * k)
        data[b:b + 4] = struct.pack('>I', VT_P)
        types.append((RES_P + b, [F(b + 48 + 4 * j) for j in range(3)], F(b + 8)))
    return bytes(data), types


def load_city(path):
    """city_dispatch_reference.py's layout at RES_C with the vtable VT_C: (blob, the grid's address, the walls' points)."""
    d, vs, data = _resource(path)
    U32 = lambda o: struct.unpack('>I', data[o:o + 4])[0]
    U16 = lambda o: struct.unpack('>H', data[o:o + 2])[0]
    I16 = lambda o: struct.unpack('>h', data[o:o + 2])[0]
    F = lambda o: struct.unpack('>f', data[o:o + 4])[0]
    ptr = _ptr(vs, len(data))
    P = lambda o: ptr(U32(o))

    def reloc(o):
        q = P(o)
        data[o:o + 4] = struct.pack('>I', RES_C + q if q is not None else 0)

    grid = P(P(0x28) + 0x28)
    assert data[grid + 4] == 8
    lst = P(grid + 0xa0)
    walls = []
    for i in range(U16(grid + 0xa4)):
        c = P(lst + 4 * i)
        if c is None:
            continue
        assert data[c + 4] == 10
        nv, nt = U32(c + 0xc8), U32(c + 0xcc)
        sc = [F(c + 0x90 + 4 * k) for k in range(3)]
        ce = [F(c + 0xa0 + 4 * k) for k in range(3)]
        polys, verts = P(c + 0x8c), P(c + 0xb0)
        nums = [data[P(c + 0xd8) + t] for t in range(nt)]
        ids = [U32(P(c + 0xd0) + 4 * k) for k in range(data[c + 0xdc])]
        for t in range(0, nt, 7):
            if (ids[nums[t]] if nums[t] < len(ids) else 0) & 0x28000:
                continue
            v = [[f32(f32(I16(verts + 6 * U16(polys + 16 * t + 4 + 2 * k) + 2 * j) * sc[j]) + ce[j]) for j in range(3)] for k in range(3)]
            walls.append([sum(v[k][j] for k in range(3)) / 3 for j in range(3)])
        b = P(c + 0xe0)
        for o in (0x8c, 0xb0, 0xd0, 0xd8, 0xe0):
            reloc(c + o)
        reloc(b)
        reloc(b + 0x60)
        data[c:c + 4] = struct.pack('>I', VT_C)
    for i in range(U16(grid + 0xa4)):
        reloc(lst + 4 * i)
    reloc(grid + 0xa0)
    data[grid:grid + 4] = struct.pack('>I', VT_C)
    return bytes(data), RES_C + grid, walls


def rotation(rnd, upright):
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    ax = [0.0, 1.0, 0.0] if upright else unit([rnd.gauss(0, 1) for _ in range(3)])
    th = rnd.uniform(-math.pi, math.pi)
    c, s = math.cos(th), math.sin(th)
    x, y, z = ax
    return [[c + x * x * (1 - c), x * y * (1 - c) + z * s, x * z * (1 - c) - y * s],
            [y * x * (1 - c) - z * s, c + y * y * (1 - c), y * z * (1 - c) + x * s],
            [z * x * (1 - c) + y * s, z * y * (1 - c) - x * s, c + z * z * (1 - c)]]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--props', default='D:/MCLA/extracted/cache/resources/prop/0x0e66973c.r131')
    ap.add_argument('--city', default='D:/MCLA/extracted/cache/resources/city/sc/0x3e5fd91d.r1')
    ap.add_argument('--sdk', default=SDK_INTRINSICS)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    ap.add_argument('--seed', type=int, default=91312)
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    blobp, types = load_props(a.props)
    blobc, grid, walls = load_city(a.city)
    rnd = random.Random(a.seed)
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    anyt = [i for i, t in enumerate(types) if t]
    cases = []
    while len(cases) < a.cases:
        ball = rnd.random() < 0.5
        city = rnd.random() < 0.45
        if city:
            anchor = [w + rnd.gauss(0, 0.8) for w in rnd.choice(walls)]
        else:
            anchor = [rnd.uniform(-2000, 2000), rnd.uniform(-5, 60), rnd.uniform(-2000, 2000)]
        objs = []
        if city or rnd.random() < 0.2:
            objs.append([0, 0, 0, 0, 0x900, 0xFFFFFFFF] + [fb(v) for v in (1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0)])
        for _ in range(rnd.choice([0, 1, 1, 2, 3])):
            ti = rnd.choice(anyt)
            b, cen, rad = types[ti]
            rot = rotation(rnd, rnd.random() < 0.7)
            near = [anchor[j] + rnd.gauss(0, 0.6 * rad + 0.5) for j in range(3)]
            # (the place puts the bound's centre near the anchor)
            place = [near[j] - sum(cen[k] * rot[k][j] for k in range(3)) for j in range(3)]
            objs.append([1, b, ti, 0, 2, 0xFFFFFFFF] + [fb(f32(v)) for row in rot + [place] for v in row])
        for _ in range(rnd.choice([0, 0, 1, 2])):
            R, L = f32(rnd.choice([0.3, 0.35, rnd.uniform(0.2, 0.5)])), f32(rnd.choice([1.1, rnd.uniform(0.6, 1.4)]))
            rot = rotation(rnd, True)
            place = [anchor[j] + rnd.gauss(0, 0.8) for j in range(3)]
            objs.append([2, fb(R), fb(L), 0, 0x20, 0xFFFFFFFF] + [fb(f32(v)) for row in rot + [place] for v in row])
        if not objs:
            continue
        rnd.shuffle(objs)
        for o in objs:
            o[3] = rnd.choice([0, 0, 0, 1, 2, 3])
            if rnd.random() < 0.1:
                o[4] = rnd.choice([0x189, 0x185, 0x1100, 0x2, 0x20, 0x900])
            if rnd.random() < 0.05:
                o[5] = rnd.choice([0, 0x10])
        R = f32(rnd.choice([0.5, 0.75, 1.0, rnd.uniform(0.1, 1.6)]))
        dv = unit([rnd.gauss(0, 1), rnd.gauss(0, 0.2), rnd.gauss(0, 1)])
        L = rnd.choice([0.0, rnd.uniform(0.1, 2.0), rnd.uniform(1.0, 10.0)])
        u0 = rnd.uniform(0, 1)
        A = [f32(anchor[j] - dv[j] * L * u0) for j in range(3)]
        B = [f32(A[j] + dv[j] * L) for j in range(3)] if not ball else [0.0, 0.0, 0.0]
        include = rnd.choice([2106] * 6 + [0x812, 0xFFFFFFFF, 0x20])
        states = rnd.choice([7] * 8 + [1, 3])
        cnt = rnd.choice([0, 1, 1, 2, 2])
        ex = [rnd.randrange(len(objs)) if rnd.random() < 0.7 else 0x5555 for _ in range(2)]
        f30 = struct.unpack('>Q', struct.pack('>d', f32(rnd.uniform(-60, 60))))[0]
        head = [int(ball), f30 >> 32, f30 & 0xFFFFFFFF] + [fb(v) for v in A + B] + [fb(R), include, states, cnt] + ex
        assert len(head) == 15 and len(objs) <= 4096 and all(len(x) == 18 for x in objs)  # (the reader's layout)
        cases.append((head, objs))
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'props.bin').write_bytes(blobp)
    (work / 'city.bin').write_bytes(blobc)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%x' % w for w in h) + ' %x ' % len(o) + ' '.join(' '.join('%x' % w for w in x) for x in o) + '\n'
                           for h, o in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    sph, cap = addr(a.tool, 'F-66549'), addr(a.tool, 'F-27454')
    cast = addr(a.tool, 'F-75805')
    material = int(addr(a.tool, 'F-10817'), 16)
    main_src = (MAIN.replace('RESP_', '0x%08Xu' % RES_P).replace('RESC_', '0x%08Xu' % RES_C).replace('VTP_', '0x%08Xu' % VT_P)
                .replace('VTC_', '0x%08Xu' % VT_C).replace('FAKE_MAT_', '0x%08Xu' % FAKE_MAT).replace('MATERIAL_', '0x%08Xu' % material)
                .replace('LEVEL_GLOBAL_', '0x828DA464u').replace('SPH_', sph).replace('CAP_', cap))
    # (the excluded-list callbacks F-53082 / F-19966 are reached through +2492 / +2460; the cast answers the object at CASTO)
    exe = rd.build(idx, ['sub_' + sph, 'sub_' + cap], main_src, work, 'level', image=img,
                   stubs={'sub_' + cast: 'ctx.r3.u64=0x40D06000;'},
                   targets=[material, int(addr(a.tool, 'F-53082'), 16), int(addr(a.tool, 'F-19966'), 16)], fakes={FAKE_MAT: 'ctx.r3.u64=(ctx.r4.u64+1)<<8;'}, limit=1600)
    r = _run([str(exe), str(a.image), str(inp), str(work / 'props.bin'), str(work / 'city.bin'), '%x' % grid], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = []
    for (h, objs), o in zip(cases, out):
        lines.append(' '.join('%x' % w for w in h) + ' | ' + ' | '.join(' '.join('%x' % w for w in x) for x in objs) + ' -> ' + o.strip())
    nl = chr(10)
    (a.out_dir / 'level_query_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split() for o in out]
    blocked = sum(1 for (h, _), x in zip(cases, res) if x[11] != '00000000')
    print(len(cases), 'cases;', blocked, 'with the first record set')


if __name__ == '__main__':
    main()
