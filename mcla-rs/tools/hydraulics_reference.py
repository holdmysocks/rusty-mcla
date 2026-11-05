"""The hydraulics (mcCarHydraulics) run offline on sequences of frames: the generated code whole and unchanged - the
wheel button handler F-39230, the switch-on F-71695, the switch-off F-28175, the frame's update F-93185 with the order
step F-21741, the ram mover F-73777 and its approach, and the dance patterns - on flat guest memory. The one callee
stubbed is the flag setter at the end of the switch-on / switch-off (it only ORs bits into an event word of the car).

The object H at 0x1000 (+4 its tune at 0x1100, +8 the car at 0x1200, +20 the dance pattern), the car's +92 sim tune
at 0x1300 (+136 the front wheel tune at 0x1400, +140 the back at 0x1500, each +80 SuspensionOffset), the car's +8
chassis at 0x1600 (+412 / +420 / +416 / +424 the wheels at 0x1700 / 0x1800 / 0x1900 / 0x1A00, each +432 the height).
The frame's step is the time block's +8.

A frame is the pad pass F-70414 and then the update: the hydraulics flag against the object's +16 (flag up and object
off: +16 = 1, +120..123 = 1, switch-on; flag down and object on: +120..123 = 1, +16 = 0, switch-off), then per wheel the
handler with 1 (pressed event), 2 (released event) or nothing, then the wheels' +432 = the object's heights (what the tick's
writer left there) and the update with the step.

Writes <out-dir>/hydraulics_ref.txt: per sequence a line 'q' with the tune's seven floats (latch, front rise / drop,
back rise / drop, rate rise / drop), the two SuspensionOffsets (hex) and the pattern; per frame a line 'f' with
<flag><event wheel 0..3> (events 0 none, 1 press, 2 release), the step (hex) '->' on, asked x4, queued x4, order x4
(d down, m middle, u up), moving x4, latched x4 (digits), then press timer x4, settle timer x4, height x4 (hex bits)."""
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


