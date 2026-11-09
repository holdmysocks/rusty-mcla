"""The gear for a forward speed F-24547 run offline (generated code unchanged, scalar mode, with its callees: the
ratio for a speed F-24970 and the engine's top-speed getter it calls run as they are).

r3 the gearbox: +4 a record whose +0 is the gear count, +8 the engine (+68 its top speed, rad/s), +12 the drivetrain
(+20 the wheel count, +24.. the wheels, each +420 its radius), +36 + 4 g the ratio of gear index g (0 reverse,
1 neutral, 2.. forward); f1 the forward speed in m/s. Returns the gear index in r3.

Cases: speeds around -1 / 10 (both sides, the edges exact), random, -0, NaN, inf, huge; top speeds and radii
random and odd (0, negative, NaN); 0 .. 4 wheels; 2 .. 9 gears with random descending, equal or odd ratios.

Writes <out-dir>/gear_for_speed_ref.txt, one case a line, hex: "fwd top n r.. count q.. -> index"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10)


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
    fgear = 'sub_' + addr(a.tool, 'F-24547')
    rnd = random.Random(24547)
    nan = float('nan')

    def speed():
        return rnd.choice((-1.0, 10.0, -0.0, 0.0, nan, float('inf'), float('-inf'), 1e30,
                           struct.unpack('>f', struct.pack('>I', fb(10.0) - 1))[0],
                           struct.unpack('>f', struct.pack('>I', fb(-1.0) + 1))[0],
                           rnd.uniform(-5, 12), rnd.uniform(10, 90), rnd.uniform(10, 90), rnd.uniform(10, 90), rnd.uniform(-30, 0)))

    def odd(lo, hi):
        return rnd.choice((rnd.uniform(lo, hi),) * 6 + (0.0, -rnd.uniform(lo, hi), nan, 1e-40))

    cases = []
    for _ in range(6000):
        top = odd(400, 1100)
        n = rnd.choice((0, 2, 2, 4, 4, 4, 1, 3))
        radii = [odd(0.25, 0.45) if rnd.random() < 0.3 else rnd.uniform(0.25, 0.45) for _ in range(n)]
        count = rnd.choice((2, 3, 4, 5, 6, 6, 7, 7, 8, 9))
        q = sorted((rnd.uniform(0.5, 4.5) for _ in range(count)), reverse=True)
        if rnd.random() < 0.2:
            q = [odd(0.5, 4.5) for _ in range(count)]
        if rnd.random() < 0.1 and count > 3:
            q[3] = q[2]
        cases.append([fb(speed()), fb(top), n] + [fb(r) for r in radii] + [count] + [fb(x) for x in q])
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = []
    for c in cases:
        flat += [len(c)] + c
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'size_t at=0;for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));' +
            'uint32_t len=D[at];const uint32_t*w=&D[at+1];at+=1+len;' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            'store(0x1000+4,0x1100);store(0x1000+8,0x1200);store(0x1000+12,0x1300);store(0x1200+68,w[1]);' +
            'uint32_t n=w[2];store(0x1300+20,n);for(uint32_t i=0;i<n;i++){store(0x1300+24+4*i,0x1400+0x200*i);store(0x1400+0x200*i+420,w[3+i]);}' +
            'uint32_t cnt=w[3+n];store(0x1100,cnt);for(uint32_t g=0;g<cnt;g++)store(0x1000+36+4*g,w[4+n+g]);' +
            f'float f;memcpy(&f,&w[0],4);ctx.f1.f64=f;{fgear}(ctx,nullptr);printf("%x",ctx.r3.u32);' +
            'printf("' + NL + '");}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fgear], main_src, a.out_dir, 'gear_for_speed')
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'gear_for_speed_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
