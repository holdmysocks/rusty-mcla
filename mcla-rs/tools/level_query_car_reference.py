"""The rail sweep's two level queries (level_query_reference.py: F-66549 / F-27454 from F-23371's block, whole) against the
player car's composite bound as the running game makes it, run offline: every vp_* car's composite (the hull, type 4, with
its own vertices, polygons, material indices +216 and table +208; 2 or 4 curved wheel bounds, type 5, with their tables +208)
laid in the arena, then, as a car is made and set up, the wheels' fit F-55382 at TireRadius 0.3 / TireWidth 0.1, and for the
set-up the fit with its tyres and the hull's reshape (the sort F-31335, F-89464 with its areas F-83274 and box / sphere slot
40 F-38448) in either order (a set-up's flag: 0 the fit first, as F-62931 calls the car's slot 15 before F-89464; 1 the
reshape first). The car is then an object of the level (type flags 0x189 mostly) with its instance matrix, among the city's
grid, props and walkers as level_query_reference.py lays them. The vtables are the real ones (the polyhedron's, the curved
bound's: the material call F-10817 reads the table +208 at the index); the composite's vtable is the props' fake one (its
material call is not reached for a composite). The cast F-75805 answers the composite to the fit and the material manager's
cast object to the queries.

The set-ups (60, at 0x41F00000 + 0x4000 k): the car's index, the order flag, the wheel count, BoundClearance / BoundAngle /
BoundBumper / BoundBumperWidth and the drop (hull_reshape_reference.py), then 4 wheels x 23 words (wheel_fit_reference.py:
tyre radius and width, SuspensionExtent / SuspensionLimit, the place of rest, the current matrix). For each the exe prints
the composite's +8 .. +60, each part's current matrix (+132, 16 words) and the hull's +8 .. +60.
A case: as level_query_reference.py, with object kind 3 the car (its set-up's index in the second word).
Writes <out-dir>/level_query_car_ref.txt (the set-ups' lines first, 'S'). Test
`camprobe::tests::level_query_car_matches_the_original`."""
import argparse
import glob
import math
import os
import random
import re
import struct
import subprocess
from pathlib import Path
import memcap  # (every harness process capped at 4 GB)
import recomp_dispatch as rd
import level_query_reference as lq   # (sets the vector shim, the compiler wrapper and the closure's extra intrinsics)

_lq_shim = rd.vector_shim


def _shim():
    s = _lq_shim()
    old = '    void setFromMask(simde__m128 mask, int imm) { int m = simde_mm_movemask_ps(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }'
    assert s.count(old) == 1
    return s.replace(old, old + chr(10) + '    void setFromMask(simde__m128i mask, int imm) { int m = simde_mm_movemask_epi8(mask); lt = m == imm; gt = 0; eq = m == 0; so = 0; }', 1)


rd.vector_shim = _shim
fb, f32 = lq.fb, lq.f32
SET, SETSZ, NSET = 0x41F00000, 0x4000, 60
VT_POLY = 0x82081784          # the polyhedron bound's vtable (slot 12 F-10817, slot 40 F-38448)
VT_CURVED = 0x82083AA4        # the curved bound's vtable (slot 12 F-10817, slot 40 F-63967, slot 46 F-96106)
INITS = ['F-50173', 'F-33402', 'F-26199', 'F-55749', 'F-97349', 'F-70580', 'F-79721', 'F-64228', 'F-24192', 'F-92824']
NW = 23
NSW = 8 + 4 * NW

