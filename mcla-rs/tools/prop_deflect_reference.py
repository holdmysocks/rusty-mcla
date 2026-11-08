"""A prop's deflection loop, F-94867, run offline (the generated code whole and unchanged, vector mode), on the path that
starts the loop (the entity's +136 empty). Stubbed: the start F-78136 (prints the request's volume +0 and pitch +4 and
starts nothing), the environment update F-65914, the voice start F-90813. The prop entity: +100 the type-43 object (fields [34] [38] [40] [44] [48] [50] from the case), +80 no
environment, +64 the place; the car: +80 -> +8 -> the chassis, +224 its speed (the case's).

Cases: the disc's prop types' fields (DEFLECT objects: the deflecting posts) and random ones; speeds below, on, inside
and past the range.

Writes <out-dir>/prop_deflect_ref.txt, one case a line: "f34 v38 f40 f44 v48 f50 speed -> none | vol pitch" (hex float bits
for the floats, decimal for the s16 and the pitch)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

ENT = 0x40100000     # the prop's audio entity
OBJ = 0x40110000     # its type-43 object
CAR = 0x40120000     # the car (+80 -> CI)
CI = 0x40130000      # (+8 -> CH)
CH = 0x40140000      # the chassis (+224 speed)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:].upper()


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def gen(rnd):
    # LA_PROP_DEFAULT-like and random fields: [34] speed from (mph), [44] to, [38] / [48] volume (dB / 100), [40] / [50] ratio
    lo = rnd.choice((5.0, 10.0, 20.0, rnd.uniform(0, 60)))
    hi = lo + rnd.choice((40.0, 60.0, rnd.uniform(0.5, 120)))
    v0, v1 = rnd.randint(-2400, 0), rnd.randint(-1200, 600)
    r0 = rnd.choice((0.5, 0.8, 1.0, rnd.uniform(0.2, 2.0)))
    r1 = rnd.choice((1.0, 1.5, 2.0, rnd.uniform(0.2, 3.0)))
    lm, hm = lo / 2.237, hi / 2.237
    sp = rnd.choice((rnd.uniform(0, lm), lm, rnd.uniform(lm, hm), rnd.uniform(lm, hm), hm, rnd.uniform(hm, hm * 2.5)))
    return f32(lo), v0, f32(r0), f32(hi), v1, f32(r1), f32(sp)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fn, play, env, start = (addr(a.tool, f) for f in ('F-94867', 'F-78136', 'F-65914', 'F-90813'))
    idx = rd.index(a.recomp)
    rnd = random.Random(94867)
    cases = [gen(rnd) for _ in range(a.cases)]
    nl = chr(10)
    rows = (',' + nl).join('{0x%08xu,0x%08xu,0x%08xu,0x%08xu,0x%08xu,0x%08xu,0x%08xu}' % (fb(c[0]), c[1] & 0xffff, fb(c[2]), fb(c[3]), c[4] & 0xffff, fb(c[5]), fb(c[6])) for c in cases)
    stubs = {
        'sub_' + play: 'HIT=1;printf("%08x %d\\n",load(ctx.r6.u32),(int)load(ctx.r6.u32+4));',
        'sub_' + env: '',
        'sub_' + start: '',
    }
    b16 = lambda at, k: 'pointer(%du+%d)[0]=uint8_t(c[%d]>>8);pointer(%du+%d)[0]=uint8_t(c[%d]);' % (OBJ, at, k, OBJ, at + 1, k)
    b32 = lambda at, k: ''.join('pointer(%du+%d)[0]=uint8_t(c[%d]>>%d);' % (OBJ, at + j, k, 24 - 8 * j) for j in range(4))
    seto = b32(34, 0) + b16(38, 1) + b32(40, 2) + b32(44, 3) + b16(48, 4) + b32(50, 5)
    import recomp_closure
    recomp_closure.EXTRA += 'static int HIT;' + nl
    main_src = (
        'static const uint32_t C[][7]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(auto&c:C){rt_reset();memset(pointer(%du),0,256);memset(pointer(%du),0,256);' % (ENT, OBJ) +
        'store(0x8288D740u,0x40150000u);store(%du+100,%du);store(%du+80,0u);store(%du+136,0u);store(%du+80,%du);store(%du+8,%du);store(%du+224,c[6]);' % (ENT, OBJ, ENT, ENT, CAR, CI, CI, CH, CH) +
        seto + 'HIT=0;Context ctx=fresh();ctx.r3.u32=%du;ctx.r4.u32=%du;sub_%s(ctx,nullptr);if(!HIT) printf("none\\n");}}' % (ENT, CAR, fn) + nl)
    exe = rd.build(idx, ['sub_' + fn], main_src, a.out_dir, 'prop_deflect', stubs=stubs, image=rd.load_image(a.image))
    out = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%08x %d %08x %08x %d %08x %08x -> %s' % (fb(c[0]), c[1], fb(c[2]), fb(c[3]), c[4], fb(c[5]), fb(c[6]), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'prop_deflect_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(1 for o in out if o != 'none'), 'loops')


if __name__ == '__main__':
    main()
