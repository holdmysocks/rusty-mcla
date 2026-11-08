"""The human rider's look at a racer beside him run offline on the original's code (generated code unchanged): F-70536 with its side
test F-48336 and the arc tangent F-65975. r3 the look state (+0 the racer looked at, +4 its look, +8 the hold timer, +12 range 20,
+16 angle 0.785398 (the image's pi / 4, 0x3F490FDB), +20 look 1 s, +24 hold 5 s: the driver constructor's), r5 the human's racer, f1 the camera's look, f2 dt.
Stubbed, as inputs of the case: the racer's driving test F-90764 (a flag per racer) and the race manager F-27863 (its racer list:
count at +3088, a slot of 192 bytes from +16 with the racer at its +0). A racer's +48 is its instance, whose matrix (rows right, up,
back, place) is at +144 or, with the thread block's byte +72 set, at +208.
Usage: rival_look_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin] [--n 1500]
Writes <out-dir>/rival_look_ref.txt, one case a line (hex words):
  "held look timer cam dt  me_index  n  (active inst slot_used  m0..m8 px py pz) x n  ->  ret rival look timer"
  where held is 0 / 1 + the index of the racer held, m0..m8 the three rows used (right, up, back), rival the index after or -1."""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import recomp_dispatch as rd

STATE, RM, RACERS, ACTIVE, CASE = 0x40800000, 0x40900000, 0x40A00000, 0x40B00000, 0x40C00000


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


