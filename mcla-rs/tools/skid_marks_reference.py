"""The skid marks' per-wheel update F-48637 run offline on sequences of frames: the generated code whole and unchanged in
vector mode, with the mark decision F-52604, the suspension's clamp F-92300, the segment writers F-99065 / F-56849 /
F-89324 and the colour pack F-44647 in the closure, on flat guest memory. Nothing is stubbed.

A sequence is one 128-point trail (the traffic's size, so the ring wraps) at T = 0x1100, initialised as F-73020 /
F-29621 leave it (+0 .. +12 = 0, 0, 0, 1; +16 = 127; +104 = 128; +108 = 256; +28 = -1; the vertices zero but each pair's
the rest as the reset makes it: it is run), and one wheel W at 0x4600 (+4 its tune at
0x4900, +336 the ground normal, +368 the contact point, +464 the material at 0x4A00 or 0, +496 the compression, +520 the
slide) whose values are the frame's. The system object (the global 0x8287E18C) at 0x4400 has the colour of the set-up
F-57145 at +208: (0.01, 0.01, 0.01, 0.5). The trail's vertex ring is at 0x2000 (128 x 64 bytes), its dirty words at
0x4000. The reset F-29621 is run on the trail first, with three dummy vertex buffers of one vertex each, marked locked (byte
+6), so that it clears a few bytes and calls nothing; its memset (the runtime's, F-29294) is the host's.

Writes <out-dir>/skid_marks_ref.txt: per sequence a line 'q' with the tune (width, travel limit, travel extent,
suspension limit) and the three materials (ptxindex 0, 1 as words, ptxthreshold 0, 1); per frame a line 'f' with the
frame's inputs (contact 3, normal 3, compression, slide, material 0..3) '->' +28, +32 .. +44 (3), +48 .. +56 (3), +96,
+100, +112, +116 / +118 bytes; after the last frame a line 'r' with the ring's 128 x 16 words and a line 'd' with the 128
dirty words (all hex; a word from 82000000 to 82ffffff has an 'x' in front)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx.h>' + chr(10) + _vshim()
    # (the signed halves the generated code reads: extsh, extsb)
    s = s.replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;', 1)
    # (recomp_closure's vector shim keeps the scalar shim's default scratch, 0x1000 .. 0x3000: this one needs to 0x6000)
    s = s.replace('objects[8192]', 'objects[0x5000]', 1).replace('a<12288', 'a<0x6000', 1)
    assert 'objects[0x5000]' in s and 'a<0x6000' in s
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("unexpected indirect call")\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

SYS_PTR = ((-32120 << 16) - 7796) & 0xFFFFFFFF
N = 128


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def unit(v):
    n = math.sqrt(sum(x * x for x in v))
    return [x / n for x in v]


def sequence(rnd, frames):
    width = rnd.choice((0.2, 0.235, 0.265, 0.3, rnd.uniform(0.15, 0.35)))
    tune = [width, rnd.uniform(0.1, 0.3), rnd.uniform(0.1, 0.3), rnd.choice((0.1, 0.12, rnd.uniform(0.05, 0.2)))]
    mats = []
    for _ in range(3):
        ix = [rnd.choice((-1, 0, 1, 2, 3, 5, 7, 8)) for _ in range(2)]
        th = [rnd.choice((0.0, 0.2, 0.5, 0.8, 0.9, rnd.uniform(0, 1))) for _ in range(2)]
        mats.append([x & 0xFFFF for x in ix] + [fb(f32(x)) for x in th])
    pos = [rnd.uniform(-1500, 1500), rnd.uniform(0, 60), rnd.uniform(-1500, 1500)]
    head = rnd.uniform(0, 2 * math.pi)
    speed = rnd.uniform(0, 30)
    slide, comp = rnd.uniform(0, 1), rnd.uniform(-0.1, 0.3)
    mat = rnd.randrange(4)
    fr = []
    for _ in range(frames):
        r = rnd.random()
        if r < 0.02:
            head += rnd.uniform(-3, 3)
        elif r < 0.25:
            head += rnd.gauss(0, 0.15)
        else:
            head += rnd.gauss(0, 0.02)
        if rnd.random() < 0.05:
            speed = rnd.choice((0.0, rnd.uniform(0, 5), rnd.uniform(5, 40), 4.95, 60.0))
        if rnd.random() < 0.01:
            pos = [pos[0] + rnd.uniform(-8, 8), pos[1], pos[2] + rnd.uniform(-8, 8)]
        step = speed / 30
        pos = [pos[0] + step * math.cos(head), pos[1] + rnd.gauss(0, 0.002), pos[2] + step * math.sin(head)]
        if rnd.random() < 0.08:
            slide = rnd.choice((0.0, rnd.uniform(0, 1), 0.9, 1.0, -0.2))
        else:
            slide = min(1.2, max(-0.2, slide + rnd.gauss(0, 0.03)))
        if rnd.random() < 0.08:
            comp = rnd.choice((rnd.uniform(-0.2, 0.4), 0.0, -0.05))
        if rnd.random() < 0.03:
            mat = rnd.randrange(4)
        nrm = unit([rnd.gauss(0, 0.05), 1.0, rnd.gauss(0, 0.05)])
        fr.append([fb(f32(x)) for x in pos] + [fb(f32(x)) for x in nrm] + [fb(f32(comp)), fb(f32(slide)), mat])
    return [fb(f32(x)) for x in tune] + sum(mats, []), fr


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--seqs', type=int, default=40)
    ap.add_argument('--frames', type=int, default=400)
    a = ap.parse_args()
    upd, reset, memset = (addr(a.tool, f) for f in ('F-48637', 'F-29621', 'F-29294'))
    rnd = random.Random(48637)
    seqs = [sequence(rnd, a.frames) for _ in range(a.seqs)]
    nl = chr(10)
    NL = chr(92) + 'n'
    qrows = nl.join('{%s},' % ','.join('0x%08x' % x for x in q) for q, _ in seqs)
    frows = nl.join('{%s},' % ','.join('0x%08x' % x for x in f) for _, fr in seqs for f in fr)
    T, W, TU, MAT, SYS, RING, DIRTY = 0x1100, 0x4600, 0x4900, 0x4A00, 0x4400, 0x2000, 0x4000
    body = ('static const uint32_t Q[][%d]={' % len(seqs[0][0]) + nl + qrows + nl + '};' + nl +
            'static const uint32_t F[][9]={' + nl + frows + nl + '};' + nl +
            'store(0x%08x,0x%x);' % (SYS_PTR, SYS) +
            'for(int s=0;s<%d;s++){const uint32_t*q=Q[s];memset(objects,0,sizeof(objects));' % len(seqs) +
            'store(0x%x+208,0x3c23d70a);store(0x%x+212,0x3c23d70a);store(0x%x+216,0x3c23d70a);store(0x%x+220,0x3f000000);' % (SYS, SYS, SYS, SYS) +
            # (the trail as F-73020 makes it and F-29621 leaves it, written out: the reset's vertex loop below)
            'store(0x%x+12,0x3f800000);store(0x%x+16,%d);store(0x%x+104,%d);store(0x%x+108,%d);store(0x%x+120,0x%x);store(0x%x+124,0x%x);' % (
                T, T, N - 1, T, N, T, 2 * N, T, RING, T, DIRTY) +
            # (three dummy vertex buffers: one vertex each at 0x5800 .. so the reset's clears stay small, marked locked)
            'store(0x%x+336,0x5700);objects[0x5700-0x1000+4]=32;for(int b=0;b<3;b++){uint32_t o=0x5740+16*b;store(0x%x+128+4*b,o);'
            'store16(o+4,1);objects[o-0x1000+6]=1;store(o+8,0x5800+64*b);}' % (SYS, T) +
            '{Context r{};r.r1.u32=0x5F00;r.r3.u32=0x%x;sub_%s(r,nullptr);}' % (T, reset) +
            'store(0x%x+4,0x%x);for(int k=0;k<4;k++)store(0x%x+8+4*(k==0?0:(k==1?16:(k==2?17:19))),q[k]);' % (W, TU, TU) +
            'for(int m=0;m<3;m++){uint32_t b=0x%x+64*m;store16(b+44,q[4+4*m]);store16(b+46,q[5+4*m]);store(b+48,q[6+4*m]);store(b+52,q[7+4*m]);}' % MAT +
            'for(int i=0;i<%d;i++){const uint32_t*f=F[s*%d+i];' % (a.frames, a.frames) +
            'store(0x%x+368,f[0]);store(0x%x+372,f[1]);store(0x%x+376,f[2]);store(0x%x+336,f[3]);store(0x%x+340,f[4]);store(0x%x+344,f[5]);' % (W, W, W, W, W, W) +
            'store(0x%x+496,f[6]);store(0x%x+520,f[7]);store(0x%x+464,f[8]?0x%x+64*(f[8]-1):0);' % (W, W, W, MAT) +
            'Context ctx{};ctx.r1.u32=0x5F00;ctx.r3.u32=0x%x;ctx.r4.u32=0x%x;sub_%s(ctx,nullptr);' % (T, W, upd) +
            'printf("%%08x %%08x %%08x %%08x %%08x %%08x %%08x %%08x %%08x %%08x %%02x %%02x%s",' % NL +
            'load(0x%x+28),load(0x%x+32),load(0x%x+36),load(0x%x+40),load(0x%x+48),load(0x%x+52),load(0x%x+56),load(0x%x+96),' % (T, T, T, T, T, T, T, T) +
            'load(0x%x+100),load(0x%x+112),objects[0x%x-0x1000+116],objects[0x%x-0x1000+118]);}' % (T, T, T, T) +
            'printf("r");for(int k=0;k<%d;k++)printf(" %%08x",load(0x%x+4*k));printf("%s");' % (N * 16, RING, NL) +
            'printf("d");for(int k=0;k<%d;k++)printf(" %%08x",load(0x%x+4*k));printf("%s");}' % (N, DIRTY, NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + upd, 'sub_' + reset], main_src, a.out_dir, 'skid_marks', vector=True, scratch_end=0x6000,
                stubs={'sub_' + memset: 'memset(pointer(ctx.r3.u32),ctx.r4.u8,ctx.r5.u32);'})
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == a.seqs * (a.frames + 2), (len(out), a.seqs)
    lines = []
    k = 0
    for q, fr in seqs:
        lines.append('q ' + ' '.join('%x' % x for x in q))
        for f in fr:
            lines.append('f ' + ' '.join('%x' % x for x in f) + ' -> ' + out[k])
            k += 1
        lines += [out[k], out[k + 1]]
        k += 2
    # (a word that reads like an address - eight digits from 82 - is written with an 'x' in front: the registry's
    # check takes those for raw addresses; the test strips it)
    lines = [' '.join('x' + wd if len(wd) == 8 and wd.startswith('82') else wd for wd in l.split(' ')) for l in lines]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'skid_marks_ref.txt').write_text(nl.join(lines) + nl)
    fl = [l for l in lines if l.startswith('f ')]
    print(len(seqs), 'sequences,', len(fl), 'frames; marking', sum(l.split('-> ')[1].split()[0] != 'ffffffff' for l in fl),
          '; wrapped', sum(int(l.split('-> ')[1].split()[8], 16) < 5 for l in fl))


if __name__ == '__main__':
    main()
