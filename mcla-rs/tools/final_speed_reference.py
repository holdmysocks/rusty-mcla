"""Run the original's Pursuit final speed (F-49680, the Pursuit brain state's slot 24) offline and record it, for the port's
`police::pursuit_final_speed` (crates/game/src/testdata/final_speed_ref.txt; test police::tests::final_speed_matches_the_original).

F-49680 runs with every test it calls unchanged: F-42951 (the cop object +564's byte +872 and a race, F-27863, stubbed), F-43975 (moving: the target car's +224 over fMovingSpeed x 0.447), F-29545 / F-39842 (within 2 x fSafeDist on the
squared distance), F-80249 (the two velocities), F-53459 / F-99390 (the cop's row c against the target), F-41260 (the speed)
and the default F-85634. The state +12 -> +20 is the brain (+4 the cop car, +680 the target car, +564 the cop object); each
car as tools/squad_swipe_reference.py lays it out (matrix through +48 -> +28 -> +16, the car +48 -> +8: +224 the speed,
+128 -> +272 the velocity). The tune block (fSafeDist +192, fMovingSpeed +312) is written in the image.

  s<safe> m<moving speed> o<object: 0 none, 1 byte 0, 2 byte 1 no race, 3 byte 1 race> c<pos,row c,vel> t<pos,vel,speed> -> <f1>
"""
import argparse
import math
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

TUNE = 0x8282CAC8               # the police tune block (-13624 off 0x82830000)

MAIN = r'''
static void store8(uint32_t a,uint8_t v){*pointer(a)=v;}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t S=0x40800000u,F=0x40810000u,B=0x40820000u,O=0x40830000u,CV=0x40A00000u,TV=0x40B00000u,RACE=0x40C00000u;
 auto car=[](uint32_t v){store(v+48,v+0x400);store(v+0x400+28,v+0x800);store(v+0x400+8,v+0xC00);store(v+0xC00+128,v+0x1000);};
 auto rd=[&](uint32_t a){uint32_t u; in>>std::hex>>u>>std::dec; store(a,u);};
 while(in>>tok){
  if(tok!="case") break; int mode; rt_reset();
  for(uint32_t a=S;a<S+0x40000;a+=4) store(a,0);
  rd(TUNEA+192); rd(TUNEA+312); in>>mode;
  car(CV); car(TV);
  for(int i=0;i<3;i++) rd(CV+0x800+16+48+4*i); for(int i=0;i<3;i++) rd(CV+0x800+16+32+4*i); for(int i=0;i<3;i++) rd(CV+0x1000+272+4*i);
  for(int i=0;i<3;i++) rd(TV+0x800+16+48+4*i); for(int i=0;i<3;i++) rd(TV+0x1000+272+4*i); rd(TV+0xC00+224);
  store(S+12,F); store(F+20,B); store(B+4,CV); store(B+680,TV);
  store(B+564,mode==0?0u:O); store8(O+872,mode>=2?1:0); store(RACE,mode==3?1u:0u);
  Context c=fresh(); c.r3.u32=S; FS(c,nullptr);
  float f=(float)c.f1.f64; uint32_t u; memcpy(&u,&f,4); printf("%08x\n",u); }
 printf("end\n");
}
'''


def f32(v):
    return struct.unpack('<f', struct.pack('<f', v))[0]


def fbits(v):
    return struct.unpack('<I', struct.pack('<f', v))[0]


def cases():
    g = random.Random(49680)
    out = []
    for _ in range(4000):
        safe = f32(g.choice((40.0, 40.0, 40.0, 25.0, g.uniform(1, 120))))
        mv = f32(g.choice((1.0, 1.0, 5.0, g.uniform(0, 10))))
        mode = g.choice((0, 1, 2, 2, 2, 3))
        cpos = [f32(g.uniform(-3000, 3000)), f32(g.uniform(-10, 100)), f32(g.uniform(-3000, 3000))]
        th = g.uniform(0, 2 * math.pi)
        tilt = lambda: g.uniform(-0.05, 0.05)
        back = [f32(math.sin(th)), f32(tilt()), f32(math.cos(th))]
        r = 2 * safe
        u = g.random()
        if u < 0.15:                                   # (on the 2 x fSafeDist ring: the squared test's edge)
            d = f32(r) * g.choice((1.0, 1.0 + 2 ** -22, 1.0 - 2 ** -22, 1.0 + 2 ** -20, 1.0 - 2 ** -20))
        elif u < 0.6:
            d = g.uniform(0, r)
        else:
            d = g.uniform(0, 3 * r)
        a, e = g.uniform(0, 2 * math.pi), g.uniform(-0.1, 0.1)
        tpos = [f32(cpos[0] + d * math.cos(a)), f32(cpos[1] + d * e), f32(cpos[2] + d * math.sin(a))]
        if g.random() < 0.03:
            tpos = list(cpos)
        thr = mv * 0.447
        sp = g.choice((0.0, g.uniform(0, 2 * thr), g.uniform(0, 60), f32(f32(mv) * f32(0.447)), 30.0))
        sp = f32(sp)
        hd = g.uniform(0, 2 * math.pi)
        tvel = [f32(sp * math.cos(hd)), f32(g.uniform(-0.5, 0.5)), f32(sp * math.sin(hd))]
        cs = g.uniform(0, 50)
        ch = hd + g.choice((0.0, g.uniform(-1.5, 1.5), g.uniform(0, 2 * math.pi), math.pi / 2))
        cvel = [f32(cs * math.cos(ch)), f32(g.uniform(-0.5, 0.5)), f32(cs * math.sin(ch))]
        if g.random() < 0.05:
            cvel = [0.0, 0.0, 0.0]
        out.append((safe, mv, mode, cpos, back, cvel, tpos, tvel, sp))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-49680', 'F-42951', 'F-27863']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    # (the race F-42951 asks for, F-27863: stubbed by the case's race word)
    stubs = {name('F-27863'): 'ctx.r3.u64=load(0x40C00000u);'}
    src = MAIN.replace('FS(', name('F-49680') + '(').replace('TUNEA', '0x%08Xu' % TUNE)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-49680')], src, a.out, 'fspeed', image=img, stubs=stubs)
    cs = cases()
    cf = a.out / 'fspeed_cases.txt'
    xs = lambda v: ' '.join('%08x' % fbits(q) for q in v)
    with cf.open('w') as f:
        for safe, mv, mode, cpos, back, cvel, tpos, tvel, sp in cs:
            f.write('case %08x %08x %d %s %s %s %s %s %08x\n' % (fbits(safe), fbits(mv), mode, xs(cpos), xs(back), xs(cvel), xs(tpos), xs(tvel), fbits(sp)))
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.split('\n')[:-2]
    assert len(lines) == len(cs)
    xc = lambda v: ','.join('%08x' % fbits(q) for q in v)
    rows = []
    for (safe, mv, mode, cpos, back, cvel, tpos, tvel, sp), l in zip(cs, lines):
        rows.append('s%08x m%08x o%d c%s,%s,%s t%s,%s,%08x -> %s' % (fbits(safe), fbits(mv), mode, xc(cpos), xc(back), xc(cvel), xc(tpos), xc(tvel), fbits(sp), l))
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
