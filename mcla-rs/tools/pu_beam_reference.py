"""The shot's ribbon beam and its hit light run offline: the original's own code, unchanged generated code on flat guest memory.

GEOM  the two vertex makers of the shoot-effect manager, F-65991 (the beam: one strip) and F-82152 (the ribbon: two strips).
      The immediate-mode calls are host stubs that print what they are given: begin (primitive, count), every vertex (x y z,
      nx ny nz, the 32-bit colour, s t - the order of the 36-byte entry the vertex call writes) and end.
BEAM  the record's draw F-44101 on a target record (the landing's values: shooter point (0, 0.5, 1.5), target point (0, 0.5, 0),
      up (0, 0.5, 1.5); the shot type's look with m_beamColor / m_ribbonColor) for random shooter / target matrices and times.
      The effect's SetVar (the parameter handle, the count of vec4s, the words) and the world matrix call are host stubs that
      print; the pass begin returns 0 passes, so no geometry is drawn here (that is GEOM). The manager carries what its
      constructor stores that this path reads (+40 = 32, +92..+104 the two noise pairs; the handles +4 .. +20 are 4, 8, 12, 16, 20).
LIGHT the hit light F-24672 (with its point F-81450): up to 8 target records, the camera object's position at +48, the clock.
The start-up vector constants the static image holds as zero are stored first (0x828609E0 = +inf). Built with -msse4.1 as the
recomp itself is (vmsum3fp through the CPU's dpps).
Usage: pu_beam_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf

BEAM_DRAW, BEAM_GEOM, RIBBON_GEOM, LIGHT = 'F-44101', 'F-65991', 'F-82152', 'F-24672'
# the immediate-mode begin / vertex / end, the effect's SetVar, the world matrix, the pass begin and the pass helpers
BEGIN, VERTEX, END = 'F-33202', 'F-97666', 'F-15766'
SETVAR, WORLD, PASSES = 'F-17339', 'F-60960', 'F-94845'


def addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


HEAD = r'''
static void fresh(Context&c){c=Context{};c.r1.u32=0x2FF0;}
'''


def rot(rnd):
    # a random rotation as three float rows (the car's matrix rows), from a random unit quaternion
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(x * x for x in q))
    w, x, y, z = (v / n for v in q)
    return [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)],
            [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
            [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]


def matrix(rnd, near=None):
    r = rot(rnd)
    p = [rnd.uniform(-2000, 2000), rnd.uniform(-20, 120), rnd.uniform(-2000, 2000)] if near is None else \
        [near[i] + rnd.uniform(-60, 60) for i in range(3)]
    return [f32(v) for v in r[0] + [0.0] + r[1] + [0.0] + r[2] + [0.0] + p + [1.0]]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    n = {k: addr(a.tool, k) for k in (BEAM_DRAW, BEAM_GEOM, RIBBON_GEOM, LIGHT, BEGIN, VERTEX, END, SETVAR, WORLD, PASSES)}
    idx = recomp_closure.index(a.recomp)
    load_image = 'std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' \
                 'for(int k=0;k<4;k++)store(0x828609E0+4*k,0x7f800000);\n'
    out = []
    rnd = random.Random(50)

    # --- GEOM
    stubs = {n[BEGIN]: 'printf("B %u %u\\n",ctx.r3.u32,ctx.r4.u32);',
             n[VERTEX]: 'auto bits=[](double d){float x=float(d);uint32_t u;memcpy(&u,&x,4);return u;};printf("V %08x %08x %08x %08x %08x %08x %08x %08x %08x\\n",bits(ctx.f1.f64),bits(ctx.f2.f64),bits(ctx.f3.f64),'
                        'bits(ctx.f4.f64),bits(ctx.f5.f64),bits(ctx.f6.f64),ctx.r9.u32,bits(ctx.f7.f64),bits(ctx.f8.f64));',
             n[END]: 'printf("E\\n");'}
    main_src = HEAD + 'int main(int argc,char**argv){' + load_image + \
        ' Context c;fresh(c);%s(c,nullptr);printf("R\\n");fresh(c);%s(c,nullptr);}\n' % (n[BEAM_GEOM], n[RIBBON_GEOM])
    exe = recomp_closure.build(idx, [n[BEAM_GEOM], n[RIBBON_GEOM]], main_src, a.work, 'pu_beam_geom', stubs=stubs, vector=True, flags=('-msse4.1',))
    geom = recomp_closure.run(exe, a.image).split('\n')
    part, verts = 'BEAMGEOM', []
    for line in geom:
        if line.startswith('B '):
            out.append('%s BEGIN %s' % (part, line[2:]))
        elif line.startswith('V '):
            out.append('%s V %s' % (part, line[2:]))
        elif line == 'E':
            out.append('%s END' % part)
        elif line == 'R':
            part = 'RIBBONGEOM'

    # --- BEAM
    cases = []
    for k in range(240):
        start = f32(rnd.choice([0.0, rnd.uniform(0, 900)]))
        dt = [0.0, 1 / 192, 1 / 12, 1 / 6, 0.25, 1 / 3, 0.5, 1.0, 1.999][k] if k < 9 else rnd.uniform(0, 2)
        clock = f32(start + dt)
        ms = matrix(rnd)
        mt = matrix(rnd, ms[12:15])
        bc = [f32(rnd.choice([0.372, 0.67451, 0.745098, 0.952941, 0.980392])), f32(rnd.random()), f32(rnd.random()), 1.0]
        rc = [f32(rnd.choice([0.537, 0.741176, 0.898039, 1.0])), f32(rnd.random()), f32(rnd.random()), 1.0]
        cases.append((start, clock, ms, mt, bc, rc))
    rows = ',\n'.join('{%s}' % ','.join('0x%08x' % fb(v) for v in [s, c] + ms + mt + bc + rc) for s, c, ms, mt, bc, rc in cases)
    noise_pairs = [0x82037528, 0x82053A3C, 0x820CF258, 0x82053A34]   # the constructor's +92, +96, +100, +104 (its sources)
    body = ('static const uint32_t C[][42]={\n' + rows + '\n};\n'
            'const uint32_t MGR=0x1000,EFF=0x1400,MS=0x1500,MT=0x1540,REC=0x1A00,LOOK=0x1B00;\n'
            'for(auto&q:C){memset(objects,0,sizeof(objects));store(MGR,EFF);for(int h=1;h<=5;h++)store(MGR+4*h,4*h);'
            'store(MGR+40,0x42000000);' + ''.join('store(MGR+%d,load(0x%08X));' % (92 + 4 * i, s) for i, s in enumerate(noise_pairs)) +
            'store(0x827D7518,q[1]);store(REC,q[0]);store(REC+4,MS);store(REC+32,MT);'
            'for(int i=0;i<16;i++){store(MS+4*i,q[2+i]);store(MT+4*i,q[18+i]);}'
            'store(REC+20,0x3f000000);store(REC+24,0x3fc00000);store(REC+52,0x3f000000);store(REC+68,0x3f000000);store(REC+72,0x3fc00000);'
            'store(REC+84,LOOK);for(int i=0;i<4;i++){store(LOOK+192+4*i,q[34+i]);store(LOOK+208+4*i,q[38+i]);}store(REC+81,0);'
            'Context c;fresh(c);c.r3.u32=MGR;c.r4.u32=REC;%s(c,nullptr);printf("X\\n");}\n' % n[BEAM_DRAW])
    stubs = {n[SETVAR]: 'printf("S %u %u",ctx.r5.u32,ctx.r8.u32);for(uint32_t i=0;i<ctx.r8.u32*4;i++)printf(" %08x",load(ctx.r6.u32+4*i));printf("\\n");',
             n[WORLD]: 'printf("M");for(int i=0;i<16;i++)printf(" %08x",load(ctx.r4.u32+4*i));printf("\\n");',
             n[PASSES]: 'ctx.r3.u32=0;'}
    for f in ('F-89493', 'F-90259', 'F-46773'):   # pass begin / end / effect end: never reached with 0 passes
        stubs[addr(a.tool, f)] = 'throw std::runtime_error("pass helper reached");'
    stubs[n[BEAM_GEOM]] = stubs[n[RIBBON_GEOM]] = 'throw std::runtime_error("geometry reached");'
    main_src = HEAD + 'int main(int argc,char**argv){' + load_image + body + '}\n'
    exe = recomp_closure.build(idx, [n[BEAM_DRAW]], main_src, a.work, 'pu_beam_draw', stubs=stubs, vector=True, flags=('-msse4.1',))
    res = recomp_closure.run(exe, a.image).split('X\n')
    skin_zero = 0
    for (s, c, ms, mt, bc, rc), r in zip(cases, res):
        v = {}
        m = None
        for line in r.strip().split('\n'):
            w = line.split()
            if w[0] == 'S':
                v[int(w[1])] = w[3:]
            elif w[0] == 'M':
                m = w[1:]
        assert set(v) == {4, 8, 12, 16, 20} and len(v[4]) == 132 and len(v[8]) == 256 and m, r[:200]
        # the shader reads entry 0 of both arrays only (TEXCOORD0's z is 0, see the evidence); entry 0 is zero in every case
        if all(int(x, 16) & 0x7fffffff == 0 for x in v[4][:4] + v[8][:2]):
            skin_zero += 1
        ins = ' '.join('%08x' % fb(x) for x in [s, c] + ms + mt + bc + rc)
        out.append('BEAM %s -> %s' % (ins, ' '.join(v[12] + v[16] + v[20] + v[4][:4] + v[8][:4] + m)))
    out.append('SKIN %s' % ' '.join(v[4]))
    out.append('RSKIN %s' % ' '.join(v[8]))

    # --- LIGHT
    lcases = []
    for k in range(400):
        clock = f32(rnd.uniform(0, 900))
        cam = [f32(rnd.uniform(-2000, 2000)), f32(rnd.uniform(0, 100)), f32(rnd.uniform(-2000, 2000))]
        recs = []
        for i in range(8):
            if rnd.random() < 0.5:
                recs.append(None)
                continue
            dt = rnd.choice([rnd.uniform(0, 2), rnd.uniform(0, 0.4), 1 / 6, 0.25, 1 / 3, 0.0])
            ms = matrix(rnd, cam if rnd.random() < 0.8 else None)
            mt = matrix(rnd, ms[12:15])
            recs.append((f32(clock - dt), ms, mt))
        lcases.append((clock, cam, recs))
    rows = []
    for clock, cam, recs in lcases:
        w = [fb(clock)] + [fb(x) for x in cam]
        for r in recs:
            w += [0] * 34 if r is None else [1, fb(r[0])] + [fb(x) for x in r[1] + r[2]]
        rows.append('{%s}' % ','.join('0x%08x' % x for x in w))
    # (the manager's colours as its constructor stores them: +48..+56 and +64..+72 from the image, +60 / +76 never stored)
    body = ('static const uint32_t C[][276]={\n' + ',\n'.join(rows) + '\n};\n'
            'const uint32_t MGR=0x1000,MAT=0x1500,CAM=0x1C00,POS=0x1D00,COL=0x1D10;\n'
            'for(auto&q:C){memset(objects,0,sizeof(objects));store(MGR+40,0x42000000);'
            'for(int i=0;i<3;i++){store(MGR+48+4*i,load(0x820B43C0-4*i));store(MGR+64+4*i,load(0x820B43B4-4*i));}'
            'store(0x827D7518,q[0]);for(int i=0;i<3;i++)store(CAM+48+4*i,q[1+i]);'
            'for(int r=0;r<8;r++){const uint32_t*p=q+4+34*r;uint32_t R=MGR+112+96*r;'
            '*pointer(R+81)=uint8_t(p[0]);if(!p[0])continue;store(R,p[1]);store(R+4,MAT+128*r);store(R+32,MAT+128*r+64);'
            'for(int i=0;i<16;i++){store(MAT+128*r+4*i,p[2+i]);store(MAT+128*r+64+4*i,p[18+i]);}'
            'store(R+20,0x3f000000);store(R+24,0x3fc00000);}'
            'Context c;fresh(c);c.r3.u32=MGR;c.r4.u32=CAM;c.r5.u32=POS;c.r6.u32=COL;%s(c,nullptr);'
            'printf("%%u",c.r3.u32);for(int i=0;i<4;i++)printf(" %%08x",load(POS+4*i));for(int i=0;i<4;i++)printf(" %%08x",load(COL+4*i));printf("\\n");}\n'
            % n[LIGHT])
    main_src = HEAD + 'int main(int argc,char**argv){' + load_image + body + '}\n'
    exe = recomp_closure.build(idx, [n[LIGHT]], main_src, a.work, 'pu_beam_light', vector=True, flags=('-msse4.1',))
    res = [x for x in recomp_closure.run(exe, a.image).split('\n') if x]
    assert len(res) == len(lcases)
    for (clock, cam, recs), r in zip(lcases, res):
        w = ['%08x' % fb(clock)] + ['%08x' % fb(x) for x in cam]
        for rec in recs:
            w.append('-' if rec is None else '%08x' % fb(rec[0]) + ':' + ','.join('%08x' % fb(x) for x in rec[1] + rec[2]))
        out.append('LIGHT %s -> %s' % (' '.join(w), r))
    a.out.parent.mkdir(parents=True, exist_ok=True)
    head = ['# tools/pu_beam_reference.py: %d BEAM cases (skinning entry 0 zero in %d), %d LIGHT cases' % (len(cases), skin_zero, len(lcases))]
    a.out.write_text('\n'.join(head + out) + '\n')
    print(head[0])


if __name__ == '__main__':
    main()
