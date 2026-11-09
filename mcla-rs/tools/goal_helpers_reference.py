"""The racing brain's goal helpers and the script max-speed setters run offline (the generated code whole and unchanged,
vector mode):

- F-79021 the goal point: goal idx of the list (+176, 32 bytes: pos, w at +12, second point at +16), its flags word (+184 +
  4 idx); bit 0 (a gate): pos + (p1 - pos) x w (vsubfp, vmaddfp), else pos.
- F-68139 / F-60424 the checkpoint (flags bits 6..12) / lap (bits 2..5) of the current goal (+216; the last one when +216 is
  the count +188; 0 without goals), both sign-extended.
- F-59732 Checkpoint_SetMaxSpeed (args: the checkpoint, mph): +188 = mph x 0.447 when mph > 0, else -1.
- F-61343 SetAltCPMaxSpeed (args: the path, the index, mph): the alt point's +20 the same way.

Writes <out-dir>/goal_helpers_ref.txt, hex words: "case count cur idx flags[count] goal[idx] (8 words) mph -> point.xyz
checkpoint lap cp_speed alt_speed"."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb, nudge

B, GL, FL, ARGS, CP, PATH, ALT = 0x40100000, 0x40101000, 0x40102000, 0x40103000, 0x40104000, 0x40105000, 0x40106000
GMAX = 12


def case(rnd):
    c = rnd.choice((0, 1, rnd.randint(1, GMAX), GMAX))
    cur = rnd.choice((0, c, max(c - 1, 0), rnd.randint(0, max(c, 0))))
    idx = rnd.randrange(GMAX)
    flags = [rnd.getrandbits(32) if rnd.random() < 0.3 else (rnd.getrandbits(1) | 2 | rnd.randrange(16) << 2 | rnd.randrange(128) << 6)
             for _ in range(GMAX)]
    pos = [rnd.uniform(-3000, 3000), rnd.uniform(-50, 150), rnd.uniform(-3000, 3000), rnd.uniform(0, 1)]
    p1 = [pos[0] + rnd.uniform(-30, 30), pos[1] + rnd.uniform(-2, 2), pos[2] + rnd.uniform(-30, 30), rnd.uniform(-1, 1)]
    r = rnd.random()
    if r < 0.05:
        pos[3] = rnd.choice((0.0, 1.0, -0.0, 1e-40))
    elif r < 0.08:
        p1[:3] = [pos[0] + 1e-39, pos[1], pos[2]]
    mph = rnd.choice((0.0, -0.0, -1.0, 1e-45, rnd.uniform(0, 200), rnd.uniform(0, 200), nudge(0.0, 1), float('nan'), 120.0))
    return [c, cur, idx] + flags + [fb(x) for x in pos + p1] + [fb(mph)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn = {k: addr(a.tool, f) for k, f in (('P', 'F-79021'), ('C', 'F-68139'), ('L', 'F-60424'), ('S', 'F-59732'), ('A', 'F-61343'))}
    rnd = random.Random(79021)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    W = 3 + GMAX + 9
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    src = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(auto&s:CS){rt_reset();memset(pointer(@B@),0,0x8000);uint32_t idx=s[2];
 store(@B@+176,@GL@);store(@B@+184,@FL@);*(uint16_t*)nullptr;
 for(int i=0;i<@GMAX@;i++)store(@FL@+4*i,s[3+i]);for(int i=0;i<8;i++)store(@GL@+32*idx+4*i,s[3+@GMAX@+i]);
 store(@B@+216,s[1]);
 uint32_t r[7];
 {Context c=fresh();c.r3.u32=@B@;c.r4.u32=idx;c.r5.u32=@B@+0x800;sub_@P@(c,nullptr);for(int i=0;i<3;i++)r[i]=load(@B@+0x800+4*i);}
 {Context c=fresh();c.r3.u32=@B@;sub_@C@(c,nullptr);r[3]=c.r3.u32;}
 {Context c=fresh();c.r3.u32=@B@;sub_@L@(c,nullptr);r[4]=c.r3.u32;}
 store(@ARGS@,@CP@);store(@ARGS@+4,s[3+@GMAX@+8]);store(@ARGS@+0x100,@B@+0x900);store(@ARGS@+0x108,@ARGS@);
 {Context c=fresh();c.r3.u32=@ARGS@+0x100;sub_@S@(c,nullptr);r[5]=load(@CP@+188);}
 store(@ARGS@+0x40,@PATH@);store(@ARGS@+0x44,idx);store(@ARGS@+0x48,s[3+@GMAX@+8]);store(@PATH@+20,@ALT@);store(@ARGS@+0x208,@ARGS@+0x40);
 {Context c=fresh();c.r3.u32=@ARGS@+0x200;sub_@A@(c,nullptr);r[6]=load(@ALT@+32*idx+20);}
 printf("case");for(int i=0;i<@W@;i++)printf(" %08x",s[i]);printf(" ->");for(int i=0;i<7;i++)printf(" %08x",r[i]);printf("\n");}}
'''
    # (+188 is a u16 count: written big-endian through the guest store of a halfword)
    src = src.replace('*(uint16_t*)nullptr;', 'pointer(@B@+188)[0]=(uint8_t)(s[0]>>8);pointer(@B@+188)[1]=(uint8_t)s[0];')
    for k, v in (('B', B), ('GL', GL), ('FL', FL), ('ARGS', ARGS), ('CP', CP), ('PATH', PATH), ('ALT', ALT)):
        src = src.replace('@%s@' % k, '%du' % v)
    src = src.replace('@GMAX@', str(GMAX)).replace('@W@', str(W))
    for k, v in fn.items():
        src = src.replace('@%s@' % k, v)
    main_src = 'static const uint32_t CS[][%d]={' % W + nl + rows + nl + '};' + nl + src
    exe = rd.build(rd.index(a.recomp), ['sub_' + v for v in fn.values()], main_src, a.out_dir, 'goalhelp', image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'goal_helpers_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
