"""Run the original's speeding test F-36879 offline and record it, for the port's `police::speed_level`
(crates/game/src/testdata/speed_limit_ref.txt; test police::tests::speed_level_matches_the_original).

F-36879(racer r4, out level r5, out mph r6, out limit r7, out low / mid / high bands r8 / r9 / r10): no car +48, no +8 or no
navigation manager -> 0 and nothing written. Else the road node within reach of the matrix's origin (F-55639; stubbed: the
case's node or none), its limit (F-87157; stubbed: the case's value) x 2.237; no node or a product equal to 0 -> fSpeedLimitLow.
Each band: limit + offset against float(int percent + 100) x limit x 0.01, the first unless the difference is negative. The
car's speed (+8's +224) x 2.237 over the high, mid, low band -> 3, 2, 1, else 0; returns level != 0.

  <car> <obj> <mgr> <node> <node limit> <speed> <low> <off lo mid hi> <pct lo mid hi> (floats as bits, percents as ints) ->
  <ret> <level> <mph> <limit> <lo> <mid> <hi>  (or `<ret> -` when nothing is written)
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

TUNE = 0x8282CAC8            # the police tune globals: +236 fSpeedLimitLow, +240 / +248 / +256 offsets, +244 / +252 / +260 percents
MGR = 0x8286D8E4             # the navigation manager's pointer (the function's lis / lwz)

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t R=0x40800000u, CH=0x40810000u, O=0x40820000u, G=0x40A00000u, OUT=0x40B00000u;
 while(in>>tok){ if(tok!="case") break; uint32_t car,obj,mgr,node,w[9]; int32_t pct[3]; in>>car>>obj>>mgr>>node; for(int k=0;k<6;k++) in>>std::hex>>w[k]; in>>std::dec>>pct[0]>>pct[1]>>pct[2];
  rt_reset(); store(G,node); store(G+4,w[0]);
  store(R+48,car?CH:0); store(CH+8,obj?O:0); store(MGRADDR,mgr?0x40C00000u:0); store(O+224,w[1]);
  store(TUNEADDR+236,w[2]); for(int k=0;k<3;k++){ store(TUNEADDR+240+8*k,w[3+k]); store(TUNEADDR+244+8*k,(uint32_t)pct[k]); }
  for(uint32_t a=OUT;a<OUT+32;a+=4) store(a,0xDEADBEEFu);
  Context c=fresh(); c.r3.u32=0x40870000u; c.r4.u32=R; c.r5.u32=OUT; c.r6.u32=OUT+4; c.r7.u32=OUT+8; c.r8.u32=OUT+12; c.r9.u32=OUT+16; c.r10.u32=OUT+20;
  SL(c,nullptr);
  printf("%u",(unsigned)(c.r3.u32&0xff));
  if(load(OUT)==0xDEADBEEFu) printf(" -\n"); else { printf(" %u",load(OUT)); for(int k=1;k<6;k++) printf(" %08x",load(OUT+4*k)); printf("\n"); } }
 printf("end\n");
}
'''


def b(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def cases():
    g = random.Random(36879)
    out = []
    for i in range(10000):
        car = 1 if g.random() < 0.97 else 0
        obj = 1 if g.random() < 0.97 else 0
        mgr = 1 if g.random() < 0.97 else 0
        node = 1 if g.random() < 0.85 else 0
        nl = g.choice([0.0, 11.176, 13.4112, 17.8816, 22.352, 26.8224, 31.2928, g.uniform(0, 40), 1e-40])
        low = g.choice([50.0, 50.0, g.uniform(10, 80)])
        offs = [g.choice([d, d, g.uniform(-10, 60)]) for d in (15.0, 30.0, 45.0)]
        pcts = [g.choice([d, d, g.randrange(-50, 200)]) for d in (1, 50, 80)]
        # the speed: random, or on a band (the mph equal to a band's value up to rounding)
        lim = f32(f32(nl * f32(2.237))) if node and f32(nl * f32(2.237)) != 0 else low
        bands = [max(f32(lim + o), f32(f32(f32(p + 100) * lim) * f32(0.01))) for o, p in zip(offs, pcts)]
        sp = g.uniform(0, 80)
        if g.random() < 0.3:
            sp = f32(g.choice(bands) / f32(2.237))
            sp = g.choice([sp, struct.unpack('<f', struct.pack('<I', b(sp) + 1))[0], struct.unpack('<f', struct.pack('<I', max(b(sp) - 1, 0)))[0]])
        out.append((car, obj, mgr, node, [b(x) for x in [nl, sp, low] + offs], pcts))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-36879', 'F-35658', 'F-55639', 'F-87157', 'F-47121']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-35658'): 'ctx.r3.u64=ctx.r3.u32+0x1000u;',
             name('F-55639'): 'ctx.r3.u64=load(0x40A00000u)?0x40D00000u:0u;',
             name('F-87157'): '{uint32_t v=load(0x40A00004u); float x; memcpy(&x,&v,4); ctx.f1.f64=x;}',
             name('F-47121'): ''}
    src = MAIN.replace('SL', name('F-36879')).replace('TUNEADDR', '0x%08Xu' % TUNE).replace('MGRADDR', '0x%08Xu' % MGR)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-36879')], src, a.out, 'speed_limit', image=img, stubs=stubs)
    cs = cases()
    cf = a.out / 'speed_limit_cases.txt'
    line = lambda c: '%d %d %d %d %s %s' % (*c[:4], ' '.join('%08x' % w for w in c[4]), ' '.join('%d' % p for p in c[5]))
    cf.write_text(''.join('case %s\n' % line(c) for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    res = r.stdout.split('\n')[:-2]
    assert len(res) == len(cs)
    a.ref.write_text(''.join('%s -> %s\n' % (line(c), o) for c, o in zip(cs, res)))
    lv = [o.split()[1] for o in res if not o.endswith('-')]
    print('cases', len(res), 'levels', {k: lv.count(k) for k in '0123'}, '->', a.ref)


if __name__ == '__main__':
    main()
