"""The traffic footprint probe's level segment query run whole offline on the original's code: F-57005 with r4 = 1 (the
synchronous path F-39379 takes), from its parameter block to the record it reads - the query object's ctor F-91446, the
set-up F-46779 with one record, the material mask (the block's +16 shifted by the material manager's flag shift, 8 here),
the excluded list with its callback F-58895, the segment driver F-97077 with the culler's ctor F-33380 and segment volume
F-33196, the culler's walk over a level (as level_query_reference.py), the frame step F-58085, the prune F-71512 ->
F-58997, the dispatch F-29290 with everything below it (F-75650's primitives F-53035 / F-40312 / F-54304 and the curved
cast F-60260 -> F-81943, the polygon test F-20854, the grid's cell walk F-31396 / F-87746, the cells' BVH walk F-27068 /
F-21427, the composites' parts F-97104 / F-17017), the commit F-77105 with r5 = 0.

The level is level_query_car_reference.py's: the city's grid, the prop types' composites, walker capsules and the player
car's composite as the running game sets it up (60 set-ups: hull type 4 and curved wheels type 5 with their real
vtables). A case uses its header as there with word 0 = 2 (the segment), word 1 the block's +16 (the material mask
before the shift, 512 as F-39379 sets it), word 2 = 0, then start, end, word 9 unused, the include mask +20, the states
+24, the excluded count +4 and words +8 / +12.
Out: 0, the query object's list (+36, +44 .. +84, read from F-57005's stack frame after the call) and the record at the
block's +96. Writes <out-dir>/level_segment_ref.txt (the set-ups' lines first, 'S'). Test
`camprobe::tests::level_segment_matches_the_original`."""
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
import level_query_reference as lq
import level_query_car_reference as lqc

fb, f32 = lq.fb, lq.f32