BUILD = r'''
 { std::ifstream b(argv[7],std::ios::binary); std::vector<uint8_t> sb; sb.assign(std::istreambuf_iterator<char>(b),{});
   if(sb.size()%SETSZ_ || sb.size()/SETSZ_>NSET_){fprintf(stderr,"bad set-up blob\n"); return 2;}
   FILE*sf=fopen(argv[6],"r"); unsigned k=0, sw[NSW_];
   put(LEVEL_GLOBAL_,0);
   rt_reset(); { INITCALLS }
   if(load(0x82860950)!=0xffffffffu || load(0x828609E0)!=0x7F800000u) { printf("EXC constants\n"); return 1; }
   while(fscanf(sf,"%x",&sw[0])==1){
    for(int i=1;i<NSW_;i++) if(fscanf(sf,"%x",&sw[i])!=1) return 1;
    if(k>=sb.size()/SETSZ_||(sw[2]!=2&&sw[2]!=4)){fprintf(stderr,"bad set-up %u\n",k); return 2;}
    const uint32_t CAR=0x40D00000, A=0x40D00400, WARR=0x40D00900, WH=0x40D02000, TU=0x40D04000, T=0x40D06000, comp=SET_+SETSZ_*k;
    memcpy(pointer(comp),sb.data()+size_t(k)*SETSZ_,SETSZ_);
    uint32_t n=load16(comp+146); if(n<3||n>5){fprintf(stderr,"bad blob: %u parts\n",n); return 2;}
    rt_reset(); store(0x40000200u+32,load(0x40000200u+28));
    for(uint32_t a=CAR;a<T+0x400;a+=4) store(a,0);
    store(CAR+4,A); store(A+28,A+0x40); store(A+0x40+4,A+0x80); store(A+0x80+12,comp); store(A+132,A+0x200);
    store(CAR+24,sw[2]); store(CAR+28,WARR); store(CAR+392,0xFFFFFFFFu); store(CAR+412,WH); store(CAR+368,T);
    for(int j=0;j<10;j++) store(CAR+680+4*j,0xFFFFFFFFu);
    store(T+320,sw[3]); store(T+324,sw[4]); store(T+328,sw[5]); store(T+336,sw[6]);
    const unsigned*w=sw+8;
    for(unsigned i=0;i<4;i++){ uint32_t wh=WH+0x400*i, tu=TU+0x100*i; const unsigned*x=w+NW_*i; float t03=0.3f,t01=0.1f; uint32_t b3,b1; memcpy(&b3,&t03,4); memcpy(&b1,&t01,4);
     store(WARR+4*i,wh); store(wh+4,tu); store(tu+4,b3); store(tu+8,b1); store(tu+88,x[2]); store(tu+84,x[3]);
     for(int j=0;j<3;j++) store(wh+384+4*j,x[4+j]);
     if(i+1<n) for(int q=0;q<16;q++) store(load(comp+132)+64*(i+1)+4*q,x[7+q]); }
    try{
     { Context c=fresh(); c.r1.u64=0x40DF0000; c.r3.u64=CAR; sub_FIT_(c,nullptr); }
     for(unsigned i=0;i<4;i++){ uint32_t tu=TU+0x100*i; store(tu+4,w[NW_*i]); store(tu+8,w[NW_*i+1]); }
     float drop; memcpy(&drop,&sw[7],4);
     for(int step=0;step<2;step++){
      if((step==0)==(sw[1]==1)){ Context c=fresh(); c.r1.u64=0x40DF0000; c.r3.u64=CAR; sub_SORT_(c,nullptr);
        Context d=fresh(); d.r1.u64=0x40DF0000; d.r3.u64=CAR; d.f1.f64=drop; sub_MOVE_(d,nullptr); }
      else { Context c=fresh(); c.r1.u64=0x40DF0000; c.r3.u64=CAR; sub_FIT_(c,nullptr); } }
    }catch(std::exception&e){ printf("EXC set-up %u %s\n",k,e.what()); return 1; }
    printf("S %x",k); for(int o=8;o<64;o+=4) printf(" %08x",load(comp+o));
    for(unsigned p=0;p<n;p++) for(int q=0;q<16;q++) printf(" %08x",load(load(comp+132)+64*p+4*q));
    uint32_t hb=load(load(comp+128)); for(int o=8;o<64;o+=4) printf(" %08x",load(hb+o));
    printf("\n"); k++; }
   put(LEVEL_GLOBAL_,LV); }
'''


