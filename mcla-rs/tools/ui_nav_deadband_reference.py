"""The GPS map cursor's 1 m dead band run offline (recomp_dispatch): F-31695 (this, x, y, z) with the cursor NM+448 as it was.
Its outline test F-90999 is a stub that notes it was reached and answers inside, NM+496 is 0 (the visibility path) and the
movie calls are stubs, so a case tells whether the move was taken and the stored cursor. Writes ui_nav_deadband_ref.txt, one
case per line, numbers as float bits in decimal:
  old_x old_y old_z new_x new_y new_z -> moved x y z
Usage: ui_nav_deadband_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
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
# (the static recomp's guard in F-66209: an unset edge end - the position taken as it came)
GUARD = r'''}
template<class R> static bool mcla_map_cursor_guard(R&r31,R&r30,R&r26){if(r31.u32)return false;
 if(r26.u32!=r30.u32)for(int i=0;i<16;i+=4)store(r26.u32+i,load(r30.u32+i));return true;}
static void unused_guard_(Context &ctx,const uint8_t *base){'''


TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
VF = 0x7FFF0010       # the map's vtable slot 117 (+468): the movie (a fake address)
GLOBALS = 0x7FFF0020  # never called: carries the global and the recomp's guard below into the source


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t v[6];
 while(scanf("%u %u %u %u %u %u",&v[0],&v[1],&v[2],&v[3],&v[4],&v[5])==6){
  rt_reset();MOVED=0;
  uint32_t nm=rt_alloc(2048,16),vt=rt_alloc(1024,16);
  for(uint32_t i=0;i<2048;i+=4)store(nm+i,0);
  store(0x8286CE0Cu,0xFFFFFFFFu);sub_MASK(*new Context(fresh()),nullptr);sub_MASK2(*new Context(fresh()),nullptr);
  store(nm,vt);store(vt+468,VF_);store(nm+448,v[0]);store(nm+452,v[1]);store(nm+456,v[2]);store(nm+460,0);
  Context c=fresh();c.r3.u32=nm;float f;
  memcpy(&f,&v[3],4);c.f1.f64=f;memcpy(&f,&v[4],4);c.f2.f64=f;memcpy(&f,&v[5],4);c.f3.f64=f;
  sub_BAND(c,nullptr);
  printf("OUT %d %u %u %u\n",MOVED,load(nm+448),load(nm+452),load(nm+456));fflush(stdout);
 }
}
'''


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def next_after(x, up):
    b = fbits(x)
    if x == 0.0:
        return struct.unpack('>f', struct.pack('>I', 1 if up else 0x80000001))[0]
    b += 1 if (x > 0) == up else -1
    return struct.unpack('>f', struct.pack('>I', b))[0]


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 4000
    r = random.Random(31695)
    cases = []
    for _ in range(count):
        old = [f32(r.uniform(-3000, 3000)), f32(r.choice([0.0, r.uniform(-30, 80)])), f32(r.uniform(-2400, 2200))]
        new = list(old)
        for k in range(3):
            m = r.random()
            if m < 0.25:
                d = r.uniform(-1.0, 1.0)                     # inside
            elif m < 0.45:
                d = r.choice([-1.0, 1.0])                    # on the edge
            elif m < 0.6:
                d = r.uniform(-5.0, 5.0)
            elif m < 0.75:
                d = 0.0
            else:
                d = None                                     # one ulp either side of the edge
            if d is None:
                e = f32(old[k] + r.choice([-1.0, 1.0]))
                new[k] = next_after(e, r.random() < 0.5)
            else:
                new[k] = f32(old[k] + d)
        cases.append([fbits(x) for x in old + new])
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_navband'
    work.mkdir(exist_ok=True)
    band, inside, movie = 'sub_' + addr('F-31695'), 'sub_' + addr('F-90999'), 'sub_' + addr('F-59146')
    # (the xyz mask the band's test loads, filled at start-up: the word by F-50173, the vector by F-33402 / F-26199)
    mask, mask2 = 'sub_' + addr('F-33402'), 'sub_' + addr('F-26199')
    main_src = MAIN.replace('VF_', '0x%Xu' % VF).replace('sub_BAND', band).replace('sub_MASK2', mask2).replace('sub_MASK', mask)
    exe = rd.build(idx, [band, mask, mask2], main_src, work, 'navband', image=img,
                   stubs={inside: 'MOVED=1;ctx.r3.u64=1;', movie: ''},
                   fakes={GLOBALS: GUARD.replace('}', '}static int MOVED;', 1), VF: ''})
    inp = ''.join('%u %u %u %u %u %u\n' % tuple(c) for c in cases)
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[4:] for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%u %u %u %u %u %u -> %s\n' % (*c, o) for c, o in zip(cases, res)))
    print(len(res), 'cases,', sum(o.startswith('1') for o in res), 'moved')


if __name__ == '__main__':
    main()
