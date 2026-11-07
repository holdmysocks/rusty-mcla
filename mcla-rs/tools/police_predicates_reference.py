"""Run two of the police manager's small tests offline and record them for the port's tests
(crates/game/src/testdata/police_predicates_ref.txt; tests police::tests::damaged_out_matches_the_original and
police::tests::moving_target_matches_the_original).

F-12032 (racer r4): racer +48 -> car, car +20 -> its damage object, damage +8 -> its tune; damaged out = !(damage +36 <
tune +24 MaxDamage). Cases: MaxDamage 250, 256, 300, 400, 1000, 2000 x points 0, 0.5 max, 0.999 max, max - 1, the f32
just under max, max, the f32 just over, 2 max, +inf, NaN.
F-96080 (r3 the tactic's target record): +32 -> object, +48 -> racer, +8 -> car, car +224 the speed; moving = speed >
f32(tune +312 fMovingSpeed x 0.447 (the image's constant)). The police tune block sits in the image at 8282CAC8 (its
+312 is set per case). Cases: fMovingSpeed 0, 1, 2.5, 10 x speeds around the product (its f32 neighbours), 0, NaN.

Line per case: D <max bits> <points bits> <result>  /  M <fMovingSpeed bits> <speed bits> <result>
"""
import argparse
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
TUNE = IMG + 0x82CAC8

def fb(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]

def nb(b, d):
    return (b + d) & 0xFFFFFFFF

MAIN = r'''
static void sf(uint32_t a,uint32_t u){store(a,u);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 static const uint32_t D[][2]={DCASES};
 for(auto&c:D){rt_reset();const uint32_t R=0x40800000u,CAR=0x40801000u,DM=0x40802000u,TU=0x40803000u;
  for(uint32_t a=R;a<0x40804000u;a+=4)store(a,0);
  store(R+48,CAR);store(CAR+20,DM);store(DM+8,TU);sf(DM+36,c[1]);sf(TU+24,c[0]);
  Context x=fresh();x.r4.u32=R;DAMAGED(x,nullptr);printf("D %08x %08x %u\n",c[0],c[1],x.r3.u32&0xff);}
 static const uint32_t M[][2]={MCASES};
 for(auto&c:M){rt_reset();const uint32_t T=0x40800000u,O=0x40801000u,R=0x40802000u,CAR=0x40803000u;
  for(uint32_t a=T;a<0x40804000u;a+=4)store(a,0);
  store(T+32,O);store(O+48,R);store(R+8,CAR);sf(CAR+224,c[1]);sf(TUNE+312,c[0]);
  Context x=fresh();x.r3.u32=T;MOVING(x,nullptr);printf("M %08x %08x %u\n",c[0],c[1],x.r3.u32&0xff);}
}
'''

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    r = subprocess.run([a.tool, 'registry', 'addr', 'F-12032', 'F-96080'], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    dc = []
    for mx in (250.0, 256.0, 300.0, 400.0, 1000.0, 2000.0):
        m = fb(mx)
        for p in (fb(0.0), fb(mx * 0.5), fb(mx * 0.999), fb(mx - 1.0), nb(m, -1), m, nb(m, 1), fb(mx * 2), 0x7F800000, 0x7FC00000):
            dc.append((m, p))
    k = struct.unpack('>f', struct.pack('>f', 0.447))[0]
    mc = []
    for ms in (0.0, 1.0, 2.5, 10.0):
        prod = fb(struct.unpack('>f', struct.pack('>f', ms * k))[0])
        for s in (fb(0.0), nb(prod, -1), prod, nb(prod, 1), fb(ms), 0x7FC00000):
            mc.append((fb(ms), s))
    assert len(dc) < 1000 and len(mc) < 1000
    cs = lambda l: ','.join('{0x%08xu,0x%08xu}' % c for c in l)
    src = MAIN.replace('DCASES', cs(dc)).replace('MCASES', cs(mc)).replace('DAMAGED', name('F-12032')).replace('MOVING', name('F-96080')).replace('TUNE', '0x%08Xu' % TUNE)
    img = rdisp.load_image(a.image)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-12032'), name('F-96080')], src, a.out, 'police_predicates', image=img, limit=10)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, timeout=120)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.splitlines()
    assert len(lines) == len(dc) + len(mc), len(lines)
    a.ref.write_text('\n'.join(lines) + '\n')
    print(len(lines), 'cases ->', a.ref)

if __name__ == '__main__':
    main()
