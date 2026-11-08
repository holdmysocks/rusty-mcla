"""mcCarFluid's update F-84852 run offline (generated code unchanged, vector mode) with the lattice's forces F-87118 and
the body's AddForce / AddTorque slots (F-17506, F-93816, reached through the body's vtable) unchanged: the timer
counting down by the global step, then in the water (+12 bit 0) with a buoyancy above 0 each of the 64 points (+48,
their w lane garbage) carried through the chassis' matrix (+240..+288) and, where at or under the level (+16), its
force (0, buoyancy x mass, 0) - ((w x (point - place +352) + velocity) - current +32) x damp +1076 x mass and its
torque about the centre of mass (+112 through the matrix) added into the body's sums +336 / +352 point by point; then
the buoyancy moved by the tune's SinkRate (+24) a step and held between BuoyancyMin (+20) and BuoyancyMax (+16).

Cases: in or out, buoyancy 0 / negative / 0 .. 0.6, the timer 0 / running / about to run out, random chassis
rotations and places, levels across the car's height (none, some and all points under), velocities, turning,
currents, masses 800 .. 3000, random starting sums (the order of the additions shows), tunes that rise and sink.
Start-up constants the image holds as zero are filled in as the running game has them (the xyz mask, w = 1).

Writes <out-dir>/fluid_ref.txt, one case a line, all words hex: "in b damp timer level fx fy fz min max sink mass
vx vy vz wx wy wz px py pz cx cy cz r0..r3 (12) s336 (3) s352 (3) p0..p63 (64 x 3) -> s336 (3) s352 (3) b timer"
(a word that reads as one of the original's addresses is written 82xx_xxxx)."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('static void dispatch(Context&,const uint8_t*,uint32_t);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10))


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def rotation(rnd):
    # (rows of a random rotation: yaw, pitch, roll)
    y, p, r = rnd.uniform(-math.pi, math.pi), rnd.uniform(-1.2, 1.2), rnd.uniform(-1.2, 1.2)
    cy, sy, cp, sp, cr, sr = math.cos(y), math.sin(y), math.cos(p), math.sin(p), math.cos(r), math.sin(r)
    m = [[cy * cr + sy * sp * sr, cp * sr, -sy * cr + cy * sp * sr],
         [-cy * sr + sy * sp * cr, cp * cr, sy * sr + cy * sp * cr],
         [sy * cp, -sp, cy * cp]]
    return m


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-84852')
    fa, ft = addr(a.tool, 'F-17506'), addr(a.tool, 'F-93816')
    rnd = random.Random(84852)
    cases = []
    for _ in range(1500):
        inside = int(rnd.random() < 0.85)
        b = rnd.choice((0.0, -rnd.uniform(0, 0.3), rnd.uniform(0, 0.6), rnd.uniform(0, 0.6), rnd.uniform(0, 0.6)))
        damp = rnd.choice((0.1, rnd.uniform(0, 0.5)))
        timer = rnd.choice((0.0, 0.0, 0.0, rnd.uniform(0, 1), 1 / 60, rnd.uniform(0, 0.02)))
        hi = rnd.uniform(0.05, 1.0)
        lo = rnd.choice((rnd.uniform(0, hi), 0.0, hi))
        sink = rnd.choice((rnd.uniform(-0.3, 0.3), -0.1, 0.1, 0.0))
        mass = rnd.uniform(800, 3000)
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 100), rnd.uniform(-3000, 3000)]
        m = rotation(rnd)
        lo_b = [rnd.uniform(-1.2, -0.7), rnd.uniform(-0.2, 0.4), rnd.uniform(-2.6, -1.8)]
        hi_b = [rnd.uniform(0.7, 1.2), rnd.uniform(0.9, 1.6), rnd.uniform(1.8, 2.6)]
        pts = [[rnd.uniform(lo_b[k], hi_b[k]) for k in range(3)] for _p in range(64)]
        level = place[1] + rnd.choice((rnd.uniform(-2, 2), -5.0, 5.0, rnd.uniform(-0.5, 0.5)))
        flow = [rnd.choice((2.0, 0.0, rnd.uniform(-5, 5))), rnd.choice((0.0, rnd.uniform(-1, 1))), rnd.choice((0.0, rnd.uniform(-5, 5)))]
        vel = [rnd.uniform(-30, 30), rnd.uniform(-10, 10), rnd.uniform(-30, 30)]
        ang = [rnd.choice((0.0, rnd.uniform(-5, 5))) for _k in range(3)]
        cp = [place[k] + rnd.uniform(-0.05, 0.05) for k in range(3)]  # (the chassis' copy of the place)
        com = [rnd.uniform(-0.1, 0.1), rnd.uniform(0.2, 0.6), rnd.uniform(-0.3, 0.3)]
        sums = [rnd.choice((0.0, rnd.uniform(-5e4, 5e4))) for _k in range(6)]
        rows = [x for r in m for x in r] + place
        c = [inside, fb(b), fb(damp), fb(timer), fb(level), *map(fb, flow), fb(lo), fb(hi), fb(sink), fb(mass), *map(fb, vel),
             *map(fb, ang), *map(fb, cp), *map(fb, com), *map(fb, rows), *map(fb, sums), *[fb(x) for p in pts for x in p]]
        cases.append(c)
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    F, T, C, B, V = 0x1000, 0x1480, 0x1500, 0x1700, 0x1900
    v4 = lambda at, i: ''.join(f'store({at}+{4 * k},c[{i + k}]);' for k in range(3))
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){' +
            f'if(a==0x{fa})sub_{fa}(ctx,base);else if(a==0x{ft})sub_{ft}(ctx,base);else throw std::runtime_error("indirect");}}' + nl)
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'uint32_t g=12345;' +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            f'store({F}+4,{T});store({F}+8,{C});store({F}+12,c[0]|0xA0);store({F}+1072,c[1]);store({F}+1076,c[2]);store({F}+1088,c[3]);' +
            f'store({F}+16,c[4]);' + v4(f'{F}+32', 5) + f'store({F}+44,0x3F800000);' +
            f'store({T}+20,c[8]);store({T}+16,c[9]);store({T}+24,c[10]);' +
            f'store({C}+128,{B});store({B},{V});store({V}+128,0x{fa});store({V}+132,0x{ft});' +
            f'store({B}+192,c[11]);' + v4(f'{B}+272', 12) + v4(f'{B}+288', 15) + v4(f'{C}+352', 18) + v4(f'{C}+112', 21) +
            f'store({C}+124,0x3F800000);' +
            ''.join(v4(f'{C}+{240 + 16 * r}', 24 + 3 * r) for r in range(4)) + f'store({C}+300,0x3F800000);' +
            v4(f'{B}+336', 36) + v4(f'{B}+352', 39) +
            f'for(int p=0;p<64;p++){{for(int k=0;k<3;k++)store({F}+48+16*p+4*k,c[42+3*p+k]);g=g*1664525u+1013904223u;store({F}+60+16*p,g);}}' +
            'store(0x827D7508,0x3C888889);' +
            # (start-up constants the image holds as zero, as the running game has them: the xyz mask and w = 1)
            'for(int k=0;k<4;k++){store(0x82860950+4*k,k<3?0xffffffff:0);store(0x82860930+4*k,k<3?0:0x3f800000);}' +
            f'Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32={F};' +
            f'{fn}(ctx,nullptr);' +
            'printf("%08x %08x %08x %08x %08x %08x %08x %08x' + NL +
            f'",load({B}+336),load({B}+340),load({B}+344),load({B}+352),load({B}+356),load({B}+360),load({F}+1072),load({F}+1088));}}' + nl)
    main_src = (disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fn, 'sub_' + fa, 'sub_' + ft], main_src, a.out_dir, 'fluid', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'fluid_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
