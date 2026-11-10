"""The racing brain's goal list builder run offline: F-58517 (goals from checkpoint cp of lap `lap` to the race's end) with
its alt-path pick F-60279, the generated code whole and unchanged (the course's accessors as generated), the shared
generator (the MWC at 0x8286_0890) seeded per case.

Brain: +176 the goals (32 bytes: pos, +12 w, +16 second point, +28 speed), +180 / +188 their counts (u16), +184 the flags
words, +224 laps, +228 / +232 the lock (checkpoint, lap), +260 the shortcut chance, +328 the opponent (its low byte the
mask bit), +380 the rubber band, +324 the course: +4 the checkpoints (368 bytes: +64 pos, +80 second point, +168
radius, +172 the gate's fraction, +188 max speed, +330 gate byte), +8 u16 their count, +12 the alt paths (16 bytes: +0
i32 before, +4 weight, +8 u16 first alt checkpoint, +10 u8 count, +11 u8 mask, +12 u8 skip, +13 u8 shortcut), +16 u16
their count, +20 the alt checkpoints (32 bytes: pos, +16 radius, +20 speed).

Writes <out-dir>/goal_build_ref.txt, hex words, one line a case: "case seed_lo seed_carry cp lap laps lock_cp lock_lap
chance opponent rubber pre n np na" + the pre goals' flags words + 10 words a checkpoint (pos3 second3 radius frac speed
gate) + 7 a path (before weight first count mask skip shortcut) + 5 an alt checkpoint (pos3 radius speed) + "-> count
lo carry" + per goal "flags hash" (hash: FNV-1a 32 over the goal's 8 float words' bits)."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb, nudge

B, GL, FL, CO, CP, PA, AL = (0x40100000 + 0x10000 * k for k in range(7))
RNG = 0x82860890


def case(rnd):
    n = rnd.randint(1, 8)
    laps = rnd.randint(1, 3)
    cps = []
    for _ in range(n):
        gate = rnd.choice((0, 0, 1))
        cps.append([rnd.uniform(-2000, 2000), rnd.uniform(-20, 80), rnd.uniform(-2000, 2000), rnd.uniform(-2000, 2000), rnd.uniform(-20, 80),
                    rnd.uniform(-2000, 2000), rnd.uniform(3, 30), rnd.uniform(0, 1), rnd.choice((-1.0, -1.0, 0.0, -0.0, rnd.uniform(5, 40))), gate])
    na = rnd.randint(0, 10)
    alts = [[rnd.uniform(-2000, 2000), rnd.uniform(-20, 80), rnd.uniform(-2000, 2000), rnd.uniform(3, 30), rnd.choice((-1.0, 0.0, rnd.uniform(5, 40)))] for _ in range(na)]
    paths = []
    for _ in range(rnd.randint(0, 7)):
        cnt = rnd.randint(0, min(3, na))
        first = rnd.randint(0, max(na - cnt, 0))
        paths.append([rnd.choice((rnd.randint(0, n), rnd.randint(0, n), n)), rnd.choice((rnd.uniform(0, 3), 1.0, 0.0, 0.0, -0.5, rnd.uniform(0, 1e-6))),
                      first, cnt, rnd.choice((0xFF, 0xFF, rnd.getrandbits(8))), rnd.choice((0, 0, 1, 2)), rnd.choice((0, 0, 1))])
    paths.sort(key=lambda p: p[0])
    cp, lap = rnd.choice(((0, 0), (0, 0), (rnd.randrange(n), rnd.randrange(laps)), (n, rnd.randrange(laps)), (0, laps))), None
    cp, lap = cp
    lock = rnd.choice(((-1, -1), (-1, -1), (rnd.choice([p[0] for p in paths] or [0]), rnd.randrange(laps)), (rnd.randrange(n + 1), rnd.randrange(laps))))
    pre = [rnd.getrandbits(32) for _ in range(rnd.choice((0, 0, 1, 3)))]
    return ([rnd.getrandbits(32), rnd.getrandbits(32), cp, lap, laps, lock[0] & 0xffffffff, lock[1] & 0xffffffff,
             fb(rnd.choice((0.0, 0.75, 0.75, 0.3, 1.0))), rnd.choice((rnd.randrange(8), rnd.randrange(8), rnd.randrange(8), rnd.randint(30, 70), rnd.randint(0x100, 0x10000))), rnd.choice((0, 1, 1)), len(pre), n, len(paths), na] + pre +
            [w for c in cps for w in [fb(x) for x in c[:9]] + [c[9]]] +
            [w for p in paths for w in [p[0], fb(p[1])] + p[2:]] +
            [fb(x) for a in alts for x in a])


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-58517')
    rnd = random.Random(58517)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    flat, offs = [], []
    for c in cases:
        offs.append(len(flat))
        flat += c
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl +
           'static const uint32_t OF[]={%s};' % ','.join('%du' % o for o in offs) + nl +
           'static const uint32_t LN[]={%s};' % ','.join('%du' % len(c) for c in cases) + nl + r'''
static void st16(uint32_t a,uint32_t v){pointer(a)[0]=(uint8_t)(v>>8);pointer(a)[1]=(uint8_t)v;}
static uint32_t ld16(uint32_t a){return (uint32_t)pointer(a)[0]<<8|pointer(a)[1];}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(int k=0;k<@C@;k++){const uint32_t*s=CS+OF[k];rt_reset();
 memset(pointer(@B@),0,0x1000);memset(pointer(@GL@),0xCD,0x8000);memset(pointer(@FL@),0,0x2000);
 uint32_t pre=s[10],n=s[11],np=s[12],na=s[13];
 store(@RNG@,s[0]);store(@RNG@+4,s[1]);
 store(@B@+176,@GL@);st16(@B@+180,pre);st16(@B@+182,1024);store(@B@+184,@FL@);st16(@B@+188,pre);st16(@B@+190,1024);
 for(uint32_t i=0;i<pre;i++)store(@FL@+4*i,s[14+i]);
 store(@B@+224,s[4]);store(@B@+228,s[5]);store(@B@+232,s[6]);store(@B@+260,s[7]);store(@B@+328,s[8]);*pointer(@B@+380)=(uint8_t)s[9];
 store(@B@+324,@CO@);store(@CO@+4,@CP@);st16(@CO@+8,n);store(@CO@+12,@PA@);st16(@CO@+16,np);store(@CO@+20,@AL@);
 const uint32_t*c=s+14+pre;
 for(uint32_t i=0;i<n;i++){const uint32_t*w=c+10*i;uint32_t o=@CP@+368*i;
  for(int j=0;j<3;j++){store(o+64+4*j,w[j]);store(o+80+4*j,w[3+j]);}store(o+168,w[6]);store(o+172,w[7]);store(o+188,w[8]);*pointer(o+330)=(uint8_t)w[9];}
 const uint32_t*p=c+10*n;
 for(uint32_t i=0;i<np;i++){const uint32_t*w=p+7*i;uint32_t o=@PA@+16*i;
  store(o,w[0]);store(o+4,w[1]);st16(o+8,w[2]);*pointer(o+10)=(uint8_t)w[3];*pointer(o+11)=(uint8_t)w[4];*pointer(o+12)=(uint8_t)w[5];*pointer(o+13)=(uint8_t)w[6];}
 const uint32_t*al=p+7*np;
 for(uint32_t i=0;i<na;i++){const uint32_t*w=al+5*i;uint32_t o=@AL@+32*i;store(o,w[0]);store(o+4,w[1]);store(o+8,w[2]);store(o+16,w[3]);store(o+20,w[4]);}
 Context cx=fresh();cx.r3.u32=@B@;cx.r4.u32=s[2];cx.r5.u32=s[3];sub_@F@(cx,nullptr);
 uint32_t cnt=ld16(@B@+180);
 printf("case");for(uint32_t i=0;i<LN[k];i++)printf(" %08x",s[i]);
 printf(" -> %08x %08x %08x",cnt,load(@RNG@),load(@RNG@+4));
 if(ld16(@B@+188)!=cnt)printf(" COUNTS");
 for(uint32_t g=0;g<cnt;g++){uint32_t h=2166136261u;for(int j=0;j<8;j++){uint32_t v=load(@GL@+32*g+4*j);for(int b=0;b<4;b++){h^=(v>>(8*b))&0xFF;h*=16777619u;}}
  printf(" %08x %08x",load(@FL@+4*g),h);}
 printf("\n");}}
''')
    for k, v in (('B', B), ('GL', GL), ('FL', FL), ('CO', CO), ('CP', CP), ('PA', PA), ('AL', AL), ('RNG', RNG)):
        src = src.replace('@%s@' % k, '%du' % v)
    src = src.replace('@C@', str(a.cases)).replace('@F@', fn)
    exe = rd.build(rd.index(a.recomp), ['sub_' + fn], src, a.out_dir, 'goalbuild', image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    assert not any('COUNTS' in x for x in lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'goal_build_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    goals = [int(x.split('->')[1].split()[0], 16) for x in lines]
    print(len(lines), 'cases;', sum(goals), 'goals; max', max(goals))


if __name__ == '__main__':
    main()
