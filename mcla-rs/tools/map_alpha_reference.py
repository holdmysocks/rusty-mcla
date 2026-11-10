"""The sector draw F-75080 run offline as generated (cameras2-map2), written to crates/game/src/testdata/map_alpha_ref.txt for
the port's tests. recomp_closure imports tools/memcap.py, so every process of this script (g++, the harness) runs in a 4 GB job.
Everything the function calls is stubbed with a recorder (the fade / shader-parameter setters, the sector draw F-65459, the far
model draw F-73936, the state set-up / restore pair and the camera helpers of the mirror pass): this checks what the draw DECIDES
and which alphas it hands them. The function itself is unchanged, as generated.
One line per case, "SEC <inputs> -> <events>"; floats are f32 bit patterns in hex. Events in call order:
  S   the render-state set-up F-24823            A<bits>  the light-manager fade F-18250 (f1)
  P<slot>:<bits>  the shader-parameter setter F-33110      D<pass>  the sector's drawables drawn F-65459
  F<mode>  the far model drawn F-73936           R  the render-state restore F-75431
  V W X<n>  the camera helpers F-60960 / F-37365 / F-62018 of the mirror pass (r4 == 2)
SEC <pass> <arg5> <+252 state> <+295 fade state> <+296 choice> <+268 dist> <+288 t> <thr> <thr2> <forced fade> <gate>
    <det on> <byte at D+93> <f at D+120> <rn+370> <rn+393> <far drawable> <far table entry> <far +8> <byte 14391> <night> -> events
Usage: map_alpha_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def hi(x):
    return (x << 16) & 0xFFFFFFFF


H1 = hi(-32130)            # the city block's page
H2 = hi(-32120)            # the renderer's page

COMMON = r'''
static char evb[2048];
static int evn;
static uint32_t g_night;
'''

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t R=0x40010000,RN=0x40011000,FD=0x40010800,TB=0x40010A00;
 while(in>>k){rt_reset();evn=0;evb[0]=0;
  uint32_t pass,a5,st,fs,ch,dist,t,thr,thr2,forced,gate,det,b93,f120,r370,r393,far,ent,f8,b14391,night;
  in>>std::hex>>pass>>a5>>std::dec>>st>>fs>>ch>>std::hex>>dist>>t>>thr>>thr2>>forced>>gate>>std::dec>>det>>b93>>std::hex>>f120>>std::dec>>r370>>r393>>far>>ent>>f8>>b14391>>night;
  for(uint32_t a=R;a<R+0x400;a++)*pointer(a)=0;
  for(uint32_t a=RN;a<RN+0x200;a++)*pointer(a)=0;
  for(uint32_t a=FD;a<FD+0x40;a++)*pointer(a)=0;
  store(R+244,2);store(R+252,st);store(R+268,dist);store(R+288,t);*pointer(R+295)=fs;*pointer(R+296)=ch;
  store(H1_+3616,gate);*pointer(H1_+3621)=b93;store(H1_+3648,f120);store(H1_+3664,thr);store(H1_+3668,thr2);*pointer(H1_+3676)=det;store(H1_+3720,forced);
  store(H1_+3680,0x11111111);store(H1_+3684,0x22222222);
  store(H2_-8092,RN);*pointer(RN+370)=r370;*pointer(RN+393)=r393;
  store(H2_-14380,far?FD:0);store(FD+8,f8);store(FD+12,TB);store(TB+8,ent);*pointer(H2_-14391)=b14391;
  g_night=night;
  Context cx=fresh();cx.r3.u32=R;cx.r4.u32=pass;cx.r5.u32=a5;sub_SEC(cx,nullptr);
  printf("%s\n",evn?evb:"-");
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
    ap.add_argument('--seed', type=int, default=75080)
    ap.add_argument('--n', type=int, default=20000)
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    a.work.mkdir(parents=True, exist_ok=True)

    def name(fid):
        return 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    rnd = random.Random(a.seed)
    rc.EXTRA += COMMON

    def ev(fmt, args=''):
        return 'evn+=sprintf(evb+evn,"%s"%s);' % (fmt, (',' + args) if args else '')
    stubs = {
        name('F-24823'): ev('S '),
        name('F-18250'): '{float f=float(ctx.f1.f64);uint32_t b;std::memcpy(&b,&f,4);' + ev('A%08x ', 'b') + '}',
        name('F-33110'): ev('P%u:%08x ', 'ctx.r3.u32,ctx.r4.u32'),
        name('F-65459'): ev('D%x ', 'ctx.r4.u32'),
        name('F-73936'): ev('F%u ', 'ctx.r5.u32'),
        name('F-75431'): ev('R '),
        name('F-60960'): ev('V '),
        name('F-37365'): 'ctx.r3.u32=7;' + ev('W '),
        name('F-62018'): ev('X%u ', 'ctx.r3.u32'),
        name('F-47292'): 'ctx.r3.u32=g_night;',
    }
    src = (MAIN.replace('sub_SEC(', name('F-75080') + '(').replace('H1_', '0x%08Xu' % H1).replace('H2_', '0x%08Xu' % H2))
    exe = rd.build(idx, [name('F-75080')], src, a.work, 'map_alpha', image=img, stubs=stubs)

    passes = [0x80000, 0x80000, 0x800, 0x800, 2, 1, 4, 8, 0x10, 0x20, 0x40, 0x80, 0x100, 0x200, 0x400, 0x1000, 0x2000, 0x4000, 0x8000,
              0x10000, 0x20000, 0x40000, 0x100000, 0x1000000, 0x2000000, 0x80800, 0x146000, 0x1E1800, 0x1800, 0x402, 0x7FE]
    ts = [0.0, 0.0, 0.25, 0.5, 0.6666667, 0.9, 0.99, 1.0, 0.3333333, 0.4999999, 0.5000001]
    cases = []
    for _ in range(a.n):
        pas = rnd.choice([0x80000, 0x800]) if rnd.random() < 0.35 else rnd.choice(passes + [rnd.getrandbits(25)])
        a5 = rnd.choice([0, 1, 0xFFFF, rnd.getrandbits(16), 0x12345678])
        st = rnd.choice([3, 3, 3, 3, 3, 0, 1, 2, 4])
        fs = rnd.choice([1, 1, 2, 4, 2, 4, 0, 3, 0xFF])
        thr = rnd.choice([450.0, 300.0, 300.0, rnd.uniform(0, 900)])
        thr2 = rnd.choice([thr, rnd.uniform(0, 900), 300.0])
        dist = rnd.choice([thr, 0.0, rnd.uniform(0, 900), thr * 0.999, thr * 1.001, 1e9, thr2, thr2 * 0.99])
        f32 = lambda v: struct.unpack('>f', struct.pack('>f', v))[0]
        ch = rnd.choice([0, 1, 2, 4, 5, 6, 7, 6, 5, 6, 5, 3, 8, 0xFF]) if rnd.random() < 0.4 else (4 if st == 3 else 0) | (2 if st == 3 and f32(dist) < f32(thr) else 1)
        t = rnd.choice(ts + [rnd.uniform(0, 1.2), rnd.uniform(-0.1, 1.0)])
        forced = rnd.choice([-1.0] * 8 + [0.0, 1.0, 0.5, rnd.uniform(0, 1), rnd.uniform(0, 1), 0.25, 0.3333333, -0.5])
        gate = rnd.choice([450.0, 300.0, 0.0, rnd.uniform(0, 900), 1e9])
        shipped = rnd.random() < 0.85
        det, b93, f120, r370, r393, far, ent, f8, b14 = (1, 0, 1.0, 0, 0, 1, 1, 1, 0) if shipped else (
            rnd.choice([0, 1]), rnd.choice([0, 1]), rnd.choice([1.0, 0.75, 0.0]), rnd.choice([0, 1]), rnd.choice([0, 1]), rnd.choice([0, 1, 1]),
            rnd.choice([0, 1, 1]), rnd.choice([0, 1, 1]), rnd.choice([0, 1]))
        night = rnd.choice([0, 1])
        cases.append(f'SEC {pas:x} {a5:x} {st} {fs} {ch} {fb(dist):08x} {fb(t):08x} {fb(thr):08x} {fb(thr2):08x} {fb(forced):08x} {fb(gate):08x} '
                     f'{det} {b93} {fb(f120):08x} {r370} {r393} {far} {ent} {f8} {b14} {night}')
    cf = a.work / 'SEC_cases.txt'
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
