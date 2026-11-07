"""F-35029, the undirected capsule's dispatch, on the props' composite bounds, with the result list it fills - run offline
on the original's code whole: the list's reset F-16848, F-35029 itself (its composite loop with F-66128, F-16177,
F-47011, F-77105 with r5 = 1, F-15412 and F-67153; F-11472 on the spheres, capsules and boxes; F-43031 on each polygon
of the polyhedra with the vertex fetch F-54719), the list's slot allocator F-89028 (not stubbed here: the records, the
box test, the full list's replacement) and, after each dispatch, the commit F-77105 with r5 = 0 by an object matrix
(its record transform F-57315). Only the bounds' material call is faked ((index + 1) << 8).

The prop type file (resources/prop/0x0e66973c.r131) is laid into the arena as it is, its pointers that the dispatch
follows (the composite's parts, matrices and masks, each polyhedron's vertices, polygons and material bytes) moved to
where it lies, each bound's vtable word the fake one: so the original reads the very bytes the port's reader
(fragphys::parse_frag_types) reads.

A case: the list's number of records (0 .. 3), the caller's f30 (two words), then one to three dispatches on the same
list: the type index, the capsule's ends and radius, the box (min, max), the material mask, and the commit's matrix
(rows and place, 12 words). Out: per dispatch F-35029's and F-77105's answers, then the query's ends and box, the
list's counts, bits and scores, and every record. Writes <out-dir>/capsule_dispatch_ref.txt. Test
`camprobe::tests::capsule_dispatch_matches_the_original`."""
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


