"""The bike rider's riding states' anim player calls run offline on the original's code (generated code unchanged): the start's
action F-88016, the enters of cruise F-27733, stopped idle / burnout centre F-73617, tuck F-14953, wheelie F-33885, endo F-23193,
nos boost F-22152, burnout left F-13481 / right F-26376, shift F-54589 and reverse F-93384, and the updates of cruise F-61324,
stopped idle F-63745, tuck F-77590, wheelie F-56690, endo F-14383, nos boost F-23398, shift F-32007 and reverse F-30945, and the
exits of stopped idle F-52588 and of the fidget / gesture F-53813 (machine +40 a zeroed racer, the game manager zeroed). Each
callback takes r3 the machine (+28 the context, +32 the anim player). The anim player's entry points are stubs that log their
arguments; the clip lookup F-66148 answers the name's own address (so a logged clip is its name), the pan lookup F-56423 the
same or 0 (the case's pan bit); the phase remap F-25740 and the context's rescale F-34842 run.
Usage: rider_cmd_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin] [-n 150]
Writes <out-dir>/rider_cmd_ref.txt, per case "<callback> <pan> <+8 +44 +92 +96 +148> -> <call>; <call>; ..." (hex words), each
call "<F-id> <r4 ...> <f1 as float bits>" with the integer registers the entry point reads (none for the setters), a clip
written as its name."""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import memcap
import recomp_dispatch as rd

CALLBACKS = ['F-88016', 'F-27733', 'F-73617', 'F-14953', 'F-33885', 'F-23193', 'F-22152', 'F-13481', 'F-26376', 'F-54589',
             'F-93384', 'F-61324', 'F-63745', 'F-77590', 'F-56690', 'F-14383', 'F-23398', 'F-32007', 'F-30945',
             'F-52588', 'F-53813']
# the anim player's entry points (logged): +108 / +112 setters, play, pair, pair overlay, one overlay, stop overlay, the phase /
# rate / rate5 / phase2 / weight / overlay rate setters, the time left readers
LOGGED = ['F-96595', 'F-63058', 'F-24441', 'F-85393', 'F-42342', 'F-85022', 'F-72270', 'F-40742', 'F-53423', 'F-84631',
          'F-62772', 'F-22341', 'F-72214', 'F-87745', 'F-70114']
LOOKUPS = ['F-66148', 'F-56423']
# the integer arguments each one reads (r4 on): the clip / clips, looping, no blend; the setters read f1 alone
NREGS = {'F-96595': 1, 'F-63058': 1, 'F-24441': 3, 'F-85393': 3, 'F-42342': 3, 'F-85022': 2}
IN_OFFS = [8, 44, 92, 96, 148]


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


