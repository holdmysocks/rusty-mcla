"""The air-intake one-shot run offline: the original's F-26427 (the car audio entity's VEH_AUX_AIR_INTAKE_MASTER, TURBO+10),
the generated code whole and unchanged, on random cars (intake part or not, ONESHOT+60) driven through random frames: the
torque now and before (+160 / +176), the clock (+156) and the limiter's last cut (+240), how many wheels touch (the
chassis' vt+44, F-33415), the full-throttle torque at speed (F-71482), the gear index (trans +20), whether the slot's
sound ended before the frame and whether a make succeeds. Stubbed, as inputs of the case: the parts getter (its +132 +30
byte), F-71482, the request init, the make F-30971 (records hash and level; fills the slot or not), the stop F-31667
(records; clears the slot, as it does through the sound's +144), set position and SetVolume (record).
Writes <out-dir>/intake_ref.txt (each number led by a letter, so no column reads as an address): per car a line
'C part step', then per frame 'F' and its inputs, '->', what the frame did: k stop, p play (level bits), w set volume
(level bits), o set position, a armed after, s slot filled after."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static uint32_t REC[8];' + chr(10)

ENT = 0x40100000     # the entity
CAR = 0x40110000     # its chassis (+136)
VT = 0x40118000      # the chassis' vtable: slot 11 = the wheels touching
ENG = 0x40120000     # car+16
TRN = 0x40128000     # car+20
PRT = 0x40130000     # the parts getter's object; +132 -> PRT+0x100, +30 the intake byte
ONE = 0x40138000     # ONESHOT params (+1228)
TUR = 0x40140000     # TUNERS_TURBO (+1244)
SND = 0x40148000     # the slot's sound
BT = 0x40150000      # bucket table
VOICE = 0x40170000
IN = 0x40180000      # this frame's stub inputs: +0 wheels touching, +4 full torque, +8 make succeeds
STACK = 0x40F00000
FAKE_VT = 0x7F000010
HASH = 0x1A2B3C4D


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, parts, full, init, make, stop, setpos, setvol = (addr(a.tool, f) for f in (
        'F-26427', 'F-71293', 'F-71482', 'F-35212', 'F-30971', 'F-31667', 'F-85133', 'F-46088'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    pool = ((-32125 << 16) + 7216) & 0xffffffff
    r = random.Random(26427)
    torques = (0.0, -0.0, 1.0, -1.0, 0.5, 120.0, -30.0, 1e-30, -1e-30, 300.0, 250.0, 75.0)
    cases = []
    for _ in range(250):
        part = r.choice((0, 1, 1, 1, 2))
        step = r.choice((0.04, 0.04, 0.0, 0.1, 0.25, -0.1, 0.5, 0.03))
        clock = r.choice((0.0, 0.5, 3.0, 100.0))
        cut = r.choice((0.0, -5.0, clock))
        prev = r.choice(torques)
        live = False
        frames = []
        for _ in range(r.randrange(20, 120)):
            clock = struct.unpack('<f', struct.pack('<f', clock + r.choice((0.033, 0.0167, 0.1, 0.5, 1.0))))[0]
            if r.random() < 0.15:
                cut = clock
            now = r.choice(torques) if r.random() < 0.4 else prev
            if r.random() < 0.1:
                prev = r.choice(torques)
            fullt = r.choice((300.0, 200.0, 400.0, 0.0, -50.0, 100.0, 133.0))
            frames.append((fbits(now), fbits(prev), fbits(clock), fbits(cut), r.choice((0, 4, 4, 4, 2, 1)), fbits(fullt),
                           r.randrange(0, 8), int(r.random() < 0.1), int(r.random() < 0.85)))
            prev = now
        cases.append((part, fbits(step), frames))
    data = ','.join('%d,%du,%d,%s' % (p, s, len(fr), ','.join(','.join('%du' % v for v in f) for f in fr)) for p, s, fr in cases)
    stubs = {
        'sub_' + parts: 'ctx.r3.u32=%du;' % PRT,
        'sub_' + full: '{uint32_t b=load(%du+4);ctx.f1.f64=(double)*(float*)&b;}' % IN,
        'sub_' + init: '',
        'sub_' + make: 'REC[1]=1;REC[2]=ctx.r4.u32;REC[3]=load(ctx.r6.u32);'
                       'store(ctx.r5.u32,load(%du+8)?%du:0);' % (IN, SND),
        'sub_' + stop: 'REC[0]=1+ctx.r4.u32;store(%du+1384,0);' % ENT,
        'sub_' + setpos: 'REC[4]=1;',
        'sub_' + setvol: 'REC[5]=1;REC[6]=ctx.r3.u32;{float v=(float)ctx.f1.f64;REC[7]=*(uint32_t*)&v;}',
    }
    c = r"""
