"""The wipeout's steam loops run offline: the car effects' smoke trail start F-89706 (with 1) and stop F-87058, as the
effects update F-21480 calls them by the wipeout flag (chassis +592 & 0x10), with the sound entity's RADIATOR_STEAM_MASTER
loops F-26850 (slot 1428, record +1784) and F-96861 (slot 1432, record +1844) and their reset F-73124, the generated code
whole and unchanged, on random runs of frames: wiped out or not, whether each record has its matrix (+0: H+48 / H+112, the pointer F-89706 hands over), whether its effect instance is made, whether a
slot's sound ended before the frame and whether a make succeeds. Stubbed, as inputs of the case: the record start F-87024
(active byte, instance or none), the instance release F-50844, the make F-30971 (records hash, position pointer and the
request's P+52 / flags; fills the slot or not), the stop F-31667 (records its r4; clears the slot as through the sound's
+144), set position F-85133 (records the pointer).
Writes <out-dir>/steam_ref.txt (each number led by a letter, so no column reads as an address): per run a line 'R m0 m1', then
per frame 'F wipeout made0 made1 ended0 ended1 sound' '->' per slot: p play (h hash right, q position = the record's
matrix +48, e P+52, g flags), o set position (to the matrix +48), k stop (r4 + 1), s slot filled after; then the records'
active bytes and instances after."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static uint32_t REC[2][8];' + chr(10)

H = 0x40100000       # the car effects object (G+7188)
G = 0x40110000       # H+8; +16 the sound entity
ENT = 0x40120000
PRT = 0x40130000     # GLOBAL_CAR_PARTICLES (+1256), +22 RADIATOR_STEAM_MASTER
INST = (0x40140000, 0x40148000)
SND = (0x40150000, 0x40158000)
IN = 0x40160000      # this frame's stub inputs: +0 / +4 instance made, +8 make succeeds
STACK = 0x40F00000
HASH = 0x5EA75EA7


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    start, stopfx, recstart, release, make, stop, setpos = (addr(a.tool, f) for f in (
        'F-89706', 'F-87058', 'F-87024', 'F-50844', 'F-30971', 'F-31667', 'F-85133'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    r = random.Random(73124)
    runs = []
    for _ in range(300):
        frames = []
        wip = r.random() < 0.5
        for _ in range(r.randrange(5, 60)):
            if r.random() < 0.15:
                wip = not wip
            frames.append((int(wip), int(r.random() < 0.9), int(r.random() < 0.9), int(r.random() < 0.05), int(r.random() < 0.05),
                           int(r.random() < 0.9)))
        runs.append(((r.random() < 0.9, r.random() < 0.9), frames))
    data = ','.join('%d,%d,%d,%s' % (m[0], m[1], len(fr), ','.join(','.join('%d' % v for v in f) for f in fr)) for m, fr in runs)
    # the record start: active, and an instance if it has none and the case makes one
    stubs = {
        'sub_' + recstart: 'uint32_t k=ctx.r3.u32==%du+1784?0:1;*pointer(ctx.r3.u32+8)=1;'
                           'if(!load(ctx.r3.u32+12)&&load(%du+4*k))store(ctx.r3.u32+12,k?%du:%du);' % (H, IN, INST[1], INST[0]),
        'sub_' + release: '',
        'sub_' + make: 'uint32_t k=ctx.r5.u32==%du+1428?0:1;REC[k][0]=1;REC[k][1]=ctx.r4.u32;REC[k][2]=load(ctx.r6.u32+20);'
                       'REC[k][3]=load(ctx.r6.u32+52);REC[k][4]=*pointer(ctx.r6.u32+70);'
                       'store(ctx.r5.u32,load(%du+8)?(k?%du:%du):0);' % (ENT, IN, SND[1], SND[0]),
        'sub_' + stop: 'uint32_t k=ctx.r3.u32==%du?0:1;REC[k][5]=1+ctx.r4.u32;store(%du+(k?1432:1428),0);' % (SND[0], ENT),
        'sub_' + setpos: 'uint32_t k=ctx.r3.u32==%du?0:1;REC[k][6]=ctx.r4.u32;' % SND[0],
    }
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(size_t i=0;i<sizeof(C)/4;){uint32_t m0=C[i],m1=C[i+1],n=C[i+2];const uint32_t*fr=C+i+3;i+=3+6*n;
 printf("R m%u m%u\n",m0,m1);
 memset(pointer(@H@u),0,0x1000);memset(pointer(@ENT@u),0,0x1000);
 store(@H@u+8,@G@u);store(@G@u+16,@ENT@u);store(@ENT@u+1256,@PRT@u);store(@PRT@u+22,@HASH@u);store(@ENT@u+96,7);
 if(m0)store(@H@u+1784,@H@u+48);if(m1)store(@H@u+1844,@H@u+112);
 for(uint32_t k=0;k<n;k++){const uint32_t*f=fr+6*k;
  if(f[3])store(@ENT@u+1428,0);if(f[4])store(@ENT@u+1432,0);
  store(@IN@u,f[1]);store(@IN@u+4,f[2]);store(@IN@u+8,f[5]);
  memset(REC,0,sizeof(REC));
  Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@H@u;ctx.r4.u32=0;ctx.r5.u32=1;
  if(f[0])sub_@START@(ctx,nullptr);else sub_@STOPFX@(ctx,nullptr);
  printf("F w%u i%u i%u e%u e%u m%u ->",f[0],f[1],f[2],f[3],f[4],f[5]);
  for(int s=0;s<2;s++){uint32_t in=s?@H@u+112:@H@u+48;
   printf(" p%u h%u q%u e%u g%u o%u k%u s%u",REC[s][0],REC[s][1]==@HASH@u,REC[s][2]==in+48,REC[s][3],REC[s][4],REC[s][6]==in+48,REC[s][5],load(@ENT@u+(s?1432:1428))!=0);}
  printf(" a%u a%u n%u n%u\n",(unsigned)*pointer(@H@u+1784+8),(unsigned)*pointer(@H@u+1844+8),load(@H@u+1784+12)!=0,load(@H@u+1844+12)!=0);}
 printf("=\n");}}
"""
    for k, v in (('DATA', data), ('H', H), ('G', G), ('ENT', ENT), ('PRT', PRT), ('HASH', HASH), ('IN', IN), ('STACK', STACK),
                 ('START', start), ('STOPFX', stopfx)):
        c = c.replace('@%s@' % k, str(v))
    exe = rd.build(idx, ['sub_' + start, 'sub_' + stopfx], c, a.out_dir, 'steam', stubs=stubs, image=img)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'steam_ref.txt').write_text(out)
    fr = [l for l in out.split(chr(10)) if l.startswith('F ')]
    print(len(runs), 'runs,', len(fr), 'frames,', sum(l.count(' p1 ') for l in fr), 'plays,', sum(l.count(' o1 ') for l in fr), 'moves,',
          sum(l.count(' k1 ') + l.count(' k2 ') for l in fr), 'stops')


if __name__ == '__main__':
    main()
