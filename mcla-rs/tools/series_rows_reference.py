"""The Series_RESULTS rows run offline: F-23726, the generated code unchanged with its row array (F-59937, F-61067),
the CRT qsort F-79639 and the rows' comparator F-33081 with the entry's score F-66331 (called through the vtable);
stubbed and answering from the case: the allocator F-15488 / free F-13813, the list clear F-51797, the new row
F-40813 (row i at 0x1800 + 16 i: fs_text1 +0, fs_property1 +8), the row's name filler F-26903 (logs which record),
the series' member lookup F-84339 (nRacerWins_<i>, nRacesToWin), the "%s%d" / "%d / %d" formatter (host snprintf)
and the string table's set F-41524 (logs key and text). The results block at 0x4000: racer count +3080, 192-byte
records from +8 (+16 type, +40 finish time, +172 place). Writes <out-dir>/series_rows_ref.txt:
"<count> <need> <type,time,place,wins per racer> -> <per row: record,position,property1,key=text>"."""
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
    rnd = random.Random(23726)
    out = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 4, 6, 8, 12, 16, rnd.randrange(0, 17)))
        need = rnd.choice((2, 3, 3, rnd.randrange(0, 6)))
        types = [rnd.choice((2, 2, 2, 1)) for _ in range(n)]
        if n and rnd.random() < 0.85:
            types[rnd.randrange(n)] = 0
        places = list(range(1, n + 1))
        rnd.shuffle(places)
        if n > 1 and rnd.random() < 0.3:
            # (equal places: the comparator's tie rules)
            for _ in range(rnd.randrange(1, n)):
                places[rnd.randrange(n)] = places[rnd.randrange(n)]
        times = [rnd.choice((0x7FFFFFFF, -1, rnd.randrange(60000, 400000), 200000)) for _ in range(n)]
        wins = [rnd.randrange(0, 4) for _ in range(n)]
        out.append((n, need, types, times, places, wins))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-23726')
    cmp_, score, dtor = (addr(a.tool, x) for x in ('F-33081', 'F-66331', 'F-38932'))
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
        'F-40813': 'ctx.r3.u32=0x1800+16*g_rows;g_rows++;',
        'F-26903': 'printf("r%d,%d ",(int)((load(ctx.r3.u32+4)-0x4008)/192),(int)ctx.r4.s32);',
        'F-84339': rd('r4', 32) + 'if(!strcmp(s,"nRacesToWin"))ctx.r3.u32=0x1580;'
                   'else if(!strncmp(s,"nRacerWins_",11))ctx.r3.u32=0x1600+8*atoi(s+11);'
                   'else if(!strcmp(s,"fs_text1"))ctx.r3.u32=ctx.r3.u32+0;else if(!strcmp(s,"fs_property1"))ctx.r3.u32=ctx.r3.u32+8;'
                   'else{printf("?%s ",s);ctx.r3.u32=0x1A00;}',
        'F-32585': 'char f[16],a[32];for(int i=0;i<15;i++){f[i]=*pointer(ctx.r5.u32+i);if(!f[i])break;}f[15]=0;'
                   'char o[64];if(!strcmp(f,"%s%d")){for(int i=0;i<31;i++){a[i]=*pointer(ctx.r6.u32+i);if(!a[i])break;}a[31]=0;snprintf(o,sizeof o,f,a,(int)ctx.r7.s32);}'
                   'else snprintf(o,sizeof o,f,(int)ctx.r6.s32,(int)ctx.r7.s32);'
                   'for(int i=0;i<(int)ctx.r4.u32;i++){*pointer(ctx.r3.u32+i)=o[i];if(!o[i])break;}',
        'F-41524': 'char k[32],v[32];for(int i=0;i<31;i++){k[i]=*pointer(ctx.r4.u32+i);if(!k[i])break;}k[31]=0;'
                   'for(int i=0;i<31;i++){v[i]=*pointer(ctx.r5.u32+i);if(!v[i])break;}v[31]=0;'
                   'for(char*q=v;*q;q++)if(*q==32)*q=95;printf("%s=%s ",k,v);',
    }
    stubs = {'sub_' + addr(a.tool, k): v for k, v in stubs.items()}
    recomp_closure.EXTRA += ('static uint32_t g_heap,g_rows;static void dispatch(Context&ctx,uint32_t a);\n'
                             '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,a)\n')
    rows = (',' + nl).join('{%d,%d,{%s},{%s},{%s},{%s}}' % (n, nd, ','.join(map(str, t)) or '0', ','.join('(int)%dLL' % x for x in tm) or '0',
                                                         ','.join(map(str, p)) or '0', ','.join(map(str, w)) or '0') for n, nd, t, tm, p, w in cs)
    body = ('void dispatch(Context&ctx,uint32_t a){if(a==0x%su)sub_%s(ctx,nullptr);else if(a==0x%su)sub_%s(ctx,nullptr);'
            'else if(a==0x%su){}else{printf("?call ");}}' % (cmp_, cmp_, score, score, dtor) + nl +
            'struct K{int n,nd;int t[16],tm[16],p[16],w[16];};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));g_heap=0x5000;g_rows=0;'
            'store(0x1580+4,c.nd);for(int k=0;k<16;k++)store(0x1600+8*k+4,c.w[k]);store(0x4000+3080,c.n);'
            'for(int i=0;i<c.n;i++){uint32_t b=0x4000+192*i;store(b+8,0x3000+i);store(b+16,c.t[i]);store(b+40,c.tm[i]);store(b+172,c.p[i]);}'
            '{Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x7000;ctx.r4.u32=0x1500;ctx.r5.u32=0x4000;sub_%s(ctx,nullptr);}' % f +
            'printf("| ");for(uint32_t i=0;i<g_rows;i++)printf("%u,%u,%u ",load(0x1800+16*i+4),load(0x1800+16*i+12),load(0x1800+16*i+8));'
            'printf("' + chr(92) + 'n");}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f, 'sub_' + cmp_, 'sub_' + score], body, a.out_dir, 'serrows', stubs=stubs, scratch_end=0x8000)
    out = [x.strip() for x in run(exe, a.image).split(nl)][:len(cs)]
    assert len(out) == len(cs)
    lines = ['%d %d %s -> %s' % (n, nd, ' '.join('%d,%d,%d,%d' % q for q in zip(t, tm, p, w)) or '-', o) for (n, nd, t, tm, p, w), o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'series_rows_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
