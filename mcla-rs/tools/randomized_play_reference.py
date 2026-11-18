"""A Randomized sound's Play F-15148 run offline: the generated code whole and unchanged, with the audio rand (the shared
state pair 0x82836800 / +4, chained), the common Play F-85565 stubbed to go on (returns 1) and the child factory F-15997
stubbed to hand back a pooled object (its r4, the chosen child's hash, recorded).
The object: +112 the flag word (pair 5 = 1: step mode), +148 the payload (+0 step variable 0, +4 the index, +5 the history
length, +6.. the history, then the count and the 8-byte entries (hash, weight)), +64 bucket 0; +72 = the child's pool index.
Inputs: every Randomized payload of the disc's sounds.dat (the history length <= 16, 1 - 64 children) in weighted and in step
mode, each played three times in a row on the same payload (index and history carried), from the disc's start (index 0,
history 0xFF) and from a random index / history.
Writes <out-dir>/randomized_play_ref.txt: 'R <step 0/1> S<lo> S<carry> i<index> h<history length> n<count> x<history> ...
w<weight bits> ... -> r<r3> c<child number> i<index> x<history> ... S<lo> S<carry>' (hex)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run
from sound_pick_reference import disc_payloads

OBJ = 0x40100000
PAY = 0x40110000
POOLS = 0x82831C30
TABLE = 0x40200000
BASE = 0x40300000
SIZE = 0x100
STACK = 0x40F00000
STATE = 0x82836800
MAXN = 64
recomp_closure.EXTRA += 'static uint32_t made = 0;' + chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--sounds', type=Path, required=True, help="the disc's audio/X360/config/sounds.dat")
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, common, factory = (addr(a.tool, k) for k in ('F-15148', 'F-85565', 'F-15997'))
    r = random.Random(15148)
    cases = []
    for nh, w in [c for c in disc_payloads(a.sounds) if 0 < len(c[1]) <= MAXN and c[0] <= 16]:
        n = len(w)
        for step in (0, 1):
            cases.append((step, 0, nh, w, [0xFF] * nh))
            lim = n if step else max(nh, 1)
            cases.append((step, r.randrange(lim) if (step or nh) else 0, nh, w, [r.choice((0xFF, r.randrange(n))) for _ in range(nh)]))
    rows = []
    for step, idx, nh, w, hist in cases:
        rows.append([step, idx, nh, len(w)] + hist + [0] * (16 - nh) + w + [0] * (MAXN - len(w)))
    W = 4 + 16 + MAXN
    data = ','.join(','.join('%du' % x for x in row) for row in rows)
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@STATE@u,0x12345678u);store(@STATE@u+4,0x0badf00du);
store(@POOLS@u+196,@SIZE@u);store(@POOLS@u+232,@TABLE@u);store(@TABLE@u+11440u,@BASE@u);
for(size_t i=0;i<sizeof(C)/4;i+=@W@){const uint32_t*w=C+i;uint32_t step=w[0],nh=w[2],n=w[3];
 memset(pointer(@PAY@u),0,0x400);
 pointer(@PAY@u+4)[0]=(uint8_t)w[1];pointer(@PAY@u+5)[0]=(uint8_t)nh;for(uint32_t k=0;k<nh;k++)pointer(@PAY@u+6+k)[0]=(uint8_t)w[4+k];
 pointer(@PAY@u+6+nh)[0]=(uint8_t)n;
 for(uint32_t k=0;k<n;k++){store(@PAY@u+7+nh+8*k,0x1000u+k);store(@PAY@u+7+nh+8*k+4,w[20+k]);}
 for(int play=0;play<3;play++){
  memset(pointer(@OBJ@u),0,0x100);store(@OBJ@u+112,step?0x400u:0x800u);store(@OBJ@u+148,@PAY@u);
  uint32_t s0=load(@STATE@u),s1=load(@STATE@u+4);
  printf("R %x S%08x S%08x i%x h%x n%x",step,s0,s1,pointer(@PAY@u+4)[0],nh,n);
  for(uint32_t k=0;k<nh;k++)printf(" x%02x",pointer(@PAY@u+6+k)[0]);
  for(uint32_t k=0;k<n;k++)printf(" w%08x",w[20+k]);
  made=0;Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@OBJ@u;ctx.r4.u32=0;ctx.r5.u32=0;ctx.r6.u32=0;sub_@FN@(ctx,nullptr);
  printf(" -> r%x c%x i%x",ctx.r3.u32,made-0x1000u,pointer(@PAY@u+4)[0]);
  for(uint32_t k=0;k<nh;k++)printf(" x%02x",pointer(@PAY@u+6+k)[0]);
  printf(" S%08x S%08x\n",load(@STATE@u),load(@STATE@u+4));
  if(pointer(@OBJ@u+72)[0]!=7)printf("BAD pool index %d\n",pointer(@OBJ@u+72)[0]);}}}
"""
    stubs = {'sub_' + common: 'ctx.r3.u32=1;',
             'sub_' + factory: 'made=ctx.r4.u32;memset(pointer(%du),0x11,%d);ctx.r3.u32=%du;' % (BASE + 7 * SIZE, SIZE, BASE + 7 * SIZE)}
    for k, v in (('DATA', data), ('OBJ', OBJ), ('PAY', PAY), ('POOLS', POOLS), ('TABLE', TABLE), ('BASE', BASE), ('SIZE', SIZE),
                 ('STACK', STACK), ('STATE', STATE), ('FN', fn), ('W', W)):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(index(a.recomp), ['sub_' + fn], c, a.out_dir, 'randomized_play', stubs=stubs, image=img)
    lines = [x for x in run(exe, a.image).split(chr(10)) if x]
    assert not any(x.startswith('BAD') for x in lines), [x for x in lines if x.startswith('BAD')][:3]
    assert len(lines) == 3 * len(cases), (len(lines), len(cases))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'randomized_play_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(lines), 'plays,', len(cases), 'payload cases')


if __name__ == '__main__':
    main()