static const uint32_t C[]={@DATA@};
static void putf(uint32_t a,uint32_t b){store(a,b);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@POOL@u+232,@BT@u);store(@POOL@u+200,64);store(@BT@u+3*11488+11444,@VOICE@u-5*64);
store(@VT@u+44,@FAKE@u);
for(size_t i=0;i<sizeof(C)/4;){const uint32_t*w=C+i;uint32_t n=w[2];const uint32_t*fr=w+3;i+=3+9*n;
 printf("C q%u x%08x\n",w[0],w[1]);
 memset(pointer(@ENT@u),0,0x4000);memset(pointer(@PRT@u),0,0x400);memset(pointer(@ONE@u),0,0x100);memset(pointer(@TUR@u),0,0x100);
 store(@ENT@u+136,@CAR@u);store(@CAR@u,@VT@u);store(@CAR@u+16,@ENG@u);store(@CAR@u+20,@TRN@u);
 store(@PRT@u+132,@PRT@u+0x100);*pointer(@PRT@u+0x100+30)=(uint8_t)w[0];
 store(@ENT@u+1228,@ONE@u);putf(@ONE@u+60,w[1]);store(@ENT@u+1244,@TUR@u);store(@TUR@u+10,@HASH@u);
 *pointer(@SND@u+4)=5;*pointer(@SND@u+64)=3;
 for(uint32_t k=0;k<n;k++){const uint32_t*f=fr+9*k;
  if(f[7])store(@ENT@u+1384,0);
  putf(@ENT@u+160,f[0]);putf(@ENT@u+176,f[1]);putf(@ENT@u+156,f[2]);putf(@ENT@u+240,f[3]);
  store(@IN@u,f[4]);putf(@IN@u+4,f[5]);store(@IN@u+8,f[8]);putf(@ENG@u+16,f[0]);store(@TRN@u+20,f[6]);
  memset(REC,0,sizeof(REC));
  Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@ENT@u;sub_@FN@(ctx,nullptr);
  printf("F t%08x t%08x c%08x c%08x n%u f%08x g%u e%u m%u -> k%u p%u h%u v%08x o%u w%u r%u l%08x a%u s%u\n",f[0],f[1],f[2],f[3],f[4],f[5],f[6],f[7],f[8],
   REC[0],REC[1],REC[2]==@HASH@u,REC[3],REC[4],REC[5],REC[6]==@VOICE@u,REC[7],(unsigned)*pointer(@ENT@u+1948),load(@ENT@u+1384)!=0);}
 printf("=\n");}}
"""
    for k, v in (('DATA', data), ('POOL', pool), ('BT', BT), ('VOICE', VOICE), ('VT', VT), ('FAKE', FAKE_VT), ('ENT', ENT),
                 ('CAR', CAR), ('ENG', ENG), ('TRN', TRN), ('PRT', PRT), ('ONE', ONE), ('TUR', TUR), ('HASH', HASH), ('SND', SND),
                 ('IN', IN), ('STACK', STACK), ('FN', fn)):
        c = c.replace('@%s@' % k, str(v))
    exe = rd.build(idx, ['sub_' + fn], c, a.out_dir, 'intake', stubs=stubs, image=img, fakes={FAKE_VT: 'ctx.r3.u32=load(%du);' % IN})
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'intake_ref.txt').write_text(out)
    lines = out.split(chr(10))
    fr = [l for l in lines if l.startswith('F ')]
    print(len(cases), 'cars,', len(fr), 'frames,', sum(' p1 ' in l for l in fr), 'plays,', sum(' k2 ' in l or ' k1 ' in l for l in fr), 'stops')


if __name__ == '__main__':
    main()
