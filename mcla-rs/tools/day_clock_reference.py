"""The light manager's day clock F-64257 run offline on the original's code, the pause object clear.
Auto cases (auto +14545 on, no warp): the hour (+14540) and the frame step (the timer word +8 the clock reads); out: the hour after
and the band rate it stored (+14548).
Warp cases (warp +14552 set, auto off): the hour, the target +14556, the rate +14548 and the unscaled step (the timer word +88);
out: the hour after and the warp byte.
Usage: day_clock_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin] [-n 2000]
Writes <out-dir>/day_clock_ref.txt: "a hour step -> hour rate" and "w hour target rate step -> hour warp" (float bits, hex)."""
import argparse, random, struct, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import memcap
import recomp_dispatch as rd

FID = 'F-64257'


def fw(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('-n', type=int, default=2000)
    a = ap.parse_args()
    memcap.apply()
    work = a.out.resolve()
    work.mkdir(parents=True, exist_ok=True)
    fn = 'sub_%08X' % int(subprocess.run([a.tool, 'registry', 'addr', FID], capture_output=True, text=True, check=True).stdout.split()[1], 16)
    rnd = random.Random(64257)
    edges = [0.0, 4.5, 7.0, 16.0, 19.0, 24.0, 23.999, 23.9999, 12.0]
    steps = [fw(1 / 30), 0x3D088889, fw(1 / 60), fw(0.1), 0, fw(0.5), fw(30.0)]
    bad = lambda *ws: any(('%08x' % w).startswith('82') for w in ws)
    cases = []
    while len(cases) < a.n:
        if rnd.random() < 0.4:
            e = fw(rnd.choice(edges))
            h = (e + rnd.randint(-3, 3)) & 0xFFFFFFFF if e else rnd.choice((0, 1, 0x80000000))
        else:
            h = fw(rnd.uniform(-1.0, 25.0))
        s = rnd.choice(steps) if rnd.random() < 0.7 else fw(rnd.uniform(0, 0.2))
        if not bad(h, s):
            cases.append((h, s))
    warps = []
    while len(warps) < a.n:
        h = fw(rnd.uniform(-0.5, 24.5)) if rnd.random() < 0.8 else fw(rnd.choice(edges))
        t = fw(rnd.choice((17.0, 0.0, 12.0, 23.5, 6.0, rnd.uniform(0, 24), rnd.uniform(0, 24))))
        if rnd.random() < 0.15:
            t = (h + rnd.randint(-2, 2)) & 0xFFFFFFFF
        r = fw(rnd.choice((100.0, 100.0, 1.0, 0.5, rnd.uniform(0, 200))))
        st = rnd.choice(steps) if rnd.random() < 0.7 else fw(rnd.uniform(0, 0.2))
        if not bad(h, t, r, st):
            warps.append((h, t, r, st))
    rows = ',\n'.join('{0,0x%08Xu,0x%08Xu,0,0}' % c for c in cases) + ',\n' + ',\n'.join('{1,0x%08Xu,0x%08Xu,0x%08Xu,0x%08Xu}' % c for c in warps)
    main_src = r'''
static const uint32_t J[][5]={
''' + rows + r'''
};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t M=0x40800000u, P=0x40900000u;
 for(auto&j:J){ rt_reset();
  for(uint32_t o=14520;o<14580;o+=4) store(M+o,0);
  store(P+4,0); store(P+8,0); store(0x828CCEA0u,P); store(0x8288B9F0u,0);
  if(!j[0]){ store(M+14540,j[1]); *pointer(M+14545)=1; *pointer(M+14552)=0; store(0x827D7508u,j[2]); }
  else { store(M+14540,j[1]); store(M+14556,j[2]); store(M+14548,j[3]); *pointer(M+14545)=0; *pointer(M+14552)=1; store(0x827D7558u,j[4]); }
  Context c=fresh(); c.r3.u64=M; FN(c,nullptr);
  if(!j[0]) printf("%08x %08x\n",load(M+14540),load(M+14548)); else printf("%08x %d\n",load(M+14540),*pointer(M+14552)); }
}
'''.replace('FN', fn)
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [fn], main_src, work, 'day_clock', image=img)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = r.stdout.splitlines()
    assert len(out) == len(cases) + len(warps)
    text = ''.join('a %08x %08x -> %s\n' % (h, s, o) for (h, s), o in zip(cases, out))
    text += ''.join('w %08x %08x %08x %08x -> %s\n' % (*c, o) for c, o in zip(warps, out[len(cases):]))
    (work / 'day_clock_ref.txt').write_text(text)
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
