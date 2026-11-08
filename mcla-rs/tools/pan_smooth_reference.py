"""The rider's pan-sweep overlay arithmetic run offline on the original's code (generated code unchanged): the smoother F-36491
(r3 the smoother: +0 centre, +4 rate, +8 hold, +12 value, +16 timer; f1 the target, f2 dt), the target F-66750 (r3 the context,
its +120 the look) and the pan clip's time F-47264 (r3 the anim player: its +76 slot -> +8 the node, type 4 at +6; the node's
player F-77198 is stubbed to hand back a player whose +24 is the clip with its duration at +12; f1 the phase; the time is the
player's +4).
Usage: pan_smooth_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin] [--n 3000]
Writes <out-dir>/pan_smooth_ref.txt, per case (hex words): "value timer target dt look duration phase -> value timer target time"
with the smoother at the constructor's centre 0.5, rate 5, hold 0.5."""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import recomp_dispatch as rd

SM, CTX, AP, SLOT, NODE, PL, CLIP = 0x40800000, 0x40810000, 0x40820000, 0x40821000, 0x40822000, 0x40823000, 0x40824000


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


def f2w(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


MAIN = r"""
static float F(uint32_t w){float f; memcpy(&f,&w,4); return f;}
static uint32_t W(double d){float f=(float)d; uint32_t w; memcpy(&w,&f,4); return w;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); uint32_t v,t,g,dt,lk,du,ph;
 for(;;){
  if(!(in>>std::hex>>v>>t>>g>>dt>>lk>>du>>ph)) return 0;
  rt_reset();
  store(SM_,W(0.5)); store(SM_+4,W(5.0)); store(SM_+8,W(0.5)); store(SM_+12,v); store(SM_+16,t);
  {Context c=fresh(); c.r3.u64=SM_; c.f1.f64=F(g); c.f2.f64=F(dt); SMOOTH(c,nullptr);}
  store(CTX_+120,lk);
  Context c2=fresh(); c2.r3.u64=CTX_; TARGET(c2,nullptr);
  store(AP_+76,SLOT_); store(SLOT_+8,NODE_); store(NODE_+4,0x00000004u); store(PL_+24,CLIP_); store(CLIP_+12,du);
  {Context c=fresh(); c.r3.u64=AP_; c.f1.f64=F(ph); TIME(c,nullptr);}
  printf("%08x %08x %08x %08x\n",load(SM_+12),load(SM_+16),W(c2.f1.f64),load(PL_+4)); }
}
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('--n', type=int, default=3000)
    a = ap.parse_args()
    work = a.out.resolve(); work.mkdir(parents=True, exist_ok=True)
    fn = {k: 'sub_%08X' % addr(a.tool, k) for k in ('F-36491', 'F-66750', 'F-47264', 'F-77198')}
    src = MAIN.replace('SMOOTH', fn['F-36491']).replace('TARGET', fn['F-66750']).replace('TIME', fn['F-47264'])
    for k, v in (('SM_', SM), ('CTX_', CTX), ('AP_', AP), ('SLOT_', SLOT), ('NODE_', NODE), ('PL_', PL), ('CLIP_', CLIP)):
        src = src.replace(k, '0x%08Xu' % v)
    stubs = {fn['F-77198']: f'ctx.r3.u64={PL:#x}u;'}
    idx = rd.index(a.recomp); img = rd.load_image(a.image)
    exe = rd.build(idx, [fn['F-36491'], fn['F-66750'], fn['F-47264']], src, work, 'pan_smooth', stubs=stubs, image=img)
    rnd = random.Random(20261107)

    def near(c, spread):
        return c + rnd.choice((0.0, 0.0049, -0.0049, 0.005, -0.005, 0.0051, -0.0051, 1e-7, rnd.uniform(-spread, spread)))
    cases = []
    while len(cases) < a.n:
        v = near(0.5, 0.6) if rnd.random() < 0.7 else rnd.uniform(-1, 2)
        t = rnd.choice((-1.0, 0.5, 0.0, 1e-3, -1e-3, rnd.uniform(-0.2, 0.6), 1 / 30, 1 / 60))
        g = near(0.5, 0.6)
        dt = rnd.choice((1 / 30, 1 / 60, rnd.uniform(0, 0.1), 0.0))
        lk = rnd.choice((0.0, 1.0, -1.0, 0.8, rnd.uniform(-1.2, 1.2), 0.0099, -0.0099, 0.01, -0.01))
        du = rnd.choice((1.0, 0.0, 2.3333, rnd.uniform(0, 5)))
        ph = rnd.choice((0.0, 1.0, 0.5, -0.1, 1.1, rnd.uniform(-0.5, 1.5), v))
        c = [f2w(x) for x in (v, t, g, dt, lk, du, ph)]
        if any(w >> 24 == 0x82 for w in c):
            continue
        cases.append(c)
    cf = work / 'pan_smooth_cases.txt'
    cf.write_text(''.join(' '.join(f'{w:08x}' for w in c) + '\n' for c in cases))
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    rows = [' '.join(f'{w:08x}' for w in c) + ' -> ' + o + '\n' for c, o in zip(cases, out)]
    assert not any(w.startswith('82') for row in rows for w in row.split()), 'a word that reads as an address'
    (work / 'pan_smooth_ref.txt').write_text(''.join(rows))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
