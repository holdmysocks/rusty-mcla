"""Run the original's cone pass F-71942 offline (the generated code as it is) and record, for each listed lamp, what it
hands the per-lamp setup F-19533 (density f1, the faded strength f2, edge f3, the mesh r4, the entry r5) and which mesh
the draw F-20535 gets, for the port's test (crates/game/src/testdata/cone_fade_reference.txt).

Stubs: the CRT qsort F-79639 (the list is given in the order the case wants: the sort itself is the library's, its
compare F-88996 is a truncated difference and not checked here), F-19533 and F-20535 print what they get.
The list (128-byte entries: +0 the distance, +112 the entry's glow factor) sits at the global the code reads, its count
the u16 at +4; the pass object holds two mesh words (near +0, far +4) and +68.

  CASE <f1 density> <f2 strength> <f3 edge> <count>
  ENT <+0 distance> <+112 factor>            (one per entry)
  LAMP <f1> <f2> <f3> <r4 mesh> <r5 entry index>   (one per lamp set up, in order)
  DRAW <r3 mesh> <r4>
  END <count after>

python cone_fade_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


# the list descriptor F-71942 reads: +0 the entries' pointer, +4 the u16 count
LIST = 0x8288B8D8


def cases(rnd):
    out = []
    S = [0.0, -0.0, 20.0, 30.0, 75.0, 250.0, 300.0, 19.999998, 20.000002, 75.00001, 249.99998, 250.00002, 1e30, float('inf'), float('nan')]
    for k in range(500):
        dens = rnd.choice((1.0, 0.5, 0.0, 0.01, rnd.uniform(0, 2)))
        stre = rnd.choice((1.0, 0.4, 0.0, 0.01, rnd.uniform(0, 2)))
        edge = rnd.uniform(0, 3)
        n = rnd.choice((0, 1, 3, 20, rnd.randrange(1, 41)))  # (at most 40: the entries must fit below the stack)
        ents = []
        for _ in range(n):
            d = rnd.choice(S) if rnd.random() < 0.15 else rnd.choice((rnd.uniform(0, 40), rnd.uniform(15, 320), rnd.uniform(60, 90), rnd.uniform(240, 310)))
            fac = rnd.choice((1.0, 1.0, 0.0, 0.5, rnd.uniform(0, 1)))
            ents.append((fbits(d), fbits(fac)))
        out.append((fbits(dens), fbits(stre), fbits(edge), ents))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, qs, setup, draw = (addr(a.tool, f) for f in ('F-71942', 'F-79639', 'F-19533', 'F-20535'))
    cs = cases(random.Random(71942))
    nl, NL = chr(10), chr(92) + 'n'
    fl = 'float g[3]={float(ctx.f1.f64),float(ctx.f2.f64),float(ctx.f3.f64)};uint32_t b[3];memcpy(b,g,12);'
    stubs = {'sub_' + qs: '',
             'sub_' + setup: fl + 'printf("LAMP %08x %08x %08x %08x %u' + NL + '",b[0],b[1],b[2],ctx.r4.u32,(ctx.r5.u32-16-0x1100)/128);',
             'sub_' + draw: 'printf("DRAW %08x %08x' + NL + '",ctx.r3.u32,ctx.r4.u32);'}
    flat = []
    for dens, stre, edge, ents in cs:
        flat += [dens, stre, edge, len(ents)] + [w for e in ents for w in e]
    body = ('static const uint32_t C[]={' + ','.join('0x%08xu' % v for v in flat) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'size_t k=0;while(k<sizeof(C)/4){memset(objects,0,sizeof(objects));uint32_t n=C[k+3];' +
            'printf("CASE %08x %08x %08x %u' + NL + '",C[k],C[k+1],C[k+2],n);' +
            # the entries at 0x1100 (128 bytes each), the descriptor at the global; the pass at 0x1000: meshes 0xAAAA0000 / 0xBBBB0000, +68 0xCCCC
            ('store(0x%08X,0x1100);' % LIST) + ('pointer(0x%08X)[0]=uint8_t(n>>8);pointer(0x%08X)[1]=uint8_t(n);' % (LIST + 4, LIST + 4)) +
            'store(0x1000,0xAAAA0000u);store(0x1004,0xBBBB0000u);store(0x1000+68,0xCCCCu);' +
            'for(uint32_t r=0;r<n;r++){store(0x1100+128*r,C[k+4+2*r]);store(0x1100+128*r+112,C[k+5+2*r]);printf("ENT %08x %08x' + NL + '",C[k+4+2*r],C[k+5+2*r]);}' +
            'float g[3];memcpy(g,&C[k],12);k+=4+2*n;' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.f1.f64=g[0];ctx.f2.f64=g[1];ctx.f3.f64=g[2];' + ('sub_%s(ctx,nullptr);' % fn) +
            'printf("END %u' + NL + ('",unsigned(pointer(0x%08X)[0])<<8|pointer(0x%08X)[1]);}}' % (LIST + 4, LIST + 4)) + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'cone_fade', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs) and out.count('END') == len(cs), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases', out.count('ENT'), 'entries', out.count('LAMP'), 'lamps')


if __name__ == '__main__':
    main()
