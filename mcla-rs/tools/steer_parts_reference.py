"""The car-parts object's eased steering and speedo values run offline: the original's F-69494 (called once a frame by
F-98654) with its slew F-54584, the generated code whole and unchanged, on random sequences of frames. Inputs of a
frame: D's fields +1108 .. +1176 (between frames the runner changes some at random: the racer's values, rates, the
hold), the chassis's speed (+224 of D's +4; no chassis in some frames), the garage showroom byte 0x82908D5D and the
frame's step (the image word 0x827D7508 the slew reads).
Writes <out-dir>/steer_parts_ref.txt, one frame a line: +1108 +1112 +1120 +1124 +1136 +1140 +1144 +1148 +1152 +1156
+1160 +1164 +1168 +1172 +1176, the speed, the garage byte, the step (f32 bits), then '->' +1120 +1124 +1160 +1164."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

D = 0x40100000
CHASSIS = 0x40110000
STACK = 0x40F00000
GARAGE = 0x82908D5D
STEP = 0x827D7508
FIELDS = [1108, 1112, 1120, 1124, 1136, 1140, 1144, 1148, 1152, 1156, 1160, 1164, 1168, 1172, 1176]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def start(rnd):
    s = dict.fromkeys(FIELDS, 0.0)
    s.update({1108: 1.0, 1112: 10.0, 1136: 1.0, 1140: 3.0, 1144: 7.0, 1152: 0.0, 1156: 1.0, 1176: 1.0})
    if rnd.random() < 0.5:
        s.update({1140: 55.0, 1144: 100.0, 1152: 10.0})
    if rnd.random() < 0.3:
        s.update({1108: 7.0, 1112: 55.0, 1156: rnd.uniform(0, 1)})
    return s


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-69494')
    rnd = random.Random(69494)
    seqs = []
    for _ in range(300):
        s = start(rnd)
        frames = []
        for k in range(rnd.randrange(20, 80)):
            if rnd.random() < 0.25:
                s[1172] = rnd.choice((0.0, 1.0, -1.0, rnd.uniform(-1, 1), rnd.uniform(-0.05, 0.05)))
            if rnd.random() < 0.1:
                s[1176] = rnd.choice((1.0, 1.0, 0.5, rnd.uniform(0.3, 1), 0.0, -0.5))
            if rnd.random() < 0.15:
                s[1168] = rnd.choice((0.0, 0.0, -0.0, rnd.uniform(-1, 1)))
            if rnd.random() < 0.04:
                s[1148] = rnd.choice((0.0, 0.0, 25.0, 7.0))
                s[1152] = rnd.choice((10.0, 20.0))
            if rnd.random() < 0.03:
                s[1136] = rnd.choice((1.0, rnd.uniform(0.7, 7)))
            if rnd.random() < 0.02:
                s[1160] = s[1164] = s[1172] / s[1176] * 0.5 if s[1176] else 0.0
            speed = rnd.choice((0.0, 0.567056, 0.6, rnd.uniform(0, 1), rnd.uniform(0, 60)))
            chassis = rnd.random() < 0.9
            garage = int(rnd.random() < 0.05)
            dt = rnd.choice((1 / 30, 1 / 30, 1 / 60, rnd.uniform(0.001, 0.2)))
            frames.append(([fb(s[f]) for f in FIELDS], fb(speed), chassis, garage, fb(dt)))
            # (the runner keeps the four eased values from the original's run: they are read back below)
        seqs.append(frames)
    nl = chr(10)
    data = []
    for frames in seqs:
        data.append('%d,' % len(frames) + ','.join(','.join('%du' % w for w in f[0]) + ',%du,%d,%d,%du' % f[1:] for f in frames) + ',')
    per = len(FIELDS) + 4
    keep = '{%s}' % ','.join(str(FIELDS.index(f)) for f in (1120, 1124, 1160, 1164))
    main_src = (
        'static const uint32_t C[]={' + nl + nl.join(data) + nl + '};' + nl +
        'static const int K[4]=%s;' % keep + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'static const int F[%d]={%s};' % (len(FIELDS), ','.join(map(str, FIELDS))) +
        'for(size_t i=0;i<sizeof(C)/4;){uint32_t n=C[i++];uint32_t held[4];bool first=true;'
        'memset(pointer(%du),0,0x800);memset(pointer(%du),0,0x400);' % (D, CHASSIS) +
        'for(uint32_t k=0;k<n;k++,i+=%d){const uint32_t*w=C+i;uint32_t in[%d];' % (per, len(FIELDS)) +
        'for(int j=0;j<%d;j++)in[j]=w[j];if(!first)for(int j=0;j<4;j++)in[K[j]]=held[j];first=false;' % len(FIELDS) +
        'for(int j=0;j<%d;j++)store(%du+F[j],in[j]);' % (len(FIELDS), D) +
        'store(%du+224,w[%d]);store(%du+4,w[%d]?%du:0u);*pointer(%du)=uint8_t(w[%d]);store(%du,w[%d]);'
        % (CHASSIS, len(FIELDS), D, len(FIELDS) + 1, CHASSIS, GARAGE, len(FIELDS) + 2, STEP, len(FIELDS) + 3) +
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;sub_%s(ctx,nullptr);}' % (STACK, D, fn) +
        'for(int j=0;j<%d;j++)printf("%%08x ",in[j]);' % len(FIELDS) +
        'printf("%%08x %%08x %%08x ->",w[%d]?w[%d]:0u,w[%d]?0x3f800000u:0u,w[%d]);' % (len(FIELDS) + 1, len(FIELDS), len(FIELDS) + 2, len(FIELDS) + 3) +
        'for(int j=0;j<4;j++){held[j]=load(%du+F[K[j]]);printf(" %%08x",held[j]);}printf("\\n");}}}' % D + nl)
    idx = index(a.recomp)
    exe = rd.build(idx, ['sub_' + fn], main_src, a.out_dir, 'steer_parts', image=rd.load_image(a.image))
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'steer_parts_ref.txt').write_text(out)
    lines = [x for x in out.split(nl) if x]
    print(len(seqs), 'sequences,', len(lines), 'frames')


if __name__ == '__main__':
    main()
