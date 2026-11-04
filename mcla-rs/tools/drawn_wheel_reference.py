"""A set-up car's drawn wheel pose (F-47822, the visual wheel update) run offline: the whole function, unchanged
generated code on flat guest memory in vector mode, for its four wheels. Kept callees: the turn about an axis (F-41945),
the guest sine and atan2, the axle's displacement (F-48387). Stubs: the is-motorcycle test (F-54299, returns the case's
flag) and the two turns that come after the pose (the roll about x, F-52786, and the half turn of the right-hand wheels,
F-25963): the pose they are applied to is what is compared.
Guest memory: car 0x1000 (+20 the car's objects 0x2400; +4312 + 4i the wheel's output matrix 0x5000 + 64i; +4328 + 4i 0),
objects 0x2400 (+8 0x2600: +28 the wheel list 0x2680; +28 0x2700 with the body's matrix at +16; +76 0x2A00: +96 the
mcCarModelTune 0x2B00; +84 0x2C00 with the bike's +1164; +132 the set-up 0x3000, byte +2203 the wide-body flag), wheels
0x4000 + 0x280i (+4 the vehWheelTune 0x4A00 + 0x80i; +16..+64 the wheel's matrix; +384 rest; +400 steering axis; +420
radius; +424 width; +432 offset; +496 compression; +516 lean), the thread context 0x5800 (its +72 byte 0: the body
matrix from +28).
Writes <out-dir>/drawn_wheel_ref.txt, one car a line: the inputs, then "->" per wheel the pose's rows x, y, z and place
(xyz each); floats as hex bits."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.EXTRA += '#define ctr r2\nstatic uint32_t cur_bike;\n'
recomp_closure.shim = lambda: recomp_leaf.shim(0x8000)
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
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def rot(axis, ang):
    x, y, z = axis
    n = math.sqrt(x * x + y * y + z * z)
    x, y, z = x / n, y / n, z / n
    c, s, t = math.cos(ang), math.sin(ang), 1 - math.cos(ang)
    return [[t * x * x + c, t * x * y + s * z, t * x * z - s * y], [t * x * y - s * z, t * y * y + c, t * y * z + s * x],
            [t * x * z + s * y, t * y * z - s * x, t * z * z + c]]


def mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)] for i in range(3)]


# one case = 19 car words + 4 x 22 wheel words
CAR = ['bike', 'wide', 'extra', 'b00', 'b01', 'b02', 'b10', 'b11', 'b12', 'b20', 'b21', 'b22', 'bx', 'by', 'bz',
       'ceil', 'ceil2', 'tsl', 'tso']
WHEEL = ['m00', 'm01', 'm02', 'm10', 'm11', 'm12', 'm20', 'm21', 'm22', 'px', 'py', 'pz', 'rx', 'ry', 'rz', 'rad', 'wid',
         'steer_limit', 'camber_limit', 'lean', 'comp', 'off']


def case(rnd):
    c = {'bike': int(rnd.random() < 0.15), 'wide': int(rnd.random() < 0.3), 'extra': f32(rnd.uniform(-0.2, 0.2))}
    yaw = rnd.uniform(-math.pi, math.pi)
    body = mul(rot((0, 1, 0), yaw), rot((rnd.uniform(-1, 1), 0, rnd.uniform(-1, 1)), rnd.uniform(-0.2, 0.2)))
    for i in range(3):
        for j in range(3):
            c['b%d%d' % (i, j)] = f32(body[i][j])
    for k, v in zip('xyz', [rnd.uniform(-3000, 3000), rnd.uniform(-20, 80), rnd.uniform(-3000, 3000)]):
        c['b' + k] = f32(v)
    for k in ('ceil', 'ceil2'):
        c[k] = f32(rnd.choice((0.0, 0.0, rnd.uniform(0.3, 0.65), -0.1)))
    c['tsl'] = f32(rnd.choice((0.0, 0.4, 0.4, rnd.uniform(0, 0.8), -0.2)))
    c['tso'] = f32(rnd.choice((0.0, 0.8, 0.9, rnd.uniform(0, 1.2), -0.3)))
    wheels = []
    for i in range(4):
        w = {}
        front = i < 2
        x = (-1 if i % 2 == 0 else 1) * rnd.uniform(0.6, 0.9)
        z = (-1 if front else 1) * rnd.uniform(1.0, 1.7)
        y = rnd.uniform(0.15, 0.5)
        steer = rnd.uniform(-0.6, 0.6) if front else rnd.choice((0.0, rnd.uniform(-0.05, 0.05)))
        lean = rnd.choice((0.0, 0.0, rnd.uniform(-0.08, 0.08)))
        loc = mul(rot((0, 0, 1), lean), rot((0, 1, 0), steer))
        world = mul(loc, body)
        for a in range(3):
            for b in range(3):
                w['m%d%d' % (a, b)] = f32(world[a][b])
        lp = [x + rnd.uniform(-0.02, 0.02), y + rnd.uniform(-0.15, 0.15), z]
        wp = [c['b' + 'xyz'[j]] + sum(lp[k] * body[k][j] for k in range(3)) for j in range(3)]
        for k, v in zip(('px', 'py', 'pz'), wp):
            w[k] = f32(v)
        for k, v in zip(('rx', 'ry', 'rz'), (x, y - rnd.uniform(0, 0.1), z)):
            w[k] = f32(v)
        w['rad'] = f32(rnd.uniform(0.28, 0.4))
        w['wid'] = f32(rnd.uniform(0.18, 0.33))
        w['steer_limit'] = f32(rnd.choice((0.0, 0.1, 0.6, 0.66, 0.75)) if front else rnd.choice((0.0, 0.0, 0.1000001)))
        w['camber_limit'] = f32(rnd.choice((0.0, 0.0, 0.05, -0.05, rnd.uniform(-0.1, 0.1))))
        w['lean'] = f32(lean if rnd.random() < 0.8 else rnd.uniform(-0.1, 0.1))
        w['comp'] = f32(rnd.uniform(-0.3, 0.3))
        w['off'] = f32(rnd.choice((0.0, rnd.uniform(-0.05, 0.05))))
        wheels.append(w)
    return [c[k] for k in CAR] + [w[k] for w in wheels for k in WHEEL]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-47822')
    bike, roll, half = (addr(a.tool, f) for f in ('F-54299', 'F-52786', 'F-25963'))
    rnd = random.Random(47822)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % (x if i < 2 else fb(x)) for i, x in enumerate(c)) for c in cases)
    nw = len(WHEEL)
    # (the tune pair a set-up picks by the wide-body flag: [stock, wide]; the case's ceiling goes into the picked slot,
    # the other one gets a different value so a wrong pick shows)
    setup = ('memset(objects,0,0x7000);'
             'store(0x1000+20,0x2400);for(int i=0;i<4;i++){store(0x1000+4312+4*i,0x5000+64*i);}'
             'store(0x2400+8,0x2600);store(0x2600+28,0x2680);store(0x2400+28,0x2700);store(0x2400+76,0x2A00);'
             'store(0x2A00+96,0x2B00);store(0x2400+84,0x2C00);store(0x2C00+1164,c[2]);store(0x2400+132,0x3000);'
             '*pointer(0x3000+2203)=uint8_t(c[1]);'
             'for(int k=0;k<9;k++)store(0x2700+16+16*(k/3)+4*(k%%3),c[3+k]);for(int k=0;k<3;k++)store(0x2700+64+4*k,c[12+k]);'
             'store(0x2B00+40+4*c[1],c[15]);store(0x2B00+40+4*(1-c[1]),0x3e99999a);'
             'store(0x2B00+48+4*c[1],c[16]);store(0x2B00+48+4*(1-c[1]),0x3e99999a);'
             'store(0x2B00+136,c[17]);store(0x2B00+140,c[18]);'
             'store(0x5800,0x5840);'
             'for(int i=0;i<4;i++){const uint32_t*w=c+19+%d*i;uint32_t o=0x4000+0x280*i,t=0x4A00+0x80*i;store(0x2680+4*i,o);'
             'store(o+4,t);for(int k=0;k<9;k++)store(o+16+16*(k/3)+4*(k%%3),w[k]);for(int k=0;k<3;k++)store(o+64+4*k,w[9+k]);'
             'for(int k=0;k<3;k++)store(o+384+4*k,w[12+k]);store(o+400+4,0x3f800000);store(o+420,w[15]);store(o+424,w[16]);'
             'store(t+40,w[17]);store(t+44,w[18]);store(o+516,w[19]);store(o+496,w[20]);store(o+432,w[21]);'
             'store(t+72,0x3e800000);store(t+76,0x3e800000);}' % nw)
    body = ('static const uint32_t C[][%d]={' % (19 + 4 * nw) + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;cur_bike=c[0];' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;'
            'sub_%s(ctx,nullptr);' % fn +
            'for(int i=0;i<4;i++)for(int r=0;r<4;r++)for(int k=0;k<3;k++)printf("%%08x%%s",load(0x5000+64*i+16*r+4*k),(i==3&&r==3&&k==2)?"%s":" ");}' % NL)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    stubs = {'sub_' + bike: 'ctx.r3.u64=cur_bike;', 'sub_' + roll: '', 'sub_' + half: ''}
    exe = build(recomp_closure.index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'drawn_wheel', stubs=stubs, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join(('%d' % x) if i < 2 else '%08x' % fb(x) for i, x in enumerate(c)), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'drawn_wheel_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cars')


if __name__ == '__main__':
    main()
