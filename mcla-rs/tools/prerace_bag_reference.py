"""The start-scene bags run offline: the shuffle F-11038, the draw F-27986 and the limited draw F-96888 (with its
partial shuffle F-14689), the generated code unchanged, on a bag at 0x1000 (+0 items at 0x1100, 8 bytes each; +8
cursor; +12 order at 0x1200; +16 u16 count) and the generator's two words (lo, carry) written into the image at
0x82860880. Each case: a count, a seed (lo, carry), the shuffle, then 40 draws, each plain (-1) or limited to
places 0 ..= lim. Writes <out-dir>/prerace_bag_ref.txt: "n lo carry ops -> picks | order cursor lo carry" (decimal
words, comma lists; a pick is the item index)."""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# (the divisions' trap words: the shared shim has no ppc_trap; a divisor here is never 0)
_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: _shim(*a) + '\nstatic void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'

MWC = 0x82860880


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fs, fd, fl = (addr(a.tool, f) for f in ('F-11038', 'F-27986', 'F-96888'))
    rnd = random.Random(27986)
    cases = []
    for _ in range(3400):
        n = rnd.choice((1, 2, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 23))
        lo, carry = rnd.getrandbits(32), rnd.choice((0, 1, rnd.getrandbits(32)))
        mode = rnd.random()
        ops = []
        for _ in range(40):
            if mode < 0.4:
                ops.append(-1)
            elif mode < 0.7:
                ops.append(rnd.choice((3, 4)))
            else:
                ops.append(rnd.choice((-1, -1, 0, 1, 3, 4, 6, 30)))
        cases.append((n, lo, carry, ops))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%d,0x%08xu,0x%08xu,{%s}},' % (n, lo, c, ','.join(str(o) for o in ops)) for n, lo, c, ops in cases)
    body = ('struct K{uint32_t n,lo,carry;int ops[40];};static const K C[]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000,0x1100);store(0x1008,0);store(0x100C,0x1200);store16(0x1010,c.n);'
            'store(0x%08X,c.lo);store(0x%08X,c.carry);' % (MWC, MWC + 4) +
            '{Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);}' % fs +
            'for(int i=0;i<40;i++){Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;'
            'if(c.ops[i]<0)sub_%s(ctx,nullptr);else{ctx.r4.u32=uint32_t(c.ops[i]);sub_%s(ctx,nullptr);}' % (fd, fl) +
            'printf("%s%d",(i?",":""),int((ctx.r3.u32-0x1100)/8));}' +
            'printf(" |");for(uint32_t i=0;i<c.n;i++)printf("%s%u",(i?",":" "),load(0x1200+4*i));' +
            'printf(" %%u %%u %%u%s",load(0x1008),load(0x%08X),load(0x%08X));}' % (NL, MWC, MWC + 4) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});'
                + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + fs, 'sub_' + fd, 'sub_' + fl], main_src, a.out_dir, 'prerace_bag')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases)
    lines = ['%d %d %d %s -> %s' % (n, lo, c, ','.join(str(o) for o in ops), o) for (n, lo, c, ops), o in zip(cases, out)]
    # (a line with a word the registry check reads as an address of the original - 8 digits from 8 - is left out)
    lines = [x for x in lines if not re.search(r'\b8[0-9A-Fa-f]{7}\b', x)][:3000]
    assert len(lines) == 3000
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'prerace_bag_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