def car_blob(path, base):
    """The car file's composite laid at `base` (SETSZ bytes): wheel_fit_reference.py's layout (header, parts, current / last
    matrices, part boxes, the hull's header, each wheel bound's header, corners, curved edges and faces) and here also the hull's
    vertices (+128, 16 bytes each), polygons (+140), material indices (+216) and table (+208), and each wheel bound's table
    (+208). Returns (blob, parts, the parts' file places) or None."""
    raw = Path(path).read_bytes()
    assert raw[:4] == b'MCLR'
    d = raw[16:16 + struct.unpack_from('<I', raw, 12)[0]]
    u32 = lambda a: struct.unpack_from('>I', d, a)[0]
    u16 = lambda a: struct.unpack_from('>H', d, a)[0]
    at = lambda a: u32(a) & 0xfffffff if u32(a) >> 28 == 5 else None
    for o in range(0, len(d) - 0xa0, 16):
        if d[o + 4] != 12 or not 0 < u16(o + 0x92) <= 5 or at(o + 0x80) is None:
            continue
        n = u16(o + 0x92)
        parts = [at(at(o + 0x80) + 4 * i) for i in range(n)]
        if None in parts or [d[p + 4] for p in parts] not in ([4, 5, 5, 5, 5], [4, 5, 5]):
            continue
        mats, last, local = at(o + 0x84), at(o + 0x88), at(o + 0x8c)
        if None in (mats, last, local):
            continue
        b = bytearray(SETSZ)
        P, M, L, B, H = 0x100, 0x200, 0x400, 0x600, 0x800
        HV, HP, HM, HT, WT = 0x3000, 0x3400, 0x3A00, 0x3B00, 0x3B40
        b[0:0xa0] = d[o:o + 0xa0]
        put = lambda off, v: b.__setitem__(slice(off, off + 4), struct.pack('>I', v))
        put(0, lq.VT_P)
        for off, v in ((0x80, base + P), (0x84, base + M), (0x88, base + L), (0x8c, base + B)):
            put(off, v)
        b[M:M + 64 * n] = d[mats:mats + 64 * n]
        b[L:L + 64 * n] = d[last:last + 64 * n]
        b[B:B + 32 * n] = d[local:local + 32 * n]
        h = parts[0]
        b[H:H + 0xf0] = d[h:h + 0xf0]
        for off in range(0x80, 0xf0, 4):
            put(H + off, 0) if b[H + off] >> 4 in (5, 6) else None
        nv, nt, nm = u32(h + 200), u32(h + 204), d[h + 220]
        assert 4 <= nv <= 64 and 1 <= nt <= 96 and 1 <= nm <= 16, (path, nv, nt, nm)
        b[HV:HV + 16 * nv] = d[at(h + 128):at(h + 128) + 16 * nv]
        b[HP:HP + 16 * nt] = d[at(h + 140):at(h + 140) + 16 * nt]
        b[HM:HM + nt] = d[at(h + 216):at(h + 216) + nt]
        b[HT:HT + 4 * nm] = d[at(h + 208):at(h + 208) + 4 * nm]
        put(H, VT_POLY)
        put(H + 128, base + HV)
        put(H + 140, base + HP)
        put(H + 216, base + HM)
        put(H + 208, base + HT)
        put(P, base + H)
        for k, p in enumerate(parts[1:]):
            W = 0x1000 + 0x800 * k
            assert u32(p + 0xC8) == 4 and u32(p + 0xE8) == 4 and u32(p + 0xEC) == 6, 'not the curved wheel bound'
            v, fc, e = at(p + 0x80), at(p + 0xE0), at(p + 0xE4)
            b[W:W + 0xf0] = d[p:p + 0xf0]
            b[W + 0x100:W + 0x140] = d[v:v + 64]
            b[W + 0x200:W + 0x320] = d[e:e + 288]
            b[W + 0x400:W + 0x580] = d[fc:fc + 384]
            nw = d[p + 220]
            assert 1 <= nw <= 8
            b[WT + 0x20 * k:WT + 0x20 * k + 4 * nw] = d[at(p + 208):at(p + 208) + 4 * nw]
            put(W, VT_CURVED)
            put(W + 128, base + W + 0x100)
            put(W + 132, 0)
            put(W + 208, base + WT + 0x20 * k)
            put(W + 216, 0)
            put(W + 224, base + W + 0x400)
            put(W + 228, base + W + 0x200)
            put(P + 4 * (k + 1), base + W)
        places = [[struct.unpack_from('>f', d, mats + 64 * i + 48 + 4 * j)[0] for j in range(3)] for i in range(n)]
        assert len(b) == SETSZ
        return bytes(b), n, places
    return None


