"""The power-up options record's bit-field copy (F-90625) and operator= (F-37780) run offline: the original's code, unchanged, on random
source / destination 28-byte records (recomp_closure, flat guest memory). Each line: kind, destination before, source, destination after
(28 bytes each, hex, '-' between bytes so no word reads as an address). F-90625 runs alone ("BIT"), F-37780 with its callee ("ASN").
Usage: pu_options_copy_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --tool mcla-tool --work DIR --out FILE"""
import argparse
import random
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

N = 1024


def addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--tool', required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    a = ap.parse_args()
    idx = index(a.recomp)
    rnd = random.Random(20261106)
    cases = []
    for i in range(N):
        d = bytes(rnd.randrange(256) for _ in range(28))
        s = bytes(rnd.randrange(256) for _ in range(28))
        if i % 8 == 0:
            s = bytes(b ^ 0xFF for b in d)  # complement: every bit differs
        cases.append((d, s))
    out = []
    for kind, fid in (('BIT', 'F-90625'), ('ASN', 'F-37780')):
        fn = addr(a.tool, fid)
        # the records come on stdin as hex: destination at 0x1000, source at 0x1100
        main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                    'image.assign(std::istreambuf_iterator<char>(bin),{});\n'
                    ' char dh[80],sh[80];\n'
                    ' while(std::scanf("%%s %%s",dh,sh)==2){std::memset(objects,0,sizeof(objects));\n'
                    '  for(int i=0;i<28;i++){unsigned v;std::sscanf(dh+2*i,"%%2x",&v);*pointer(0x1000+i)=v;std::sscanf(sh+2*i,"%%2x",&v);*pointer(0x1100+i)=v;}\n'
                    '  Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r1.u32=0x2800;%s(ctx,nullptr);\n'
                    '  for(int i=0;i<28;i++)std::printf("%%02x",*pointer(0x1000+i));std::puts("");}}\n' % fn)
        exe = build(idx, [fn], main_src, a.work, 'pu_opt_' + kind.lower())
        inp = ''.join('%s %s\n' % (d.hex(), s.hex()) for d, s in cases)
        res = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True, check=True).stdout.split()
        assert len(res) == N, len(res)
        for (d, s), r in zip(cases, res):
            out.append('%s %s %s %s' % (kind, d.hex('-'), s.hex('-'), bytes.fromhex(r).hex('-')))
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'lines')


if __name__ == '__main__':
    main()
