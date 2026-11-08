"""A motorcycle's brake places run offline: F-43091 (the wheel and brake matrices at the car's set-up) on the generated code
unchanged, its bike test F-54299 stubbed to answer "a motorcycle". The render object (0x1000) holds
the bike's per-axle rim fields +4092 / +4096 (d) and +4100 / +4104 (b), +4036 = 1.0, +4040 = 0.03; the model tune (0x3300)
fRotorSize +120 / +124 and fRotorOffset +128 / +132. The hub_N matrices V (table +4224) and wheel_N matrices W (+4192) are
random turns and places.

Writes <out-dir>/bike_brake_ref.txt, one wheel a line:
  "wheel b d off rs W(16) V(16) -> caliper(16) rotorA(16) rotorB(16)" (hex float bits; the caliper +1600 + 64 i, rotor A
  +1856 + 128 i, rotor B +1920 + 128 i). Test `vehicles::brake_tests::bike_brake_place_is_the_originals`."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import recomp_leaf

recomp_closure.EXTRA += '#define ctr r2\n'
recomp_closure.shim = lambda: recomp_leaf.shim(0x8000).replace(
    'throw std::runtime_error("unexpected guest access");', '{fprintf(stderr,"guest %08x ",a);throw std::runtime_error("unexpected guest access");}')
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
# (the recomp's own x86 flags: vmsum3fp through the CPU's dpps, as the generated code is built)
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def mat(r):
    # a turn about y, a tilt about z and a spin about x, at a random place (rows: x, y, z, place)
    a, t, s = r.uniform(-3, 3), r.uniform(-0.3, 0.3), r.uniform(-3, 3)
    ca, sa, ct, st, cs, ss = math.cos(a), math.sin(a), math.cos(t), math.sin(t), math.cos(s), math.sin(s)
    Y = [[ca, 0, -sa], [0, 1, 0], [sa, 0, ca]]
    Z = [[ct, st, 0], [-st, ct, 0], [0, 0, 1]]
    X = [[1, 0, 0], [0, cs, ss], [0, -ss, cs]]
    mul = lambda p, q: [[sum(p[i][k] * q[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    m = mul(mul(X, Z), Y)
    return [m[0][0], m[0][1], m[0][2], 0, m[1][0], m[1][1], m[1][2], 0, m[2][0], m[2][1], m[2][2], 0,
            r.uniform(-5, 5), r.uniform(0, 1), r.uniform(-5, 5), 1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=60)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-43091')
    rnd = random.Random(5)
    cases = []
    for k in range(a.cases):
        # (b, d per axle: the shipped bikes' 0.35 / 0.31 and spans 0.12 - 0.23 about; offsets and rotor sizes about the data)
        c = [rnd.uniform(0.2, 0.5), rnd.uniform(0.2, 0.5), rnd.uniform(0.08, 0.3), rnd.uniform(0.08, 0.3),
             rnd.choice((0.0, rnd.uniform(-0.08, 0.08))), rnd.choice((0.0, rnd.uniform(-0.08, 0.08))),
             rnd.choice((1.0, rnd.uniform(0.7, 1.3))), rnd.choice((1.0, rnd.uniform(0.7, 1.3)))]
        if k == 0:
            c = [0.35, 0.31, 0.12792, 0.22, 0.0, 0.0, 1.0, 1.0]
        for i in range(8):
            c += mat(rnd)
        cases.append(c)
    rows = ',\n'.join('{%s}' % ','.join('0x%08x' % fb(float(x)) for x in c) for c in cases)
    setup = ('memset(objects,0,0x7000);'
             'for(int k=0;k<4;k++){store(0x827D85D0+16*k+4*k,0x3f800000);store(0x827D8450+4*k,0x3f800000);store(0x827D8460+4*k,0x3f000000);store(0x828609E0+4*k,0x7f800000);}'
             'store(0x1000+20,0x3000);store(0x3000+8,0x3100);store(0x3100+24,4);store(0x3000+76,0x3200);store(0x3200+76,0);'
             'store(0x3200+96,0x3300);store(0x3300+120,c[6]);store(0x3300+124,c[7]);store(0x3300+128,c[4]);store(0x3300+132,c[5]);'
             'store(0x3000+132,0x3400);for(int a=0;a<2;a++){*pointer(0x3400+2108+a)=3;*pointer(0x3400+2110+a)=17;*pointer(0x3400+2112+a)=3;}'
             'store(0x1000+4036,0x3f800000);store(0x1000+4040,0x3cf5c28f);store(0x5800,0x5840);'
             'store(0x1000+4076,0x3f11eb85);store(0x1000+4080,0x3f11eb85);store(0x1000+4084,0x3f51eb85);store(0x1000+4088,0x3f51eb85);'
             'store(0x1000+4092,c[2]);store(0x1000+4096,c[3]);store(0x1000+4100,c[0]);store(0x1000+4104,c[1]);'
             'for(int i=0;i<4;i++){store(0x1000+4192+4*i,0x6000+64*i);store(0x1000+4224+4*i,0x6400+64*i);'
             'for(int k=0;k<16;k++){store(0x6000+64*i+4*k,c[8+16*i+k]);store(0x6400+64*i+4*k,c[8+64+16*i+k]);}}')
    nl = chr(10)
    body = ('static const uint32_t C[][%d]={' % len(cases[0]) + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);'
            'for(int i=0;i<4;i++){for(int o=0;o<16;o++)printf("%08x ",load(0x1000+1600+64*i+4*o));'
            'for(int o=0;o<32;o++)printf("%08x ",load(0x1000+1856+128*i+4*o));printf("' + chr(92) + 'n");}}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    # (the bike test F-54299 reads the vehicle-type table the image does not hold: it answers "a motorcycle" here)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'bikebrk', stubs={'sub_' + addr(a.tool, 'F-54299'): 'ctx.r3.u64=1;'},
                               vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == 4 * len(cases), (len(out), len(cases))
    lines = []
    for k, c in enumerate(cases):
        for i in range(4):
            ax = 0 if i < 2 else 1
            ins = [i, c[ax], c[2 + ax], c[4 + ax] if i in (0, 2) else 0.0, c[6 + ax] if i in (0, 2) else 1.0]
            w, v = c[8 + 16 * i:8 + 16 * i + 16], c[8 + 64 + 16 * i:8 + 64 + 16 * i + 16]
            lines.append('%d %08x %08x %08x %08x ' % (i, fb(ins[1]), fb(ins[2]), fb(ins[3]), fb(ins[4]))
                         + ' '.join('%08x' % fb(x) for x in w + v) + ' -> ' + out[4 * k + i])
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'bike_brake_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'wheels')


if __name__ == '__main__':
    main()
