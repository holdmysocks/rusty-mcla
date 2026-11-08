"""The listener history update F-74282 run offline (the generated code whole and unchanged, vector mode) over sequences of
listener places and audio-clock times, from the state the environment constructor F-36018 leaves (ring index +3660 = 1,
+3664 = 0, every weight, velocity, speed and the stored places +3600 zero). Before each call the step does what SetListener
F-92830 does for listener 0 (its matrix into the ring slot +3660, row 3 = the place; its weight +3328 = 1.0); F-74282 then
gets the clock in ms (the frame clock x 1000 truncated, as the audio update makes it).

Cases: steady motion at 0 - 80 m/s, standing still, teleports, frames of 2, 3 and 4 ticks and repeated times (paused: the
same ms twice), clocks starting at 0, a few seconds and an hour, places near the origin and far from it.

Writes <out-dir>/listener_hist_ref.txt: "reset" before each sequence, then one call a line: "ms px py pz -> vx vy vz speed"
(decimal ms, hex float bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

ENV = 0x40100000     # the environment (F-74282's this: the audio manager + 1344)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:].upper()


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def gen(rnd, steps):
    clock = f32(rnd.choice((0.0, 0.0, rnd.uniform(0, 5), 3600.0 + rnd.uniform(0, 1), rnd.uniform(0, 20000))))
    far = rnd.random() < 0.7
    p = [rnd.uniform(-3000, 3000) if far else rnd.uniform(-0.5, 0.5) for _ in range(3)]
    v = [rnd.uniform(-1, 1) * rnd.choice((0.0, 5.0, 30.0, 80.0)) for _ in range(3)]
    out = []
    for _ in range(steps):
        fs = rnd.choice((2, 2, 2, 2, 3, 4, 0)) / 60.0
        clock = f32(clock + f32(fs))
        ms = int(f32(clock * 1000.0)) & 0xFFFFFFFF
        r = rnd.random()
        if r < 0.08:                       # a teleport
            p = [x + rnd.uniform(-2000, 2000) for x in p]
        elif r < 0.16:                     # a new speed
            v = [rnd.uniform(-1, 1) * rnd.choice((0.0, 10.0, 40.0, 80.0)) for _ in range(3)]
        p = [f32(x + y * fs) for x, y in zip(p, v)]
        out.append((ms, p))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=600)
    ap.add_argument('--steps', type=int, default=16)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-74282')
    idx = rd.index(a.recomp)
    rnd = random.Random(74282)
    cases = [gen(rnd, a.steps) for _ in range(a.cases)]
    nl = chr(10)
    rows = (',' + nl).join('{0x%08xu,0x%08xu,0x%08xu,0x%08xu}' % (ms, fb(p[0]), fb(p[1]), fb(p[2])) for c in cases for ms, p in c)
    main_src = (
        'static const uint32_t C[][4]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'const int S=%d; int n=sizeof C/sizeof C[0];' % a.steps +
        'for(int k=0;k<n;k++){auto&c=C[k];if(k%%S==0){rt_reset();memset(pointer(%du),0,3680);store(%du+3660,1u);store(%du+3664,0u);printf("reset\\n");}' % (ENV, ENV, ENV) +
        'uint32_t cur=load(%du+3660);uint32_t m=%du+64*cur;store(m+48,c[1]);store(m+52,c[2]);store(m+56,c[3]);store(m+60,0x3f800000u);' % (ENV, ENV) +
        'store(%du+3328+4*cur,0x3f800000u);' % ENV +
        'Context ctx=fresh();ctx.r3.u32=%du;ctx.r4.u32=c[0];sub_%s(ctx,nullptr);' % (ENV, fn) +
        'printf("%%u %%08x %%08x %%08x -> %%08x %%08x %%08x %%08x\\n",c[0],c[1],c[2],c[3],load(%du+3136+16*cur),load(%du+3140+16*cur),load(%du+3144+16*cur),load(%du+3424+4*cur));}}' % (ENV, ENV, ENV, ENV) + nl)
    exe = rd.build(idx, ['sub_' + fn], main_src, a.out_dir, 'listener_hist', image=rd.load_image(a.image))
    out = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == a.cases * (a.steps + 1), (len(out), a.cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'listener_hist_ref.txt').write_text(nl.join(out) + nl)
    held = sum(1 for x in out if x != 'reset' and x.split('-> ')[1].split()[:3] == ['00000000'] * 3)
    print(a.cases, 'sequences,', len(out) - a.cases, 'calls,', held, 'with zero velocity')


if __name__ == '__main__':
    main()
