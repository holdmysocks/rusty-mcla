"""A TwinLoop's cross-fade run offline: the original's Init F-73377 (with the curve name hashing F-58929, the fader range
F-94657 and rates F-46548, the fade length F-46637 from the audio generator F-60851), Play F-38194 (the swap time F-63616 /
F-62480) and Update F-52106 (StartCrossfade F-27729, the fader step F-94292, the sine F-72580), the generated code whole
and unchanged, on random TwinLoops (MinSwap / MaxSwap, MinCrossfade / MaxCrossfade, EQUAL_POWER_RISE or LINEAR_FALL, the
generator's seed) played and updated at random times. Stubbed, as inputs of the case: the base Init F-85565 (ready), the
children's make F-15997 (two child objects in the bucket's slot table, state 2: both play throughout), their SetVolume
F-66882 (the level is recorded), set offset F-26920, Play F-91431 and update F-68220 (running); the variable lookup
(vtable slot 5) gives none.
Writes <out-dir>/twinloop_ref.txt (each number led by a letter, so no column reads as an address): per TwinLoop a line 'T swapmin swapmax xmin xmax linear seed_lo seed_carry', then per
call (P = Play, U = Update) the time, then '->' the two children's levels (f32 bits), the next swap time, the target and
the fader's value (bits) and the generator's state after. The first line 'H' gives the original's hashes of EQUAL_POWER_RISE and LINEAR_FALL."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static uint32_t VOL[64];' + chr(10)

BT = 0x40100000
VT = 0x40180000      # the TwinLoop's vtable: slot 5 = the variable lookup (none)
PAY = 0x40190000     # its payload (+148)
CTX = 0x401A0000
SLOT = 256
STACK = 0x40F00000
FAKE_VAR = 0x7F000010


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    init, play, upd, base_init, make, setvol, setoff, cplay, cupd = (addr(a.tool, f) for f in (
        'F-73377', 'F-38194', 'F-52106', 'F-85565', 'F-15997', 'F-66882', 'F-26920', 'F-91431', 'F-68220'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    pool = ((-32125 << 16) + 7216) & 0xffffffff
    rng = ((-32125 << 16) + 26624) & 0xffffffff
    sb = BT + 0x4000
    this = sb
    r = random.Random(38194)
    cases = []
    for _ in range(400):
        lo = r.choice((0, 100, 500, 1000, 2000, 5000))
        sw = (lo, lo + r.choice((0, 1, 50, 1000, 4000, 20000)))
        xl = r.choice((0, 1, 2, 100, 500, 1500))
        xf = (xl, xl + r.choice((0, 1, 200, 2000, 6000)))
        lin = int(r.random() < 0.25)
        seed = (r.getrandbits(32), r.getrandbits(32) % 0x5CDCFAA7)
        t = r.randrange(0, 1 << 31)
        steps = []
        for _ in range(r.randrange(10, 120)):
            t = (t + r.choice((16, 17, 33, 1, 0, r.randrange(0, 3000), r.randrange(0, 40000)))) & 0xffffffff
            steps.append(t)
        cases.append((sw, xf, lin, seed, steps))
    nl = chr(10)
    data = ','.join('%d,%d,%d,%d,%d,%du,%du,%d,%s' % (s[0], s[1], x[0], x[1], l, sd[0], sd[1], len(st), ','.join('%du' % v for v in st))
                    for s, x, l, sd, st in cases)
    stubs = {
        'sub_' + base_init: 'ctx.r3.u32=1;',
        'sub_' + make: 'ctx.r3.u32=%du+%d*(ctx.r4.u32==0x22222222u?33:32);store16(ctx.r3.u32+6,2);' % (sb, SLOT),
        'sub_' + setvol: 'if(ctx.r3.u32){float v=(float)ctx.f1.f64;VOL[(ctx.r3.u32-%du)/%d]=*(uint32_t*)&v;}' % (sb, SLOT),
        'sub_' + setoff: '',
        'sub_' + cplay: '',
        'sub_' + cupd: 'ctx.r3.u32=1;',
    }
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@POOL@u+196,@SLOT@);store(@POOL@u+232,@BT@u);store(@VT@u+20,@FAKE@u);
uint32_t ep=0,lf=0;
{Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@S_EP@u;ctx.r4.u32=0;sub_@HASH@(ctx,nullptr);ep=ctx.r3.u32;}
{Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@S_LF@u;ctx.r4.u32=0;sub_@HASH@(ctx,nullptr);lf=ctx.r3.u32;}
printf("H h%u h%u\n",ep,lf);
for(size_t i=0;i<sizeof(C)/4;){const uint32_t*w=C+i;uint32_t n=w[7];const uint32_t*st=w+8;i+=8+n;
 printf("T s%u s%u x%u x%u c%u g%u g%u\n",w[0],w[1],w[2],w[3],w[4],w[5],w[6]);
 memset(pointer(@BT@u),0,0x8000);store(@BT@u+11440,@SB@u);memset(pointer(@PAY@u),0,64);
 store16(@PAY@u+0,w[0]);store16(@PAY@u+2,w[1]);store16(@PAY@u+4,w[2]);store16(@PAY@u+6,w[3]);store(@PAY@u+8,w[4]?lf:ep);
 store(@PAY@u+29,0x11111111u);store(@PAY@u+37,0x22222222u);
 uint32_t T=@SB@u;store(T,@VT@u);*pointer(T+64)=0;store(T+148,@PAY@u);*pointer(T+232)=0x40;
 store(@RNG@u,w[5]);store(@RNG@u+4,w[6]);
 {Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=T;ctx.r4.u32=0;ctx.r5.u32=0;ctx.r6.u32=@CTX@u;sub_@INIT@(ctx,nullptr);
  if(ctx.r3.u32!=1){printf("init %u\n=\n",ctx.r3.u32);continue;}}
 for(uint32_t k=0;k<n;k++){Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=T;ctx.r4.u32=st[k];VOL[32]=VOL[33]=0xFFFFFFFFu;
  if(k==0)sub_@PLAY@(ctx,nullptr);else sub_@UPD@(ctx,nullptr);
  printf("%c t%u -> l%08x l%08x s%u r%08x f%08x g%u g%u\n",k?'U':'P',st[k],VOL[32],VOL[33],load(T+164),load(T+200),load(T+224),load(@RNG@u),load(@RNG@u+4));}
 printf("=\n");}}
"""
    hash_fn = addr(a.tool, 'F-58929')
    for k, v in (('DATA', data), ('POOL', pool), ('SLOT', SLOT), ('BT', BT), ('VT', VT), ('FAKE', FAKE_VAR), ('STACK', STACK),
                 ('S_EP', 0x82000000 + 26448), ('S_LF', 0x82000000 + 26436), ('HASH', hash_fn), ('SB', sb), ('PAY', PAY),
                 ('RNG', rng), ('CTX', CTX), ('INIT', init), ('PLAY', play), ('UPD', upd)):
        c = c.replace('@%s@' % k, str(v))
    main_src = c
    exe = rd.build(idx, ['sub_' + init, 'sub_' + play, 'sub_' + upd, 'sub_' + hash_fn], main_src, a.out_dir, 'twinloop',
                   stubs=stubs, image=img, fakes={FAKE_VAR: 'ctx.r3.u32=0;'})
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'twinloop_ref.txt').write_text(out)
    lines = out.split(nl)
    print(len(cases), 'TwinLoops,', sum(l.startswith(('P ', 'U ')) for l in lines), 'calls,', sum(l.startswith('init') for l in lines), 'init failures')


if __name__ == '__main__':
    main()
