"""Run the original's checkpoint placement step F-34461 (the +160 "moved" handler: the draw point +112, the ground probe, the
gate offset +96, then the ceiling height F-64647) and the ceiling height alone offline.

Stubbed, and said so in the evidence: the level probe F-20855 (called with the segment start r4,
the end at r4 + 16 and the result record r5: it is a recording stub that prints the segment and, when the case says "hit",
writes the case's hit point (record + 16) and normal (record + 32)). Everything else - the vector arithmetic, the normalisation of
the gate offset (dot product, 1 / sqrt, one Newton step), the line interpolation, the +/- 2 m probe, the 25 m ceiling probe and
its sqrt - is the generated code unchanged. The static image holds zero where the start-up code stores +inf (0x828609E0).
Output (f32 bit words, hex):
  GATE <line> <pos 3> <second 3> <draw fraction> <hit1 0/1> <hitpos1 3> <normal1 3> <hit2> <hitpos2 3> <normal2 3> |
       P <start 3> <end 3> (ground probe) P <start 3> <end 3> (ceiling probe) | <+96 3> <+112 3> <+128 3> <+160> <+176>
  CEIL <marker 3> <hit 0/1> <hitpos 3> | P <start 3> <end 3> | <+176>
Test: `checkpoints::tests::placement_matches_the_original` (crates/game/tests/data/checkpoint_gate_reference.txt).
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import numpy as np
from recomp_closure import index, build, run


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return float(np.float32(x))


def cases():
    rnd = random.Random(20261109)
    g = []
    ints = lambda k: float(rnd.randint(-k, k))
    fr = [0.5, 0.0, 1.0, 0.25, 0.75, 0.3333, 0.1, 0.9]
    for _ in range(2400):  # lines: axis aligned and diagonal on a grid, degenerate, and random
        kind = rnd.choice(['grid', 'grid', 'rand', 'degenerate', 'tiny'])
        if kind == 'grid':
            a = (ints(40), ints(10), ints(40))
            b = (a[0] + rnd.choice([0, 3, -3, 10, -10, 25]), ints(10), a[2] + rnd.choice([0, 4, -4, 10, -10, 30]))
        elif kind == 'rand':
            a = tuple(f32(rnd.uniform(-900, 900)) for _ in range(3))
            b = tuple(f32(a[i] + rnd.uniform(-70, 70)) for i in range(3))
        elif kind == 'tiny':
            a = tuple(f32(rnd.uniform(-900, 900)) for _ in range(3))
            b = tuple(f32(a[i] + rnd.choice([1e-3, -1e-4, 1e-6, 0.0])) for i in range(3))
        else:
            a = tuple(f32(rnd.uniform(-900, 900)) for _ in range(3))
            b = (a[0], f32(a[1] + rnd.uniform(-5, 5)), a[2])
        f = rnd.choice(fr) if rnd.random() < 0.6 else f32(rnd.uniform(-0.2, 1.2))
        g.append((1, a, b, f))
    for _ in range(600):  # round checkpoints (the second position is ignored)
        a = tuple(f32(rnd.uniform(-900, 900)) for _ in range(3))
        g.append((0, a, a, 0.5))
    out = []
    for line, a, b, f in g:
        probes = []
        for _ in range(2):
            hit = rnd.random() < 0.6
            hp = tuple(f32(a[i] + rnd.uniform(-30, 30)) for i in range(3))
            hn = tuple(f32(rnd.uniform(-1, 1)) for _ in range(3))
            if rnd.random() < 0.3:
                hp = (a[0], f32(a[1] + rnd.choice([8.0, 25.0, 0.5])), a[2])
            probes.append((hit, hp, hn))
        out.append(('GATE', line, a, b, f, probes))
    ceil = []
    for _ in range(1500):
        m = tuple(f32(rnd.uniform(-900, 900)) for _ in range(3))
        hit = rnd.random() < 0.6
        hp = tuple(f32(m[i] + rnd.uniform(-40, 40)) for i in range(3))
        if rnd.random() < 0.4:
            hp = (m[0], f32(m[1] + rnd.choice([8.0, 25.0, 0.5, 24.99, 3.0])), m[2])
        ceil.append(('CEIL', m, (hit, hp, (0.0, 1.0, 0.0))))
    return out, ceil


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nm = lambda f: subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[2]
    gate, ceil = nm('F-34461'), nm('F-64647')
    probe = nm('F-20855')  # the level probe
    gs, cs = cases()

    def pr(p):
        return [int(p[0]), *[bits(v) for v in p[1]], *[bits(v) for v in p[2]]]
    garr = ','.join('{%s}' % ','.join('0x%08xu' % v for v in (c[1], *[bits(x) for x in c[2]], *[bits(x) for x in c[3]], bits(c[4]), *pr(c[5][0]), *pr(c[5][1]))) for c in gs)
    carr = ','.join('{%s}' % ','.join('0x%08xu' % v for v in (*[bits(x) for x in c[1]], *pr(c[2]))) for c in cs)
    body = '''
static const uint32_t GATES[][22]={%s};
static const uint32_t CEILS[][10]={%s};
static void vec3(const uint32_t*w){for(int i=0;i<3;i++)std::printf(" %%08x",w[i]);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int i=0;i<4;i++)store(0x828609E0+4*i,0x7f800000); // the start-up vector constant +inf (zero in the static image)
 store(0x828da464,0x1234); // the level pointer the probes test first (any non-zero)
 for(auto&c:GATES){
  std::memset(objects,0,sizeof(objects));
  // the probe records: n-th probe call reads record 0x1900 + 64 n: hit word, hit position (+16), normal (+32)
  for(int k=0;k<2;k++){uint32_t r=0x1900+64*k;store(r,c[8+7*k]);for(int i=0;i<3;i++){store(r+16+4*i,c[9+7*k+i]);store(r+32+4*i,c[12+7*k+i]);}}
  *pointer(0x1000+160)=1;*pointer(0x1000+330)=uint8_t(c[0]);
  for(int i=0;i<3;i++){store(0x1000+64+4*i,c[1+i]);store(0x1000+80+4*i,c[4+i]);}
  store(0x1000+172,c[7]);
  std::printf("GATE %%u",c[0]);for(int i=1;i<8;i++)std::printf(" %%08x",c[i]);for(int i=8;i<22;i++)std::printf(i==8||i==15?" %%u":" %%08x",c[i]);
  std::printf(" |");
  Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2800;
  sub_%s(ctx,nullptr);
  std::printf(" |");
  for(uint32_t o:{96u,112u,128u}){uint32_t w[3];for(int i=0;i<3;i++)w[i]=load(0x1000+o+4*i);vec3(w);}
  std::printf(" %%u %%08x\\n",unsigned(*pointer(0x1000+160)),load(0x1000+176));}
 for(auto&c:CEILS){
  std::memset(objects,0,sizeof(objects));
  uint32_t r=0x1900;store(r,c[3]);for(int i=0;i<3;i++){store(r+16+4*i,c[4+i]);store(r+32+4*i,c[7+i]);}
  for(int i=0;i<3;i++)store(0x1000+112+4*i,c[i]);
  std::printf("CEIL");for(int i=0;i<3;i++)std::printf(" %%08x",c[i]);std::printf(" %%u",c[3]);for(int i=4;i<7;i++)std::printf(" %%08x",c[i]);
  std::printf(" |");
  Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2800;
  sub_%s(ctx,nullptr);
  std::printf(" %%08x\\n",load(0x1000+176));}
}
''' % (garr, carr, gate[4:], ceil[4:])
    rec = ('uint32_t n=load(0x2F00);store(0x2F00,n+1);uint32_t rec=0x1900+64*n;'
           'std::printf(" P");for(int i=0;i<3;i++)std::printf(" %08x",load(ctx.r4.u32+4*i));for(int i=0;i<3;i++)std::printf(" %08x",load(ctx.r4.u32+16+4*i));'
           'if(load(rec)){for(int i=0;i<4;i++){store(ctx.r5.u32+16+4*i,load(rec+16+4*i));store(ctx.r5.u32+32+4*i,load(rec+32+4*i));}ctx.r3.u32=1;}else ctx.r3.u32=0;')
    exe = build(index(a.recomp), [gate, ceil], body, a.out, 'cp_gate', {probe: rec}, vector=True)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
