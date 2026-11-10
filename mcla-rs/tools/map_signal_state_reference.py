"""The traffic signal head's colour choice F-83493 run offline as generated (cameras2-map4), written to
crates/game/src/testdata/map_signal_state_ref.txt for the port's tests. recomp_closure imports tools/memcap.py, so every process of
this script (g++, the harness) runs in a 4 GB job.
The function is run unchanged. Stubbed: the shader-variable setter F-17339 (records the three floats it is handed: the head's
SignalColor, x red, y yellow, z green). Built by hand: the junction (+160 the current arm, +164 the timer, +168 the head list, +172 its
count), the head object (+288 its shader-variable block, +292 a count: the function sets nothing when either is zero) and the ambient
manager the all-green debug byte is read through (the manager pointer in the image's word, byte +0x64234 of it; the manager's
constructor F-24607 and the ambient message handler F-74980 set it; the mode switch gives it 1 in the online mode only).
One line per case: "SIG <junction present> <list length> <index of the head in the list or -1> <current> <timer> <byte> <+288 present> <+292>
-> <calls> <x> <y> <z>" (floats as f32 bits in hex; the colour words are '-' when the function sets nothing).
Usage: map_signal_state_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def hi(x):
    return (x << 16) & 0xFFFFFFFF


COMMON = r'''
static int ncalls;static uint32_t rec[3];
'''

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t J=ARENA_+0x1000,L=ARENA_+0x1100,H=ARENA_+0x1200,B=ARENA_+0x1300,M=ARENA_+0x2000;
 while(in>>k){rt_reset();ncalls=0;rec[0]=rec[1]=rec[2]=0;
  uint32_t jp,n,idx,cur,tb,byt,bp,cnt;int32_t idxs;in>>std::dec>>jp>>n>>idxs>>cur>>std::hex>>tb>>std::dec>>byt>>bp>>cnt;idx=idxs;
  store(MGRPTR_,M);*pointer(M+0x64234)=byt;
  store(J+160,cur);store(J+164,tb);store(J+168,L);store16(J+172,n);
  for(uint32_t i=0;i<n;i++)store(L+4*i,(int32_t)idx>=0&&i==idx?H:H+0x100+0x10*i);
  store(H+292,cnt);store(H+288,bp?B:0);store(B+20,B+0x80);
  Context cx=fresh();cx.r3.u32=jp?J:0;cx.r4.u32=H;sub_SIG(cx,nullptr);
  printf("%d",ncalls);
  if(ncalls)printf(" %08x %08x %08x",rec[0],rec[1],rec[2]);else printf(" - - -");
  printf("\n");
 }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--n', type=int, default=6000)
    a = ap.parse_args()
    if not 0 < a.n <= 100000:
        raise SystemExit('case count out of range')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    a.work.mkdir(parents=True, exist_ok=True)

    def name(fid):
        return 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    rc.EXTRA += COMMON
    stubs = {name('F-17339'): '{ncalls++;rec[0]=load(ctx.r6.u32);rec[1]=load(ctx.r6.u32+4);rec[2]=load(ctx.r6.u32+8);}'}
    mgrptr = hi(-32121) - 10012
    src = MAIN.replace('sub_SIG', name('F-83493')).replace('ARENA_', '0x%08Xu' % rd.ARENA).replace('MGRPTR_', '0x%08Xu' % mgrptr)
    exe = rd.build(idx, [name('F-83493')], src, a.work, 'map_signal_state', image=img, stubs=stubs)
    rnd = random.Random(83493)
    cases = []
    for _ in range(a.n):
        n = rnd.choice([0, 1, 2, 3, 4, 5, 6, 8])
        i = rnd.randrange(n) if n and rnd.random() < 0.85 else -1
        cur = rnd.choice([0, 1, 2, 3, 4, 5, 6, 7, 0xFFFFFFFF, 0x100, i if i >= 0 else 0, i if i >= 0 else 1, i if i >= 0 else 2])
        r = rnd.random()
        t = (rnd.uniform(0, 12) if r < 0.4 else rnd.choice([0.0, 6.4999995, 6.5, 6.5000005, 8.4999995, 8.5, 8.5000005, 10.0, 15.0, -0.5, 1e9])
             if r < 0.8 else rnd.uniform(5.5, 9.5))
        byt = rnd.choice([0, 0, 0, 1, 2, 255])
        jp = 0 if rnd.random() < 0.04 else 1
        bp = 0 if rnd.random() < 0.06 else 1
        cnt = rnd.choice([1, 1, 1, 4, 0, 0xFFFFFFFF])
        cases.append(f'SIG {jp} {n} {i} {cur} {fb(f32(t)):08x} {byt} {bp} {cnt}')
    cf = a.work / 'SIG_cases.txt'
    cf.write_text('\n'.join(cases) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    outs = p.stdout.splitlines()
    assert len(outs) == len(cases), (len(outs), len(cases))
    a.out.write_text('\n'.join(f'{c} -> {o}' for c, o in zip(cases, outs)) + '\n')
    print(len(cases), 'cases')


if __name__ == '__main__':
    main()
