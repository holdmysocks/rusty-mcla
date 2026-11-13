"""A mixer controller's ramps run offline: the command pass F-83565 (kind 0 sets, kind 1 the ramp setters F-58120 gain,
F-27321 pitch, F-86551 LPF, F-43839 HPF) and the step F-44036, the generated code whole and unchanged, on random
command series against one controller (64 bytes, idle at the start: gain 1, pitch 1, LPF 0, HPF -10.2228) at random
frame times; and the LPF value of RampLPF F-30645 (Hz clamped, the CRT log F-96865) on random Hz, its command post
F-42774 stubbed to record the command's value.
Writes <out-dir>/ctl_ramp_ref.txt (each number led by a letter, so no column reads as an address): per series 'S',
then per frame 'F dt' and its commands 'c kind param value time' (f32 bits), '->' the twelve block words after the
step (cur, target, rate for gain, pitch, LPF, HPF; f32 bits); then per Hz 'L hz -> value' (f32 bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static uint32_t VAL;' + chr(10)

CMD = 0x40100000     # the frame's commands, 16 bytes each
CTRL = 0x40110000    # the controller (index 0)
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    apply_, step, lpf, post = (addr(a.tool, f) for f in ('F-83565', 'F-44036', 'F-30645', 'F-42774'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    r = random.Random(83565)
    gains = (0.0, 1e-5, 1.0000001e-5, 1.0, 0.5, 0.25, 0.031622775, 1.5848932, 2.0, 0.999, 1e-6, -0.5)
    filts = (0.0, -10.222795, -1.0, -3.5, -0.0001, -5.0, -10.0, 0.5, -7.25)
    times = (0.0, -1.0, 0.5, 1.0, 2.5, 3.0, 10.0, 12.0, 0.016666668, 0.1, float('nan'))
    dts = (1 / 60, 1 / 30, 0.0, 0.1, 0.5, 0.033333335, 2.0)
    series = []
    for _ in range(400):
        frames = []
        for _ in range(r.randrange(5, 80)):
            cmds = []
            if r.random() < 0.3:
                for _ in range(r.choice((1, 1, 1, 2, 3))):
                    kind = r.choice((0, 1, 1, 1))
                    param = r.choice((0, 0, 1, 2, 2, 3))
                    v = r.choice(gains) if param < 2 else r.choice(filts)
                    t = r.choice(times)
                    cmds.append((kind, param, fb(v), fb(t)))
            frames.append((fb(r.choice(dts)), cmds))
        series.append(frames)
    hz = [0.0, 19.0, 20.0, 21.0, 100.0, 1000.0, 2500.0, 8000.0, 23899.0, 23900.0, 30000.0, -5.0]
    hz += [float(r.randrange(0, 26000)) for _ in range(2000)]
    data = ','.join('%d,%s' % (len(fr), ','.join('%du,%d,%s' % (f[0], len(f[1]), ','.join('%d,%d,%du,%du' % c for c in f[1]))
                                                       if f[1] else '%du,0' % f[0] for f in fr)) for fr in series)
    hzs = ','.join('%du' % fb(h) for h in hz)
    stubs = {'sub_' + post: 'VAL=load(ctx.r5.u32+8);'}
    c = r"""
static const uint32_t C[]={@DATA@};
static const uint32_t H[]={@HZ@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(size_t i=0;i<sizeof(C)/4;){uint32_t m=C[i++];
 printf("S\n");memset(pointer(@CTRL@u),0,64);
 store(@CTRL@u+4,0x3f800000u);store(@CTRL@u+16,0x3f800000u);store(@CTRL@u+40,0xc1239091u);
 for(uint32_t j=0;j<m;j++){uint32_t dt=C[i++],n=C[i++];printf("F d%08x",dt);
  for(uint32_t k=0;k<n;k++){const uint32_t*w=C+i;i+=4;uint32_t b=@CMD@u+16*k;
   store(b,0);store(b+4,w[3]);store(b+8,w[2]);store16(b+12,w[0]);store16(b+14,w[1]);
   printf(" c%u p%u v%08x t%08x",w[0],w[1],w[2],w[3]);}
  if(n){Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@CMD@u;ctx.r4.u32=n;ctx.r5.u32=@CTRL@u;sub_@APPLY@(ctx,nullptr);}
  {Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@CTRL@u;float d;memcpy(&d,&dt,4);ctx.f1.f64=d;sub_@STEP@(ctx,nullptr);}
  printf(" ->");for(int q=0;q<4;q++)for(int e=0;e<3;e++)printf(" w%08x",load(@CTRL@u+4+12*q+4*e));printf("\n");}
 printf("=\n");}
for(size_t i=0;i<sizeof(H)/4;i++){Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@CTRL@u;float h;memcpy(&h,&H[i],4);
 ctx.f1.f64=h;ctx.f2.f64=1.0;VAL=0;sub_@LPF@(ctx,nullptr);printf("L h%08x -> v%08x\n",H[i],VAL);}}
"""
    for k, v in (('DATA', data), ('HZ', hzs), ('CMD', CMD), ('CTRL', CTRL), ('STACK', STACK), ('APPLY', apply_),
                 ('STEP', step), ('LPF', lpf)):
        c = c.replace('@%s@' % k, str(v))
    exe = rd.build(idx, ['sub_' + apply_, 'sub_' + step, 'sub_' + lpf], c, a.out_dir, 'ctl_ramp', stubs=stubs, image=img)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'ctl_ramp_ref.txt').write_text(out)
    fr = [l for l in out.split(chr(10)) if l.startswith('F ')]
    print(len(series), 'series,', len(fr), 'frames,', sum(l.count(' c') for l in fr), 'commands,',
          out.count(chr(10) + 'L '), 'Hz values')


if __name__ == '__main__':
    main()