RIM = [0x3ea90ffa, 0x3eb61134, 0x3ec3126f, 0x3ed013a9, 0x3edd14e4, 0x3eea161f, 0x3ef71759, 0x3f020c4a, 0x3f088ce7, 0x3f0f0d84,
       0x3f158e21, 0x3f1c0ebe, 0x3f228f5c, 0x3f290ff9, 0x3f2f9096, 0x3f361134, 0x3f3c91d1]


def tyre(rnd):
    """A tyre a set-up gives (simtune::wheel_radius): (radius, width), as wheel_fit_reference.py draws them."""
    w = f32(0.195 + 0.01 * rnd.randrange(17))
    p = f32(0.20 + 0.05 * rnd.randrange(14))
    t = f32(p * w - struct.unpack('>f', struct.pack('>I', 0x3d020c4a))[0])
    return f32(struct.unpack('>f', struct.pack('>I', RIM[rnd.randrange(17)]))[0] * 0.5 + t), w


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, default=Path('D:/MCLA/extracted/cache'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=1500)
    ap.add_argument('--seed', type=int, default=23371)
    a = ap.parse_args()
    props = str(a.data / 'resources/prop/0x0e66973c.r131')
    city = str(a.data / 'resources/city/sc/0x3e5fd91d.r1')
    blobp, types = lq.load_props(props)
    blobc, grid, walls = lq.load_city(city)
    cars = []
    for dr in sorted(glob.glob(str(a.data / 'resources/vehicle/vp*'))):
        for f in sorted(glob.glob(dr + '/*.r83')):
            if car_blob(f, SET):
                cars.append((os.path.basename(dr), f))
                break
    assert cars
    rnd = random.Random(a.seed)
    setups, sblobs = [], []
    picks = list(range(len(cars)))
    rnd.shuffle(picks)
    for k in range(NSET):
        ci = picks[k % len(picks)]
        blob, n, places = car_blob(cars[ci][1], SET + SETSZ * k)
        nw = n - 1
        flag = k % 2
        tune = [fb(rnd.uniform(0.05, 0.35)), fb(rnd.uniform(0.0, 0.8)), fb(0.0 if rnd.random() < 0.3 else rnd.uniform(0.2, 0.7)), fb(rnd.uniform(0.3, 1.0))]
        drop = fb(0.0 if rnd.random() < 0.5 else rnd.uniform(0.0, 0.15))
        tyres = [tyre(rnd) for _ in range(2)]
        ws = []
        for i in range(4):
            fr = i < 2 if nw == 4 else i == 0
            r, wd = tyres[0 if fr else 1]
            ext = f32(rnd.choice([0.1, 0.2, 0.3, 0.5, 0.6, rnd.uniform(0.05, 0.8)]))
            lim = f32(rnd.choice([0.1, 0.15, 0.2, rnd.uniform(0.05, 0.4)]))
            pl = places[1 + i] if i < nw else [0.0, 0.0, 0.0]
            rest = [f32(pl[0]), f32(pl[1] - rnd.uniform(0.0, 0.05)), f32(pl[2])]
            mat = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, rest[0], f32(pl[1]), rest[2], 1.0]
            ws += [fb(r), fb(wd), fb(ext), fb(lim)] + [fb(x) for x in rest] + [fb(f32(x)) for x in mat]
        sw = [ci, flag, nw] + tune + [drop] + ws
        assert len(sw) == NSW  # (the reader's layout)
        setups.append(sw)
        sblobs.append(blob)
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    anyt = [i for i, t in enumerate(types) if t]
    cases = []
    while len(cases) < a.cases:
        ball = rnd.random() < 0.5
        k = rnd.randrange(NSET)
        rot = lq.rotation(rnd, rnd.random() < 0.85)
        place = [rnd.uniform(-1500, 1500), rnd.uniform(-5, 60), rnd.uniform(-1500, 1500)]
        if rnd.random() < 0.3:
            place = [w_ + rnd.gauss(0, 2.0) for w_ in rnd.choice(walls)]
        objs = [[3, k, 0, 0, 0x189, 0xFFFFFFFF] + [fb(f32(v)) for row in rot + [place] for v in row]]
        local = [rnd.gauss(0, 1.2), rnd.gauss(0.5, 0.6), rnd.gauss(0, 2.2)]
        anchor = [place[j] + sum(local[m] * rot[m][j] for m in range(3)) for j in range(3)]
        if rnd.random() < 0.3:
            objs.append([0, 0, 0, 0, 0x900, 0xFFFFFFFF] + [fb(v) for v in (1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0)])
        for _ in range(rnd.choice([0, 0, 0, 1])):
            ti = rnd.choice(anyt)
            b, cen, rad = types[ti]
            r2 = lq.rotation(rnd, True)
            near = [anchor[j] + rnd.gauss(0, 0.6 * rad + 0.5) for j in range(3)]
            pl2 = [near[j] - sum(cen[m] * r2[m][j] for m in range(3)) for j in range(3)]
            objs.append([1, b, ti, 0, 2, 0xFFFFFFFF] + [fb(f32(v)) for row in r2 + [pl2] for v in row])
        for _ in range(rnd.choice([0, 0, 0, 1])):
            R, L = f32(rnd.choice([0.3, 0.35])), f32(rnd.choice([1.1, rnd.uniform(0.6, 1.4)]))
            r2 = lq.rotation(rnd, True)
            pl2 = [anchor[j] + rnd.gauss(0, 0.8) for j in range(3)]
            objs.append([2, fb(R), fb(L), 0, 0x20, 0xFFFFFFFF] + [fb(f32(v)) for row in r2 + [pl2] for v in row])
        rnd.shuffle(objs)
        for o in objs:
            o[3] = rnd.choice([0, 0, 0, 0, 1, 2, 3])
            if rnd.random() < 0.08:
                o[4] = rnd.choice([0x189, 0x185, 0x1100, 0x2, 0x20, 0x900])
        R = f32(rnd.choice([0.5, 0.75, 1.0, rnd.uniform(0.1, 1.6)]))
        dv = unit([rnd.gauss(0, 1), rnd.gauss(0, 0.2), rnd.gauss(0, 1)])
        L = rnd.choice([0.0, rnd.uniform(0.1, 2.0), rnd.uniform(1.0, 8.0)])
        u0 = rnd.uniform(0, 1)
        A = [f32(anchor[j] - dv[j] * L * u0) for j in range(3)]
        B = [f32(A[j] + dv[j] * L) for j in range(3)] if not ball else [0.0, 0.0, 0.0]
        include = rnd.choice([2106] * 8 + [0x812, 0xFFFFFFFF, 0x20])
        states = rnd.choice([7] * 8 + [1, 3])
        cnt = rnd.choice([0, 0, 1, 1, 2])
        ex = [rnd.randrange(len(objs)) if rnd.random() < 0.5 else 0x5555 for _ in range(2)]
        f30 = struct.unpack('>Q', struct.pack('>d', f32(rnd.uniform(-60, 60))))[0]
        head = [int(ball), f30 >> 32, f30 & 0xFFFFFFFF] + [fb(v) for v in A + B] + [fb(R), include, states, cnt] + ex
        assert len(head) == 15 and all(len(x) == 18 for x in objs)  # (the reader's layout)
        cases.append((head, objs))
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'props.bin').write_bytes(blobp)
    (work / 'city.bin').write_bytes(blobc)
    (work / 'setups.bin').write_bytes(b''.join(sblobs))
    (work / 'setups.txt').write_text(''.join(' '.join('%x' % w for w in s) + '\n' for s in setups))
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%x' % w for w in h) + ' %x ' % len(o) + ' '.join(' '.join('%x' % w for w in x) for x in o) + '\n'
                           for h, o in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    T = lambda f: lq.addr(a.tool, f)
    sph, cap, cast = T('F-66549'), T('F-27454'), T('F-75805')
    material = int(T('F-10817'), 16)
    fit, sort, move = T('F-55382'), T('F-31335'), T('F-89464')
    inits = [T(f) for f in INITS]
    build = (BUILD.replace('NSET_', str(NSET)).replace('SETSZ_', '0x%Xu' % SETSZ).replace('SET_', '0x%08Xu' % SET).replace('NSW_', str(NSW))
             .replace('NW_', str(NW)).replace('FIT_', fit).replace('SORT_', sort).replace('MOVE_', move)
             .replace('INITCALLS', ' '.join('{Context g=fresh(); sub_%s(g,nullptr);}' % x for x in inits)))
    m = lq.MAIN
    anchor_blobs = ' memcpy(pointer(RESP_),blobp.data(),blobp.size()); memcpy(pointer(RESC_),blobc.data(),blobc.size());'
    assert m.count(anchor_blobs) == 1
    m = m.replace(anchor_blobs, anchor_blobs + build)
    anchor_kind = 'if(o[0]==0) b=GRID; else if(o[0]==1) b=o[1]; else {'
    assert m.count(anchor_kind) == 1
    m = m.replace(anchor_kind, 'if(o[0]==0) b=GRID; else if(o[0]==1) b=o[1]; else if(o[0]==3) b=0x%08Xu+0x%Xu*o[1]; else {' % (SET, SETSZ))
    anchor_reset = '  rt_reset();\n  for(uint32_t a=LV;a<CASTO+0x100;a+=4) store(a,0);'
    assert m.count(anchor_reset) == 1
    m = m.replace(anchor_reset, '  rt_reset(); store(0x40000200u+32,load(0x40000200u+28));\n  for(uint32_t a=LV;a<CASTO+0x100;a+=4) store(a,0);')
    main_src = (m.replace('RESP_', '0x%08Xu' % lq.RES_P).replace('RESC_', '0x%08Xu' % lq.RES_C).replace('VTP_', '0x%08Xu' % lq.VT_P)
                .replace('VTC_', '0x%08Xu' % lq.VT_C).replace('FAKE_MAT_', '0x%08Xu' % lq.FAKE_MAT).replace('MATERIAL_', '0x%08Xu' % material)
                .replace('LEVEL_GLOBAL_', '0x828DA464u').replace('SPH_', sph).replace('CAP_', cap))
    exe = rd.build(idx, ['sub_' + sph, 'sub_' + cap, 'sub_' + fit, 'sub_' + sort, 'sub_' + move] + ['sub_' + x for x in inits], main_src, work,
                   'level_car', image=img,
                   stubs={'sub_' + cast: 'if(ctx.r3.u32<0x%08Xu) ctx.r3.u64=0x40D06000;' % SET},
                   targets=[material, int(T('F-53082'), 16), int(T('F-19966'), 16)],
                   vtables=[(VT_POLY, [12, 40]), (VT_CURVED, [12, 40, 46]), (0x82083460, 2), (0x8207FF34, 41)],  # (+ the curved tests' result keeper and the swept capsule's vtable, as capsule_curved_reference.py)
                   fakes={lq.FAKE_MAT: 'ctx.r3.u64=(ctx.r4.u64+1)<<8;'}, limit=2400)
    r = subprocess.run([str(exe), str(a.image), str(inp), str(work / 'props.bin'), str(work / 'city.bin'), '%x' % grid,
                        str(work / 'setups.txt'), str(work / 'setups.bin')], capture_output=True, text=True)
    lines = [x for x in r.stdout.split('\n') if x]
    sl = [x for x in lines if x.startswith('S ')]
    out = [x for x in lines if not x.startswith('S ')]
    assert len(sl) == NSET and len(out) == len(cases), (len(sl), len(out), len(cases), r.stdout[-300:], r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    text = []
    for s, o in zip(setups, sl):
        text.append('S %s ' % cars[s[0]][0] + ' '.join('%x' % w for w in s) + ' -> ' + o[2:].split(' ', 1)[1])
    for (h, objs), o in zip(cases, out):
        text.append(' '.join('%x' % w for w in h) + ' | ' + ' | '.join(' '.join('%x' % w for w in x) for x in objs) + ' -> ' + o.strip())
    nl = chr(10)
    (a.out_dir / 'level_query_car_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(text) + nl))
    res = [o.split() for o in out]
    print(len(cases), 'cases;', sum(1 for x in res if x[11] != '00000000'), 'with the first record set')


if __name__ == '__main__':
    main()
