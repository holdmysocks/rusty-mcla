"""The Tournament_RESULTS rows run offline: F-26722, the generated code unchanged with its row array (F-18224, F-16296),
the CRT qsort F-79639 and the rows' comparator F-33081 with the tournament entry's score F-91345 (called through the
vtable); stubbed and answering from the case: the allocator F-15488 / free F-13813, the list clear F-51797, the new
row F-40813 (row i at 0x1800 + 32 i: fs_property1 +8, fs_text3 +16, fs_text4 +24), the row's name filler F-26903
(logs which record), the "%s%d" formatter (host snprintf) and the tournament's member lookup F-84339
(nRacerPoints_<i>), the points table (the career manager's word at 0x8286D8DC: +4 * (93 + k), k 0..16). The results
block at 0x4000: racer count +3080, 192-byte records from +8 (+16 type, +40 finish time, +172 place). Writes
<out-dir>/tournament_rows_ref.txt: "<count> <table> <type,time,place,total per racer> -> <rows: record,index> |
<per row: property1,text3,text4>"."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# (the qsort's division trap words: the shared shim has no ppc_trap)
_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: _shim(*a) + chr(10) + 'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases():
    rnd = random.Random(26722)
    out = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 4, 6, 8, 12, 16, rnd.randrange(0, 17)))
        table = [7, 4, 2, 1] + [0] * 13 if rnd.random() < 0.6 else [rnd.randrange(-3, 12) for _ in range(17)]
        types = [rnd.choice((2, 2, 2, 1)) for _ in range(n)]
        if n and rnd.random() < 0.85:
            types[rnd.randrange(n)] = 0
        places = list(range(1, n + 1))
        rnd.shuffle(places)
        if n > 1 and rnd.random() < 0.2:
            for _ in range(rnd.randrange(1, n)):
                places[rnd.randrange(n)] = places[rnd.randrange(n)]
        if rnd.random() < 0.05:
            places = [rnd.randrange(-2, 22) for _ in range(n)]
        times = [rnd.choice((0x7FFFFFFF, -1, rnd.randrange(60000, 400000), 200000)) for _ in range(n)]
        totals = [rnd.randrange(0, 40) for _ in range(n)]
        out.append((n, table, types, times, places, totals))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-26722')
    cmp_, score, dtor = (addr(a.tool, x) for x in ('F-33081', 'F-91345', 'F-38932'))
    cs = cases()
    nl = chr(10)
    rd = lambda reg, n: 'char s[%d];for(int i=0;i<%d;i++){s[i]=*pointer(ctx.%s.u32+i);if(!s[i])break;}s[%d]=0;' % (n, n - 1, reg, n - 1)
    stubs = {
        'F-15488': '{uint32_t n=(ctx.r3.u32+15)&~15u;ctx.r3.u32=g_heap;g_heap+=n;}',
        'F-13813': '',
        # (qsort's lock helpers, as in marker_sort_reference.py)
        'F-37557': '',
        'F-52205': '',
        'F-51797': '',
        'F-40813': 'ctx.r3.u32=0x1800+32*g_rows;g_rows++;',
        'F-26903': 'printf("r%d,%d ",(int)((load(ctx.r3.u32+4)-0x4008)/192),(int)ctx.r4.s32);',
        'F-84339': rd('r4', 32) + 'if(!strncmp(s,"nRacerPoints_",13))ctx.r3.u32=0x1600+8*atoi(s+13);'
                   'else if(!strcmp(s,"fs_property1"))ctx.r3.u32=ctx.r3.u32+8;else if(!strcmp(s,"fs_text3"))ctx.r3.u32=ctx.r3.u32+16;'
                   'else if(!strcmp(s,"fs_text4"))ctx.r3.u32=ctx.r3.u32+24;else{printf("?%s ",s);ctx.r3.u32=0x1A00;}',
        'F-32585': 'char f[16],a[32];for(int i=0;i<15;i++){f[i]=*pointer(ctx.r5.u32+i);if(!f[i])break;}f[15]=0;'
                   'for(int i=0;i<31;i++){a[i]=*pointer(ctx.r6.u32+i);if(!a[i])break;}a[31]=0;'
                   'char o[64];snprintf(o,sizeof o,f,a,(int)ctx.r7.s32);for(int i=0;i<(int)ctx.r4.u32;i++){*pointer(ctx.r3.u32+i)=o[i];if(!o[i])break;}',
    }
    stubs = {'sub_' + addr(a.tool, k): v for k, v in stubs.items()}
    recomp_closure.EXTRA += ('static uint32_t g_heap,g_rows;static void dispatch(Context&ctx,uint32_t a);\n'
                             '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,a)\n')
    li = lambda v: ','.join(map(str, v)) or '0'
    rows = (',' + nl).join('{%d,{%s},{%s},{%s},{%s},{%s}}' % (n, li(tb), li(t), ','.join('(int)%dLL' % x for x in tm) or '0', li(p), li(w))
                           for n, tb, t, tm, p, w in cs)
    body = ('void dispatch(Context&ctx,uint32_t a){if(a==0x%su)sub_%s(ctx,nullptr);else if(a==0x%su)sub_%s(ctx,nullptr);'
            'else if(a==0x%su){}else{printf("?call ");}}' % (cmp_, cmp_, score, score, dtor) + nl +
            'struct K{int n;int tb[17];int t[16],tm[16],p[16],w[16];};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            # (the points table's holder: the career manager's word at 0x8286D8DC (lis -32121, -10020))
            'store(0x8286D8DC,0x1100);' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));g_heap=0x5000;g_rows=0;'
            'for(int k=0;k<17;k++)store(0x1100+4*(93+k),c.tb[k]);for(int k=0;k<16;k++)store(0x1600+8*k+4,c.w[k]);store(0x4000+3080,c.n);'
            'for(int i=0;i<c.n;i++){uint32_t b=0x4000+192*i;store(b+8,0x3000+i);store(b+16,c.t[i]);store(b+40,c.tm[i]);store(b+172,c.p[i]);}'
            '{Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x7000;ctx.r4.u32=0x1500;ctx.r5.u32=0x4000;sub_%s(ctx,nullptr);}' % f +
            'printf("| ");for(uint32_t i=0;i<g_rows;i++){uint32_t r=0x1800+32*i;printf("%d,%d,%d ",(int)load(r+12),(int)load(r+20),(int)load(r+28));}'
            'printf("' + chr(92) + 'n");}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f, 'sub_' + cmp_, 'sub_' + score], body, a.out_dir, 'tourrows', stubs=stubs, scratch_end=0x8000)
    out = [x.strip() for x in run(exe, a.image).split(nl)][:len(cs)]
    assert len(out) == len(cs)
    lines = ['%d %s %s -> %s' % (n, li(tb), ' '.join('%d,%d,%d,%d' % q for q in zip(t, tm, p, w)) or '-', o) for (n, tb, t, tm, p, w), o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'tournament_rows_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
