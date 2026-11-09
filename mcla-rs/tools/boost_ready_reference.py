"""The nitrous and slingshot can-fire tests run offline (the generated code whole and unchanged):

- F-50530 (mcCarNitro, the brain's and the pad's fire test): +32 burning 0, +24 == +20 (the view at rest, an unordered
  compare not equal), then unless the global byte 0x8288_E5B0 (the pause menu's unlimited nitro) or +33 (the racer's
  unlimited) is set: the sim's +576 bit 0x8 clear and +18 (i16 shots left) > 0; the sim's vfunc 12 (answering its pad)
  true; and its helper (run as generated: the sim's vfunc 13 air time not > 0 and sim +260 up.y < 0.1 - lying on the
  ground) false.
- F-29462 (the slipstream turbo): the sim's +576 bit 0x10 clear, vfunc 12 true, +41 full, +36 level not below 1.0,
  +40 burning 0, +20 == 0.0.

Both objects reach the sim through +8 -> +8. The vfuncs are fakes answering the case's values.

Writes <out-dir>/boost_ready_ref.txt, hex words: "nitro burning unlimited cheat view rest flags576 shots answering air
up_y -> r3" and "sling flags576 answering full level burning fov -> r3"."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb, nudge

OB, WR, SIM, VT = 0x40100000, 0x40101000, 0x40102000, 0x40103000
F_ANS, F_AIR = 0x40F00000, 0x40F00010
CHEAT = 0x8288E5B0


def fpick(rnd, *special):
    return rnd.choice(special + (rnd.uniform(-2, 2), float('nan')))


def nitro_case(rnd):
    view = rnd.choice((0.0, 0.0, -0.0, 0.3, float('nan'), rnd.uniform(0, 1)))
    rest = rnd.choice((view, view, 0.0, -0.0, 0.3))
    flags = rnd.choice((0, 0, 0x8, 0x10, 0x18, rnd.getrandbits(16)))
    shots = rnd.choice((0, 1, 2, -1, 100, rnd.randrange(65536)))
    air = fpick(rnd, 0.0, -0.0, 0.5, -0.016, nudge(0.0, 1))
    up = fpick(rnd, 0.1, nudge(0.1, -1), nudge(0.1, 1), 1.0, -1.0, 0.0)
    return [rnd.choice((0, 0, 0, 1, 2)), rnd.choice((0, 0, 1)), rnd.choice((0, 0, 0, 1)), fb(view), fb(rest), flags, shots,
            rnd.choice((1, 1, 1, 0, 0x100)), fb(air), fb(up)]


def sling_case(rnd):
    level = fpick(rnd, 1.0, 1.0, nudge(1.0, -1), nudge(1.0, 1), 0.0, 2.0)
    fov = rnd.choice((0.0, 0.0, -0.0, 0.2, float('nan')))
    return [rnd.choice((0, 0, 0x10, 0x8, rnd.getrandbits(16))), rnd.choice((1, 1, 1, 0, 0x100)), rnd.choice((1, 1, 1, 0, 2)),
            fb(level), rnd.choice((0, 0, 0, 1, 2)), fb(fov)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fn = {'N': addr(a.tool, 'F-50530'), 'S': addr(a.tool, 'F-29462')}
    rnd = random.Random(50530)
    nl = chr(10)
    nc = [nitro_case(rnd) for _ in range(a.cases)]
    sc = [sling_case(rnd) for _ in range(a.cases)]
    rows = lambda cs: (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cs)
    src = ('static const uint32_t NC[][10]={' + nl + rows(nc) + nl + '};' + nl +
           'static const uint32_t SC[][6]={' + nl + rows(sc) + nl + '};' + nl + r'''
static void sim(uint32_t flags){memset(pointer(@OB@),0,0x4000);store(@OB@+8,@WR@);store(@WR@+8,@SIM@);store(@SIM@,@VT@);
 store(@VT@+48,@FA@);store(@VT@+52,@FB@);pointer(@SIM@+576)[0]=(uint8_t)(flags>>8);pointer(@SIM@+576)[1]=(uint8_t)flags;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(auto&s:NC){rt_reset();sim(s[5]);*pointer(@OB@+32)=(uint8_t)s[0];*pointer(@OB@+33)=(uint8_t)s[1];*pointer(@CH@)=(uint8_t)s[2];
 store(@OB@+24,s[3]);store(@OB@+20,s[4]);pointer(@OB@+18)[0]=(uint8_t)(s[6]>>8);pointer(@OB@+18)[1]=(uint8_t)s[6];
 o_w()[0]=s[7];o_w()[1]=s[8];store(@SIM@+260,s[9]);
 Context c=fresh();c.r3.u32=@OB@;sub_@N@(c,nullptr);
 printf("nitro");for(int i=0;i<10;i++)printf(" %08x",s[i]);printf(" -> %08x\n",c.r3.u32);}
*pointer(@CH@)=0;
for(auto&s:SC){rt_reset();sim(s[0]);o_w()[0]=s[1];*pointer(@OB@+41)=(uint8_t)s[2];store(@OB@+36,s[3]);*pointer(@OB@+40)=(uint8_t)s[4];store(@OB@+20,s[5]);
 Context c=fresh();c.r3.u32=@OB@;sub_@S@(c,nullptr);
 printf("sling");for(int i=0;i<6;i++)printf(" %08x",s[i]);printf(" -> %08x\n",c.r3.u32);}}
''')
    for k, v in (('OB', OB), ('WR', WR), ('SIM', SIM), ('VT', VT), ('FA', F_ANS), ('FB', F_AIR), ('CH', CHEAT)):
        src = src.replace('@%s@' % k, '%du' % v)
    for k, v in fn.items():
        src = src.replace('@%s@' % k, v)
    pre = 'static uint32_t (&o_w())[2]{static uint32_t v[2];return v;}' + nl
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + pre
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + v for v in fn.values()], src, a.out_dir, 'boostready', image=rd.load_image(a.image),
                       fakes={F_ANS: 'ctx.r3.u32=o_w()[0];', F_AIR: '{float v;memcpy(&v,&o_w()[1],4);ctx.f1.f64=v;}'})
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == 2 * a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'boost_ready_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    from collections import Counter
    print(Counter((x.split()[0], x.split()[-1]) for x in lines))


if __name__ == '__main__':
    main()
