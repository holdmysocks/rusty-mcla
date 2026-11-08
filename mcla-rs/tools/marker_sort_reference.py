"""The icon queue's far-first sort runs offline: F-34858 (rebuilds the pointer array of the queued entries, then the C
runtime's qsort F-79639 with the compare F-13438 on the distance at +100), the generated code unchanged; the qsort's
bad-parameter paths F-37557 / F-52205 are stubbed (never reached with a valid array) and its indirect compare call goes
to F-13438. The icon manager at 0x1000: entries at +5504 (112 bytes each), count +12672, pointer array (+12688 base at
0x5000, +12692 u16 count). Cases: 0..64 entries, distances drawn from small sets (many ties, +-0, NaN, inf) or random.
Writes <out-dir>/marker_sort_ref.txt: "<distance bits, decimal> -> <entry index order>"."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


# (the qsort's division trap words: the shared shim has no ppc_trap; the divisor is the element size 4)
_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: _shim(*a) + '\nstatic void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def bits(f):
    return struct.unpack('<I', struct.pack('<f', f))[0]


def cases():
    rnd = random.Random(34858)
    out = []
    for _ in range(3200):
        n = rnd.choice((0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 20, 31, 40, 64, rnd.randrange(0, 65)))
        k = rnd.random()
        if k < 0.35:
            pool = [rnd.choice((1.0, 2.0, 3.0, 5.0)) for _ in range(rnd.randrange(1, 4))]
            d = [rnd.choice(pool) for _ in range(n)]
        elif k < 0.5:
            d = [rnd.choice((0.0, -0.0, 7.5, float('inf'), float('nan'), 7.5)) for _ in range(n)]
        elif k < 0.6:
            d = sorted(rnd.uniform(0, 300) for _ in range(n))
            if rnd.random() < 0.5:
                d.reverse()
        else:
            d = [rnd.uniform(0, 300) for _ in range(n)]
        out.append([bits(x) for x in d])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fsort, fcmp, fbad1, fbad2 = (addr(a.tool, f) for f in ('F-34858', 'F-13438', 'F-37557', 'F-52205'))
    cs = cases()
    nl = chr(10)
    recomp_closure.EXTRA += ('void sub_%s(Context &ctx,const uint8_t *base);\n'
                             '#define REX_CALL_INDIRECT_FUNC(a) do{if((a)==0x%sU)sub_%s(ctx,base);'
                             'else throw std::runtime_error("unexpected indirect call");}while(0)\n') % (fcmp, fcmp, fcmp)
    rows = (',' + nl).join('{%d,{%s}}' % (len(d), ','.join('%uu' % x for x in d) or '0') for d in cs)
    body = ('struct K{uint32_t n;uint32_t d[64];};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000+12672,c.n);store(0x1000+12688,0x5000);store16(0x1000+12692,0);'
            'for(uint32_t i=0;i<c.n;i++)store(0x1000+5504+112*i+100,c.d[i]);' +
            '{Context ctx{};ctx.r1.u32=0x8F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);}' % fsort +
            'uint32_t m=load16(0x1000+12692);printf("%u",m);for(uint32_t i=0;i<m;i++)printf(" %u",(load(0x5000+4*i)-(0x1000+5504))/112);'
            'printf("' + chr(92) + 'n");}' + nl + '}' + nl)
    stubs = {'sub_' + fbad1: '', 'sub_' + fbad2: ''}
    exe = build(index(a.recomp), ['sub_' + fsort, 'sub_' + fcmp], body, a.out_dir, 'marker_sort', stubs=stubs, scratch_end=0x9000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cs)
    lines = ['%s -> %s' % (' '.join(str(x) for x in d) or '-', o) for d, o in zip(cs, out)]
    # (a line with a word the registry check reads as an address of the original - 8 digits from 8 - is left out)
    lines = [x for x in lines if not re.search(r'\b8[0-9A-Fa-f]{7}\b', x)][:3000]
    assert len(lines) == 3000, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'marker_sort_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
