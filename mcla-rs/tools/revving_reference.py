"""The racer brain's pre-race rev cycle run offline: the driving output F-17058 (the generated code whole and unchanged, vector
mode) with its case 2 reached - hold still (+257) off, the global word 0x82874690 0, revving on (+254) or its timer (+280)
still running; when it is not, the chassis word +592 = 2 (the grid) takes the frame to case 3. The aiv output F-65301 and the wheel-speed helper F-59682 run before the cases and do not reach case
2's result: they are stubs (the aiv answers valid). Everything case 2 reads is real memory: the brain's +254 .. +288, the
engine object (chassis +16: +8 speed, +68 / +72 / +76 the greatest / best power / idle speeds, rad/s, read through the
rpm getters' x 9.549296), the car tune (chassis +368: +252 RevLimitMin, +256 RevLimitMax), dt [0x827D7508] and the shared
generator (0x82860890: lo, carry).

Cases: sequences of 10 frames from random states - both phases, revving on and off, timers running or out, targets set
or -1, RevLimitMax 0 or not, engine speeds walking round the targets and jumping, dt 1/30 mostly.

Writes <out-dir>/revving_ref.txt: "case e h r t u d lo carry i68 i72 i76 lmin lmax dt" then one frame a line:
"speed -> rpm timer up down lo carry throttle handbrake" (hex words; e / h decimal); then SetRevvingEnabled F-58174
on 400 states: "set arg rpm up down lo carry -> +254 rpm timer up down lo carry +256"."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

B, R, C, S, X, E, G, T, IN, AIV = (0x40100000 + 0x1000 * k for k in range(10))
DT, MWC, GW = 0x827D7508, 0x82860890, 0x82874690


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:].upper()


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', f32(x)))[0]


def gen(rnd, steps):
    idle = rnd.uniform(70, 120)
    opt = rnd.uniform(400, 700)
    mx = opt + rnd.uniform(0, 250)
    lmax = rnd.choice((0.0, 0.0, rnd.uniform(0.01, 0.2)))
    lmin = rnd.choice((0.0, rnd.uniform(0.0, 1.0)))
    K = 9.549296
    up = rnd.choice((-1.0, rnd.uniform(idle, mx) * K))
    down = rnd.choice((-1.0, -1.0, rnd.uniform(idle, mx) * K))
    st = dict(e=rnd.choice((0, 1, 1, 1)), h=rnd.choice((0, 1)), r=rnd.choice((0.0, rnd.uniform(0, mx * K))),
              t=rnd.choice((0.0, -0.01, rnd.uniform(-0.5, 3.0), 0.02)), u=up, d=down,
              lo=rnd.getrandbits(32), carry=rnd.getrandbits(32) & 0x5CDCFAA6,
              i68=mx, i72=opt, i76=idle, lmin=lmin, lmax=lmax, dt=rnd.choice((1 / 30, 1 / 30, 1 / 30, 1 / 60, 0.1, 0.0)))
    w = rnd.uniform(idle, mx)
    speeds = []
    for _ in range(steps):
        r = rnd.random()
        if r < 0.15:
            w = rnd.uniform(0, mx * 1.1)
        elif r < 0.25 and up > 0:
            w = up / K + rnd.choice((-1e-3, 0.0, 1e-3))
        else:
            w = max(0.0, w + rnd.uniform(-60, 60))
        speeds.append(w)
    return st, speeds


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=800)
    ap.add_argument('--steps', type=int, default=10)
    ap.add_argument('--sets', type=int, default=400)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-17058')
    idx = rd.index(a.recomp)
    rnd = random.Random(17058)
    cases = [gen(rnd, a.steps) for _ in range(a.cases)]
    nl = chr(10)
    keys = ('r', 't', 'u', 'd', 'i68', 'i72', 'i76', 'lmin', 'lmax', 'dt')
    srows = (',' + nl).join('{%du,%du,%du,%du,%s}' % (s['e'], s['h'], s['lo'], s['carry'], ','.join('0x%08xu' % fb(s[k]) for k in keys))
                            for s, _ in cases)
    sets = [(rnd.choice((0, 1)), fb(rnd.choice((0.0, rnd.uniform(0, 8000)))), fb(rnd.choice((-1.0, rnd.uniform(0, 8000)))),
             fb(rnd.choice((-1.0, rnd.uniform(0, 8000)))), rnd.getrandbits(32), rnd.getrandbits(32) & 0x5CDCFAA6) for _ in range(a.sets)]
    se_rows = (',' + nl).join('{%du,0x%08xu,0x%08xu,0x%08xu,%du,%du}' % x for x in sets)
    setfn = addr(a.tool, 'F-58174')
    wrows = (',' + nl).join('{' + ','.join('0x%08xu' % fb(w) for w in ws) + '}' for _, ws in cases)
    main_src = (
        'static void store8(uint32_t a,uint8_t v){*pointer(a)=v;}' + nl +
        'static const uint32_t SC[][14]={' + nl + srows + nl + '};' + nl +
        'static const uint32_t W[][%d]={' % a.steps + nl + wrows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'int n=sizeof SC/sizeof SC[0];for(int k=0;k<n;k++){auto&s=SC[k];rt_reset();' +
        'memset(pointer(%du),0,0xA000);' % B +
        'store(%du+4,%du);store(%du+48,%du);store(%du+8,%du);store(%du+96,%du);store(%du+16,%du);store(%du+20,%du);store(%du+368,%du);'
        % (B, R, R, C, C, S, C, X, S, E, S, G, S, T) +
        'store(%du+300,%du);store(%du+180,0x3f800000u);store(%du+20,0x3f800000u);store(%du+20,0x00030000u);store(%du+592,2u);' % (B, AIV, X, E, G, S) +
        'store(%du,0u);store(%du+4,0u);' % (GW - 4, GW - 4) +
        'store8(%du+254,(uint8_t)s[0]);store8(%du+255,(uint8_t)s[1]);store(%du,s[2]);store(%du+4,s[3]);' % (B, B, MWC, MWC) +
        'store(%du+276,s[4]);store(%du+280,s[5]);store(%du+284,s[6]);store(%du+288,s[7]);' % (B, B, B, B) +
        'store(%du+68,s[8]);store(%du+72,s[9]);store(%du+76,s[10]);store(%du+252,s[11]);store(%du+256,s[12]);store(%du,s[13]);' % (E, E, E, T, T, DT) +
        'printf("case %u %u %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x\\n",s[0],s[1],s[4],s[5],s[6],s[7],s[2],s[3],s[8],s[9],s[10],s[11],s[12],s[13]);' +
        'for(int j=0;j<%d;j++){store(%du+8,W[k][j]);Context ctx=fresh();ctx.r3.u32=%du;ctx.r4.u32=%du;sub_%s(ctx,nullptr);' % (a.steps, E, B, IN, fn) +
        'printf("%%08x -> %%08x %%08x %%08x %%08x %%08x %%08x %%08x %%08x\\n",W[k][j],load(%du+276),load(%du+280),load(%du+284),load(%du+288),load(%du),load(%du+4),load(%du+156),load(%du+164));}}}'
        % (B, B, B, B, MWC, MWC, IN, IN) + nl +
        # (SetRevvingEnabled F-58174 on a brain with no racer, +4 = 0: the rev audio is not reached, +256 = 1)
        'static const uint32_t SE[][6]={' + nl + se_rows + nl + '};' + nl +
        'int set_main(){int n=sizeof SE/sizeof SE[0];for(int k=0;k<n;k++){auto&s=SE[k];rt_reset();memset(pointer(%du),0,0x400);' % B +
        'store(%du+276,s[1]);store(%du+284,s[2]);store(%du+288,s[3]);store(%du,s[4]);store(%du+4,s[5]);' % (B, B, B, MWC, MWC) +
        'Context ctx=fresh();ctx.r3.u32=%du;ctx.r4.u32=s[0];sub_%s(ctx,nullptr);' % (B, setfn) +
        'printf("set %%u %%08x %%08x %%08x %%08x %%08x -> %%u %%08x %%08x %%08x %%08x %%08x %%08x %%u\\n",s[0],s[1],s[2],s[3],s[4],s[5],*pointer(%du+254),load(%du+276),load(%du+280),load(%du+284),load(%du+288),load(%du),load(%du+4),*pointer(%du+256));}return 0;}' % (B, B, B, B, B, MWC, MWC, B) + nl)
    main_src = main_src.replace('int main(int argc,char**argv){', 'int set_main();' + nl + 'int main(int argc,char**argv){', 1)
    main_src = main_src[:main_src.rindex('}}}')] + '}}set_main();}' + main_src[main_src.rindex('}}}') + 3:]
    stubs = {'sub_' + addr(a.tool, 'F-59682'): '', 'sub_' + addr(a.tool, 'F-65301'): 'ctx.r3.u32=1;'}
    # (SetRevvingEnabled's rev audio and the other cases' callees - aiv release, stuck detector, turnaround, nitrous / slingshot tests and benefits, asin: a
    # frame that leaves case 2 stops the run)
    for f in ('F-17772', 'F-58498', 'F-34336', 'F-67810', 'F-50530', 'F-29462', 'F-80613', 'F-82077', 'F-96973'):
        stubs['sub_' + addr(a.tool, f)] = 'throw std::runtime_error("left case 2: %s");' % f
    exe = rd.build(idx, ['sub_' + fn, 'sub_' + setfn], main_src, a.out_dir, 'revving', stubs=stubs, image=rd.load_image(a.image))
    out = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == a.cases * (a.steps + 1) + a.sets, (len(out), a.cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'revving_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(out) + nl))
    frames = [x for x in out if not x.startswith(('case', 'set'))]
    print(a.cases, 'sequences,', len(frames), 'frames,', sum(1 for x in frames if x.split()[-2] == '3f800000'), 'with throttle 1,',
          sum(1 for x in frames if x.split()[4] != 'bf800000'), 'with a rev-up target')


if __name__ == '__main__':
    main()