def sequence(rnd, frames, k):
    if rnd.random() < 0.6:   # a realistic tune
        t = [0.75, rnd.uniform(0.1, 0.25), 0.025, rnd.uniform(0.1, 0.25), 0.025, rnd.uniform(3, 7), rnd.uniform(3, 7)]
    else:
        t = [rnd.choice((0.75, rnd.uniform(0.05, 2))), rnd.uniform(0.02, 0.3), rnd.choice((0.025, rnd.uniform(0, 0.1))),
             rnd.uniform(0.02, 0.3), rnd.choice((0.025, rnd.uniform(0, 0.1))), rnd.uniform(1, 9), rnd.uniform(1, 9)]
    t = [f32(x) for x in t]
    # the SuspensionOffsets: mostly 0 (the shipped cars' set-ups), some small
    offs = [f32(rnd.choice((0.0, 0.0, 0.0, 0.025, -0.05, rnd.uniform(-0.1, 0.1)))) for _ in range(2)]
    if k % 4 == 0:
        offs = [0.0, 0.0]
    pattern = rnd.randrange(1, 6) if k % 10 == 9 else 0
    dtmode = rnd.choice((0, 0, 1, 2, 3))
    held = [False] * 4
    flag = 0
    fr = []
    for _ in range(frames):
        if not flag:
            flag = 1 if rnd.random() < 0.06 else 0
        elif rnd.random() < 0.012:
            flag = 0
        ev = [0] * 4
        for i in range(4):
            if held[i]:
                if rnd.random() < rnd.choice((0.03, 0.06, 0.12)):
                    held[i] = False
                    ev[i] = 2
            elif rnd.random() < 0.04:
                held[i] = True
                ev[i] = 1
        dt = (1 / 30, 1 / 60, rnd.uniform(0.005, 0.05), rnd.choice((1 / 30, 1 / 60)))[dtmode]
        fr.append((flag, ev, f32(dt)))
    return t, offs, pattern, fr


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--sequences', type=int, default=100)
    ap.add_argument('--frames', type=int, default=300)
    a = ap.parse_args()
    ids = ('F-39230', 'F-71695', 'F-28175', 'F-93185', 'F-21741', 'F-73777')
    handler, on, off, upd, latch, mover = (addr(a.tool, f) for f in ids)
    flagset = addr(a.tool, 'F-53649')
    rnd = random.Random(77)
    seqs = [sequence(rnd, a.frames, k) for k in range(a.sequences)]
    nl = chr(10)
    NL = chr(92) + 'n'
    q = nl.join('{%s},' % ','.join(['0x%08x' % fb(x) for x in t + o] + [str(p)]) for t, o, p, _ in seqs)
    f = nl.join('{%d,%d,%d,%d,%d,0x%08x},' % (fl, e[0], e[1], e[2], e[3], fb(dt)) for _, _, _, fr in seqs for fl, e, dt in fr)
    wheels = (0x1700, 0x1800, 0x1900, 0x1A00)
    call = 'ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;'
    body = ('static const uint32_t Q[][10]={' + nl + q + nl + '};' + nl +
            'static const uint32_t F[][6]={' + nl + f + nl + '};' + nl +
            'size_t k=0;for(auto&s:Q){memset(objects,0,sizeof(objects));Context ctx{};' +
            'store(0x1000+4,0x1100);store(0x1000+8,0x1200);store(0x1000+20,s[9]);' +
            'for(int i=0;i<7;i++)store(0x1100+4*i,s[i]);' +
            'store(0x1200+92,0x1300);store(0x1200+132,0x1B00);store(0x1300+136,0x1400);store(0x1300+140,0x1500);' +
            'store(0x1400+80,s[7]);store(0x1500+80,s[8]);' +
            'store(0x1200+8,0x1600);store(0x1600+412,0x%x);store(0x1600+420,0x%x);store(0x1600+416,0x%x);store(0x1600+424,0x%x);' % wheels +
            'for(int n=0;n<%d;n++,k++){auto&c=F[k];' % a.frames +
            'store(0x%x,c[5]);uint32_t isOn' % ((0x82 << 24) + 0x7D7508) + '=load(0x1000+16);' +
            'if(c[0]&&!isOn){store(0x1000+16,1);for(int i=0;i<4;i++)pointer(0x1000+120)[i]=1;' + call + 'sub_%s(ctx,nullptr);}' % on +
            'else if(!c[0]&&isOn){for(int i=0;i<4;i++)pointer(0x1000+120)[i]=1;store(0x1000+16,0);' + call + 'sub_%s(ctx,nullptr);}' % off +
            'for(int i=0;i<4;i++){ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=i;ctx.r5.u32=c[1+i];sub_%s(ctx,nullptr);}' % handler +
            'const uint32_t W[]={0x%x,0x%x,0x%x,0x%x};' % wheels +
            'for(int i=0;i<4;i++)store(W[i]+432,load(0x1000+72+4*i));' +
            call + 'sub_%s(ctx,nullptr);' % upd +
            'printf("%x",load(0x1000+16));' +
            'for(int j=0;j<6;j++)for(int i=0;i<4;i++)printf(" %x",load(0x1000+(j==0?24:j==1?40:j==2?56:j==3?72:j==4?88:104)+4*i));' +
            'for(int i=0;i<4;i++)printf(" %x",pointer(0x1000+120)[i]);for(int i=0;i<4;i++)printf(" %x",pointer(0x1000+124)[i]);' +
            'printf("%s");}}' % NL + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + x for x in (handler, on, off, upd, latch, mover)], main_src, a.out_dir, 'hydraulics',
                stubs={'sub_' + flagset: ''})
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == a.sequences * a.frames, (len(out), a.sequences * a.frames)
    lines, k = [], 0
    for t, o, p, fr in seqs:
        lines.append('q ' + ' '.join('%08x' % fb(x) for x in t + o) + ' %d' % p)
        for fl, e, dt in fr:
            w = [int(x, 16) for x in out[k].split()]
            k += 1
            on_, asked, queued, order, height, press, settle = w[0], w[1:5], w[5:9], w[9:13], w[13:17], w[17:21], w[21:25]
            moving, latched = w[25:29], w[29:33]
            od = ''.join({0: 'm', 1: 'u', 0xFFFFFFFF: 'd'}[x] for x in order)
            lines.append('f %d%s %08x -> %d %s %s %s %s %s %s %s %s' % (
                fl, ''.join(map(str, e)), fb(dt), on_, ''.join(map(str, asked)), ''.join(map(str, queued)), od,
                ''.join(map(str, moving)), ''.join(map(str, latched)),
                ','.join('%x' % x for x in press), ','.join('%x' % x for x in settle), ','.join('%x' % x for x in height)))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'hydraulics_ref.txt').write_text(nl.join(lines) + nl)
    print(len(seqs), 'sequences,', k, 'frames')


if __name__ == '__main__':
    main()
