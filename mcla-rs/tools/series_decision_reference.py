"""The series' decision runs offline: F-63112, the generated code unchanged; its callees are stubbed and answer from the
case: the global holder's variable lookup F-27291 (bComplete -> cell 0x1100, bFailure -> cell 0x1200), the series'
member lookup F-84339 (nRacesToWin, nRacerWins_<i>: a cell whose +4 is the case's word) and the "%s%d" formatter (host
snprintf). The results block at 0x2000 has the racer count at +3080 and 192-byte records from +8 (the type at +16).
Cells start as garbage (0x55 words) to see the clear. Cases: 0..16 racers, any wins and target, one, two or no
player, null arguments. Writes <out-dir>/series_decision_ref.txt:
"<null> <count> <need> <types> <wins> -> <complete cell +88,+92,+156> <failure cell ...>"."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases():
    rnd = random.Random(63112)
    out = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 4, 8, 16, rnd.randrange(0, 17)))
        need = rnd.choice((1, 2, 3, 3, 0, -1, rnd.randrange(0, 6)))
        types = [rnd.choice((1, 2, 2)) for _ in range(n)]
        if n and rnd.random() < 0.85:
            types[rnd.randrange(n)] = 0
        wins = [rnd.choice((0, 0, 1, 2, rnd.randrange(-2, 7))) for _ in range(max(n, 1))]
        null = rnd.choice((0,) * 18 + (1, 2))
        out.append((null, n, need, types, wins))
    return out


STUBS = {
    'F-27291': 'char s[32];for(int i=0;i<31;i++){s[i]=*pointer(ctx.r4.u32+i);if(!s[i])break;}s[31]=0;'
               'ctx.r3.u32=!strcmp(s,"bComplete")?0x1100:!strcmp(s,"bFailure")?0x1200:0x1A00;',
    'F-84339': 'char s[32];for(int i=0;i<31;i++){s[i]=*pointer(ctx.r4.u32+i);if(!s[i])break;}s[31]=0;'
               'if(!strcmp(s,"nRacesToWin"))ctx.r3.u32=0x1580;'
               'else if(!strncmp(s,"nRacerWins_",11))ctx.r3.u32=0x1600+8*atoi(s+11);else{printf("?%s ",s);ctx.r3.u32=0x1A00;}',
    'F-32585': 'char f[16],a[32];for(int i=0;i<15;i++){f[i]=*pointer(ctx.r5.u32+i);if(!f[i])break;}f[15]=0;'
               'for(int i=0;i<31;i++){a[i]=*pointer(ctx.r6.u32+i);if(!a[i])break;}a[31]=0;'
               'char o[64];snprintf(o,sizeof o,f,a,(int)ctx.r7.s32);for(int i=0;i<(int)ctx.r4.u32;i++){*pointer(ctx.r3.u32+i)=o[i];if(!o[i])break;}',
}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-63112')
    cs = cases()
    nl = chr(10)
    rows = (',' + nl).join('{%d,%d,%d,{%s},{%s}}' % (nu, n, nd, ','.join(map(str, t)) or '0', ','.join(map(str, w))) for nu, n, nd, t, w in cs)
    body = ('struct K{int nu,n,nd;int t[16];int w[16];};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));'
            'for(uint32_t b:{0x1100u,0x1200u}){store(b+88,0x55555555);store(b+92,0x55555555);store(b+156,0x55555555);}'
            'store(0x1580+4,c.nd);for(int k=0;k<16;k++)store(0x1600+8*k+4,c.w[k]);'
            'store(0x2000+3080,c.n);for(int i=0;i<c.n;i++)store(0x2000+16+192*i,c.t[i]);'
            '{Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1E00;ctx.r4.u32=c.nu==1?0:0x1500;ctx.r5.u32=c.nu==2?0:0x2000;sub_%s(ctx,nullptr);}' % f +
            'for(uint32_t b:{0x1100u,0x1200u})printf("%u,%u,%u ",load(b+88),(unsigned)load16(b+92),load(b+156));printf("' + chr(92) + 'n");}' + nl + '}' + nl)
    stubs = {'sub_' + addr(a.tool, k): v for k, v in STUBS.items()}
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'serdec', stubs=stubs)
    out = [x.strip() for x in run(exe, a.image).split(nl)][:len(cs)]
    assert len(out) == len(cs)
    lines = ['%d %d %d %s %s -> %s' % (nu, n, nd, ','.join(map(str, t)) or '-', ','.join(map(str, w)), o) for (nu, n, nd, t, w), o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'series_decision_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
