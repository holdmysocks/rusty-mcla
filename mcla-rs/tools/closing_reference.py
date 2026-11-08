"""Run the police tactics' closing-in test F-32902 offline (static recompilation, native, flat guest memory) for the port's
`police.rs::closing_in` (test closing_matches_the_original).

F-32902(range f1, A r4, B r5): with range != 0, the squared distance A - B (dy dy, then dz, dx fused) must be under range^2;
B's chassis speed (+48 -> +8 -> +224) must be above fMovingSpeed (tune +312) x 0.447 and above A's; then the unit of A - B
(F-41334, eps 1e-5, fallback (1, 0, 0)) dotted with B's velocity (chassis +128 -> +272) / B's speed must be over 0.9.
Positions come from F-35658 (+48 of what it returns), stubbed here to hand back the racer + 256.

Cases: 6,000 from a fixed seed - positions within 80 m, B's velocity aimed near A (the 0.9 edge), speeds equal / either side of
the moving floor, ranges 0 / 7 / 50 / random, A on B, tiny offsets. Output, one line a case, all f32 bits in decimal:
  <range> <ax ay az> <bx by bz> <speed A> <speed B> <vx vy vz> <fMovingSpeed> <result>
"""
import argparse
import math
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
static float fb(uint32_t u){float f;memcpy(&f,&u,4);return f;}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t A=0x40810000u, B=0x40820000u, CA=0x40830000u, CB=0x40840000u, SA=0x40850000u, SB=0x40860000u, VB=0x40870000u;
 while(in>>tok){ if(tok!="case") break; uint32_t w[13]; for(int i=0;i<13;i++) in>>w[i]; rt_reset();
  for(int i=0;i<3;i++){ store(A+256+48+4*i,w[1+i]); store(B+256+48+4*i,w[4+i]); store(VB+272+4*i,w[9+i]); }
  store(A+256+60,0); store(B+256+60,0); store(VB+284,0);
  store(A+48,CA); store(CA+8,SA); store(B+48,CB); store(CB+8,SB); store(SA+224,w[7]); store(SB+224,w[8]); store(SB+128,VB);
  store(0x8282CAC8u+312,w[12]);
  Context c=fresh(); c.f1.f64=(double)fb(w[0]); c.r4.u32=A; c.r5.u32=B; CLOSE(c,nullptr);
  for(int i=0;i<13;i++) printf("%u ",w[i]); printf("%u\n",(unsigned)(c.r3.u32&1));}
 printf("end\n");
}
'''


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def cases():
    g = random.Random(32902)
    out = []
    for k in range(6000):
        a = [g.uniform(-80, 80), g.uniform(-3, 3), g.uniform(-80, 80)]
        kind = k % 10
        if kind == 0:
            b = list(a)
        elif kind == 1:
            b = [a[0] + g.uniform(-1e-3, 1e-3), a[1], a[2] + g.uniform(-1e-3, 1e-3)]
        else:
            b = [a[0] + g.uniform(-60, 60), a[1] + g.uniform(-2, 2), a[2] + g.uniform(-60, 60)]
        mv = g.choice([1.0, 1.0, g.uniform(0, 20)])
        floor = mv * 0.447
        sb = g.choice([g.uniform(0, 60), floor, floor + 1e-4, g.uniform(0, 2)])
        sa = g.choice([g.uniform(0, 60), sb, sb * 0.5])
        d = [a[i] - b[i] for i in range(3)]
        l = math.sqrt(sum(x * x for x in d)) or 1.0
        u = [x / l for x in d]
        ang = g.choice([g.uniform(0, 0.6), math.acos(0.9) + g.uniform(-0.01, 0.01), g.uniform(0, math.pi)])
        # B's velocity: the unit towards A turned by ang about y (some y kept), of B's speed or near it
        c, s = math.cos(ang), math.sin(ang)
        v = [u[0] * c - u[2] * s, u[1], u[0] * s + u[2] * c]
        vs = sb * g.choice([1.0, 1.0, g.uniform(0.5, 1.5)])
        v = [x * vs for x in v]
        r = g.choice([0.0, 7.0, 50.0, g.uniform(1, 90), math.sqrt(sum(x * x for x in d))])
        out.append([bits(r)] + [bits(x) for x in a] + [bits(x) for x in b] + [bits(sa), bits(sb)] + [bits(x) for x in v] + [bits(mv)])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-32902', 'F-35658']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-35658'): 'ctx.r3.u64=ctx.r3.u32+256u;'}
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-32902')], MAIN.replace('CLOSE', name('F-32902')), a.out, 'closing', image=img,
                      stubs=stubs, limit=50)
    cf = a.out / 'closing_cases.txt'
    cf.write_text(''.join('case ' + ' '.join(map(str, c)) + '\n' for c in cases()) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout[:-len('end\n')])
    print('cases', r.stdout.count('\n') - 1, 'true', sum(l.endswith(' 1') for l in r.stdout.splitlines()), '->', a.ref)


if __name__ == '__main__':
    main()
