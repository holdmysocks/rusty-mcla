"""The race marker of one racer runs offline: F-50704 (the colour and fade of the icon manager), its matrix F-52921 and the
queue entry F-34178 (with the quarter turn F-67091), the generated code unchanged (vector mode, the recomp's own x86 flags), on an icon manager at 0x1000, a
racer at 0x4400 and a camera at 0x5000. Stubbed: the racer's hidden test (returns 0: shown), the depth-buffer getter
(only reached with DepthTestRacerMarker; returns a non-zero handle), the bound getter (returns the box at 0x4800) and the
drawable's visibility call (an indirect call: returns visible). The runtime vector +inf (0x828609E0) is written in.

Each case: the camera's z row and position, the racer's matrix position (at +144 or +208 by the thread's byte +72),
icon type, the box's top and bottom, the tune (offset scale, scale, colours, fade windows, depth test). Output per
case: the queue count, then for an entry its matrix (16 words), colour (4), depth flag, distance - float bits as
decimal words. Writes <out-dir>/race_marker_ref.txt: "<inputs> -> <outputs>"."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# (the vector shim builds the scratch memory at its default size; this run needs 0x1000..0x9000)
_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: _shim(0x9000)

VIS = 'static uint32_t g_vis=1;\n#define REX_CALL_INDIRECT_FUNC(a) (ctx.r3.u64=g_vis)\n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def bits(f):
    return struct.unpack('<I', struct.pack('<f', f))[0]


def cases():
    rnd = random.Random(82226)
    out = []
    fades = [(20.0, 45.0, 175.0, 40.0)]
    for _ in range(4400):
        k = rnd.random()
        # the camera's z row: random directions, scaled, near-vertical and exactly vertical
        if k < 0.05:
            z = (0.0, rnd.choice((1.0, -1.0, 0.5)), 0.0)
        elif k < 0.15:
            z = (rnd.uniform(-1e-3, 1e-3), rnd.choice((1.0, -1.0)), rnd.uniform(-1e-3, 1e-3))
        else:
            v = [rnd.gauss(0, 1) for _ in range(3)]
            n = sum(c * c for c in v) ** 0.5 or 1.0
            s = 1.0 if rnd.random() < 0.7 else rnd.uniform(0.2, 3.0)
            z = tuple(c / n * s for c in v)
        cam = tuple(rnd.uniform(-3000, 3000) for _ in range(3))
        # the racer at a distance round the fade windows
        d = rnd.choice((rnd.uniform(0, 260), rnd.choice((20.0, 65.0, 175.0, 215.0)) + rnd.uniform(-1e-3, 1e-3), rnd.uniform(0, 30)))
        u = [rnd.gauss(0, 1) for _ in range(3)]
        n = sum(c * c for c in u) ** 0.5 or 1.0
        pos = tuple(c + uu / n * d for c, uu in zip(cam, u))
        icon = rnd.choice((0, 1, 5, 10, 13, 18, 18))
        top = rnd.uniform(0.9, 2.5)
        bottom = rnd.uniform(-0.8, -0.3)
        off = rnd.choice((1.9, 1.9, rnd.uniform(0.5, 3.0)))
        scale = (1.0, 1.4, 1.0) if rnd.random() < 0.7 else tuple(rnd.uniform(0.3, 2.0) for _ in range(3))
        trans = (0.0, 0.0, 0.0) if rnd.random() < 0.6 else tuple(rnd.uniform(-2, 2) for _ in range(3))
        fade = fades[0] if rnd.random() < 0.8 else (rnd.uniform(0, 40), rnd.uniform(1, 60), rnd.uniform(80, 200), rnd.uniform(1, 60))
        cols = [tuple(rnd.random() for _ in range(4)) for _ in range(3)]
        depth = rnd.choice((0, 1))
        tls = rnd.choice((0, 1))
        out.append((z, cam, pos, icon, top, bottom, off, scale, trans, fade, cols, depth, tls))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fr, fhid, fdepth, fbound = (addr(a.tool, f) for f in ('F-50704', 'F-54047', 'F-26769', 'F-98453'))
    cs = cases()
    nl = chr(10)
    def f(v):
        t = '%.9g' % v
        return t + ('f' if ('.' in t or 'e' in t) else '.0f')
    rows = []
    for z, cam, pos, icon, top, bottom, off, scale, trans, fade, cols, depth, tls in cs:
        fl = list(z) + list(cam) + list(pos) + [top, bottom, off] + list(scale) + list(trans) + list(fade) + [c for col in cols for c in col]
        rows.append('{{%s},%d,%d,%d}' % (','.join(f(v) for v in fl), icon, depth, tls))
    body = (
        'struct K{float v[34];int icon,depth,tls;};static const K C[]={' + nl + (',' + nl).join(rows) + nl + '};' + nl +
        'static void put3(uint32_t a,const float*v,float w){putf(a,v[0]);putf(a+4,v[1]);putf(a+8,v[2]);putf(a+12,w);}' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
        'for(int k=0;k<4;k++)store(0x828609E0+4*k,0x7f800000);' + nl +
        'for(auto&c:C){memset(objects,0,sizeof(objects));const float*v=c.v;' + nl +
        # the icon manager: drawable +68, offset scale +128, depth byte +162, scale +176, translate +192, colours +208/+240/+256, fade +272..
        'store(0x1000+68,0x4900);putf(0x1000+128,v[11]);*pointer(0x1000+162)=uint8_t(c.depth);put3(0x1000+176,v+12,0);put3(0x1000+192,v+15,0);'
        'for(int k=0;k<4;k++){putf(0x1000+208+4*k,v[22+k]);putf(0x1000+240+4*k,v[26+k]);putf(0x1000+256+4*k,v[30+k]);}'
        'for(int k=0;k<4;k++)putf(0x1000+272+4*k,v[18+k]);' + nl +
        # the racer: bound handle +8, the racer object +128 (icon +116), matrices +144 / +208 (identity rotation, position row)
        'store(0x4400+8,0x4800);store(0x4400+128,0x4600);store(0x4600+116,uint32_t(c.icon));'
        'for(uint32_t m:{0x4400u+144,0x4400u+208}){for(int r=0;r<4;r++)for(int q=0;q<4;q++)putf(m+16*r+4*q,r==q?1.f:0.f);}'
        'put3(0x4400+(c.tls?208:144)+48,v+6,1);' + nl +
        'putf(0x4800+16+4,v[9]);putf(0x4800+32+4,v[10]);store(0x4900,0x4A00);store(0x4A00+60,0x82000000);' + nl +
        'store(0x828CCFC4,0x4B00);store(0x4C00,0x4D00);*pointer(0x4D00+72)=uint8_t(c.tls);' + nl +
        'store(0x82839704,0x5000);put3(0x5000+96,v+0,0);put3(0x5000+112,v+3,1);store(0x827E5640,0);store(0x8287E064,0x5200);put3(0x5200+240,v+3,1);' + nl +
        '{Context ctx{};ctx.r1.u32=0x8F00;ctx.r13.u32=0x4C00;ctx.r3.u32=0x1000;ctx.r4.u32=0x4400;sub_%s(ctx,nullptr);}' % fr +
        'uint32_t n=load(0x1000+12672);printf("%%u",n);if(n){uint32_t e=0x1000+5504;'
        'for(int k=0;k<16;k++)printf(" %%u",load(e+16+4*k));for(int k=0;k<4;k++)printf(" %%u",load(e+80+4*k));'
        'printf(" %%u %%u",unsigned(*pointer(e+96)),load(e+100));}printf("%s");}' % (chr(92) + 'n') + nl + '}' + nl)
    # the hidden test F-54047 (shown), the depth-buffer getter F-26769 (a handle), the bound getter F-98453 (the box)
    stubs = {'sub_' + fhid: 'ctx.r3.u64=0;', 'sub_' + fdepth: 'ctx.r3.u64=0x5555;', 'sub_' + fbound: 'ctx.r3.u64=0x4800;'}
    recomp_closure.EXTRA += VIS
    exe = build(index(a.recomp), ['sub_' + fr], body, a.out_dir, 'race_marker', stubs=stubs, vector=True, scratch_end=0x9000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = []
    for c, o in zip(cs, out):
        z, cam, pos, icon, top, bottom, off, scale, trans, fade, cols, depth, tls = c
        fl = list(z) + list(cam) + list(pos) + [top, bottom, off] + list(scale) + list(trans) + list(fade) + [x for col in cols for x in col]
        lines.append(' '.join(str(bits(float('%.9g' % x))) for x in fl) + ' %d %d %d -> %s' % (icon, depth, tls, o))
    # (a line with a word the registry check reads as an address of the original - 8 digits from 8 - is left out)
    lines = [x for x in lines if not re.search(r'\b8[0-9A-Fa-f]{7}\b', x)][:4000]
    assert len(lines) == 4000, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'race_marker_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(1 for x in lines if not x.endswith('-> 0')), 'queued')


if __name__ == '__main__':
    main()
