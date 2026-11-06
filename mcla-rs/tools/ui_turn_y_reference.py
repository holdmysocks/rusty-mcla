"""The minimap's icon-matrix turn run offline (recomp_dispatch): F-45776, M = M * RotY(theta) with the rotation
built by F-92665 (the CRT sine and cosine). Each case: 16 floats of M (glam column order = the guest's word order)
and theta, as bits; the result's x y z of each row (12 words; the w lanes of the rotation are left unwritten by the
original, so its w lanes are not compared). Writes ui_turn_y_ref.txt:  <case> -> <12 words>
Usage: ui_turn_y_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
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
 uint32_t v[17];
 for(;;){
  for(int i=0;i<17;i++) if(scanf("%u",&v[i])!=1) return 0;
  rt_reset();
  uint32_t M=rt_alloc(64,16);
  for(int i=0;i<16;i++) store(M+4*i,v[i]);
  Context c=fresh();float th;memcpy(&th,&v[16],4);c.f1.f64=th;c.r3.u32=M;
  sub_TURN(c,nullptr);
  printf("OUT");for(int r=0;r<4;r++)for(int k=0;k<3;k++)printf(" %u",load(M+16*r+4*k));putchar(10);fflush(stdout);
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 3000
    r = random.Random(82239)
    fb = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
    cases = []
    for i in range(count):
        kind = r.random()
        if kind < 0.5:
            # an icon matrix: diag(s), translation an offset times s (the flag's), w 1
            s = r.uniform(0.5, 400.0)
            off = [r.uniform(-2, 2), r.choice([0.0, -0.0, r.uniform(-1, 1)]), r.uniform(-2, 2)]
            m = [s, 0, 0, 0, 0, s, 0, 0, 0, 0, s, 0, off[0] * s, off[1] * s, off[2] * s, 1.0]
        elif kind < 0.8:
            m = [r.uniform(-500, 500) for _ in range(16)]
        else:
            m = [r.choice([0.0, -0.0, 1e-39, -1e-39, 1e-30, r.uniform(-1, 1)]) for _ in range(16)]
        th = r.choice([r.uniform(-math.pi, math.pi), r.uniform(-20, 20), 0.0, -0.0, math.pi, math.pi / 2])
        cases.append([fb(x) for x in m] + [fb(th)])
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_turny'
    work.mkdir(exist_ok=True)
    turn = 'sub_' + addr('F-45776')
    exe = rd.build(idx, [turn], MAIN.replace('sub_TURN', turn), work, 'turny', image=img)
    inp = ''.join(' '.join(str(x) for x in c) + '\n' for c in cases)
    p = subprocess.run([str(exe), str(image)], input=inp, capture_output=True, text=True)
    res = [l[4:] for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join(' '.join(str(x) for x in c) + ' -> ' + o + '\n' for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
