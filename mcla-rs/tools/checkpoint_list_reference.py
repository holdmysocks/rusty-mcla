"""Run the checkpoint list handlers of the original offline: FindClosestOpenCheckpoint (F-98939), GetWhichHitCheckpoint
(F-37765, with the real hit test F-78734 and its callees), and the Activate (F-37816) / Deactivate (F-15277) handlers.

Stubbed, and said so in the evidence: the car's position getter (F-35658, a pointer function that picks a row of the
car record: the stub returns the harness's position vector; only FindClosest uses it) and, for the two handlers, nothing
(their globals - the manager pointer word 0x8286ecb0 - are set in the harness memory).
A checkpoint list is built at 0x2000 (368-byte objects: +64 position, +80 second, +96 gate offset, +168 radius, +180 act
height, +184 depth, +200 + 4 racer the state word, +264 + 4 racer the timer, +330 line flag).
Output (f32 bit words, hex):
  CLOSEST <n> <racer> <pos 3> then n x (<cp pos 3> <state>) | <index or -1>
  WHICH <n> <racer> <ahead 3> <prev 3> then n x (<line> <pos 3> <second 3> <radius> <act> <depth> <offset 3> <state>) | <index or -1>
  ACT <racer> <cp> <look> <cp count> <state before> <timer before> <look before> | <look after> <state after> <timer after> <others unchanged 0/1>
  DEACT <racer> <cp> <cp count> <state before> <timer before> | <state after> <timer after> <others unchanged 0/1>
Test: `checkpoints::tests::list_handlers_match_the_original` (crates/game/tests/data/checkpoint_list_reference.txt).
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


def step(x, n):
    y = np.float32(x)
    for _ in range(abs(n)):
        y = np.nextafter(y, np.float32(np.inf if n > 0 else -np.inf))
    return float(y)


def normal10(a, b):
    dx, dz = f32(b[0] - a[0]), f32(b[2] - a[2])
    l = math.sqrt(dx * dx + dz * dz)
    if l < 1e-9:
        return (0.0, 0.0, 0.0)
    return (f32(dz / l * 10), 0.0, f32(-dx / l * 10))


def closest_cases():
    rnd = random.Random(20261109)
    out = []
    for _ in range(2500):
        n = rnd.randint(1, 12)
        racer = rnd.randint(0, 15)
        p = tuple(f32(rnd.uniform(-500, 500)) for _ in range(3))
        cps = []
        for _ in range(n):
            kind = rnd.random()
            if kind < 0.15 and cps:  # a tie: the same place as an earlier one
                c = cps[rnd.randrange(len(cps))][0]
            elif kind < 0.25:  # an integer grid place (exact distances)
                c = (float(rnd.randint(-20, 20)), float(rnd.randint(-5, 5)), float(rnd.randint(-20, 20)))
                p = (float(rnd.randint(-20, 20)), float(rnd.randint(-5, 5)), float(rnd.randint(-20, 20))) if not cps else p
            elif kind < 0.3:  # far away: beyond the 25000 start value
                c = tuple(f32(rnd.uniform(-60000, 60000)) for _ in range(3))
            else:
                c = tuple(f32(p[i] + rnd.uniform(-300, 300)) for i in range(3))
            cps.append((c, rnd.choice([0, 0, 0, 1, 2, 3])))
        out.append((n, racer, p, cps))
    return out


def which_cases():
    rnd = random.Random(20261110)
    out = []
    for _ in range(2500):
        n = rnd.randint(1, 8)
        racer = rnd.randint(0, 15)
        base = (float(rnd.randint(-6, 6)), float(rnd.randint(-2, 2)), float(rnd.randint(-6, 6)))
        prev = tuple(f32(base[i] + rnd.choice([-6, -3, -1, 0, 1, 3, 6]) + rnd.choice([0, 0.5])) for i in range(3))
        ahead = tuple(f32(prev[i] + rnd.choice([-2.0, -1.0, 0.0, 1.0, 2.0])) for i in range(3))
        cps = []
        for _ in range(n):
            line = rnd.random() < 0.4
            pos = tuple(f32(base[i] + rnd.choice([-4, -2, 0, 0, 2, 4])) for i in range(3))
            sec = tuple(f32(pos[i] + rnd.choice([-8, -4, 0, 4, 8])) for i in range(3)) if line else (0.0, 0.0, 0.0)
            radius = rnd.choice([2.0, 5.0, 10.0, 12.5, 20.0])
            act = rnd.choice([4.0, 10.0, 50.0])
            cps.append((int(line), pos, sec, radius, act, 3.0, normal10(pos, sec) if line else (0.0, 0.0, 0.0), rnd.choice([0, 0, 2, 2, 1, 3])))
        out.append((n, racer, ahead, prev, cps))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nm = lambda f: subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[2]
    closest, which, act, deact = nm('F-98939'), nm('F-37765'), nm('F-37816'), nm('F-15277')
    cl, wh = closest_cases(), which_cases()
    H = lambda x: '0x%08xu' % x
    carr = ','.join('{%s}' % ','.join(H(v) for v in (c[0], c[1], *[bits(x) for x in c[2]], *[w for cp in c[3] for w in (*[bits(x) for x in cp[0]], cp[1])])) for c in cl)
    warr = ','.join('{%s}' % ','.join(H(v) for v in (c[0], c[1], *[bits(x) for x in c[2]], *[bits(x) for x in c[3]],
                                                      *[w for cp in c[4] for w in (cp[0], *[bits(x) for x in (*cp[1], *cp[2], cp[3], cp[4], cp[5], *cp[6])], cp[7])])) for c in wh)
    rnd = random.Random(99)
    aa = [(rnd.randint(0, 15), rnd.randint(0, 5), rnd.choice([0, 1, 2, 3, 4, 7, 0x12345678]), 6, rnd.choice([0, 1, 2, 3]), bits(f32(rnd.uniform(0, 1)))) for _ in range(600)]
    aarr = ','.join('{%s}' % ','.join(H(v) for v in x) for x in aa)
    body = '''
static const uint32_t CLOSEST[][2+3+12*4]={%s};
static const uint32_t WHICH[][2+6+8*14]={%s};
static const uint32_t ACTS[][6]={%s};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});std::setvbuf(stdout,nullptr,_IONBF,0);
 const uint32_t A=0x2000;
 for(auto&c:CLOSEST){
  std::memset(objects,0,sizeof(objects));
  uint32_t n=c[0],racer=c[1];
  store(0x1100+4,A);store16(0x1100+8,n);
  for(int i=0;i<3;i++)store(0x1500+4*i,c[2+i]);
  for(uint32_t k=0;k<n;k++){for(int i=0;i<3;i++)store(A+368*k+64+4*i,c[5+4*k+i]);store(A+368*k+200+4*racer,c[5+4*k+3]);}
  std::printf("CLOSEST %%u %%u",n,racer);for(int i=2;i<5+4*n;i++)std::printf(" %%08x",c[i]);std::printf(" |");
  Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r5.u32=0x1400;ctx.r6.u32=racer;ctx.r1.u32=0xF000;
  sub_%s(ctx,nullptr);
  std::printf(" %%d\\n",ctx.r3.u32?int((ctx.r3.u32-A)/368):-1);}
 for(auto&c:WHICH){
  std::memset(objects,0,sizeof(objects));
  uint32_t n=c[0],racer=c[1];
  store(0x1000+32,0x1100);store(0x1100+4,A);store16(0x1100+8,n);
  for(int i=0;i<3;i++){store(0x1200+4*i,c[2+i]);store(0x1210+4*i,c[5+i]);}
  for(uint32_t k=0;k<n;k++){const uint32_t*q=c+8+14*k;uint32_t o=A+368*k;
   *pointer(o+330)=uint8_t(q[0]);for(int i=0;i<3;i++){store(o+64+4*i,q[1+i]);store(o+80+4*i,q[4+i]);store(o+96+4*i,q[10+i]);}
   store(o+168,q[7]);store(o+180,q[8]);store(o+184,q[9]);store(o+200+4*racer,q[13]);}
  std::printf("WHICH %%u %%u",n,racer);for(int i=2;i<8+14*n;i++)std::printf(" %%08x",c[i]);std::printf(" |");
  Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=racer;ctx.r5.u32=0x1200;ctx.r6.u32=0x1210;ctx.r1.u32=0xF000;
  sub_%s(ctx,nullptr);
  std::printf(" %%d\\n",int(ctx.r3.s32));}
 // the Activate / Deactivate handlers: r3 = a script argument block whose word +8 points at (racer, cp, look)
 store(0x8286ecb0,0x1000); // the global word the handlers read the manager pointer from
 for(int pass=0;pass<2;pass++)for(auto&c:ACTS){
  std::memset(objects,0,sizeof(objects));
  uint32_t racer=c[0],cp=c[1],look=c[2],n=c[3],st=c[4],tm=c[5];
  store(0x1000+32,0x1100);store(0x1100+4,A);store16(0x1100+8,n);
  store(0x1300,racer);store(0x1304,cp);store(0x1308,look);store(0x1008,0x1300);
  for(uint32_t k=0;k<n;k++)for(uint32_t r=0;r<16;r++){uint32_t o=A+368*k;store(o+4*r,0x1000+k*16+r);store(o+200+4*r,(k+r)%%4);store(o+264+4*r,0x3f000000+k*16+r);}
  uint32_t o=A+368*cp;store(o+200+4*racer,st);store(o+264+4*racer,tm);
  uint32_t lookb=load(o+4*racer);
  std::vector<uint8_t> before(objects,objects+sizeof(objects));
  Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0xF000;
  if(pass==0)sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);
  int same=1;for(uint32_t i=0;i<sizeof(objects);i++){uint32_t a=0x1000+i;bool mine=a==o+4*racer||a==o+4*racer+1||a==o+4*racer+2||a==o+4*racer+3
    ||(a>=o+200+4*racer&&a<o+204+4*racer)||(a>=o+264+4*racer&&a<o+268+4*racer);if(!mine&&before[i]!=objects[i])same=0;}
  if(pass==0)std::printf("ACT %%u %%u %%08x %%u %%u %%08x %%08x | %%08x %%u %%08x %%d\\n",racer,cp,look,n,st,tm,lookb,load(o+4*racer),load(o+200+4*racer),load(o+264+4*racer),same);
  else std::printf("DEACT %%u %%u %%u %%u %%08x | %%u %%08x %%d\\n",racer,cp,n,st,tm,load(o+200+4*racer),load(o+264+4*racer),same);}
}
''' % (carr, warr, aarr, closest[4:], which[4:], act[4:], deact[4:])
    # the position getter stub (only FindClosest calls it): the car record's position row is the vector at 0x1500
    stubs = {nm('F-35658'): 'ctx.r3.u32=0x14D0;'}
    exe = build(index(a.recomp), [closest, which, act, deact], body, a.out, 'cp_list', stubs, vector=True, scratch_end=0x10000)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
