"""The start-scene pick run offline: the native PickRacestartCutsceneOrTauntFromGridEnum F-10331 and the mapper's pick
F-24279, with the bags' shuffle F-11038, draw F-27986 and limited draw F-96888 (and its partial shuffle), the generated
code unchanged. Stubbed: the log F-47121, the online-session test F-24338 (0: offline), the front-car `_lr_` test
F-25541 (answers the case's flag). The debug params forcepreracetaunt / disablepreracetaunt (their words at
0x828CD4E4 / 0x828CD4D0 +4) are written 0. The manager (the word at 0x8286D8F0) at 0x1000, its mapper at +3908:
+12 key array (12-byte elements, word 0 the key) at 0x2000, +16 u16 count 9, +20 bags (20 bytes: +0 items, +4 / +6 u16
their count and capacity, +8 cursor, +12 order, +16 u16 count) at 0x2100, +28 the `_sc` flag; items 8 bytes (+0 name,
+4 u16 length). Each case: the grids' keys (a permutation of 0..8), their item counts, which names hold
`_racestarter_`, the sc flag, a seed (lo, carry 1), the load shuffle of every bag in order, then 30 picks (grid key
-2..9, firstRace, camOnly, lr). Writes <out-dir>/prerace_pick_ref.txt: "keys counts starter sc lo picks -> results |
lo carry" (a result is place.item or - for none). Then the race parser's GridType lookup F-53452 on 25 names:
<out-dir>/grid_type_ref.txt, "name|key"."""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_shim = recomp_closure.shim
# (the divisions' trap words; strstr's extsb reads a register's low byte signed; stricmp uses cr1, cr5, cr7)
recomp_closure.shim = lambda *a: (_shim(*a).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;', 1).replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr5,cr6,cr7;', 1) + chr(10) +
                                  'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10))