def _shim():
    header = Path(SDK_INTRINSICS).read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    s = '#include <simde/x86/avx2.h>' + chr(10) + _vs()
    old = 'struct Condition {' + chr(10) + '    bool eq{}, lt{}, gt{};'
    assert s.count(old) == 1
    s = s.replace(old, 'struct Condition {' + chr(10) + '    bool so{}, eq{}, lt{}, gt{};' + chr(10) +
                  '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)
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
SDK_INTRINSICS = 'D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h'
FAKE_MAT = 0x7F000010
RES = 0x40100000
MAIN = r'''
static const uint32_t RES=RES_, Q=0x40E00000, REC=0x40E01000, VT=0x40E02000, MX=0x40E02100;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static std::vector<uint8_t> blob;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 for(int k=0;k<4;k++) put(0x828609E0+4*k,0x7F800000);
 {std::ifstream b(argv[3],std::ios::binary); blob.assign(std::istreambuf_iterator<char>(b),{});}
 FILE*f=fopen(argv[2],"r"); unsigned head[3];
 while(fscanf(f,"%x %x %x",&head[0],&head[1],&head[2])==3){
  unsigned nq; if(fscanf(f,"%x",&nq)!=1)break; std::vector<std::vector<unsigned>> qs(nq,std::vector<unsigned>(27));
  for(auto&q:qs) for(int i=0;i<27;i++) if(fscanf(f,"%x",&q[i])!=1) return 1;
  rt_reset(); memcpy(pointer(RES),blob.data(),blob.size());
  for(uint32_t a=Q;a<VT+0x200;a+=4) store(a,0);
  store(VT+48,FAKE_MAT_);
  store(Q+32,head[0]?REC:0); store(Q+36,head[0]); for(uint32_t a=REC;a<REC+96*4;a+=4) store(a,0xDEADBEEF);
  { Context c=fresh(); c.r3.u64=Q; sub_RESET_(c,nullptr); }
  bool bad=false;
  for(auto&q:qs){
   for(int j=0;j<3;j++){ store(Q+176+4*j,q[1+j]); store(Q+192+4*j,q[4+j]); store(Q+4*j,q[8+j]); store(Q+16+4*j,q[11+j]); }
   store(Q+12,0); store(Q+28,0); store(Q+188,0); store(Q+204,0);
   store(Q+208,q[7]); *pointer(Q+212)=0; store(Q+88,q[14]);
   for(int j=0;j<12;j++) store(MX+(j/3)*16+(j%3)*4,q[15+j]);
   for(int j=0;j<4;j++) store(MX+16*j+12,0);
   Context c=fresh(); c.r3.u64=Q; c.r4.u64=q[0]; c.r5.u64=0x1234; c.r6.u64=0xFFFFFFFF; c.r7.u64=0xFFFFFFFF;
   { uint64_t b=(uint64_t(head[1])<<32)|head[2]; memcpy(&c.f30.f64,&b,8); }
   try{ sub_DISP_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); bad=true; break; }
   int r1=c.r3.u32&0xFF, r2=0;
   if(r1){ Context d=fresh(); d.r3.u64=Q; d.r4.u64=MX; d.r5.u64=0; try{ sub_COMMIT_(d,nullptr);}catch(std::exception&e){ printf("EXC %s\n",e.what()); bad=true; break; } r2=d.r3.u32&0xFF; }
   printf("%d %d ",r1,r2);
  }
  if(bad) continue;
  for(int o: {176,180,184,192,196,200,0,4,8,16,20,24,96,100,104,112,116,120,44,48,52,64,68,72,76,80,84}) printf(" %08x",load(Q+o));
  for(unsigned k=0;k<head[0];k++){ uint32_t r=REC+96*k;
   for(int o: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",load(r+o));
   printf(" %04x %04x %02x %04x",load16(r+76),load16(r+78),*pointer(r+80),load16(r+82)); }
  printf("\n"); } }
'''


def load_props(path):
    d = Path(path).read_bytes()
    assert d[:4] == b'MCLR'
    vs = min(struct.unpack('<I', d[12:16])[0], len(d) - 16)
    data = bytearray(d[16:])
    U32 = lambda o: struct.unpack('>I', data[o:o + 4])[0]
    U16 = lambda o: struct.unpack('>H', data[o:o + 2])[0]

    def ptr(p):
        if p == 0:
            return None
        off = p & 0x0FFFFFFF
        if p >> 28 == 5:
            return off if off < vs else None
        if p >> 28 == 6:
            return vs + off if vs + off < len(data) else None
        return None

    seen = set()

    def reloc(o):
        if o in seen:
            return
        seen.add(o)
        q = ptr(U32(o))
        data[o:o + 4] = struct.pack('>I', RES + q if q is not None else 0)

    tp = ptr(U32(8))
    arr, n = ptr(U32(tp + 0x18)), U16(tp + 0x1c)
    types = []
    vt = 0x40E02000
    for i in range(n):
        fr = ptr(U32(arr + 4 * i))
        b = ptr(U32(fr + 0xec))
        if b is None or data[b + 4] != 12:
            types.append(None)
            continue
        parts, cnt = ptr(U32(b + 0x80)), U16(b + 0x90)
        info = []
        for k in range(cnt):
            p = ptr(U32(parts + 4 * k))
            if p is None:
                info.append(None)
                continue
            info.append((data[p + 4], p))
        for k, x in enumerate(info):
            if x is None:
                continue
            kind, p = x
            data[p:p + 4] = struct.pack('>I', vt)
            if kind == 4:
                for o in (0x80, 0x8c, 0xd8, 0xd0):
                    reloc(p + o)
        for o in (0x80, 0x84, 0x94):
            reloc(b + o)
        for k in range(cnt):
            reloc(parts + 4 * k)
        data[b:b + 4] = struct.pack('>I', vt)
        F = lambda o: struct.unpack('>f', data[o:o + 4])[0]
        mats = parts and ptr(struct.unpack('>I', d[16 + b + 0x84:16 + b + 0x88])[0])
        pinfo = []
        for k, x in enumerate(info):
            if x is None:
                continue
            kind, p = x
            m = [[F(mats + 64 * k + 16 * r + 4 * c) for c in range(3)] for r in range(4)]
            pinfo.append((kind, [F(p + 0x20 + 4 * c) for c in range(3)], [F(p + 0x10 + 4 * c) for c in range(3)], m, F(p + 8)))
        types.append((RES + b, pinfo))
    return bytes(data), types


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--props', default='D:/MCLA/extracted/cache/resources/prop/0x0e66973c.r131')
    ap.add_argument('--sdk', default=SDK_INTRINSICS)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    ap.add_argument('--seed', type=int, default=35029)
    a = ap.parse_args()
    globals()['SDK_INTRINSICS'] = a.sdk
    blob, types = load_props(a.props)
    rnd = random.Random(a.seed)
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    # (the types with a polyhedron part come up more often)
    poly = [i for i, t in enumerate(types) if t and any(p[0] == 4 for p in t[1])]
    anyt = [i for i, t in enumerate(types) if t]
    cases = []
    while len(cases) < a.cases:
        cap = rnd.choice([0, 1, 1, 2, 3, 3])
        f30 = struct.unpack('>Q', struct.pack('>d', f32(rnd.uniform(-50, 50))))[0]
        qs = []
        for _ in range(rnd.choice([1, 1, 2, 3])):
            ti = rnd.choice(poly) if rnd.random() < 0.3 else rnd.choice(anyt)
            base, parts = types[ti]
            kind, mn, mx, m, rad = rnd.choice(parts)
            # a point about the part (its box in its frame, carried by its matrix), a direction, a length
            loc = [rnd.uniform(mn[j] - 0.3, mx[j] + 0.3) for j in range(3)]
            on = [loc[0] * m[0][j] + loc[1] * m[1][j] + loc[2] * m[2][j] + m[3][j] for j in range(3)]
            dv = unit([rnd.gauss(0, 1) for _ in range(3)])
            if rnd.random() < 0.15:
                dv = [0.0, 0.0, 0.0]
                dv[rnd.randrange(3)] = rnd.choice([-1.0, 1.0])
            L = rnd.choice([rnd.uniform(0.05, 0.6), rnd.uniform(0.5, 3.0), rnd.uniform(2.0, 8.0)]) if rnd.random() < 0.96 else 0.0
            u0 = rnd.uniform(0, 1)
            A = [f32(on[j] - dv[j] * L * u0) for j in range(3)]
            B = [f32(A[j] + dv[j] * L) for j in range(3)]
            R = f32(rnd.choice([0.1, 0.3, 0.5, rnd.uniform(0.02, 1.5)]))
            lo = [f32(f32(min(A[j], B[j])) - R) for j in range(3)]
            hi = [f32(f32(max(A[j], B[j])) + R) for j in range(3)]
            if rnd.random() < 0.2:
                k = rnd.randrange(3)
                cut = f32(rnd.uniform(lo[k], hi[k]))
                if rnd.random() < 0.5:
                    lo[k] = cut
                else:
                    hi[k] = cut
            mask = rnd.choice([0] * 12 + [0x100])
            # the commit's matrix: a rotation about a random axis and a place
            ax = unit([rnd.gauss(0, 1) for _ in range(3)])
            th = rnd.uniform(-math.pi, math.pi)
            c, s = math.cos(th), math.sin(th)
            x, y, z = ax
            rot = [[c + x * x * (1 - c), x * y * (1 - c) + z * s, x * z * (1 - c) - y * s],
                   [y * x * (1 - c) - z * s, c + y * y * (1 - c), y * z * (1 - c) + x * s],
                   [z * x * (1 - c) + y * s, z * y * (1 - c) - x * s, c + z * z * (1 - c)]]
            place = [rnd.uniform(-500, 500), rnd.uniform(-20, 40), rnd.uniform(-500, 500)]
            mw = [fb(f32(v)) for row in rot + [place] for v in row]
            qs.append([ti, base] + [fb(v) for v in A + B] + [fb(R)] + [fb(v) for v in lo + hi] + [mask] + mw)
            assert len(qs[-1]) == 28, len(qs[-1])
        cases.append((cap, f30, qs))
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'props.bin').write_bytes(blob)
    inp = work / 'cases.txt'
    inp.write_text(''.join('%x %x %x %x ' % (c[0], c[1] >> 32, c[1] & 0xFFFFFFFF, len(c[2])) +
                           ' '.join(' '.join('%x' % w for w in q[1:]) for q in c[2]) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    disp, reset, commit = addr(a.tool, 'F-35029'), addr(a.tool, 'F-16848'), addr(a.tool, 'F-77105')
    main_src = (MAIN.replace('RES_', '0x%08Xu' % RES).replace('FAKE_MAT_', '0x%08Xu' % FAKE_MAT)
                .replace('DISP_', disp).replace('RESET_', reset).replace('COMMIT_', commit))
    exe = rd.build(idx, ['sub_' + disp, 'sub_' + reset, 'sub_' + commit], main_src, work, 'disp', image=img,
                   fakes={FAKE_MAT: 'ctx.r3.u64=(ctx.r4.u64+1)<<8;'}, limit=800)
    r = _run([str(exe), str(a.image), str(inp), str(work / 'props.bin')], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    # the type index in place of the bound's address, for the test
    lines = []
    for (cap, f30, qs), o in zip(cases, out):
        lines.append('%x %x %x %x | ' % (cap, f30 >> 32, f30 & 0xFFFFFFFF, len(qs)) +
                     ' | '.join(' '.join('%x' % w for w in [q[0]] + q[2:]) for q in qs) + ' -> ' + o.strip())
    nl = chr(10)
    (a.out_dir / 'capsule_dispatch_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split() for o in out]
    print(len(cases), 'cases;', sum(x[0] == '1' for x in res), 'first dispatch answered 1')


if __name__ == '__main__':
    main()
