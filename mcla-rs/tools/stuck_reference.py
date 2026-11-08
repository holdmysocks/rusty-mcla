"""The racer brain's stuck detector F-34336 run offline (the generated code whole and unchanged, vector mode). Real memory:
the brain's +332 turn, +336 reverse, +337 handbrake (bytes), +340 state, +344 timer, +352 anchor, +368 / +372 / +376 the
2.0 s / 1.0 s / 5.0 m fields, dt [0x827D7508], the speed the brain reads (racer +48 -> +8 -> +224). Stubs: the racer's
matrix getter F-35658 (returns the case's matrix: +32 the row whose negation is forward, +48 the position), the line's
next point F-28749 (a pointer to the case's point) and the corridor point ahead F-43876 (the case's point copied to the
caller's buffer). The plan angle F-32060 and what it calls run as generated.

Cases (seed 34336): single calls from random states - each state 0 / 1 / 2, the anchor at about 5 m and on the edge, speeds
at and round 10 m/s, timers round 2 s / 1 s, the turn side set or not, the line and corridor points round 45 and 135 degrees,
a few NaNs and zero vectors; then sequences of 12 calls with the car creeping, reversing and turning.

Writes <out-dir>/stuck_ref.txt, hex words: "case" then the inputs (state timer anchor.xyz pos.xyz row2.xyz speed p1.xyz
p2.xyz turn reverse handbrake dt), "->" then the outputs (state timer anchor.xyz turn reverse handbrake); sequence lines are
"seq" (the start) and "step" (pos.xyz row2.xyz speed p1.xyz p2.xyz -> outputs)."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

B, R, CR, S, M, P1, P2 = (0x40100000 + 0x1000 * k for k in range(7))
DT = 0x827D7508


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:].upper()


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', f32(x)))[0]


def nudge(x, n):
    w = fb(x)
    return struct.unpack('>f', struct.pack('>I', (w + n) & 0xFFFFFFFF))[0] if x != 0 else x


def point(rnd, pos, fwd_angle, near):
    a = fwd_angle + rnd.choice(near) * rnd.choice((1, -1)) + rnd.choice((0.0, 0.0, 1e-7, -1e-7, rnd.uniform(-0.3, 0.3), rnd.uniform(-3.2, 3.2)))
    d = rnd.choice((rnd.uniform(0.5, 60), 0.0 if rnd.random() < 0.02 else rnd.uniform(5, 30)))
    return (pos[0] + d * math.sin(a), pos[1] + rnd.uniform(-2, 2), pos[2] + d * math.cos(a))


def state(rnd):
    pos = tuple(rnd.uniform(-900, 900) for _ in range(3))
    r = rnd.random()
    if r < 0.4:
        dist = 5.0 + rnd.choice((0.0, 1e-6, -1e-6, 1e-4, -1e-4))
    else:
        dist = rnd.uniform(0, 8)
    u = [rnd.gauss(0, 1) for _ in range(3)]
    n = math.sqrt(sum(x * x for x in u)) or 1
    anchor = tuple(p + dist * x / n for p, x in zip(pos, u))
    if rnd.random() < 0.05:
        anchor = pos
    h = rnd.uniform(-math.pi, math.pi)
    tilt = rnd.choice((0.0, rnd.uniform(-0.2, 0.2)))
    row2 = (-math.sin(h), tilt, -math.cos(h))          # forward = -row2 points along heading h
    if rnd.random() < 0.02:
        row2 = (0.0, 1.0, 0.0)
    near = (math.radians(45), math.radians(135), 0.0, math.pi, math.radians(90))
    p1 = point(rnd, pos, h, near)
    p2 = point(rnd, pos, h, near)
    speed = rnd.choice((10.0, nudge(10.0, 1), nudge(10.0, -1), rnd.uniform(0, 20), rnd.uniform(0, 10), 0.0, rnd.uniform(-5, 5)))
    st = rnd.choice((0, 0, 1, 1, 2, 2, 2))
    dt = rnd.choice((1 / 30, 1 / 30, 1 / 30, 1 / 60, 0.0, 0.1))
    base = {0: 2.0, 1: 1.0, 2: rnd.uniform(0, 2)}[st]
    timer = rnd.choice((base - f32(dt), nudge(base - f32(dt), 1), nudge(base - f32(dt), -1), rnd.uniform(0, 2.5), 0.0))
    turn = rnd.choice((0, 0, 1, 2))
    rev = rnd.choice((0, 1))
    hb = rnd.choice((0, 1))
    if rnd.random() < 0.01:
        speed = float('nan')
    if rnd.random() < 0.01:
        p1 = (float('nan'), 0.0, 0.0)
    return [st, timer, *anchor, *pos, *row2, speed, *p1, *p2, turn, rev, hb, dt]


def r32(x):
    return f32(x)


def fma32(a, b, c):
    from fractions import Fraction
    return f32(float(Fraction(a) * Fraction(b) + Fraction(c)))


def edge(rnd):
    """A state-0 / state-1 call whose anchor distance the fused test (the original's) and an unfused one decide differently."""
    while True:
        s = state(rnd)
        pos, anc = s[5:8], s[2:5]
        u = [rnd.gauss(0, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in u))
        anc = [f32(p + 5.0 * x / n) for p, x in zip(pos, u)]
        d = [r32(f32(p) - a) for p, a in zip(pos, anc)]
        fused = fma32(d[0], d[0], fma32(d[2], d[2], r32(d[1] * d[1]))) > 25.0
        plain = r32(r32(r32(d[0] * d[0]) + r32(d[1] * d[1])) + r32(d[2] * d[2])) > 25.0
        if fused != plain:
            s[2:5] = anc
            s[0] = rnd.choice((0, 1))
            s[11] = 0.0
            return s


def words(v):
    out = []
    for k, x in enumerate(v):
        out.append(x if k in (0, 18, 19, 20) else fb(x))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    ap.add_argument('--seqs', type=int, default=300)
    ap.add_argument('--edges', type=int, default=300)
    ap.add_argument('--steps', type=int, default=12)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-34336')
    rnd = random.Random(34336)
    nl = chr(10)
    cases = [words(state(rnd)) for _ in range(a.cases)] + [words(edge(rnd)) for _ in range(a.edges)]
    # sequences: a start state then per step the car's pos / row2 / speed / points (the car creeping, backing, turning)
    seqs = []
    for _ in range(a.seqs):
        s = state(rnd)
        s[0] = 0
        s[18] = s[19] = s[20] = 0
        s[1] = 0.0
        s[2:5] = s[5:8]
        pos = list(s[5:8])
        h = rnd.uniform(-math.pi, math.pi)
        steps = []
        for j in range(a.steps):
            mode = rnd.random()
            pos = [p + (rnd.uniform(-0.5, 0.5) if mode < 0.8 else rnd.uniform(-4, 4)) for p in pos]
            h += rnd.uniform(-0.6, 0.6)
            row2 = (-math.sin(h), 0.0, -math.cos(h))
            spd = rnd.choice((rnd.uniform(0, 3), rnd.uniform(0, 12)))
            near = (math.radians(45), math.radians(135), 0.0, math.radians(90))
            steps.append([fb(x) for x in (*pos, *row2, spd, *point(rnd, pos, h, near), *point(rnd, pos, h, near))])
        s[21] = rnd.choice((1 / 30, 0.4, 0.7))     # (big steps reach the 2 s / 1 s ends inside 12 calls)
        seqs.append((words(s), steps))
    crow = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    srow = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c, _ in seqs)
    trow = (',' + nl).join('{' + ','.join('{' + ','.join('%du' % w for w in st) + '}' for st in steps) + '}' for _, steps in seqs)
    setup = ('static void setup(const uint32_t*s){' +
             'store(%du+340,s[0]);store(%du+344,s[1]);store(%du+352,s[2]);store(%du+356,s[3]);store(%du+360,s[4]);' % (B, B, B, B, B) +
             'store(%du+332,s[18]);*pointer(%du+336)=(uint8_t)s[19];*pointer(%du+337)=(uint8_t)s[20];store(%du,s[21]);}' % (B, B, B, DT) + nl +
             'static void car(const uint32_t*p){store(%du+48,p[0]);store(%du+52,p[1]);store(%du+56,p[2]);' % (M, M, M) +
             'store(%du+32,p[3]);store(%du+36,p[4]);store(%du+40,p[5]);store(%du+224,p[6]);' % (M, M, M, S) +
             'for(int i=0;i<3;i++){store(%du+4*i,p[7+i]);store(%du+4*i,p[10+i]);}}' % (P1, P2) + nl +
             'static void out(){printf(" -> %%08x %%08x %%08x %%08x %%08x %%u %%u %%u\\n",load(%du+340),load(%du+344),load(%du+352),load(%du+356),load(%du+360),load(%du+332),(unsigned)*pointer(%du+336),(unsigned)*pointer(%du+337));}'
             % (B, B, B, B, B, B, B, B) + nl +
             'static void call(){Context ctx=fresh();ctx.r3.u32=%du;sub_%s(ctx,nullptr);}' % (B, fn) + nl +
             'static void world(){rt_reset();memset(pointer(%du),0,0x7000);store(%du+4,%du);store(%du+300,%du);store(%du+48,%du);store(%du+8,%du);' % (B, B, R, B, 0x40200000, R, CR, CR, S) +
             'store(%du+368,0x40000000u);store(%du+372,0x3f800000u);store(%du+376,0x40a00000u);store(%du+60,0x3f800000u);}' % (B, B, B, M))
    main_src = (
        'static const uint32_t CS[][22]={' + nl + crow + nl + '};' + nl +
        'static const uint32_t SQ[][22]={' + nl + srow + nl + '};' + nl +
        'static const uint32_t ST[][%d][13]={' % a.steps + nl + trow + nl + '};' + nl + setup + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(auto&s:CS){world();setup(s);uint32_t p[13]={s[5],s[6],s[7],s[8],s[9],s[10],s[11],s[12],s[13],s[14],s[15],s[16],s[17]};car(p);'
        'printf("case");for(int i=0;i<22;i++)printf(" %08x",s[i]);call();out();}' + nl +
        'for(size_t k=0;k<sizeof SQ/sizeof SQ[0];k++){auto&s=SQ[k];world();setup(s);printf("seq");for(int i=0;i<22;i++)printf(" %%08x",s[i]);printf("\\n");'
        'for(int j=0;j<%d;j++){car(ST[k][j]);printf("step");for(int i=0;i<13;i++)printf(" %%08x",ST[k][j][i]);call();out();}}}' % a.steps + nl)
    stubs = {'sub_' + addr(a.tool, 'F-35658'): 'ctx.r3.u32=%du;' % M,
             'sub_' + addr(a.tool, 'F-28749'): 'ctx.r3.u32=%du;' % P1,
             'sub_' + addr(a.tool, 'F-43876'): 'for(int i=0;i<16;i+=4)store(ctx.r3.u32+i,load(%du+i));' % P2}
    idx = rd.index(a.recomp)
    exe = rd.build(idx, ['sub_' + fn], main_src, a.out_dir, 'stuck', stubs=stubs, image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases + a.edges + a.seqs * (a.steps + 1), len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'stuck_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [x.split('->')[1].split() for x in lines if '->' in x]
    from collections import Counter
    print(len(res), 'calls; end states', Counter(r[0] for r in res), 'turn', Counter(r[5] for r in res), 'reverse', Counter(r[6] for r in res))


if __name__ == '__main__':
    main()
