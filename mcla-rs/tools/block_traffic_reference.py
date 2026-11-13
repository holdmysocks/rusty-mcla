"""Run the original's block traffic condition F-60011 offline and record it, for the port's `police::block_rule`
(crates/game/src/testdata/block_traffic_ref.txt; test police::tests::block_traffic_matches_the_original).

F-60011(manager r3, racer r4) as generated, with its world calls stubbed by the case: the nav global (8286D8E4), the racer's
tactic (F-59036) and the tactic's slot 56, the matrix getter (F-35658: the matrix at racer + 0x1000), the four polygon tests
(ai_road F-90092, ai_shortcut F-66554, ai_sidewalk F-75972, ai_grass F-86151), the road node and the junction ahead (F-65298:
the road through its out pointer, the junction returned), the racer's slot 40, the road's limit (F-87157), the junction's
green / yellow arm (F-75755 / F-93041) and the level probe (F-20855: S and E recorded, the hit from the case). The angle
F-88594 and pow F-21955 run as generated. The tune globals' fSpeedLimitLow (+236) and iBlockTrafficSpeedPrcnt (+280) from
the case.

  case <nav> <tactic> <vt56> <car> <chassis> <road_poly> <shortcut> <sidewalk> <grass> <road> <junction> <arms> <racer>
       <lane> <bound> <hit> <pct> then floats as bits: speed limit low pos(xyz) row_b(xyz) row_c(xyz) box16(xyz) box32(xyz)
       centre(xyz) reach green(xyz) yellow(xyz)
  -> <result> [S(xyz) E(xyz)]   (S / E when the probe ran)
"""
import argparse
import math
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

