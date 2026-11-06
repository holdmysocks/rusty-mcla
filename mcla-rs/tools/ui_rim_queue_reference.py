"""The minimap's projection, inside test and rim-dot queue run offline (recomp_dispatch):
  mode 0: F-75199 (view_proj times (p, 1), times 1 / w) -> x y z of the result
  mode 1: F-72569 (inside: |projected| < radius; the point is +48 of the element matrix) -> the byte
  mode 2: F-40572 (the rim-dot queue) with the list push F-12952 a stub that records the angle f1, the
          colour word (as four bytes A R G B), r5, the kind r7 and the selected byte r8.
Each case: mode, 16 matrix floats (glam column order = the guest's word order), p x y z w, radius,
colour r g b a, selected, kind; floats as bits. Writes ui_rim_queue_ref.txt:
  <case> -> <outputs>
Usage: ui_rim_queue_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import math
import os
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure

# (the vector unit's flush mode as the SDK's: FTZ and DAZ on the host while the code has it on; the runner's
# default shim makes it a no-op)
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = _vshim()
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t v[30];
 for(;;){
  for(int i=0;i<30;i++) if(scanf("%u",&v[i])!=1) return 0;
  rt_reset();
  uint32_t F=rt_alloc(128,16),P=rt_alloc(16,16),C=rt_alloc(16,16),O=rt_alloc(16,16),E=rt_alloc(64,16);
  for(int i=0;i<16;i++) store(F+16+4*i,v[1+i]);
  store(F+4,0xB000u);
  for(int i=0;i<4;i++){store(P+4*i,v[17+i]);store(C+4*i,v[22+i]);store(E+48+4*i,v[17+i]);}
  Context c=fresh();
  if(v[0]==0){
   c.r3.u32=O;c.r4.u32=F+16;
   for(int i=0;i<4;i++){float f;uint32_t w=v[17+i];memcpy(&f,&w,4);c.v1.f32[3-i]=f;}
   sub_PROJ(c,nullptr);
   printf("OUT %u %u %u",load(O),load(O+4),load(O+8));putchar(10);fflush(stdout);
  } else if(v[0]==1){
   float r;memcpy(&r,&v[21],4);c.f1.f64=r;c.r3.u32=E;c.r4.u32=F+16;
   sub_INSIDE(c,nullptr);
   printf("OUT %u",c.r3.u32&255);putchar(10);fflush(stdout);
  } else {
   c.r3.u32=0;c.r4.u32=F;c.r5.u32=P;c.r7.u32=C;c.r8.u32=v[26];c.r9.u32=v[27];
   sub_QUEUE(c,nullptr);
  }
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 3000
    r = random.Random(82235)
    fb = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
    cases = []
    for i in range(count):
        mode = i % 3
        if r.random() < 0.7:
            # a perspective view_proj from a random eye and look (35 degrees, near 10, far 9000)
            eye = [r.uniform(-4000, 4000), r.uniform(5, 2000), r.uniform(-4000, 4000)]
            yaw, pitch = r.uniform(-math.pi, math.pi), r.uniform(-1.5, -0.2)
            fwd = [math.cos(pitch) * math.sin(yaw), math.sin(pitch), -math.cos(pitch) * math.cos(yaw)]
            right = [math.cos(yaw), 0.0, math.sin(yaw)]
            up = [right[1] * fwd[2] - right[2] * fwd[1], right[2] * fwd[0] - right[0] * fwd[2], right[0] * fwd[1] - right[1] * fwd[0]]
            f = 1 / math.tan(math.radians(35) / 2)
            n, fa = 10.0, 9000.0
            dot = lambda a, b: sum(x * y for x, y in zip(a, b))
            view = [right + [-dot(right, eye)], up + [-dot(up, eye)], [-x for x in fwd] + [dot(fwd, eye)], [0, 0, 0, 1]]
            proj = [[f, 0, 0, 0], [0, f, 0, 0], [0, 0, fa / (n - fa), fa * n / (n - fa)], [0, 0, -1, 0]]
            m = [[sum(proj[a][k] * view[k][b] for k in range(4)) for b in range(4)] for a in range(4)]
            cols = [m[row][col] for col in range(4) for row in range(4)]
            p = [eye[0] + r.uniform(-900, 900), r.uniform(-50, 300), eye[2] + r.uniform(-900, 900)]
        else:
            cols = [r.uniform(-3, 3) for _ in range(16)]
            p = [r.uniform(-100, 100) for _ in range(3)]
            if r.random() < 0.2:
                cols[3] = cols[7] = cols[11] = 0.0
                cols[15] = r.choice([0.0, 1e-30, -1e-30, 1.0])
            elif r.random() < 0.25:
                # (denormal rows and a large w, so the flush is exercised)
                for k in r.sample(range(16), 6):
                    cols[k] = r.choice([1e-39, -1e-39, 3e-40])
                cols[15] = r.choice([1e8, -1e8, 1.0])
                cols[3] = cols[7] = cols[11] = 0.0
        w = r.choice([1.0, 0.0, r.uniform(-5, 5)])
        rad = r.choice([1.25, r.uniform(0.1, 3.0)])
        col = [r.choice([r.random(), 0.0, 1.0]) for _ in range(4)]
        sel = r.choice([0, 1, 256])
        kind = r.choice([0, 1, 2, 10, 7])
        cases.append([mode] + [fb(x) for x in cols] + [fb(x) for x in p + [w, rad]] + [fb(x) for x in col] + [sel, kind, 0, 0])
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_rimqueue'
    work.mkdir(exist_ok=True)
    proj, inside, queue, push = ('sub_' + addr(x) for x in ('F-75199', 'F-72569', 'F-40572', 'F-12952'))
    main_src = MAIN.replace('sub_PROJ', proj).replace('sub_INSIDE', inside).replace('sub_QUEUE', queue)
    rec = ('{float a=float(ctx.f1.f64);uint32_t ab;memcpy(&ab,&a,4);uint32_t w=ctx.r6.u32;'
           'printf("OUT %u %u %u %u %u %u %u %u",ab,w>>24,(w>>16)&255,(w>>8)&255,w&255,ctx.r5.u32,ctx.r7.u32,ctx.r8.u32&255);'
           'putchar(10);fflush(stdout);}')
    exe = rd.build(idx, [proj, inside, queue], main_src, work, 'rimqueue', image=img, stubs={push: rec})
    inp = ''.join(' '.join(str(x) for x in c) + '\n' for c in cases)
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[4:] for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join(' '.join(str(x) for x in c[:28]) + ' -> ' + o + '\n' for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
