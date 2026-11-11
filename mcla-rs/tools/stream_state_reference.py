"""A racer's stream state machine F-60675 (racer vfunc 116, stepped each frame for every opponent by F-45069) run offline,
the generated code whole and unchanged.

The racer at R: +0 vtable (a table whose +128 / +132 are fake functions that log), +16 evict request, +17 load request
(bytes), +28 the load wait (s32), +48 the car (C: +12 its instance + 16 or 0, +80 its world), +52 / +60 name holders
(0), +140 uuid, +656 the state, +664, +668 the evict wait. The callees: the log F-47121 (printf to the debug channel,
recorded), F-98197 (take the car out of the world: recorded with r3 r4), the vinyl test (returns the case's flag) and
the vinyl pump (recorded). The word the load wait is copied from (in the image, 6) is set per case.

Writes <out-dir>/stream_state_ref.txt: "case" + state 16 17 28 668 global car-instance vinyl + "->" + state 16 17 28
664 668 + the call log (L log, T take-out r3 r4, V vinyl test, W vinyl pump, A vfunc 128, B vfunc 132)."""
import argparse
import random
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr

R = 0x40200000
C = 0x40210000
VT = 0x40220000
FAKE_A, FAKE_B = 0x40F00000, 0x40F00010
# the word F-60675 copies to +28: its lis r10,-32130 / lwz r10,-292(r10) pair
WAIT_WORD = ((-32130 << 16) - 292) & 0xFFFFFFFF


def case(rnd):
    st = rnd.choice((0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 7))
    b16, b17 = rnd.randrange(2) * rnd.choice((1, 1, 255)), rnd.randrange(2) * rnd.choice((1, 1, 2))
    w28 = rnd.choice((0, 1, 2, 6, -1, -5, rnd.randrange(-3, 40)))
    w668 = rnd.choice((0, 1, 8, -1, rnd.randrange(-3, 12)))
    g = rnd.choice((6, 6, 0, 1, 30, rnd.randrange(-2, 50)))
    inst = rnd.choice((0, 0x40230010))
    vin = rnd.randrange(2)
    return [st, b16, b17, w28 & 0xFFFFFFFF, w668 & 0xFFFFFFFF, g & 0xFFFFFFFF, inst, vin]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-60675')
    rnd = random.Random(60675)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    flat = [w for c in cases for w in c]
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl +
           'std::string EV; uint32_t VIN;' + nl + r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(int c=0;c<@N@;c++){const uint32_t*s=CS+8*c;rt_reset();EV.clear();VIN=s[7];
 for(uint32_t o=0;o<1024;o+=4){store(@R@+o,0);store(@C@+o,0);}
 store(@VT@+128,@FA@);store(@VT@+132,@FB@);store(@R@,@VT@);
 *pointer(@R@+16)=(uint8_t)s[1];*pointer(@R@+17)=(uint8_t)s[2];store(@R@+28,s[3]);store(@R@+668,s[4]);store(@R@+656,s[0]);
 store(@R@+48,@C@);store(@C@+12,s[6]);store(@C@+80,0x40240000u);store(@R@+140,77u);store(@W@,s[5]);
 Context x=fresh();x.r3.u32=@R@;
 try{sub_@F@(x,nullptr);}catch(const std::exception&e){EV+=" THROW";}
 printf("case");for(int i=0;i<8;i++)printf(" %08x",s[i]);
 printf(" -> %08x %02x %02x %08x %08x %08x%s\n",load(@R@+656),(uint32_t)*pointer(@R@+16),(uint32_t)*pointer(@R@+17),load(@R@+28),load(@R@+664),load(@R@+668),EV.c_str());}}
''')
    for k, v in (('@N@', str(a.cases)), ('@R@', '%du' % R), ('@C@', '%du' % C), ('@VT@', '%du' % VT), ('@FA@', '%du' % FAKE_A),
                 ('@FB@', '%du' % FAKE_B), ('@W@', '%du' % WAIT_WORD), ('@F@', fn)):
        src = src.replace(k, v)
    stubs = {
        'sub_' + addr(a.tool, 'F-47121'): 'EV+=" L";',
        'sub_' + addr(a.tool, 'F-98197'): '{char b[40];snprintf(b,sizeof b," T %08x %08x",ctx.r3.u32,ctx.r4.u32);EV+=b;}',
        'sub_' + addr(a.tool, 'F-70250'): '{char b[20];snprintf(b,sizeof b," V %08x",ctx.r3.u32);EV+=b;ctx.r3.u32=VIN;}',
        'sub_' + addr(a.tool, 'F-88873'): '{char b[20];snprintf(b,sizeof b," W %08x",ctx.r3.u32);EV+=b;}',
    }
    # (the stubs come before the main source: each names the event log by a block-scope extern)
    ext = 'extern std::string EV;extern uint32_t VIN;'
    stubs = {k: '{' + ext + v + '}' for k, v in stubs.items()}
    fakes = {FAKE_A: '{' + ext + 'EV+=" A";}', FAKE_B: '{' + ext + 'EV+=" B";}'}
    exe = rd.build(rd.index(a.recomp), ['sub_' + fn], src, a.out_dir, 'streamstate', stubs=stubs, image=rd.load_image(a.image), fakes=fakes)
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'stream_state_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