def fw(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(n):
    rnd = random.Random(2211)
    nan = 0x7fc00000
    out = []
    for cb in range(len(CALLBACKS)):
        for k in range(n):
            flags = 0
            for b in (0x80000, 0x100000, 0x200000, 0x100, 0x80, 0x10, 0x20):
                if rnd.random() < 0.3:
                    flags |= b
            x = rnd.choice((rnd.uniform(-1.5, 1.5), rnd.uniform(-1.5, 1.5), 0.0, 1.0, -1.0, -0.5, 0.5, 0.124, 0.876, -0.752, 0.752,
                            rnd.uniform(-0.8, -0.7), rnd.uniform(0.7, 0.8)))
            w = [flags, fw(x), fw(rnd.uniform(-1, 1)), fw(rnd.choice((rnd.uniform(-5, 40), 17.8816, 22.35, 0.0, 30.0, 100.0))),
                 fw(rnd.choice((rnd.uniform(0, 3), 0.0, 1.0, 0.5, 2.0)))]
            if k % 41 == 0:
                w[1] = nan
            if k % 43 == 0:
                w[3] = nan
            if k % 47 == 0:
                w[4] = nan
            out.append((cb, k % 3 != 0, w))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('-n', type=int, default=150)
    a = ap.parse_args()
    memcap.apply()
    work = a.out.resolve()
    work.mkdir(parents=True, exist_ok=True)
    fn = {k: 'sub_%08X' % addr(a.tool, k) for k in CALLBACKS + LOGGED + LOOKUPS}
    js = cases(a.n)
    nl = chr(10)
    rows = nl.join('{%d,%d,%s},' % (cb, pan, ','.join('0x%08Xu' % x for x in w)) for cb, pan, w in js)
    calls = nl.join('  case %d: %s(c,nullptr); break;' % (i, fn[k]) for i, k in enumerate(CALLBACKS))
    offs = ','.join(str(o) for o in IN_OFFS)
    main_src = r'''
static const uint32_t J[][7]={
''' + rows + r'''
};
static const uint32_t OFF[]={''' + offs + r'''};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t M=0x40800000u, CTX=0x40810000u, TUNE=0x40820000u, P=0x40830000u, DEV=0x828CD2A8u, PANW=0x40840000u, RACER=0x40850000u, MGR=0x40860000u;
 for(auto&j:J){ rt_reset();
  for(uint32_t x=M;x<M+0x100;x++) *pointer(x)=0;
  for(uint32_t x=CTX;x<CTX+0x200;x++) *pointer(x)=0;
  for(uint32_t x=TUNE;x<TUNE+0x400;x++) *pointer(x)=0;
  {Context c=fresh(); c.r3.u64=TUNE; TUNECTOR(c,nullptr);}
  store(CTX+16,TUNE); store(CTX+64,0x3f800000u);
  store(M+28,CTX); store(M+32,P);
  for(int i=0;i<5;i++) store(CTX+OFF[i],j[2+i]);
  store(DEV,0); store(PANW,j[1]);
  for(uint32_t x=RACER;x<RACER+0x400;x++) *pointer(x)=0;
  for(uint32_t x=MGR;x<MGR+0x200;x++) *pointer(x)=0;
  if(j[0]>=19){ store(M+40,RACER); store(0x8286D8F0u,MGR); }   // (the exits: F-90946 writes the racer's +684)
  Context c=fresh(); c.r3.u64=M;
  switch(j[0]){
''' + calls + r'''
  }
  printf("E\n"); }
}
'''
    main_src = main_src.replace('TUNECTOR', fn['F-40144'] if 'F-40144' in fn else 'sub_%08X' % addr(a.tool, 'F-40144'))
    log = ('printf("C %s",T);uint64_t r[4]={ctx.r4.u64,ctx.r5.u64,ctx.r6.u64,ctx.r7.u64};'
           'for(int i=0;i<NREG;i++){uint32_t v=uint32_t(r[i]);bool str=v>=0x82000000u&&v<0x84000000u;uint32_t n=0;'
           'if(str){for(;n<64&&*pointer(v+n);n++)if(*pointer(v+n)<0x21||*pointer(v+n)>0x7e)str=false;if(n<3||n==64)str=false;}'
           'if(str)'
           'printf(" %s",cfmt(v).c_str());else printf(" %08x",v);}'
           'float f=float(ctx.f1.f64);uint32_t b;memcpy(&b,&f,4);printf(" %08x\\n",b);ctx.f1.f64=-1.0;')
    stubs = {fn[k]: 'static const char*T="%s";' % k + log.replace('NREG', str(NREGS.get(k, 0))) for k in LOGGED}
    stubs[fn['F-66148']] = 'ctx.r3.u64=ctx.r4.u32;'
    stubs[fn['F-56423']] = 'ctx.r3.u64=load(0x40840000u)?ctx.r4.u32:0;'
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    tune = 'sub_%08X' % addr(a.tool, 'F-40144')
    exe = rd.build(idx, [fn[k] for k in CALLBACKS] + [tune], main_src, work, 'rider_cmd',
                   stubs=stubs, image=img)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, encoding='latin-1')
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    groups, cur = [], []
    for x in r.stdout.split(nl):
        if x == 'E':
            groups.append(cur)
            cur = []
        elif x.startswith('C '):
            cur.append(x[2:])
    assert len(groups) == len(js), (len(groups), len(js))
    lines = ['%s %d %s -> %s' % (CALLBACKS[cb], pan, ' '.join('%08x' % x for x in w), '; '.join(g)) for (cb, pan, w), g in zip(js, groups)]
    (work / 'rider_cmd_ref.txt').write_text(nl.join(lines) + nl)
    print(len(js), 'cases;', sum(len(g) for g in groups), 'calls')


if __name__ == '__main__':
    main()
