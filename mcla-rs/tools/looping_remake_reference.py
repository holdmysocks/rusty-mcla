"""A Looping's child remake run offline: the original's Looping update F-67073, its deferred child request F-70509 / F-95377,
the bucket's deferred creation F-21229 (after the tree update) and the Looping's +32 method F-87646 with its play F-29395,
the generated code whole and unchanged, on random sequences of updates of 1 - 14 Loopings in one bucket. Stubbed, as inputs
of the case: the child's update F-68220 (still running or ended, drawn per update), the make F-15997 (a new child object in
the bucket's slot table, or none at random), the child's kill F-17136, set offset F-26920, Prepare F-80012 (ready), Play
F-91431 and the stream slot lookup F-20252. The pool global and the frame timer are laid out as the original's (pool +196
slot size, +232 bucket table; bucket +11440 slot base, +11436 queue count; Looping +64 bucket, +65 timer, +72 child slot,
+188 LoopCount, +196 plays begun, +200 child hash; child +6 state 2).
Writes <out-dir>/looping_remake_ref.txt, per update one line: per Looping still in the tree its child's update result
before (1 running, 0 ended, '-' none), then '->' per Looping the update's result, plays begun and whether it holds a child
after the deferred creation, and the request count. '=' ends a sequence; a line starting 'c' gives the Loopings' counts."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static int RUN[64], FAILNEXT, TOG[16];' + chr(10)

BT = 0x40100000      # bucket table (bucket 0)
VT = 0x40180000      # the Loopings' vtable: slot 8 = the Looping's +32
TIMERS = 0x40190000
SLOT = 256
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    upd, ready, create, child_upd, make, kill, setoff, prep, play, slot_lookup = (addr(a.tool, f) for f in (
        'F-67073', 'F-87646', 'F-21229', 'F-68220', 'F-15997', 'F-17136', 'F-26920', 'F-80012', 'F-91431', 'F-20252'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    # the pool global both F-70509 and the Looping update address: lis -32125 / addi 7216
    pool = ((-32125 << 16) + 7216) & 0xffffffff
    sb = BT + 0x4000     # the bucket's slot table
    rnd = random.Random(67073)
    seqs = []
    for _ in range(500):
        k = rnd.choice((1, 2, 3, 5, 8, 12, 13, 14))
        counts = [rnd.choice((-1, -1, 1, 2, 3, 5)) for _ in range(k)]
        n = rnd.randrange(5, 40)
        p_end = rnd.choice((0.1, 0.3, 0.6, 1.0))
        p_fail = rnd.choice((0.0, 0.0, 0.1, 0.4))
        frames = []
        for _ in range(n):
            frames.append(([int(rnd.random() >= p_end) for _ in range(k)], [int(rnd.random() < p_fail) for _ in range(16)]))
        seqs.append((counts, frames))
    nl = chr(10)
    data = []
    for counts, frames in seqs:
        data.append('%d,%d,%s,%s,' % (len(counts), len(frames), ','.join(map(str, counts)),
                                       ','.join(','.join(map(str, r + f)) for r, f in frames)))
    stubs = {
        'sub_' + child_upd: 'ctx.r3.u32=RUN[(ctx.r3.u32-%du)/%d];' % (sb, SLOT),
        'sub_' + make: ('if(FAILNEXT&1){ctx.r3.u32=0;}else{int j=(ctx.r5.u32-%du)/%d;uint32_t c=%du+%d*(32+2*j+TOG[j]);TOG[j]^=1;'
                        'memset(pointer(c),0,%d);store16(c+6,2);ctx.r3.u32=c;}FAILNEXT>>=1;') % (sb, SLOT, sb, SLOT, SLOT),
        'sub_' + kill: '',
        'sub_' + setoff: '',
        'sub_' + prep: 'ctx.r3.u32=1;',
        'sub_' + play: '',
        'sub_' + slot_lookup: 'ctx.r3.u32=0;',
    }
    main_src = (
        'static const int32_t C[]={' + nl + nl.join(data) + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'store(%du+196,%d);store(%du+232,%du);store(%du+32,0x%su);' % (pool, SLOT, pool, BT, VT, ready) +
        'for(size_t i=0;i<sizeof(C)/4;){int k=C[i],n=C[i+1];const int32_t*cnt=C+i+2;const int32_t*w=C+i+2+k;i+=2+k+n*(k+16);'
        'memset(pointer(%du),0,0x4000+%d*64);store(%du+11440,%du);int alive[16];printf("c");' % (BT, SLOT, BT, sb) +
        'for(int j=0;j<k;j++){uint32_t L=%du+%d*j;store(L,%du);*pointer(L+64)=0;*pointer(L+65)=0;' % (sb, SLOT, VT) +
        'memset(pointer(L+72),255,8);store16(L+60,0xffff);store16(L+62,0xffff);store(L+188,cnt[j]);store(L+196,1);store(L+200,0x1000+j);'
        'uint32_t c=%du+%d*(32+2*j);TOG[j]=1;store16(c+6,2);*pointer(L+72)=32+2*j;alive[j]=1;printf(" %%d",cnt[j]);}' % (sb, SLOT) +
        'printf("\\n");float T=1.0f;' +
        'for(int f=0;f<n;f++){const int32_t*r=w+f*(k+16);FAILNEXT=0;for(int b=0;b<16;b++)FAILNEXT|=r[k+b]<<b;T+=1.0f/60;'
        'store(%du+4,*(uint32_t*)&T);int ret[16];' % TIMERS +
        'for(int j=0;j<k;j++){uint32_t L=%du+%d*j;if(!alive[j]){printf("x ");continue;}uint8_t s=*pointer(L+72);' % (sb, SLOT) +
        'if(s==255)printf("- ");else{RUN[s]=r[j];printf("%%d ",r[j]);}'
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=L;ctx.r4.u32=(uint32_t)(int32_t)(T*1000);sub_%s(ctx,nullptr);ret[j]=ctx.r3.u32;}}' % (STACK, upd) +
        'int q=*pointer(%du+11436);' % BT +
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;ctx.r4.u32=%du;ctx.r5.u32=0;sub_%s(ctx,nullptr);}' % (STACK, pool, TIMERS, create) +
        'printf("->");for(int j=0;j<k;j++){uint32_t L=%du+%d*j;if(!alive[j]){printf(" x");continue;}' % (sb, SLOT) +
        'printf(" %d/%d/%d",ret[j],(int)load(L+196),*pointer(L+72)!=255);if(!ret[j])alive[j]=0;}printf(" q%d\\n",q);}'
        'printf("=\\n");}}' + nl)
    exe = rd.build(idx, ['sub_' + upd, 'sub_' + create], main_src, a.out_dir, 'looping_remake', stubs=stubs, image=img,
                   targets=[int(ready, 16)])
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'looping_remake_ref.txt').write_text(out)
    lines = [x for x in out.split(nl) if '->' in x]
    print(len(seqs), 'sequences,', len(lines), 'updates,', sum(int(x.rsplit(' q', 1)[1]) > 12 for x in lines), 'over 12 requests')


if __name__ == '__main__':
    main()