SEG_BLOCK = r'''  // F-57005's block (F-39379's at its sp + 96)
  for(int j=0;j<3;j++){ store(BLK+32+4*j,h[3+j]); store(BLK+48+4*j,h[6+j]); } store(BLK+44,0); store(BLK+60,0);
  store(BLK+16,h[1]); store(BLK+20,h[10]); *pointer(BLK+24)=h[11]; store(BLK+4,h[12]);
  for(int j=0;j<2;j++){ unsigned w=h[13+j]; store(BLK+8+4*j, w<0x100? INST+0x100*w : w); }
  uint32_t recs=BLK+96, nrec=1;
  for(uint32_t a=recs;a<recs+96*nrec;a+=4) store(a,0xDEADBEEF);
  Context c=fresh(); c.r1.u64=STK; c.r3.u64=BLK; c.r4.u64=1;
  try{ sub_SEG_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  uint32_t q=STK-2608+80;
  printf("%08x",0u);
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, default=Path('D:/MCLA/extracted/cache'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    ap.add_argument('--seed', type=int, default=39379)
    a = ap.parse_args()
    props = str(a.data / 'resources/prop/0x0e66973c.r131')
    city = str(a.data / 'resources/city/sc/0x3e5fd91d.r1')
    blobp, types = lq.load_props(props)
    blobc, grid, walls = lq.load_city(city)
    cars = []
    for dr in sorted(glob.glob(str(a.data / 'resources/vehicle/vp*'))):
        for f in sorted(glob.glob(dr + '/*.r83')):
            if lqc.car_blob(f, lqc.SET):
                cars.append((os.path.basename(dr), f))
                break
    assert cars
    rnd = random.Random(a.seed)
    setups, sblobs = [], []
    picks = list(range(len(cars)))
    rnd.shuffle(picks)
    for k in range(lqc.NSET):
        ci = picks[k % len(picks)]
        blob, n, places = lqc.car_blob(cars[ci][1], lqc.SET + lqc.SETSZ * k)
        nw = n - 1
        flag = k % 2
        tune = [fb(rnd.uniform(0.05, 0.35)), fb(rnd.uniform(0.0, 0.8)), fb(0.0 if rnd.random() < 0.3 else rnd.uniform(0.2, 0.7)), fb(rnd.uniform(0.3, 1.0))]
        drop = fb(0.0 if rnd.random() < 0.5 else rnd.uniform(0.0, 0.15))
        tyres = [lqc.tyre(rnd) for _ in range(2)]
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
        assert len(sw) == lqc.NSW  # (the reader's layout)
        setups.append(sw)
        sblobs.append(blob)
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    anyt = [i for i, t in enumerate(types) if t]
    cases = []
    while len(cases) < a.cases:
        objs = []
        kind = rnd.random()
        if kind < 0.35:
            anchor = [w + rnd.gauss(0, 1.0) for w in rnd.choice(walls)]
        elif kind < 0.75:
            anchor = None
        else:
            anchor = [rnd.uniform(-1500, 1500), rnd.uniform(-5, 60), rnd.uniform(-1500, 1500)]
        if anchor is None or rnd.random() < 0.6:
            k = rnd.randrange(lqc.NSET)
            rot = lq.rotation(rnd, rnd.random() < 0.85)
            place = [rnd.uniform(-1500, 1500), rnd.uniform(-5, 60), rnd.uniform(-1500, 1500)] if anchor is None else [anchor[j] + rnd.gauss(0, 2.0) for j in range(3)]
            if anchor is None and rnd.random() < 0.3:
                place = [w_ + rnd.gauss(0, 2.0) for w_ in rnd.choice(walls)]
            objs.append([3, k, 0, 0, rnd.choice([0x189, 0x185, 0x1100]), 0xFFFFFFFF] + [fb(f32(v)) for row in rot + [place] for v in row])
            if anchor is None:
                local = [rnd.gauss(0, 1.2), rnd.gauss(0.5, 0.6), rnd.gauss(0, 2.2)]
                anchor = [place[j] + sum(local[m] * rot[m][j] for m in range(3)) for j in range(3)]
        if rnd.random() < 0.5:
            objs.append([0, 0, 0, 0, 0x900, 0xFFFFFFFF] + [fb(v) for v in (1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0)])
        for _ in range(rnd.choice([0, 0, 1, 1, 2])):
            ti = rnd.choice(anyt)
            b, cen, rad = types[ti]
            r2 = lq.rotation(rnd, rnd.random() < 0.7)
            near = [anchor[j] + rnd.gauss(0, 0.6 * rad + 0.5) for j in range(3)]
            pl2 = [near[j] - sum(cen[m] * r2[m][j] for m in range(3)) for j in range(3)]
            objs.append([1, b, ti, 0, 2, 0xFFFFFFFF] + [fb(f32(v)) for row in r2 + [pl2] for v in row])
        for _ in range(rnd.choice([0, 0, 0, 1, 2])):
            R, L = f32(rnd.choice([0.3, 0.35, rnd.uniform(0.2, 0.5)])), f32(rnd.choice([1.1, rnd.uniform(0.6, 1.4)]))
            r2 = lq.rotation(rnd, True)
            pl2 = [anchor[j] + rnd.gauss(0, 0.8) for j in range(3)]
            objs.append([2, fb(R), fb(L), 0, 0x20, 0xFFFFFFFF] + [fb(f32(v)) for row in r2 + [pl2] for v in row])
        if not objs:
            continue
        rnd.shuffle(objs)
        for o in objs:
            o[3] = rnd.choice([0, 0, 0, 0, 1, 2, 3])
            if rnd.random() < 0.08:
                o[4] = rnd.choice([0x189, 0x185, 0x1100, 0x2, 0x20, 0x900])
            if rnd.random() < 0.04:
                o[5] = rnd.choice([0, 0x10])
        # (the probe's segment: level, about 2 x 0.9 half lengths long; some of any direction and length)
        if rnd.random() < 0.7:
            dv = unit([rnd.gauss(0, 1), rnd.gauss(0, 0.05), rnd.gauss(0, 1)])
            L = rnd.uniform(2.5, 9.0)
        else:
            dv = unit([rnd.gauss(0, 1), rnd.gauss(0, 0.6), rnd.gauss(0, 1)])
            L = rnd.choice([0.0, rnd.uniform(0.0, 0.01), rnd.uniform(0.1, 2.0), rnd.uniform(1.0, 30.0)])
        u0 = rnd.uniform(0, 1)
        A = [f32(anchor[j] - dv[j] * L * u0) for j in range(3)]
        B = [f32(A[j] + dv[j] * L) for j in range(3)]
        base = rnd.choice([512] * 10 + [0, 1, 2, 3, 0x20, 0x7FFFFF])
        include = rnd.choice([14366] * 10 + [2106, 0x812, 0xFFFFFFFF, 0x20])
        states = rnd.choice([7] * 8 + [1, 3])
        cnt = rnd.choice([0, 1, 1, 1, 2])
        ex = [rnd.randrange(len(objs)) if rnd.random() < 0.4 else 0x5555 for _ in range(2)]
        head = [2, base, 0] + [fb(v) for v in A + B] + [0, include, states, cnt] + ex
        assert len(head) == 15 and 0 < len(objs) <= 4096 and all(len(x) == 18 for x in objs)  # (the reader's layout)
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
    seg, cast = T('F-57005'), T('F-75805')
    material = int(T('F-10817'), 16)
    fit, sort, move = T('F-55382'), T('F-31335'), T('F-89464')
    inits = [T(f) for f in lqc.INITS]
    build = (lqc.BUILD.replace('NSET_', str(lqc.NSET)).replace('SETSZ_', '0x%Xu' % lqc.SETSZ).replace('SET_', '0x%08Xu' % lqc.SET)
             .replace('NSW_', str(lqc.NSW)).replace('NW_', str(lqc.NW)).replace('FIT_', fit).replace('SORT_', sort).replace('MOVE_', move)
             .replace('INITCALLS', ' '.join('{Context g=fresh(); sub_%s(g,nullptr);}' % x for x in inits)))
    # (the start-up constants the segment code reads: printed once so the port's assumptions are on record)
    build = build.replace('   if(load(0x82860950)', '   printf("C %08x %08x %08x %08x\\n",load(0x828608C0),load(0x828608D0),load(0x828608E0),load(0x828608E4));\n   if(load(0x82860950)', 1)
    m = lq.MAIN
    anchor_blobs = ' memcpy(pointer(RESP_),blobp.data(),blobp.size()); memcpy(pointer(RESC_),blobc.data(),blobc.size());'
    assert m.count(anchor_blobs) == 1
    m = m.replace(anchor_blobs, anchor_blobs + build)
    anchor_kind = 'if(o[0]==0) b=GRID; else if(o[0]==1) b=o[1]; else {'
    assert m.count(anchor_kind) == 1
    m = m.replace(anchor_kind, 'if(o[0]==0) b=GRID; else if(o[0]==1) b=o[1]; else if(o[0]==3) b=0x%08Xu+0x%Xu*o[1]; else {' % (lqc.SET, lqc.SETSZ))
    anchor_reset = '  rt_reset();\n  for(uint32_t a=LV;a<CASTO+0x100;a+=4) store(a,0);'
    assert m.count(anchor_reset) == 1
    m = m.replace(anchor_reset, '  rt_reset(); store(0x40000200u+32,load(0x40000200u+28));\n  for(uint32_t a=LV;a<CASTO+0x100;a+=4) store(a,0);')
    i0 = m.index("  // F-23371's block")
    i1 = m.index('  for(int o2: {36,44')
    m = m[:i0] + SEG_BLOCK + m[i1:]
    assert m.count('bool ball=h[0]!=0;') == 1
    m = m.replace('bool ball=h[0]!=0;', 'if(h[0]!=2){fprintf(stderr,"bad case kind\\n"); return 2;}', 1)
    main_src = (m.replace('RESP_', '0x%08Xu' % lq.RES_P).replace('RESC_', '0x%08Xu' % lq.RES_C).replace('VTP_', '0x%08Xu' % lq.VT_P)
                .replace('VTC_', '0x%08Xu' % lq.VT_C).replace('FAKE_MAT_', '0x%08Xu' % lq.FAKE_MAT).replace('MATERIAL_', '0x%08Xu' % material)
                .replace('LEVEL_GLOBAL_', '0x828DA464u').replace('SEG_', seg))
    exe = rd.build(idx, ['sub_' + seg, 'sub_' + fit, 'sub_' + sort, 'sub_' + move] + ['sub_' + x for x in inits], main_src, work,
                   'level_seg', image=img,
                   stubs={'sub_' + cast: 'if(ctx.r3.u32<0x%08Xu) ctx.r3.u64=0x40D06000;' % lqc.SET},
                   targets=[material, int(T('F-58895'), 16)],
                   vtables=[(lqc.VT_POLY, [12, 40]), (lqc.VT_CURVED, [12, 40, 46]), (0x82083460, 2), (0x8207FF34, 41)],
                   fakes={lq.FAKE_MAT: 'ctx.r3.u64=(ctx.r4.u64+1)<<8;'}, limit=2600)
    r = subprocess.run([str(exe), str(a.image), str(inp), str(work / 'props.bin'), str(work / 'city.bin'), '%x' % grid,
                        str(work / 'setups.txt'), str(work / 'setups.bin')], capture_output=True, text=True)
    lines = [x for x in r.stdout.split('\n') if x]
    cl = [x for x in lines if x.startswith('C ')]
    sl = [x for x in lines if x.startswith('S ')]
    out = [x for x in lines if not x.startswith('S ') and not x.startswith('C ')]
    assert len(sl) == lqc.NSET and len(out) == len(cases), (len(sl), len(out), len(cases), r.stdout[-300:], r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    print(cl)
    text = list(cl)
    for s, o in zip(setups, sl):
        text.append('S %s ' % cars[s[0]][0] + ' '.join('%x' % w for w in s) + ' -> ' + o[2:].split(' ', 1)[1])
    for (h, objs), o in zip(cases, out):
        text.append(' '.join('%x' % w for w in h) + ' | ' + ' | '.join(' '.join('%x' % w for w in x) for x in objs) + ' -> ' + o.strip())
    nl = chr(10)
    (a.out_dir / 'level_segment_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(text) + nl))
    res = [o.split() for o in out]
    print(len(cases), 'cases;', sum(1 for x in res if x[11] != '00000000'), 'with the record set')


if __name__ == '__main__':
    main()
