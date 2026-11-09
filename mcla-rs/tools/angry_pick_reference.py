"""The walkers' angry clip pick (F-72336) run offline: the generated code whole and unchanged, with its stack probe in the
closure, on flat guest memory. The list is the manager's avoid list (r3 -> +0 the list object: +8 the entries, 20 bytes:
+4 AvoidType, +16 the clip; u16 +12 the count), here up to 10 entries with random types and clips present or not; r4 the
walker's bSex; the shared MWC (0x82860890) is the case's.
Writes <out-dir>/angry_pick_ref.txt: the case's words (count, sex, MWC lo, carry, then type and presence per entry), '->',
the picked entry (its index; the routine returns the entry; -1 none) and the MWC after (the MWC words as two 16-bit hex halves, hi_lo)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += 'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10)

N = 10


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    pick = addr(a.tool, 'F-72336')
    rnd = random.Random(72336)
    cases = []
    for _ in range(4000):
        n = rnd.choice((0, 1, 2, 3, 6, 10, 10, 10))
        fem = rnd.choice((0, 1, 1, 2, 0x100))
        lo, carry = rnd.getrandbits(32), rnd.getrandbits(32)
        if rnd.random() < 0.05:
            lo = rnd.choice((0, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF))
        c = [n, fem, lo, carry]
        for i in range(N):
            c += [rnd.choice((0, 1, 2, 3, 4, 4, 5, 5)), int(rnd.random() < 0.85)]
        cases.append(c)
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    setup = ('memset(objects,0,sizeof(objects));'
             'store(0x1000,0x1080);store(0x1080+8,0x1100);objects[0x1080-0x1000+12]=0;objects[0x1080-0x1000+13]=c[0];'
             'for(int i=0;i<10;i++){store(0x1100+20*i+4,c[4+2*i]);store(0x1100+20*i+16,c[5+2*i]?0x1800+16*i:0);}'
             'put(0x82860890,c[2]);put(0x82860894,c[3]);')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'auto get=[](uint32_t a){const uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|p[1]<<16|p[2]<<8|p[3];};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=c[1];'
            'sub_%s(ctx,nullptr);int e=ctx.r3.u32?int((ctx.r3.u32-0x1100)/20):-1;' % pick +
            ('uint32_t lo=get(0x82860890),cy=get(0x82860894);printf("%%d %%04x_%%04x %%04x_%%04x%s",e,lo>>16,lo&0xFFFF,cy>>16,cy&0xFFFF);}' % NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + pick], main_src, a.out_dir, 'angry_pick')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join(('%04x_%04x' % (x >> 16, x & 0xFFFF)) if i in (2, 3) else '%x' % x for i, x in enumerate(c)) + ' -> ' + o
             for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'angry_pick_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] != '-1' for o in out), 'with a pick')


if __name__ == '__main__':
    main()
