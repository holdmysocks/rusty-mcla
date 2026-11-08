"""The car's prop whoosh scan, F-12768, run offline (the generated code whole and unchanged, vector mode): one prop in the
first of the nine cells per case. Stubbed, as inputs of the case: the 3 x 3 cell fetch F-66699 (puts the case's node in
cell 0, the other eight empty), the prop's velocity (its vt slot 4, a fake target writing the case's vector) and the gate
F-67165 (prints the volume f1 and the pitch f2 it is called with). The entity: +136 the car (+288 position, +128 -> the sim,
+272 velocity), +188 the control value, +2208 the GLOBAL_WHOOSHES object (fields +47 .. +73 from the case).

Cases: the disc's GLOBAL_WHOOSHES (6 m, 50 .. 70 mph, 0 .. 1, -800 .. 0 dB/100, -500 .. 200 cents) and random variants;
props inside, on and outside the reach; relative speeds below, on and above the range; control values inside and outside
0 .. 1.

Writes <out-dir>/prop_whoosh_ref.txt, one case a line: "w47 w51 w55 w59 w63 w67 w69 w71 w73 car(3) cvel(3) prop(3) pvel(3)
tx -> none | vol pitch" (hex float bits for the floats, decimal for the s16 fields)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd

recomp_closure.EXTRA += 'static int HIT;' + chr(10)

E = 0x40100000       # the vehicle audio entity
CAR = 0x40110000     # entity +136
SIM = 0x40120000     # car +128
WOBJ = 0x40130000    # GLOBAL_WHOOSHES
NODE = 0x40140000    # the cell's list node (+16 prop, +24 next)
PROP = 0x40150000    # the prop (+0 vptr, +64 position)
FVT = 0x40160000     # its vtable: slot 4 = the velocity
VEL = 0x40170000     # the case's prop velocity
FAKE_VEL = 0x7F000010


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:].upper()


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def gen(rnd):
    w = [6.0, 50.0, 70.0, 0.0, 1.0, -800, 0, -500, 200]
    if rnd.random() < 0.3:
        lo = rnd.uniform(10, 60)
        w = [rnd.uniform(2, 12), lo, lo + rnd.uniform(1, 40), rnd.uniform(-0.5, 0.5), rnd.uniform(0.6, 1.5),
             rnd.randint(-1500, 0), rnd.randint(-300, 300), rnd.randint(-1200, 0), rnd.randint(0, 1200)]
    car = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 200), rnd.uniform(-3000, 3000)]
    r = w[0] * rnd.choice((rnd.uniform(0, 1), rnd.uniform(0.9, 1.1), 1.0, rnd.uniform(0, 1.4)))
    d = [rnd.gauss(0, 1) for _ in range(3)]
    n = max(sum(x * x for x in d) ** 0.5, 1e-6)
    prop = [c + x / n * r for c, x in zip(car, d)]
    lo, hi = w[1] / 2.237, w[2] / 2.237
    s = rnd.choice((rnd.uniform(0, lo), lo, rnd.uniform(lo, hi), rnd.uniform(lo, hi), hi, rnd.uniform(hi, hi * 2)))
    dv = [rnd.gauss(0, 1) for _ in range(3)]
    n = max(sum(x * x for x in dv) ** 0.5, 1e-6)
    pvel = [0.0, 0.0, 0.0] if rnd.random() < 0.6 else [rnd.uniform(-15, 15) for _ in range(3)]
    cvel = [p - x / n * s for p, x in zip(pvel, dv)]
    tx = rnd.choice((0.0, 1.0, rnd.uniform(0, 1), rnd.uniform(0, 1), rnd.uniform(-0.3, 1.3)))
    f = lambda v: struct.unpack('>f', struct.pack('>f', v))[0]
    return [f(x) for x in w[:5]] + w[5:], [f(x) for x in car], [f(x) for x in cvel], [f(x) for x in prop], [f(x) for x in pvel], f(tx)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    scan, cells, gate = (addr(a.tool, f) for f in ('F-12768', 'F-66699', 'F-67165'))
    idx = rd.index(a.recomp)
    rnd = random.Random(12768)
    cases = [gen(rnd) for _ in range(a.cases)]
    nl = chr(10)

    def row(c):
        w, car, cv, pp, pv, tx = c
        fl = [fb(x) for x in w[:5]] + [x & 0xffff for x in w[5:]] + [fb(x) for x in car + cv + pp + pv] + [fb(tx)]
        return '{' + ','.join('0x%08xu' % x for x in fl) + '}'
    rows = (',' + nl).join(row(c) for c in cases)
    stubs = {
        'sub_' + cells: 'for(int i=0;i<9;i++) store(ctx.r5.u32+4*i,i==0?%du:0u);' % NODE,
        'sub_' + gate: 'HIT=1;float v=float(ctx.f1.f64),p=float(ctx.f2.f64);uint32_t a,b;memcpy(&a,&v,4);memcpy(&b,&p,4);'
                       'printf("%08x %08x\\n",a,b);',
    }
    fakes = {FAKE_VEL: 'for(int i=0;i<4;i++) store(ctx.r4.u32+4*i,load(%du+4*i));' % VEL}
    sw = lambda at, k: 'pointer(%du+%d)[0]=uint8_t(c[%d]>>8);pointer(%du+%d)[0]=uint8_t(c[%d]);' % (WOBJ, at, k, WOBJ, at + 1, k)
    w32 = lambda at, k: ''.join('pointer(%du+%d)[0]=uint8_t(c[%d]>>%d);' % (WOBJ, at + j, k, 24 - 8 * j) for j in range(4))
    setw = ''.join(w32(at, k) for k, at in enumerate((47, 51, 55, 59, 63))) + ''.join(sw(at, 5 + k) for k, at in enumerate((67, 69, 71, 73)))
    vec = lambda base, k: ''.join('store(%du+%d,c[%d]);' % (base, 4 * j, k + j) for j in range(3)) + 'store(%du+12,0u);' % base
    main_src = (
        'static const uint32_t C[][22]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(auto&c:C){rt_reset();memset(pointer(%du),0,4096);memset(pointer(%du),0,128);' % (WOBJ, NODE) +
        'store(%du+136,%du);store(%du+2208,%du);store(%du+188,c[21]);store(%du+128,%du);' % (E, CAR, E, WOBJ, E, CAR, SIM) +
        'store(%du+16,%du);store(%du+24,0u);store(%du,%du);store(%du+16,%du);' % (NODE, PROP, NODE, PROP, FVT, FVT, FAKE_VEL) +
        setw + vec(CAR + 288, 9) + vec(SIM + 272, 12) + vec(PROP + 64, 15) + vec(VEL, 18) +
        'HIT=0;Context ctx=fresh();ctx.r3.u32=%du;sub_%s(ctx,nullptr);if(!HIT) printf("none\\n");}}' % (E, scan) + nl)
    exe = rd.build(idx, ['sub_' + scan], main_src, a.out_dir, 'prop_whoosh', stubs=stubs, image=rd.load_image(a.image),
                   fakes=fakes)
    out = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (w, car, cv, pp, pv, tx), o in zip(cases, out):
        lines.append(' '.join(['%08x' % fb(x) for x in w[:5]] + ['%d' % x for x in w[5:]] +
                              ['%08x' % fb(x) for x in car + cv + pp + pv + [tx]]) + ' -> ' + o)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'prop_whoosh_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(1 for o in out if o != 'none'), 'whooshes')


if __name__ == '__main__':
    main()
