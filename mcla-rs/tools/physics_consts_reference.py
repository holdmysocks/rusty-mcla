"""The physics' static initialisers of one global vector each run offline (generated code unchanged, vector mode): the
solver's allowed depth F-36556 and the convex test's least squared length F-92824, relative tolerance F-70580, near
squared distance F-64228 and catch distance F-24192. Each runs on its global filled with a pattern, so the lanes it
leaves alone show.

Writes <out-dir>/physics_consts_ref.txt, one initialiser a line: "F-id -> x y z w" (hex)."""
import argparse
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += '#include <simde/x86/avx.h>' + chr(10) + '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10)

# (the initialiser and the global it fills, as an offset from BASE, so the file cites no address)
BASE = 0x8280 << 16
INITS = (('F-36556', 0xFF840), ('F-92824', 0xFF8F0), ('F-70580', 0xFF8D0), ('F-64228', 0xFF8C0), ('F-24192', 0xFF900))


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    names = ['sub_' + addr(a.tool, f) for f, _g in INITS]
    nl = chr(10)
    NL = chr(92) + 'n'
    body = ''
    for (fid, g), fn in zip(INITS, names):
        at = '0x%08x' % (BASE + g)
        body += (f'for(int k=0;k<4;k++)store({at}+4*k,0xA5A5A5A0u+k);{{Context ctx{{}};ctx.r1.u32=0x2F00;{fn}(ctx,nullptr);}}' +
                 f'printf("{fid} -> %08x %08x %08x %08x' + NL + f'",load({at}),load({at}+4),load({at}+8),load({at}+12));' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), names, main_src, a.out_dir, 'physics_consts', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(INITS), out
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'physics_consts_ref.txt').write_text(nl.join(out) + nl)
    print(nl.join(out))


if __name__ == '__main__':
    main()
