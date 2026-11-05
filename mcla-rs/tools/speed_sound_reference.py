"""The sound trigger's speed sound run offline: the original's type-1 handler of mcSoundTriggerEvent F-77205 (the
city's WHOOSHES_* triggers), the generated code whole and unchanged, with the request init F-35212 and the input getter
F-58287, on the four WHOOSHES records of game.dat and random ones, random speeds and throttles.
Stubbed, as inputs: the player getter F-65049 (the case's player or none; player +48 the vehicle or 0, vehicle +8 the
chassis, chassis +224 the speed; player +896 the input block, +12 the throttle) and the sound creation F-45091 (records the
sound hash and the request's volume +0, pitch +4 and position pointer +20; hands back no sound, so the start F-90813 is
not reached). Writes <out-dir>/speed_sound_ref.txt (each number led by a letter, so no column reads as an address)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run
from music_play_reference import gamedat

PLAYER, VEH, CH, INP, EVT, REC = 0x40100000, 0x40110000, 0x40120000, 0x40130000, 0x40200000, 0x40210000
STACK = 0x40F00000
NAMES = ('WHOOSHES_TUNNEL', 'WHOOSHES_FREEWAY_OVERPASS', 'WHOOSHES_FREEWAY_SIGN', 'WHOOSHES_PILLAR')


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--game-dat', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    handler, getp, create = (addr(a.tool, f) for f in ('F-77205', 'F-65049', 'F-45091'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    recs = [bytes(gamedat(a.game_dat, n)) for n in NAMES]
    r = random.Random(77205)
    cases = []
    for c in range(4000):
        if c % 5 == 4:
            rec = bytearray(38)
            rec[0] = 1
            struct.pack_into('>I', rec, 10, r.getrandbits(32))
            lo = r.choice((40.0, 80.0, 0.0, r.uniform(0, 120)))
            hi = r.choice((100.0, lo, lo + r.uniform(-20, 80)))
            c0 = r.choice((0.0, 0.5, r.uniform(-1, 1)))
            c1 = r.choice((1.0, c0, r.uniform(-1, 2)))
            struct.pack_into('>ffff', rec, 14, lo, hi, c0, c1)
            struct.pack_into('>hhhh', rec, 30, *(r.choice((-1200, 0, -600, 300, -300, r.randint(-32768, 32767))) for _ in range(4)))
        else:
            rec = recs[c % 5 if c % 5 < 4 else 0]
        mode = r.random()
        if mode < 0.03:
            who = 0                 # no player
        elif mode < 0.06:
            who = 1                 # a player without a vehicle
        else:
            who = 2
        speed = r.choice((0.0, r.uniform(0, 60), r.uniform(15, 50), 17.8816, 35.7632, 44.704, r.uniform(-5, 80)))
        thr = r.choice((0.0, 1.0, r.uniform(0, 1), 0.5, r.uniform(-0.5, 1.5)))
        cases.append([who, fb(speed), fb(thr)] + list(struct.unpack('>9I', bytes(rec[:36]))) + [struct.unpack('>H', bytes(rec[36:38]))[0]])
    data = ','.join('%du' % v for cs in cases for v in [len(cs)] + cs)
    stubs = {
        'sub_' + getp: 'ctx.r3.u32=load(@PLAYER@u+4)?@PLAYER@u:0;',
        'sub_' + create: '{uint32_t q=ctx.r6.u32;printf(" S h%08x v%08x p%08x a%08x",ctx.r4.u32,load(q),load(q+4),load(q+20));store(ctx.r5.u32,0);}',
    }
    c = r"""
static const uint32_t CD[]={@CD@};
int main(int argc,char**argv){
 std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
 size_t q=0;while(q<sizeof(CD)/4){uint32_t len=CD[q];const uint32_t*w=CD+q+1;q+=1+len;
  for(uint32_t k=0;k<0x100;k+=4){store(@PLAYER@u+k,0);store(@CH@u+k,0);store(@INP@u+k,0);store(@EVT@u+k,0);}
  for(uint32_t k=0;k<0x1000;k+=4)store(@PLAYER@u+k,0);
  store(@PLAYER@u+4,w[0]>0);store(@PLAYER@u+48,w[0]>1?@VEH@u:0);store(@VEH@u+8,@CH@u);store(@CH@u+224,w[1]);
  store(@PLAYER@u+896,@INP@u);store(@INP@u+12,w[2]);
  for(int k=0;k<9;k++)store(@REC@u+4*k,w[3+k]);*pointer(@REC@u+36)=(uint8_t)(w[12]>>8);*pointer(@REC@u+37)=(uint8_t)w[12];
  store(@EVT@u+160,@REC@u);
  printf("W w%u s%08x t%08x",w[0],w[1],w[2]);for(int k=0;k<9;k++)printf(" r%08x",w[3+k]);printf(" r%04x",w[12]);
  Context c{};c.r1.u32=@STACK@u;c.r3.u32=@EVT@u;sub_@HANDLER@(c,nullptr);printf("\n");}
}
"""
    for k, v in (('CD', data), ('PLAYER', PLAYER), ('VEH', VEH), ('CH', CH), ('INP', INP), ('EVT', EVT), ('REC', REC),
                 ('STACK', STACK), ('HANDLER', handler)):
        c = c.replace('@%s@' % k, str(v))
        stubs = {n: b.replace('@%s@' % k, str(v)) for n, b in stubs.items()}
    exe = rd.build(idx, ['sub_' + handler], c, a.out_dir, 'speed_sound', stubs=stubs, image=img)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'speed_sound_ref.txt').write_text(out)
    ls = [l for l in out.split(chr(10)) if l]
    print(len(ls), 'cases,', sum(' S ' in l for l in ls), 'sounds')


if __name__ == '__main__':
    main()
