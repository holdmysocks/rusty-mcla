"""The tournament's decision runs offline: F-66541 (calculateResultsTournament's decision, named by --fid), the generated code unchanged; its callees are stubbed and answer from the case:
the script-variable lookup of the global holder (bComplete -> cell 0x1100, bFailure -> cell 0x1200), the
tournament's member lookup (nCurrentRaceIndex, nTotalRaces, nRacerPoints_<i>: a cell whose +4 is the case's word),
the "%s%d" formatter (host snprintf), the event's set-up (an event object at 0x1300 or none) and the event post
(its name logged). The results block at 0x2000 has the racer count at +3080 and the difficulty at +3100.
Cases: 0..16 racers, race index and total 0..6, points 0..30 with ties, difficulty 0..3, no event object, null
arguments. Writes <out-dir>/tournament_decision_ref.txt:
"<null> <event> <count> <index> <total> <difficulty> <points...> -> <complete cell +88,+92,+156> <failure cell ...> <events>"."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases():
    rnd = random.Random(1129)
    out = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 4, 8, 12, 16, rnd.randrange(0, 17)))
        total = rnd.randrange(0, 7)
        idx = rnd.choice((total - 1, total - 1, total, max(total - 2, 0), rnd.randrange(0, 7)))
        hi = rnd.choice((2, 5, 30))
        pts = [rnd.randrange(0, hi + 1) for _ in range(max(n, 1))]
        if rnd.random() < 0.3 and n > 1:
            pts[0] = max(pts)
        diff = rnd.randrange(0, 4)
        null = rnd.choice((0,) * 18 + (1, 2))
        ev = 0 if rnd.random() < 0.05 else 1
        out.append((null, ev, n, idx, total, diff, pts))
    return out


STUBS = {
    # (the global holder's variable lookup: r4 the name)
    'F-52737': 'char s[32];for(int i=0;i<31;i++){s[i]=*pointer(ctx.r4.u32+i);if(!s[i])break;}s[31]=0;'
                    'ctx.r3.u32=!strcmp(s,"bComplete")?0x1100:!strcmp(s,"bFailure")?0x1200:0x1A00;',
    # (the tournament's member lookup: r4 the name; the cells at 0x1600 + 8 * slot, the race index / total at 0x1580 / 0x1588)
    'F-84339': 'char s[32];for(int i=0;i<31;i++){s[i]=*pointer(ctx.r4.u32+i);if(!s[i])break;}s[31]=0;'
                    'if(!strcmp(s,"nCurrentRaceIndex"))ctx.r3.u32=0x1580;else if(!strcmp(s,"nTotalRaces"))ctx.r3.u32=0x1588;'
                    'else if(!strncmp(s,"nRacerPoints_",13))ctx.r3.u32=0x1600+8*atoi(s+13);else{printf("?%s ",s);ctx.r3.u32=0x1A00;}',
    'F-32585': 'char f[16],a[32];for(int i=0;i<15;i++){f[i]=*pointer(ctx.r5.u32+i);if(!f[i])break;}f[15]=0;'
                    'for(int i=0;i<31;i++){a[i]=*pointer(ctx.r6.u32+i);if(!a[i])break;}a[31]=0;'
                    'char o[64];snprintf(o,sizeof o,f,a,(int)ctx.r7.s32);for(int i=0;i<(int)ctx.r4.u32;i++){*pointer(ctx.r3.u32+i)=o[i];if(!o[i])break;}',
    'F-17218': '',
    'F-98988': 'store(ctx.r3.u32+68,g_ev);',
    'F-41407': 'char s[40];for(int i=0;i<39;i++){s[i]=*pointer(ctx.r4.u32+i);if(!s[i])break;}s[39]=0;printf("%s:%u ",s,ctx.r5.u32);',
}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--fid', required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, a.fid)
    cs = cases()
    nl = chr(10)
    recomp_closure.EXTRA += 'static uint32_t g_ev;\n'
    rows = (',' + nl).join('{%d,%d,%d,%d,%d,%d,{%s}}' % (nu, ev, n, i, t, d, ','.join(map(str, p))) for nu, ev, n, i, t, d, p in cs)
    body = ('struct K{int nu,ev,n,i,t,d;int p[16];};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            # (the holders the code reads from 0x8290A18C (lis -32111, -24180; its +10248 is read, so it points into the image's
            # unused tail) and 0x8286D810 (lis -32121, -10224))
            'store(0x8290A18C,0x829D0000);store(0x8286D810,0x1480);' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x8290A18C,0x829D0000);g_ev=c.ev?0x1300:0;'
            'store(0x1580+4,c.i);store(0x1588+4,c.t);for(int k=0;k<16;k++)store(0x1600+8*k+4,c.p[k]);'
            'store(0x2000+3080,c.n);store(0x2000+3100,c.d);'
            '{Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1E00;ctx.r4.u32=c.nu==1?0:0x1500;ctx.r5.u32=c.nu==2?0:0x2000;sub_%s(ctx,nullptr);}' % f +
            'for(uint32_t b:{0x1100u,0x1200u})printf("%u,%u,%u ",load(b+88),(unsigned)load16(b+92),load(b+156));printf("' + chr(92) + 'n");}' + nl + '}' + nl)
    stubs = {'sub_' + addr(a.tool, k): v for k, v in STUBS.items()}
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'tourdec', stubs=stubs)
    out = [x.strip() for x in run(exe, a.image).split(nl)][:len(cs)]
    assert len(out) == len(cs)
    lines = ['%d %d %d %d %d %d %s -> %s' % (nu, ev, n, i, t, d, ','.join(map(str, p)), o) for (nu, ev, n, i, t, d, p), o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'tournament_decision_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
