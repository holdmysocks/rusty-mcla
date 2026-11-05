"""The air ride (mcCarAirRide) run offline on sequences of frames: the generated code whole and unchanged - the ctor
F-74609, the reset F-80081, the switch-on F-47200 (with the options getter F-82698), the switch-off F-94916 and the
frame's update F-89410 with the bag mover F-30541 and its approach F-90102 - on flat guest memory. Nothing is stubbed.

The object A at 0x1000 (+4 its tune at 0x1100, +8 the car at 0x1200), the car's +92 sim tune at 0x1300 (+136 the front
wheel tune at 0x1400, +140 the back at 0x1500, each +80 SuspensionOffset), the car's +8 chassis at 0x1600 (+412 / +420 /
+416 / +424 the wheels at 0x1700 / 0x1800 / 0x1900 / 0x1A00, each +432 the height). The frame's step is the time block's
+8 (0x827D7508), the lowering pattern the options' +1324 (0x8288EE4C).

A sequence: ctor, +4 / +8 set as the init F-47925 sets them, +12 = 1 (fitted), the reset; then per frame an action - 0
nothing, 1 the switch-on when the object is off and no bag moves (F-32886's own test past the car's), 2 the switch-off -
with the wheels' +432 the object's heights (what the tick's writer F-71524 left there), then the update with the frame's
step.

Writes <out-dir>/air_ride_ref.txt: per sequence a line 'q' with the tune's six floats and the two offsets; per frame a
line 'f' with the action, the step and the pattern byte '->' +13, +14, +16 .. +28, +32 .. +44, +48 .. +51 (all hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += '#define ctr r2\n#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("unexpected indirect call")\n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def sequence(rnd, frames):
    rise = [rnd.choice((0.0, 0.0, f32(rnd.uniform(0, 0.08)))) for _ in range(2)]
    drop = [f32(rnd.choice((0.1676, rnd.uniform(0, 0.25), rnd.uniform(-0.05, 0.25)))) for _ in range(2)]
    rates = [f32(rnd.choice((0.1, rnd.uniform(0.02, 0.4)))) for _ in range(2)]
    tune = [rise[0], drop[0], rise[1], drop[1], rates[0], rates[1]]
    offs = [f32(rnd.choice((0.0, 0.0, 0.025, -0.05, -0.1, 0.075, rnd.uniform(-0.15, 0.08)))) for _ in range(2)]
    fr = []
    for _ in range(frames):
        r = rnd.random()
        act = 1 if r < 0.04 else 2 if r < 0.06 else 0
        dt = f32(rnd.choice((1 / 30, 1 / 60, rnd.uniform(0.012, 0.04), rnd.uniform(0.0, 0.1))))
        fr.append((act, dt, rnd.randrange(5)))
    return tune, offs, fr


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--sequences', type=int, default=200)
    ap.add_argument('--frames', type=int, default=400)
    a = ap.parse_args()
    ctor, reset, on, off, upd, opts, mover, approach = (addr(a.tool, f) for f in
                                                        ('F-74609', 'F-80081', 'F-47200', 'F-94916', 'F-89410', 'F-82698', 'F-30541', 'F-90102'))
    rnd = random.Random(41)
    seqs = [sequence(rnd, a.frames) for _ in range(a.sequences)]
    nl = chr(10)
    NL = chr(92) + 'n'
    q = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in t + o) for t, o, _ in seqs)
    f = nl.join('{%d,0x%08x,%d},' % (act, fb(dt), p) for _, _, fr in seqs for act, dt, p in fr)
    wheels = (0x1700, 0x1800, 0x1900, 0x1A00)
    body = ('static const uint32_t Q[][8]={' + nl + q + nl + '};' + nl +
            'static const uint32_t F[][3]={' + nl + f + nl + '};' + nl +
            'size_t k=0;for(auto&s:Q){memset(objects,0,sizeof(objects));' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);' % ctor +
            'store(0x1000+4,0x1100);store(0x1000+8,0x1200);pointer(0x1000+12)[0]=1;' +
            'ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);' % reset +
            'for(int i=0;i<6;i++)store(0x1100+4*i,s[i]);' +
            'store(0x1200+92,0x1300);store(0x1300+136,0x1400);store(0x1300+140,0x1500);store(0x1400+80,s[6]);store(0x1500+80,s[7]);' +
            'store(0x1200+8,0x1600);store(0x1600+412,0x%x);store(0x1600+420,0x%x);store(0x1600+416,0x%x);store(0x1600+424,0x%x);' % wheels +
            'for(int n=0;n<%d;n++,k++){auto&c=F[k];' % a.frames +
            'for(int i=0;i<4;i++)store((%s)[i]+432,load(0x1000+32+4*i));' % ('(const uint32_t[]){0x%x,0x%x,0x%x,0x%x}' % wheels) +
            'pointer(0x8288EE4C)[0]=c[2];store(0x827D7508,c[1]);' +
            'bool mv=false;for(int i=0;i<4;i++)mv|=pointer(0x1000+48)[i]!=0;' +
            'if(c[0]==1&&!pointer(0x1000+13)[0]&&!mv){ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);}' % on +
            'if(c[0]==2){ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);}' % off +
            'ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);' % upd +
            'printf("%x %x",pointer(0x1000+13)[0],pointer(0x1000+14)[0]);' +
            'for(int i=0;i<8;i++)printf(" %x",load(0x1000+16+4*i));' +
            'for(int i=0;i<4;i++)printf(" %%x",pointer(0x1000+48)[i]);printf("%s");}}' % NL + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + x for x in (ctor, reset, on, off, upd, opts, mover, approach)], main_src, a.out_dir, 'air_ride')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == a.sequences * a.frames, (len(out), a.sequences * a.frames)
    lines, k = [], 0
    for t, o, fr in seqs:
        lines.append('q ' + ' '.join('%08x' % fb(x) for x in t + o))
        for act, dt, p in fr:
            lines.append('f %d %08x %d -> %s' % (act, fb(dt), p, out[k]))
            k += 1
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'air_ride_ref.txt').write_text(nl.join(lines) + nl)
    print(len(seqs), 'sequences,', k, 'frames')


if __name__ == '__main__':
    main()
