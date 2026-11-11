"""The bike rider's riding states' context writes run offline on the original's code (generated code unchanged): the start's
action F-88016, the enters of cruise F-27733, wheelie F-33885, endo F-23193, nos boost F-22152 and reverse F-93384, the updates
of cruise F-61324, tuck F-77590, endo F-14383, nos boost F-23398, stopped idle F-63745 and reverse F-30945, and the exit F-64876
(nos boost's and reverse's). The tune (context +16) is made by its constructor F-40144 and the context reset by F-81756. Each
callback takes r3 the machine (+28 the context, +32 the anim player, +45 the jaw byte); the anim player's calls and the clip
lookups are stubbed (they do not touch the context); the phase remap F-25740 and the context's rescale F-34842 run.
Usage: rider_ctx_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin]
Writes <out-dir>/rider_ctx_ref.txt: line 1 "tune <+124> <+128> <+132>", line 2 "reset <+12> <+52> <+56> <+60> <+64>", then per
case "<callback> <dev> <+8 +12 +24 +28 +44 +52 +56 +60 +64 +92 +96> -> <+12 +24 +28 +52 +56 +60 +64> <byte +45>" (hex words)."""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import memcap
import recomp_dispatch as rd

CALLBACKS = ['F-88016', 'F-27733', 'F-33885', 'F-23193', 'F-22152', 'F-93384', 'F-61324', 'F-77590', 'F-14383', 'F-23398',
             'F-63745', 'F-30945', 'F-64876']
# the anim player's calls and the clip lookups: no context writes (stubbed; F-66148 / F-56423 answer no clip)
STUBBED = ['F-96595', 'F-63058', 'F-66148', 'F-56423', 'F-24441', 'F-40742', 'F-53423', 'F-85393', 'F-84631', 'F-62772', 'F-22341',
           'F-85022', 'F-72214']
IN_OFFS = [8, 12, 24, 28, 44, 52, 56, 60, 64, 92, 96]


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


def fw(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(n):
    rnd = random.Random(1111)
    nan = 0x7fc00000
    out = []
    for cb in range(len(CALLBACKS)):
        for k in range(n):
            flags = 0
            for b in (0x80000, 0x100000, 0x200000, 0x100, 0x80, 0x10):
                if rnd.random() < 0.3:
                    flags |= b
            flags2 = rnd.choice((0, 8, 0x20, 0x28, 4, 0xC, 0xFFFFFFFF, 0x3E000000))
            ph = rnd.choice((rnd.uniform(-1.5, 1.5), rnd.uniform(-1.5, 1.5), rnd.uniform(-1.5, 1.5), 0.0, 1.0, -1.0, 0.124, 0.876,
                             -0.752, 0.752, rnd.uniform(-0.8, -0.7), rnd.uniform(0.7, 0.8)))
            w = [flags, flags2, fw(rnd.uniform(-1.2, 1.2)), fw(rnd.uniform(-1.2, 1.2)), fw(ph), fw(rnd.uniform(0, 1.2)),
                 fw(rnd.uniform(0, 30)), fw(rnd.uniform(0, 30)), fw(rnd.choice((rnd.uniform(0.2, 3), 1.0, 0.9))),
                 rnd.choice((fw(rnd.uniform(-1, 1)), 0, 0x80000000, fw(1.0), fw(-1.0))),
                 fw(rnd.choice((rnd.uniform(-5, 40), 17.8816, 22.35, 0.0, 30.0, 100.0)))]
            if k % 41 == 0:
                w[4] = nan
            if k % 43 == 0:
                w[10] = nan
            if k % 47 == 0:
                w[9] = nan
            out.append((cb, k % 5 == 0, w))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('-n', type=int, default=400)
    a = ap.parse_args()
    memcap.apply()
    work = a.out.resolve()
    work.mkdir(parents=True, exist_ok=True)
    fn = {k: 'sub_%08X' % addr(a.tool, k) for k in CALLBACKS + STUBBED + ['F-40144', 'F-81756']}
    js = cases(a.n)
    nl = chr(10)
    rows = nl.join('{%d,%d,%s},' % (cb, dev, ','.join('0x%08Xu' % x for x in w)) for cb, dev, w in js)
    calls = nl.join('  case %d: %s(c,nullptr); break;' % (i, fn[k]) for i, k in enumerate(CALLBACKS))
    offs = ','.join(str(o) for o in IN_OFFS)
    main_src = r'''
static const uint32_t J[][13]={
''' + rows + r'''
};
static const uint32_t OFF[]={''' + offs + r'''};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t M=0x40800000u, CTX=0x40810000u, TUNE=0x40820000u, P=0x40830000u, DEV=0x828CD2A8u;
 rt_reset();
 for(uint32_t x=TUNE;x<TUNE+0x400;x++) *pointer(x)=0;
 {Context c=fresh(); c.r3.u64=TUNE; TUNECTOR(c,nullptr);}
 printf("tune %08x %08x %08x\n",load(TUNE+124),load(TUNE+128),load(TUNE+132));
 for(uint32_t x=CTX;x<CTX+0x200;x++) *pointer(x)=0xcd;
 store(CTX+16,TUNE);
 {Context c=fresh(); c.r3.u64=CTX; RESET(c,nullptr);}
 printf("reset %08x %08x %08x %08x %08x\n",load(CTX+12),load(CTX+52),load(CTX+56),load(CTX+60),load(CTX+64));
 for(auto&j:J){ rt_reset();
  for(uint32_t x=M;x<M+0x100;x++) *pointer(x)=0;
  store(M+28,CTX); store(M+32,P); *pointer(M+45)=1;
  for(int i=0;i<11;i++) store(CTX+OFF[i],j[2+i]);
  store(DEV,j[1]);
  Context c=fresh(); c.r3.u64=M;
  switch(j[0]){
''' + calls + r'''
  }
  printf("%08x %08x %08x %08x %08x %08x %08x %u\n",load(CTX+12),load(CTX+24),load(CTX+28),load(CTX+52),load(CTX+56),load(CTX+60),load(CTX+64),*pointer(M+45)); }
}
'''
    main_src = main_src.replace('TUNECTOR', fn['F-40144']).replace('RESET', fn['F-81756'])
    stubs = {fn[k]: '' for k in STUBBED}
    stubs[fn['F-66148']] = 'ctx.r3.u64=0;'
    stubs[fn['F-56423']] = 'ctx.r3.u64=0;'
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [fn[k] for k in CALLBACKS] + [fn['F-40144'], fn['F-81756']], main_src, work, 'rider_ctx', stubs=stubs,
                   image=img)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = [x for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(js) + 2, (len(out), len(js))
    lines = out[:2] + ['%s %d %s -> %s' % (CALLBACKS[cb], dev, ' '.join('%08x' % x for x in w), o) for (cb, dev, w), o in zip(js, out[2:])]
    (work / 'rider_ctx_ref.txt').write_text(nl.join(lines) + nl)
    print(len(js), 'cases;', out[0], '|', out[1])


if __name__ == '__main__':
    main()
