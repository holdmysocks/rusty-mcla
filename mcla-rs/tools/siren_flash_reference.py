"""Run the original's siren light step F-72231 offline, as the generated code has it, and record the light it adds, for
the port's test (crates/game/src/testdata/siren_flash_reference.txt).

The manager object holds what its ctor F-93597 stores (strength 200 at +14624, blue at +14640, red at +14656, on / off
0.5 at +14672 / +14676, 3 flashes at +14680). The clock F-50525 is stubbed to return the case's milliseconds; the light
add F-31349 is stubbed to print its strength (f1) and colour (r6). The library fmod F-39069 runs as compiled.

  CASE <ms> -> <strength> <colour r g b>

python siren_flash_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import random
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases(rnd):
    ms = list(range(0, 1300)) + [rnd.randrange(0, 1 << 32) for _ in range(3000)]
    # the boundaries far out, where t = ms x 0.01 loses its fraction
    for b in (1 << 24, 1 << 25, 1 << 27, 1 << 30, (1 << 32) - 1):
        ms += [b + d for d in range(-40, 40) if 0 <= b + d < (1 << 32)]
    return ms


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, clock, add = (addr(a.tool, f) for f in ('F-72231', 'F-50525', 'F-31349'))
    cs = cases(random.Random(72))
    nl, NL = chr(10), chr(92) + 'n'
    stubs = {'sub_' + clock: 'extern uint32_t MS;ctx.r3.u64=MS;',
             'sub_' + add: 'float g=float(ctx.f1.f64);uint32_t b;memcpy(&b,&g,4);printf(" -> %08x %08x %08x %08x' + NL + '",b,load(ctx.r6.u32),load(ctx.r6.u32+4),load(ctx.r6.u32+8));'}
    body = ('uint32_t MS;static const uint32_t C[]={' + ','.join('%du' % v for v in cs) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(uint32_t m:C){memset(objects,0,sizeof(objects));MS=m;' +
            # the manager at 0xFFFFD8E0 so that its fields (+14624 ..) land at 0x1200 (the address adds wrap), the light object pointer 0x8287E2F4 -> 0x1000, the car at 0x1100
            'store(0xFFFFD8E0u+14624,0x43480000u);for(int i=0;i<4;i++){store(0xFFFFD8E0u+14640+4*i,i==2?0x3F800000u:0);store(0xFFFFD8E0u+14656+4*i,i==0?0x3F800000u:0);}' +
            'store(0xFFFFD8E0u+14672,0x3F000000u);store(0xFFFFD8E0u+14676,0x3F000000u);store(0xFFFFD8E0u+14680,3);' +
            'printf("CASE %u",m);Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0xFFFFD8E0u;ctx.r4.u32=0x1100;' +
            ('sub_%s(ctx,nullptr);}}' % fn) + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'siren_flash', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    lines = [l for l in out.split(nl) if l]
    assert len(lines) == len(cs), (len(lines), out[:300])
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