MWC = 0x82860880


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases():
    rnd = random.Random(10331)
    out = []
    for _ in range(3300):
        keys = list(range(9))
        if rnd.random() < 0.6:
            rnd.shuffle(keys)
        counts = [rnd.choice((1, 2, 3, 4, 5, 6, 8, 12, 16, 23)) for _ in range(9)]
        starter = [[1 if rnd.random() < (0.3 if rnd.random() < 0.3 else 0.0) else 0 for _ in range(c)] for c in counts]
        sc = rnd.random() < 0.7
        lo = rnd.getrandbits(32)
        picks = []
        for _ in range(30):
            g = rnd.choice((-1, -2, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, rnd.randrange(0, 9)))
            picks.append((g, int(rnd.random() < 0.4), int(rnd.random() < 0.25), int(rnd.random() < 0.4)))
        out.append((keys, counts, starter, sc, lo, picks))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nat, fs = addr(a.tool, 'F-10331'), addr(a.tool, 'F-11038')
    log = 'sub_' + addr(a.tool, 'F-47121')
    stubs = {log: '', 'sub_' + addr(a.tool, 'F-24338'): 'ctx.r3.u32=0;', 'sub_' + addr(a.tool, 'F-25541'): 'ctx.r3.u32=g_lr;'}
    recomp_closure.EXTRA += 'static uint32_t g_lr;' + chr(10)
    cs = cases()
    nl = chr(10)
    NL = chr(92) + 'n'
    li = lambda v: ','.join(map(str, v))
    rows = []
    for keys, counts, starter, sc, lo, picks in cs:
        st = ','.join('0x%xu' % sum(b << i for i, b in enumerate(s)) for s in starter)
        rows.append('{{%s},{%s},{%s},%d,0x%08xu,{%s}}' % (li(keys), li(counts), st, sc, lo, ','.join('{%d,%d,%d,%d}' % p for p in picks)))
    body = ('struct P{int g,f,c,l;};struct K{int keys[9],counts[9];uint32_t st[9];int sc;uint32_t lo;P p[30];};static const K C[]={' + nl +
            (',' + nl).join(rows) + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'store(0x8286D8F0,0x1000);store(0x828CD4E4+4,0);store(0x828CD4D0+4,0);' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));uint32_t m=0x1000+3908;store(m+12,0x2000);store16(m+16,9);store(m+20,0x2100);*pointer(m+28)=uint8_t(c.sc);'
            'for(int g=0;g<9;g++){store(0x2000+12*g,c.keys[g]);uint32_t b=0x2100+20*g;store(b,0x2300+0x100*g);store(b+8,0);store(b+12,0x3300+0x80*g);'
            'store16(b+4,c.counts[g]);store16(b+6,c.counts[g]);store16(b+16,c.counts[g]);'
            'for(int i=0;i<c.counts[g];i++){uint32_t s=0x4000+0x400*g+32*i;char t[32];snprintf(t,32,(c.st[g]>>i)&1?"x_racestarter_%d_%d":"s_%d_%d",g,i);'
            'int n=strlen(t);for(int j=0;j<=n;j++)*pointer(s+j)=t[j];store(0x2300+0x100*g+8*i,s);store16(0x2300+0x100*g+8*i+4,n);}}'
            'store(' + hex(MWC) + ',c.lo);store(' + hex(MWC + 4) + ',1);'
            'for(int g=0;g<9;g++){Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x2100+20*g;sub_' + fs + '(ctx,nullptr);}'
            'for(int i=0;i<30;i++){Context ctx{};ctx.r1.u32=0x7F00;g_lr=c.p[i].l;ctx.r3.u32=c.p[i].g;ctx.r4.u32=c.p[i].f;ctx.r5.u32=c.p[i].c;sub_' + nat + '(ctx,nullptr);'
            'uint32_t r=ctx.r3.u32;if(r<0x4000||r>=0x6400){printf("%s-",i?",":"");}else{printf("%s%d.%d",i?",":"",(int)((r-0x4000)/0x400),(int)((r-0x4000)%0x400/32));}}'
            'printf(" | %u %u' + NL + '",load(' + hex(MWC) + '),load(' + hex(MWC + 4) + '));}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + nat, 'sub_' + fs], body, a.out_dir, 'prerace_pick', stubs=stubs, scratch_end=0x8000)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cs)
    lines = []
    for (keys, counts, starter, sc, lo, picks), o in zip(cs, out):
        st = ','.join('%x' % sum(b << i for i, b in enumerate(s)) for s in starter)
        lines.append('%s %s %s %d %d %s -> %s' % (li(keys), li(counts), st, sc, lo, ';'.join('%d,%d,%d,%d' % p for p in picks), o))
    # (a line with a word the registry check reads as an address of the original - 8 digits from 8 - is left out)
    lines = [x for x in lines if not re.search(chr(92) + 'b8[0-9A-Fa-f]{7}' + chr(92) + 'b', x)][:3000]
    assert len(lines) == 3000
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'prerace_pick_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')
    # (the race parser's GridType lookup F-53452 on names: as written, in other cases, near misses, unknown ones)
    gt = addr(a.tool, 'F-53452')
    names = ['None', 'none', 'NONE', '2x4_Race_Grid', '2X4_RACE_GRID', '2x4_Split_Race_Grid', '2x4_split_race_grid', '3x3_Race_Grid',
             '4x2_Race_Grid', '4x2_race_grid', '3x3_Split_Race_Grid', '1x8_Race_Grid', '1x8_RACE_grid', '16_Race_Grid', '1x1_Race_Grid',
             'Taunt', 'Safety', '', '2x4_Race_Grid ', ' 2x4_Race_Grid', '2x4_Race_Gri', '2x4_Race_Grids', '3x3', 'NoneX', '4x2_Split_Race_Grid']
    body = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'static const char*N[]={' + ','.join('"%s"' % n for n in names) + '};' + nl +
            'for(auto n:N){memset(objects,0,sizeof(objects));int l=strlen(n);for(int j=0;j<=l;j++)*pointer(0x1000+j)=n[j];'
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x1000;sub_' + gt + '(ctx,nullptr);printf("%d' + NL + '",(int)ctx.r3.s32);}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + gt], body, a.out_dir, 'grid_type', stubs={log: ''}, scratch_end=0x8000)
    got = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(got) == len(names)
    (a.out_dir / 'grid_type_ref.txt').write_text(''.join('%s|%s' % (n, g) + nl for n, g in zip(names, got)))
    print(len(names), 'grid names')


if __name__ == '__main__':
    main()
