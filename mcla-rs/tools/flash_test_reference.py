"""Racer_CanFlashHookman's test F-96972 run offline as generated: racer A (r3) and B (r4), each with its vehicle (+48), the
vehicle's chassis (+8: the state word +592, the speed +224) and instance (+28, its matrix at +16: row 2 the back axis, row 3
the position). The script's distance (f1) at most 30, the speeds within 25, the back axis and the direction to B each
normalised by the vector unit's refined reciprocal root with a dot of at most -0.2, then the segment 0.5 above A to 0.5
above B through the probe F-20855 (stubbed: it records the segment and answers the case's hit). The answer is returned and
stored at B's vehicle +113.
Line:  <+592> <dist> <speed A> <speed B> <back x y z> <A x y z> <B x y z> <hit> -> <answer> <+113> <probed> <start x y z> <end x y z>
Usage: flash_test_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
// (RT_BLOCK + 72: the thread's byte that picks the matrix at +272 - clear, so the instance's)
static float rf(std::istream&in){std::string s;in>>s;return strtof(s.c_str(),nullptr);}
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static float lf(uint32_t a){uint32_t u=load(a);float f;memcpy(&f,&u,4);return f;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t A=0x40010000,B=0x40010100,VA=0x40010200,VB=0x40010400,CA=0x40010600,CB=0x40010A00,IA=0x40010E00,IB=0x40010F00,R=0x40014000;
 uint32_t w;
 while(in>>w){float d=rf(in),sa=rf(in),sb=rf(in),bk[3],pa[3],pb[3];for(auto&x:bk)x=rf(in);for(auto&x:pa)x=rf(in);for(auto&x:pb)x=rf(in);
  uint32_t hit;in>>hit;rt_reset();
  for(uint32_t x=A;x<0x40011000;x+=4)store(x,0);for(uint32_t x=R;x<R+64;x+=4)store(x,0);
  store(0x40000248u,0);store(R+40,hit);
  for(int k=0;k<4;k++){store(V_INF+4*k,0x7F800000u);store(V_ZERO+4*k,0);}
  store(A+48,VA);store(B+48,VB);store(VA+8,CA);store(VB+8,CB);store(VA+28,IA);store(VB+28,IB);
  store(CA+592,w);sf(CA+224,sa);sf(CB+224,sb);
  for(int k=0;k<3;k++){sf(IA+16+32+4*k,bk[k]);sf(IA+16+48+4*k,pa[k]);sf(IB+16+48+4*k,pb[k]);}
  Context cx=fresh();cx.r3.u32=A;cx.r4.u32=B;cx.f1.f64=d;sub_FLASH(cx,nullptr);
  printf("%u %u %u",cx.r3.u32&255,(load(VB+112)>>16)&255,load(R+36));
  for(int k=0;k<6;k++)printf(" %.9g",lf(R+4*k));printf("\n");
 }
}
'''


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def fmt(x):
    return 'nan' if x != x else ('%.9g' % f32(x))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    flash, probe = addr('F-96972'), addr('F-20855')
    img = rd.load_image(a.image)
    idx = rd.index(a.recomp)
    # (the start-up vector constants the refine reads, zero in the image: +inf and 0 - docs/reference/ai/obstacle_cell.md)
    v_inf, v_zero = ((-32122 << 16) + 2528) & 0xFFFFFFFF, ((-32122 << 16) + 2240) & 0xFFFFFFFF
    # (the probe: r4 the segment - start, then end 16 bytes on; it records both and answers the case's hit)
    pbody = ('for(int k=0;k<3;k++){store(0x40014000u+4*k,load(ctx.r4.u32+4*k));store(0x4001400Cu+4*k,load(ctx.r4.u32+16+4*k));}'
             'store(0x40014024u,1);ctx.r3.u32=load(0x40014028u);')
    exe = rd.build(idx, [flash], MAIN.replace('sub_FLASH(', flash + '(').replace('V_INF', hex(v_inf) + 'u').replace('V_ZERO', hex(v_zero) + 'u'), a.work, 'flash_test', image=img, stubs={probe: pbody})
    rnd = random.Random(96972)
    bad = re.compile(r'\b8[0-9A-Fa-f]{7}\b')
    lines = []
    while len(lines) < 3000:
        k = len(lines) % 6
        pa = [rnd.uniform(-900, 900), rnd.uniform(-20, 60), rnd.uniform(-900, 900)]
        yaw = rnd.uniform(-math.pi, math.pi)
        back = [math.sin(yaw), rnd.choice([0.0, rnd.uniform(-0.1, 0.1)]), math.cos(yaw)]
        if rnd.random() < 0.3:
            s = rnd.uniform(0.2, 3.0)
            back = [v * s for v in back]
        # (the man in front at about the -0.2 edge, or anywhere around)
        ang = math.acos(0.2) + rnd.choice([0.0, rnd.uniform(-1e-6, 1e-6), rnd.uniform(-0.05, 0.05), rnd.uniform(-math.pi, math.pi)])
        r = rnd.choice([rnd.uniform(0.0, 40.0), 30.0, rnd.uniform(29.999, 30.001)])
        fx, fz = -back[0], -back[2]
        n = math.hypot(fx, fz) or 1.0
        fx, fz = fx / n, fz / n
        dx, dz = fx * math.cos(ang) - fz * math.sin(ang), fx * math.sin(ang) + fz * math.cos(ang)
        pb = [pa[0] + dx * r, pa[1] + rnd.uniform(-2, 2), pa[2] + dz * r]
        if k == 5:
            pb = list(pa)
        sa = rnd.uniform(0, 60)
        sb = rnd.choice([sa + rnd.uniform(-30, 30), sa + 25.0, sa - 25.0, sa + rnd.uniform(24.999, 25.001)])
        dist = rnd.choice([math.dist(pa, pb), 30.0, rnd.uniform(29.999, 30.001), rnd.uniform(0, 50)])
        if k == 4 and rnd.random() < 0.2:
            dist = float('nan')
        w = rnd.choice([0, 0, 0, 0, 2, 0x10, 0x14])
        hit = rnd.randint(0, 1)
        line = ' '.join([str(w)] + [fmt(v) for v in [dist, sa, sb] + back + pa + pb] + [str(hit)])
        if not bad.search(line):
            lines.append(line)
    cases = a.work / 'flash_test_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines), (len(rows), len(lines))
    out = [l + ' -> ' + r for l, r in zip(lines, rows)]
    keep = [o for o in out if not bad.search(o)]
    a.out.write_text(''.join(o + '\n' for o in keep))
    print(len(keep), 'cases', sum(o.split(' -> ')[1].startswith('1') for o in keep), 'true')


if __name__ == '__main__':
    main()