TUNE = 0x8282CAC8
NAV = 0x8286D8E4             # lis -32121 / lwz -10012
NI, NW = 17, 28              # ints, float words per case

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok;
 const uint32_t MG=0x40700000u, R=0x40800000u, RVT=0x40880000u, CH=0x40810000u, O=0x40820000u, TA=0x40840000u, TVT=0x40848000u,
  B1=0x40850000u, B2=0x40851000u, B3=0x40852000u, J=0x40860000u, GR=0x40861000u, YE=0x40862000u, RD=0x40863000u, G=0x40A00000u;
 for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFFu);store(0x82860930+4*k,0);} store(0x8286095C,0); store(0x8286093C,0x3F800000u);
 while(in>>tok){ if(tok!="case") break; uint32_t v[NI],w[NW]; for(int k=0;k<NI;k++) in>>v[k]; for(int k=0;k<NW;k++) in>>std::hex>>w[k]; in>>std::dec;
  rt_reset();
  for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFFu);store(0x82860930+4*k,0);} store(0x8286095C,0); store(0x8286093C,0x3F800000u);
  store(NAVADDR, v[0]?0x40710000u:0);
  store(G+0, v[1]?TA:0); store(TA,TVT); store(TVT+56,0x7F000056u); store(G+4,v[2]);
  store(R,RVT); store(RVT+40,0x7F000040u); store(G+8,v[12]);
  store(R+48, v[3]?CH:0); store(CH+8, v[4]?O:0); store(O+224,w[0]);
  store(G+12,v[5]); store(G+16,v[6]); store(G+20,v[7]); store(G+24,v[8]);
  store(G+28, v[9]?RD:0); store(G+32, v[10]?J:0);
  store(R+640, v[13]?0x40870000u:0);
  store(G+36,w[1]); store(TUNEADDR+236,w[2]); store(TUNEADDR+280,v[16]);
  for(int k=0;k<3;k++){ store(R+0x1000+48+4*k,w[3+k]); store(R+0x1000+16+4*k,w[6+k]); store(R+0x1000+32+4*k,w[9+k]); }
  store(R+0x1000+60,0x3F800000u); store(R+0x1000+28,0); store(R+0x1000+44,0);
  store(CH+28, v[14]?B1:0); store(B1+4,B2); store(B2+12,B3);
  for(int k=0;k<3;k++){ store(B3+16+4*k,w[12+k]); store(B3+32+4*k,w[15+k]); } store(B3+28,0); store(B3+44,0);
  store16(J+48,(uint16_t)v[11]); for(int k=0;k<3;k++){ store(J+80+4*k,w[18+k]); store(GR+4*k,w[22+k]); store(YE+4*k,w[25+k]); }
  store(J+92,0); store(GR+12,0); store(YE+12,0); store(J+64,w[21]);
  store(G+40,v[15]); store(G+44,0);
  Context c=fresh(); c.r3.u32=MG; c.r4.u32=R;
  BT(c,nullptr);
  printf("%u",(unsigned)(c.r3.u32&0xff));
  if(load(G+44)) { for(int k=0;k<6;k++) printf(" %08x",load(G+48+4*k)); }
  printf("\n"); }
 printf("end\n");
}
'''


def b(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def unit(g):
    while True:
        v = [g.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(t * t for t in v))
        if n > 0.1:
            return [t / n for t in v]


def cases():
    g = random.Random(60011)
    out = []
    for i in range(8000):
        pick = lambda p: 1 if g.random() < p else 0
        nav, tactic, vt56 = pick(0.97), pick(0.2), g.choice([0, 1, 1, 2, 3])
        car, chassis = pick(0.97), pick(0.97)
        road_poly, shortcut, sidewalk, grass = pick(0.9), pick(0.06), pick(0.06), pick(0.06)
        road, junction, arms = pick(0.85), pick(0.7), g.choice([0, 0, 1, 3, 4])
        racer, lane, bound, hit = pick(0.9), pick(0.92), pick(0.96), pick(0.5)
        pct = g.choice([10, 10, 10, 0, 25, 100, -5])
        low = g.choice([50.0, 50.0, 30.0, 0.0, g.uniform(0, 80)])
        limit = g.choice([0.0, 0.0, 13.41, 22.35, g.uniform(0, 40), float('nan'), -0.0])
        lim_eff = limit if limit else low * 0.447
        thr = pct * 0.01 * (lim_eff if lim_eff == lim_eff else 0)
        speed = g.choice([0.0, g.uniform(0, 3), 2.235, struct.unpack('<f', struct.pack('<f', 2.235))[0] * 1.0000001,
                          g.uniform(0, max(thr, 0.1)), thr, g.uniform(0, 30), float('nan')])
        pos = [g.uniform(-2000, 2000), g.uniform(-20, 60), g.uniform(-2000, 2000)]
        a = g.uniform(-math.pi, math.pi)
        fwd = [math.sin(a), 0.0, math.cos(a)] if g.random() < 0.6 else unit(g)
        up = [0.0, 1.0, 0.0] if g.random() < 0.6 else unit(g)
        row_c = [-t for t in fwd]
        hi = [g.uniform(0.7, 1.2), g.uniform(1.0, 1.8), g.uniform(1.8, 2.8)]
        lo = [-g.uniform(0.7, 1.2), g.uniform(-0.6, 0.0), -g.uniform(1.8, 2.8)]
        if g.random() < 0.2:
            hi, lo = lo, hi
        reach = g.choice([20.0, 30.0, 40.0, g.uniform(0, 60), float('nan')])
        r = 0.5 * (reach if reach == reach else 0) + 10.0
        d = g.choice([g.uniform(0, r * 1.2), r, g.uniform(0, 80), g.uniform(0, 5)])
        b_ = g.uniform(-math.pi, math.pi)
        cy = g.choice([0.0, pos[1], g.uniform(-5, 30)])
        centre = [pos[0] + d * math.sin(b_), cy, pos[2] + d * math.cos(b_)]
        to = [centre[0] - pos[0], centre[1], centre[2] - pos[2]]

        def arm():
            k = g.random()
            if k < 0.3:
                return [0.0, 0.0, 0.0]
            if k < 0.35:
                return [-0.0, 0.0, -0.0]
            n = math.sqrt(sum(t * t for t in to)) or 1
            if k < 0.6:      # near 150 degrees from the direction to the centre
                t = [x / n for x in to]
                p = unit(g)
                dd = sum(x * y for x, y in zip(p, t))
                p = [x - dd * y for x, y in zip(p, t)]
                pn = math.sqrt(sum(x * x for x in p)) or 1
                ang = 2.61799 + g.uniform(-0.01, 0.01)
                return [math.cos(ang) * x + math.sin(ang) * y / pn for x, y in zip(t, p)]
            return unit(g)
        green, yellow = arm(), arm()
        ints = [nav, tactic, vt56, car, chassis, road_poly, shortcut, sidewalk, grass, road, junction, arms, racer, lane, bound, hit,
                pct]
        fl = [speed, limit, low] + pos + up + row_c + hi + lo + centre + [reach] + green + yellow
        out.append((ints, [b(x) for x in fl]))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-60011', 'F-59036', 'F-35658', 'F-90092', 'F-66554', 'F-75972', 'F-86151', 'F-65298', 'F-87157', 'F-75755', 'F-93041',
           'F-20855']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    G = 0x40A00000
    flag = lambda off: 'ctx.r3.u64=load(0x%08Xu);' % (G + off)
    stubs = {name('F-59036'): flag(0),
             name('F-35658'): 'ctx.r3.u64=ctx.r3.u32+0x1000u;',
             name('F-90092'): flag(12), name('F-66554'): flag(16), name('F-75972'): flag(20), name('F-86151'): flag(24),
             name('F-65298'): 'store(ctx.r5.u32,load(0x%08Xu)); ctx.r3.u64=load(0x%08Xu);' % (G + 28, G + 32),
             name('F-87157'): 'uint32_t u=load(0x%08Xu); float f; memcpy(&f,&u,4); ctx.f1.f64=f;' % (G + 36),
             name('F-75755'): 'ctx.r3.u64=0x40861000u;', name('F-93041'): 'ctx.r3.u64=0x40862000u;',
             name('F-20855'): ('store(0x%08Xu,1); for(int k=0;k<3;k++){store(0x%08Xu+4*k,load(ctx.r4.u32+4*k));'
                               'store(0x%08Xu+4*k,load(ctx.r4.u32+16+4*k));} store(ctx.r5.u32,load(0x%08Xu));')
             % (G + 44, G + 48, G + 60, G + 40)}
    fakes = {0x7F000056: 'ctx.r3.u64=load(0x%08Xu);' % (G + 4), 0x7F000040: 'ctx.r3.u64=load(0x%08Xu);' % (G + 8)}
    src = (MAIN.replace('BT', name('F-60011')).replace('TUNEADDR', '0x%08Xu' % TUNE).replace('NAVADDR', '0x%08Xu' % NAV)
           .replace('NI', str(NI)).replace('NW', str(NW)))
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-60011')], src, a.out, 'block_traffic', image=img, stubs=stubs, fakes=fakes)
    cs = cases()
    cf = a.out / 'block_traffic_cases.txt'
    line = lambda c: '%s %s' % (' '.join(str(x) for x in c[0]), ' '.join('%08x' % w for w in c[1]))
    cf.write_text(''.join('case %s\n' % line(c) for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=300)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    res = r.stdout.split('\n')[:-2]
    assert len(res) == len(cs)
    a.ref.write_text(''.join('%s -> %s\n' % (line(c), o) for c, o in zip(cs, res)))
    print('cases', len(res), 'blocking', sum(o.startswith('1') for o in res), 'probed', sum(len(o.split()) > 1 for o in res), '->', a.ref)


if __name__ == '__main__':
    main()