def f2w(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


MAIN = r"""
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); uint32_t n,a,w,f1,f2,me;
 for(;;){
  if(!(in>>std::hex>>n)) return 0;
  rt_reset();
  for(uint32_t i=0;i<n;i++){ in>>a>>w; if(a&0x80000000u) *pointer(a&0x7fffffffu)=(uint8_t)w; else store(a,w); }
  in>>me>>f1>>f2;
  Context c=fresh(); c.r3.u64=STATE_; c.r4.u64=0; c.r5.u64=me; float x; memcpy(&x,&f1,4); c.f1.f64=x; memcpy(&x,&f2,4); c.f2.f64=x;
  FN(c,nullptr);
  float r=(float)c.f1.f64; uint32_t rw; memcpy(&rw,&r,4);
  printf("%08x %08x %08x %08x\n",rw,load(STATE_),load(STATE_+4),load(STATE_+8)); }
}
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('--n', type=int, default=1500)
    a = ap.parse_args()
    work = a.out.resolve(); work.mkdir(parents=True, exist_ok=True)
    fn = {k: 'sub_%08X' % addr(a.tool, k) for k in ('F-70536', 'F-48336', 'F-90764', 'F-27863')}
    main_src = MAIN.replace('STATE_', '0x%08Xu' % STATE).replace('FN', fn['F-70536'])
    stubs = {fn['F-90764']: f'ctx.r3.u64=load({ACTIVE:#x}u+((ctx.r3.u32-{RACERS:#x}u)>>12)*4);',
             fn['F-27863']: f'ctx.r3.u64={RM:#x}u;'}
    idx = rd.index(a.recomp); img = rd.load_image(a.image)
    exe = rd.build(idx, [fn['F-70536']], main_src, work, 'rival_look', stubs=stubs, image=img)
    rnd = random.Random(20261107)

    def fl(lo, hi):
        return struct.unpack('>f', struct.pack('>f', rnd.uniform(lo, hi)))[0]

    def rot():
        # a rotation matrix from a random quaternion (rows right, up, back), rounded to f32 as a stored matrix is
        q = [rnd.gauss(0, 1) for _ in range(4)]
        if rnd.random() < 0.5:
            q = [q[0] * 0.1, q[1], q[2] * 0.1, q[3]]       # mostly upright
        s = sum(x * x for x in q) ** 0.5
        x, y, z, w = (v / s for v in q)
        m = [[1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)],
             [2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w)],
             [2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y)]]
        return [struct.unpack('>f', struct.pack('>f', v))[0] for r in m for v in r]
    lines, ref_in = [], []
    for _ in range(a.n):
        nr = rnd.randint(1, 7)
        me = rnd.randrange(nr)
        flag = rnd.random() < 0.3
        mats = []
        mem = [(STATE + 12, f2w(20.0)), (STATE + 16, 0x3F490FDB), (STATE + 20, f2w(1.0)), (STATE + 24, f2w(5.0)),
               (RM + 8 + 3080, nr), (rd.RT_BLOCK + 72 | 0x80000000, int(flag))]
        centre = [fl(-2000, 2000), fl(-50, 50), fl(-2000, 2000)]
        for i in range(nr):
            rp = RACERS + 0x1000 * i
            used = i == me or rnd.random() < 0.9
            active = i == me or rnd.random() < 0.85
            inst = i == me or rnd.random() < 0.9
            m = rot()
            reach = rnd.choice((6.0, 15.0, 25.0, 60.0))
            p = [centre[0] + fl(-reach, reach), centre[1] + fl(-3, 3), centre[2] + fl(-reach, reach)]
            if rnd.random() < 0.05:
                p = list(centre)
            mats.append((int(active), int(inst), int(used), m, p))
            mem.append((RM + 16 + 192 * i, rp if used else 0))
            mem.append((ACTIVE + 4 * i, int(active)))
            mem.append((rp + 48, rp + 0x100 if inst else 0))
            at, other = (rp + 0x100 + 208, rp + 0x100 + 144) if flag else (rp + 0x100 + 144, rp + 0x100 + 208)
            for r in range(3):
                for k in range(3):
                    mem.append((at + 16 * r + 4 * k, f2w(m[3 * r + k])))
                    mem.append((other + 16 * r + 4 * k, f2w(fl(-1, 1))))
            for k in range(3):
                mem.append((at + 48 + 4 * k, f2w(p[k])))
                mem.append((other + 48 + 4 * k, f2w(fl(-3000, 3000))))
        held = rnd.random() < 0.35
        hi = rnd.randrange(nr)
        look = rnd.choice((1.0, -1.0, 0.3))
        timer = rnd.choice((5.0, fl(-0.1, 5.1), 4.0, 0.0, fl(3.9, 4.1), fl(-0.05, 0.05)))
        cam, dt = fl(-1, 1), rnd.choice((1 / 30, 1 / 60, fl(0, 0.2), 0.0))
        dt = struct.unpack('>f', struct.pack('>f', dt))[0]
        mem += [(STATE, RACERS + 0x1000 * hi if held else 0), (STATE + 4, f2w(look)), (STATE + 8, f2w(timer))]
        lines.append('%x ' % len(mem) + ' '.join('%x %x' % (x, y) for x, y in mem) + ' %x %x %x\n' % (RACERS + 0x1000 * me, f2w(cam), f2w(dt)))
        ref_in.append(' '.join(['%x' % (int(held) + hi if held else 0), '%08x' % f2w(look), '%08x' % f2w(timer), '%08x' % f2w(cam),
                                '%08x' % f2w(dt), '%x' % me, '%x' % nr] +
                               [' '.join(['%x' % ac, '%x' % ins, '%x' % us] + ['%08x' % f2w(v) for v in m + p])
                                for ac, ins, us, m, p in mats]))
    cf = work / 'rival_look_cases.txt'
    cf.write_text(''.join(lines))
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = r.stdout.splitlines()
    assert len(out) == len(ref_in), (len(out), len(ref_in))
    rows = []
    for i, o in zip(ref_in, out):
        ret, rv, lk, tm = o.split()
        rv = int(rv, 16)
        ri = -1 if rv == 0 else (rv - RACERS) >> 12
        rows.append(f'{i} -> {ret} {ri} {lk} {tm}\n')
    (work / 'rival_look_ref.txt').write_text(''.join(rows))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
