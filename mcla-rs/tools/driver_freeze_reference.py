"""The drivers' distance LOD (F-12028, the mcCharacterPlus slot-16 update) run offline on the original's code: a racer's driver
freezes (its input context byte +48 = 1, the inputs reset once through F-55405) when its car is more than 20 m (x / z) from the
player's car, or from the camera when there is none. The callees that ask the game's state are stubs answering from the case:
F-52539 (a cine script running), F-65988 (the car is the player's), the online test (D7, always 0 here), the player-manager
lookups, and F-55405 itself (it only records that it was called).
Lines: `DF frozen active scene show show_scene no_damage b370 car cine player flags592 refkind rx rz cx cz px pz -> frozen reset`
(floats as the hex of their bits; refkind 0 no player, 1 the player's car, 2 player without a car instance, 3 player not valid)
Usage: driver_freeze_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

LOD = 'F-12028'
CINE, PLAYER, ONLINE, HASPLAYER, VALID, RESET = 'F-52539', 'F-65988', 'F-24338', 'F-37636', 'F-90764', 'F-55405'

MAIN = r'''
static void st8(uint32_t a,uint32_t v){*pointer(a)=uint8_t(v);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);unsigned fr,ac,sc,s5,s7,nd,b3,car,cine,pl,fl,rk;uint32_t rx,rz,cx,cz,px,pz;
 const uint32_t R=0x40800000,C=0x40801000,I=0x40802000,F=0x40803000,X=0x40804000,CAM=0x40805000,PM=0x40806000,P=0x40807000,PI=0x40808000,ANS=0x40809000;
 while(in>>std::dec>>fr>>ac>>sc>>s5>>s7>>nd>>b3>>car>>cine>>pl>>fl>>rk>>std::hex>>rx>>rz>>cx>>cz>>px>>pz){rt_reset();
  for(uint32_t a=R;a<ANS+0x100;a++)st8(a,0);
  st8(0x828CD214u,sc);
  st8(R+421,ac);st8(R+145,s5);st8(R+147,s7);st8(R+370,b3);store(R+96,car?C:0);store(R+412,X);st8(X+48,fr);
  store(C+48,I);store(I+8,F);store(F+592,fl);store(I+144+48,px);store(I+144+56,pz);
  store(0x8287E064u,CAM);st8(CAM+435,nd);uint32_t ci=CAM+(load(0x827E5640u)<<6)+240;store(ci,cx);store(ci+8,cz);
  store(0x82874374u,PM);store(PM+8,P);store(P+48,rk==2?0:PI);store(PI+144+48,rx);store(PI+144+56,rz);
  store(ANS+0,cine);store(ANS+4,pl);store(ANS+8,rk!=0);store(ANS+12,rk!=3);store(ANS+16,0);
  Context c=fresh();c.r3.u32=R;sub_LOD(c,nullptr);
  printf("%u %u\n",unsigned(*pointer(X+48)),load(ANS+16));}
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fn = 'sub_' + fid_addr(a.tool, LOD)
    base = 0x40809000
    stubs = {'sub_' + fid_addr(a.tool, CINE): f'ctx.r3.u64=load({base:#x}u);',
             'sub_' + fid_addr(a.tool, PLAYER): f'ctx.r3.u64=load({base + 4:#x}u);',
             'sub_' + fid_addr(a.tool, ONLINE): 'ctx.r3.u64=0;',
             'sub_' + fid_addr(a.tool, HASPLAYER): f'ctx.r3.u64=load({base + 8:#x}u);',
             'sub_' + fid_addr(a.tool, VALID): f'ctx.r3.u64=load({base + 12:#x}u);',
             'sub_' + fid_addr(a.tool, RESET): f'store({base + 16:#x}u,1);'}
    exe = rd.build(idx, [fn], MAIN.replace('sub_LOD', fn), a.work, 'driver_freeze', image=img, stubs=stubs)
    rnd = random.Random(20261104)
    cases = []

    def case(**k):
        d = dict(fr=0, ac=1, sc=0, s5=1, s7=0, nd=0, b3=0, car=1, cine=0, pl=0, fl=0, rk=1)
        d.update(k)
        r = d.pop('r', (0.0, 0.0))
        cam = d.pop('cam', (0.0, 0.0))
        p = d.pop('p', (0.0, 0.0))
        cases.append([d[n] for n in ('fr', 'ac', 'sc', 's5', 's7', 'nd', 'b3', 'car', 'cine', 'pl', 'fl', 'rk')] +
                     [bits(v) for v in (*r, *cam, *p)])

    def around(c, d):
        t = rnd.uniform(0, 2 * math.pi)
        return (c[0] + d * math.cos(t), c[1] + d * math.sin(t))

    # the boundary: exactly 400, and the floats next to 20 m
    for fr in (0, 1):
        for rk in (0, 1, 2, 3):
            for p in ((20.0, 0.0), (0.0, -20.0), (20.000002, 0.0), (19.999998, 0.0), (12.0, 16.0), (12.000001, 16.0)):
                case(fr=fr, rk=rk, r=(0.0, 0.0), cam=(0.0, 0.0), p=p)
    # every gate, near and far, frozen or not
    for _ in range(4000):
        r = (rnd.uniform(-3000, 3000), rnd.uniform(-3000, 3000))
        cam = around(r, rnd.choice((rnd.uniform(0, 40), rnd.uniform(19.99, 20.01))))
        ref = r if rnd.random() < 0.7 else cam
        p = around(ref, rnd.choice((rnd.uniform(0, 60), rnd.uniform(19.999, 20.001), rnd.uniform(0, 5000))))
        case(fr=rnd.randrange(2), ac=int(rnd.random() < 0.9), sc=int(rnd.random() < 0.1), s5=int(rnd.random() < 0.85),
             s7=int(rnd.random() < 0.2), nd=int(rnd.random() < 0.2), b3=int(rnd.random() < 0.3), car=int(rnd.random() < 0.95),
             cine=int(rnd.random() < 0.1), pl=int(rnd.random() < 0.1), fl=rnd.choice((0, 0, 0, 0x10, 0x04, 0x08, 0x14, 0x03)),
             rk=rnd.choice((0, 1, 1, 1, 2, 3)), r=r, cam=cam, p=p)
    cf = a.work / 'driver_freeze_cases.txt'
    cf.write_text(''.join(' '.join(map(str, c[:12])) + ' ' + ' '.join(f'{v:08x}' for v in c[12:]) + '\n' for c in cases))
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = [f'DF {" ".join(map(str, c[:12]))} {" ".join(f"{v:08x}" for v in c[12:])} -> {o}' for c, o in zip(cases, r.stdout.splitlines())]
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
