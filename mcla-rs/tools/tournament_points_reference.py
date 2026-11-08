"""calculateResultsTournament's points run offline: F-31368, the generated code unchanged with the player put last
F-87120 inside it; the other callees are stubbed and answer from the case: the tournament lookup F-77906 (an object or
none), the current race F-27863 (the results block at 0x2000 = race + 8; +3144 the online word, +3080 the racer count,
192-byte racers from +8: the racer word, +8 the type (0 = the player), +164 the place), the UI registry lookup F-64376
(raceOverTrigger: +88 set or not; raceOverCommand: the text at +92, "replay" or not), the damaged-out test F-98898
(per racer), the "%s%d" formatter (host snprintf), the tournament's member lookup F-84339 (nRacerPoints_<i> cells at
0x1600 + 8 i), the points table (the career manager's word at 0x8286D8DC: +4 * (93 + k), k 0..16) and the three
calls after (the estimate F-23843, the decision F-66541, the results fill F-26722: logged). Writes
<out-dir>/tournament_points_ref.txt: "<case words> -> <places> | <cells +0,+4> | <calls>"."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases():
    rnd = random.Random(31368)
    out = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 4, 8, 16, rnd.randrange(0, 17)))
        types = [rnd.choice((1, 1, 2)) for _ in range(n)]
        if n and rnd.random() < 0.9:
            types[0 if rnd.random() < 0.7 else rnd.randrange(n)] = 0
        if rnd.random() < 0.7:
            places = list(range(1, n + 1))
            rnd.shuffle(places)
        else:
            places = [rnd.choice((rnd.randrange(-3, 22), -2147483648, 2147483647)) for _ in range(n)]
        dmg = [1 if rnd.random() < 0.15 else 0 for _ in range(n)]
        pts = [rnd.randrange(0, 40) for _ in range(n)]
        table = [7, 4, 2, 1] + [0] * 13 if rnd.random() < 0.5 else [rnd.randrange(-5, 30) for _ in range(17)]
        null = rnd.choice((0,) * 16 + (1, 2, 3))   # 1 no tournament, 2 no race, 3 online word set
        trig = rnd.choice((0, 1, 1, 2))            # 0 none, 1 set, 2 present but +88 clear
        replay = rnd.choice((0, 1))
        out.append((null, trig, replay, n, types, places, dmg, pts, table))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-31368')
    cs = cases()
    nl = chr(10)
    name = 'char s[40];for(int i=0;i<39;i++){s[i]=*pointer(ctx.r4.u32+i);if(!s[i])break;}s[39]=0;'
    stubs = {
        'F-77906': 'ctx.r3.u32=g_c->nu==1?0:0x1500;',
        'F-27863': 'ctx.r3.u32=g_c->nu==2?0:0x1FF8;',
        'F-64376': name + 'if(!strcmp(s,"raceOverTrigger"))ctx.r3.u32=g_c->tr?0x1100:0;else if(!strcmp(s,"raceOverCommand"))ctx.r3.u32=0x1200;else{printf("?%s ",s);ctx.r3.u32=0;}',
        'F-98898': 'ctx.r3.u32=g_c->dm[ctx.r3.u32-1];',
        'F-32585': 'char f[16],a[32];for(int i=0;i<15;i++){f[i]=*pointer(ctx.r5.u32+i);if(!f[i])break;}f[15]=0;'
                   'for(int i=0;i<31;i++){a[i]=*pointer(ctx.r6.u32+i);if(!a[i])break;}a[31]=0;'
                   'char o[64];snprintf(o,sizeof o,f,a,(int)ctx.r7.s32);for(int i=0;i<(int)ctx.r4.u32;i++){*pointer(ctx.r3.u32+i)=o[i];if(!o[i])break;}',
        'F-84339': name + 'if(!strncmp(s,"nRacerPoints_",13))ctx.r3.u32=0x1600+8*atoi(s+13);else{printf("?%s ",s);ctx.r3.u32=0x1A00;}',
        'F-23843': 'printf("est ");',
        'F-66541': 'printf("dec ");',
        'F-26722': 'printf("fill ");',
    }
    stubs = {'sub_' + addr(a.tool, k): v for k, v in stubs.items()}
    recomp_closure.EXTRA += 'struct K{int nu,tr,re,n;int t[16],p[16],dm[16],pt[16],tb[17];};static const K*g_c;\n'
    li = lambda v: ','.join(map(str, v)) or '0'
    rows = (',' + nl).join('{%d,%d,%d,%d,{%s},{%s},{%s},{%s},{%s}}' % (nu, tr, re, n, li(t), ','.join('(int)(%dLL)' % x for x in p) or '0', li(d), li(pt), li(tb))
                           for nu, tr, re, n, t, p, d, pt, tb in cs)
    body = ('static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            # (the points table's holder, the career manager's word at 0x8286D8DC (lis -32121, -10020); the holder
            # at 0x8290A18C (lis -32111, -24180) is only passed to the stubbed lookup)
            'store(0x8286D8DC,0x1700);store(0x8290A18C,0);' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));g_c=&c;'
            'for(int k=0;k<17;k++)store(0x1700+4*(93+k),c.tb[k]);for(int k=0;k<16;k++)store(0x1600+8*k+4,c.pt[k]);'
            'store(0x1100+88,c.tr==1);const char*cm=c.re?"replay":"next";for(int i=0;i<=(int)strlen(cm);i++)*pointer(0x1200+92+i)=cm[i];'
            'store(0x2000+3144,c.nu==3);store(0x2000+3080,c.n);'
            'for(int i=0;i<c.n;i++){store(0x2000+8+192*i,i+1);store(0x2000+16+192*i,c.t[i]);store(0x2000+172+192*i,c.p[i]);}'
            '{Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1E00;sub_%s(ctx,nullptr);}' % f +
            'printf("| ");for(int i=0;i<c.n;i++)printf("%d ",(int)load(0x2000+172+192*i));printf("| ");'
            'for(int i=0;i<c.n;i++)printf("%u,%d ",load(0x1600+8*i),(int)load(0x1600+8*i+4));printf("' + chr(92) + 'n");}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'tourpts', stubs=stubs)
    out = [x.strip() for x in run(exe, a.image).split(nl)][:len(cs)]
    assert len(out) == len(cs)
    lines = ['%d %d %d %d %s %s %s %s %s -> %s' % (nu, tr, re, n, li(t), li(p), li(d), li(pt), li(tb), o)
             for (nu, tr, re, n, t, p, d, pt, tb), o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'tournament_points_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
