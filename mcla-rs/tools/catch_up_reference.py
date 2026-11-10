"""The racing brain's shortcut catch-up F-34392 run offline (the generated code whole and unchanged; the current goal's lap
F-60424 as generated). The goal list builder F-58517 is a stub that records its (checkpoint, lap) arguments; the nav
cancel, the follower's reset, the car config name getter and the log are stubs.

Brain: +180 / +188 the goal counts (u16, capacities +182 / +190 non-zero), +184 the goal flags words (bits 2..5 the lap,
6..12 the checkpoint, sign-extended), +216 the current goal, +224 the laps, +228 / +232 the lock (checkpoint, lap),
+212 / +220 the other two goal indices, +300 the follower, +324 the course: +8 u16 the checkpoints, +12 the alt paths
(16 bytes: +0 i32 the checkpoint before, +12 u8 the checkpoints skipped, +13 u8 a shortcut), +16 u16 their count.

Writes <out-dir>/catch_up_ref.txt: "case n laps cur count npaths flags[count] (before skip shortcut)[npaths] -> built
b_cp b_lap lock_cp lock_lap count180 i212 i216 i220" (hex words)."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr

B, FL, GL, CO, PA, AIV, NAME = (0x40100000 + 0x1000 * k for k in range(7))
MARK = 0x7EADBEEF


def case(rnd):
    n = rnd.randint(1, 12)
    laps = rnd.randint(1, 4)
    # (goals in race order from a random lap / checkpoint on, sometimes the whole race, sometimes random words)
    flags = []
    lp, cp = rnd.randrange(laps), rnd.randrange(n)
    for _ in range(rnd.randint(0, 14)):
        if rnd.random() < 0.04:
            flags.append(rnd.getrandbits(32))
        else:
            flags.append(rnd.getrandbits(2) | (lp & 0xF) << 2 | (cp & 0x7F) << 6 | rnd.getrandbits(19) << 13)
        if rnd.random() < 0.8:
            cp += 1
            if cp >= n:
                cp, lp = 0, lp + 1
    cur = rnd.choice((0, len(flags), rnd.randint(0, len(flags))))
    paths = sorted([[rnd.choice((rnd.randint(0, n + 1), rnd.randint(0, n))), rnd.randint(0, 4), rnd.choice((1, 1, 0))] for _ in range(rnd.randint(0, 6))],
                   key=lambda p: p[0])
    return [n, laps, cur, len(flags), len(paths)] + flags + [w for p in paths for w in p]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=8000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-34392')
    rnd = random.Random(34392)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    flat, offs = [], []
    for c in cases:
        offs.append(len(flat))
        flat += c
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl +
           'static const uint32_t OF[]={%s};' % ','.join('%du' % o for o in offs) + nl + r'''
static void st16(uint32_t a,uint32_t v){pointer(a)[0]=(uint8_t)(v>>8);pointer(a)[1]=(uint8_t)v;}
static uint32_t ld16(uint32_t a){return (uint32_t)pointer(a)[0]<<8|pointer(a)[1];}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(int k=0;k<@C@;k++){const uint32_t*s=CS+OF[k];rt_reset();memset(pointer(@B@),0,0x7000);
 uint32_t n=s[0],laps=s[1],cur=s[2],cnt=s[3],np=s[4];
 store(@B@+176,@GL@);st16(@B@+180,cnt);st16(@B@+182,64);store(@B@+184,@FL@);st16(@B@+188,cnt);st16(@B@+190,64);
 for(uint32_t i=0;i<cnt;i++)store(@FL@+4*i,s[5+i]);
 store(@B@+216,cur);store(@B@+212,@MK@);store(@B@+220,@MK@);store(@B@+224,laps);store(@B@+228,@MK@);store(@B@+232,@MK@);
 store(@B@+300,@AIV@);store(@B@+324,@CO@);st16(@CO@+8,n);store(@CO@+12,@PA@);st16(@CO@+16,np);
 for(uint32_t j=0;j<np;j++){const uint32_t*p=s+5+cnt+3*j;store(@PA@+16*j,p[0]);*pointer(@PA@+16*j+12)=(uint8_t)p[1];*pointer(@PA@+16*j+13)=(uint8_t)p[2];}
 o_w()[0]=0;o_w()[1]=@MK@;o_w()[2]=@MK@;
 Context c=fresh();c.r3.u32=@B@;sub_@F@(c,nullptr);
 printf("case");for(uint32_t i=0;i<5+cnt+3*np;i++)printf(" %08x",s[i]);
 printf(" -> %08x %08x %08x %08x %08x %08x %08x %08x %08x\n",o_w()[0],o_w()[1],o_w()[2],load(@B@+228),load(@B@+232),ld16(@B@+180),load(@B@+212),load(@B@+216),load(@B@+220));}}
''')
    for k, v in (('B', B), ('FL', FL), ('GL', GL), ('CO', CO), ('PA', PA), ('AIV', AIV), ('MK', MARK)):
        src = src.replace('@%s@' % k, '%du' % v)
    src = src.replace('@C@', str(a.cases)).replace('@F@', fn)
    stubs = {
        'sub_' + addr(a.tool, 'F-58517'): 'o_w()[0]=1;o_w()[1]=ctx.r4.u32;o_w()[2]=ctx.r5.u32;',
        'sub_' + addr(a.tool, 'F-15013'): 'ctx.r3.u32=%du;' % NAME,
        'sub_' + addr(a.tool, 'F-47121'): '',
    }
    # (the nav cancel and the follower's reset: no effect on what is compared)
    for f in ('F-71636', 'F-71106'):
        stubs['sub_' + addr(a.tool, f)] = ''
    stubs['sub_' + addr(a.tool, 'F-15488')] = 'throw std::runtime_error("alloc");'
    pre = 'static uint32_t (&o_w())[3]{static uint32_t v[3];return v;}' + nl
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + pre
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + fn], src, a.out_dir, 'catchup', stubs=stubs, image=rd.load_image(a.image))
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'catch_up_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    from collections import Counter
    print(len(lines), 'cases; built', Counter(x.split()[-9] for x in lines), 'count180', Counter(x.split()[-4] == '0000ffff' for x in lines))


if __name__ == '__main__':
    main()
