"""The GPS map's outline run offline (recomp_dispatch): the point-in-outline test F-90999 and the pull-back F-66209 (with
F-37810, F-37290, F-36590, F-56767, F-66886, F-95591, the angle F-88594 and cos as they are) on the "sc" outline of F-93744.
Writes ui_nav_outline_ref.txt, one case per line, numbers as float bits in hex:
  I x z -> 0|1                       F-90999 (x, z, the outline, 9)
  P px py pz gx gy gz -> ox oy oz    F-66209 (the outline, &prev, &g, &out)
Usage: ui_nav_outline_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd

_vshim = recomp_closure.vector_shim


def vector_shim():
    # (the vector compares set cr6 from their mask; the vector unit flushes denormals - as crossing_unspawn_reference.py)
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)
GLOBALS = 0x7FFF0020  # never called: carries the recomp's guard below into the source
# (the static recomp's guard in F-66209: an unset edge end - the position taken as it came)
GUARD = r'''}
template<class R> static bool mcla_map_cursor_guard(R&r31,R&r30,R&r26){if(r31.u32)return false;
 if(r26.u32!=r30.u32)for(int i=0;i<16;i+=4)store(r26.u32+i,load(r30.u32+i));return true;}
static void unused_guard_(Context &ctx,const uint8_t *base){'''

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
OUTLINE = [(-3100, -1200), (-1900, -2400), (590, -2400), (2740, -1079), (2830, 217), (1312, 2173), (-1235, 2173),
           (-3368, 1073), (-3484, 558)]


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 char kind[4];
 while(scanf("%3s",kind)==1){
  rt_reset();
  // (the start-up xyz compare mask, zero in the image; the pull-back's depth counter)
  store(0x82860950u,0xFFFFFFFFu);store(0x82860954u,0xFFFFFFFFu);store(0x82860958u,0xFFFFFFFFu);store(0x8286095Cu,0);
  store(0x829061A8u,0);
  uint32_t po=rt_alloc(256,16),v=rt_alloc(64,16);
  for(uint32_t i=0;i<256;i+=4)store(po+i,0);
  OUTLINE_
  store(po+160,9);
  printf("OUT");
  if(kind[0]=='I'){
   uint32_t x,z;scanf("%x %x",&x,&z);
   Context c=fresh();float fx,fz;memcpy(&fx,&x,4);memcpy(&fz,&z,4);c.f1.f64=fx;c.f2.f64=fz;c.r5.u32=po;c.r6.u32=9;
   sub_IN(c,nullptr);printf(" %d",int(c.r3.u32&1));
  }else{
   uint32_t a[6];for(int i=0;i<6;i++)scanf("%x",&a[i]);
   for(int i=0;i<3;i++){store(v+4*i,a[i]);store(v+16+4*i,a[3+i]);}store(v+12,0);store(v+28,0);
   Context c=fresh();c.r3.u32=po;c.r4.u32=v;c.r5.u32=v+16;c.r6.u32=v+32;sub_PULL(c,nullptr);
   printf(" %x %x %x",load(v+32),load(v+36),load(v+40));
  }
  printf("\n");fflush(stdout);
 }
}
'''


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 4000
    r = random.Random(66209)
    pts = [(float(x), float(z)) for x, z in OUTLINE]

    def inside_pt():
        # (a point near the outline's inner side, or anywhere in its box)
        if r.random() < 0.5:
            i = r.randrange(9)
            (ax, az), (bx, bz) = pts[i], pts[(i + 1) % 9]
            t = r.random()
            return ax + (bx - ax) * t + r.uniform(-300, 300) * 0.0 + r.uniform(-1, 1) * r.choice([0.5, 30, 300]), az + (bz - az) * t + r.uniform(-1, 1) * r.choice([0.5, 30, 300])
        return r.uniform(-3600, 2900), r.uniform(-2500, 2300)

    cases = []
    for k in range(count):
        if k % 4 == 0:
            x, z = inside_pt() if r.random() < 0.7 else r.choice(pts)
            cases.append('I %x %x' % (fbits(x), fbits(z)))
            continue
        kind = r.random()
        if kind < 0.1:
            px, pz = r.choice(pts)                       # from a vertex (the equal-vertex branch)
        else:
            px, pz = inside_pt()
        gx, gz = px + r.uniform(-1, 1) * r.choice([5, 100, 800, 3000]), pz + r.uniform(-1, 1) * r.choice([5, 100, 800, 3000])
        if kind > 0.95:
            gx, gz = px, pz
        py = 0.0 if r.random() < 0.9 else r.uniform(-5, 5)
        cases.append('P %x %x %x %x %x %x' % (fbits(px), fbits(py), fbits(pz), fbits(gx), 0, fbits(gz)))
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_navoutline'
    work.mkdir(exist_ok=True)
    f = {k: 'sub_' + addr(k) for k in ('F-90999', 'F-66209')}
    outline = ''.join('store(po+%d,0x%Xu);store(po+%d,0x%Xu);' % (16 * i, fbits(x), 16 * i + 8, fbits(z)) for i, (x, z) in enumerate(pts))
    main_src = MAIN.replace('OUTLINE_', outline).replace('sub_IN', f['F-90999']).replace('sub_PULL', f['F-66209'])
    exe = rd.build(idx, list(f.values()), main_src, work, 'navoutline', image=img, fakes={GLOBALS: GUARD})
    inp = '\n'.join(cases) + '\n'
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[3:].strip('\n') for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%s ->%s\n' % (c, o) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
