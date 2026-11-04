"""The animation frame's per-DOF blend run offline: the generated code whole and unchanged (vector mode), on flat guest
memory, with the flush mode the SDK's (FTZ and DAZ on the host while the code has it on).

Parts (one exe; the mode letter is the case's first word):
- 'B': crFrame::Blend F-27951 (w clamped to [0, 1]) -> the dispatcher F-63377 -> the same-layout blend F-56920, with
  no filter and both frames of one layout (the path the game takes: reanimate_crossfade.md). Its other paths (filter,
  other layout: F-61066, F-18341, F-57365, F-92421, F-27171, F-67228, F-26296) and F-56920's unused tail (F-89700,
  F-32159) are stubs that throw. The destination frame 0x1000 (+8 the layout, +12 the DOF pointers 0x1100, u16 +16
  the count), the source 0x1040 (pointers 0x1180); DOF i of the destination at 0x2000 + 32 i, of the source at
  0x3000 + 32 i (+4 the flag byte: low nibble the type, 0x10 = no value; +16 the 16-byte value).
- 'V' / 'Q' / 'F': the DOF's Blend (vt#4) of crFrameDofVector3 F-97455, crFrameDofQuaternion F-53163 and
  crFrameDofFloat F-50451 on the DOF 0x2000, the track sample (F-57894 / F-43181 / F-11500) a stub that writes the
  case's sample to its out pointer (r5); f1 = t (unused once stubbed), f2 = w.
- 'v' / 'q' / 'f': the threshold take-over (vt#6) F-45588 / F-77946 (with the rotation angle F-88592, the C library's
  acos F-68873 in the closure) / F-75044: this 0x2000, the other 0x2040, f1 = r.
- 's' / 'k': Scale (vt#7) F-19412 (vector) / F-96406 (float), f1. 'm' / 'M': Mirror (vt#8) F-23634 / F-19630, r4 the
  axis. 'r' / 'R' / 'z': Reset (vt#10) F-12453 / F-87045 / F-89552. 'e' / 'E' / 'g': Set F-80553 / F-10930 (r4 -> 16
  bytes at 0x2100) / F-58690 (f1). 'w' / 'W': Swap (vt#9) F-94553 (Vector3 and Quaternion) / F-81105 (float).
Writes <out-dir>/frame_blend_ref.txt: the case's words, '->', the DOF(s) after (flag byte, then the four value words);
words as hex, a word whose top byte is 0x82 written as two 16-bit halves hhhh_llll."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

recomp_closure.shim = lambda: recomp_leaf.shim(0x10000)
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t){throw std::runtime_error("indirect");}\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                         'static uint32_t W[96];\n')
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    # (the SDK's flush mode: FTZ | DAZ in MXCSR while the code has it on)
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u))[0]


def word(x):
    return '%04x_%04x' % (x >> 16, x & 0xFFFF) if x >> 24 == 0x82 else '%x' % x


MODES = 'BVQFvqfskmMrRzeEgwW'


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = dict(blend='F-27951', disp='F-63377', same='F-56920', other='F-26296', v4='F-97455', q4='F-53163', f4='F-50451',
               vs='F-57894', qs='F-43181', fs='F-11500', v6='F-45588', q6='F-77946', f6='F-75044', vsc='F-19412',
               fsc='F-96406', vmi='F-23634', qmi='F-19630', vre='F-12453', qre='F-87045', fre='F-89552', vse='F-80553',
               qse='F-10930', fse='F-58690', sw='F-94553', fsw='F-81105')
    f = {k: addr(a.tool, v) for k, v in ids.items()}
    throw = {'sub_' + addr(a.tool, x): 'throw std::runtime_error("path");' for x in
             ('F-61066', 'F-18341', 'F-57365', 'F-92421', 'F-27171', 'F-67228', 'F-26296', 'F-89700', 'F-32159')}
    rnd = random.Random(56920)
    W1, W999, WH = fb(0.001), fb(0.999), fb(0.5)

    def weight():
        r = rnd.random()
        if r < 0.45:
            # at the thresholds, a float step either side
            # (a step below +0 wraps to a negative NaN)
            return (rnd.choice((W1, W999, WH, fb(1.0), 0, fb(1e-30))) + rnd.choice((-2, -1, 0, 1, 2))) & 0xFFFFFFFF
        if r < 0.5:
            return rnd.choice((0x80000000, 0x00000001, 0x007FFFFF, 0x80000001, fb(1.0) + 1, fb(-0.0001), fb(-1.0),
                               fb(2.0), 0x7FC00000, 0xFFC00000, 0x7F800000, 0xFF800000))
        if r < 0.6:
            return fb(rnd.uniform(0.0, 0.002))
        if r < 0.7:
            return fb(rnd.uniform(0.998, 1.0))
        return fb(rnd.random())

    def scalar():
        r = rnd.random()
        if r < 0.06:
            return rnd.choice((0, 0x80000000, 1, 0x80000001, 0x007FFFFF, 0x807FFFFF, 0x00400000, 0x7F7FFFFF, 0x7F800000,
                               0xFF800000, 0x7FC00000, 0x7FA00000, 0xFFC00001))
        if r < 0.12:
            return fb(rnd.choice((1, -1)) * rnd.uniform(0, 2) * 2.0 ** rnd.randint(-140, -120))
        if r < 0.2:
            return fb(rnd.uniform(-1e6, 1e6))
        return fb(rnd.uniform(-3, 3))

    def vec():
        return [scalar() for _ in range(4)]

    def unit():
        q = [rnd.gauss(0, 1) for _ in range(4)]
        n = math.sqrt(sum(x * x for x in q)) or 1.0
        return [x / n for x in q]

    def quat_pair():
        """this and the other: unit, nearly opposite, the dot about 0 either side, unnormalised, zero, special"""
        d = unit()
        r = rnd.random()
        if r < 0.25:
            # the dot about 0: the other perpendicular to this, then a tiny bit of this either way
            p = unit()
            k = sum(x * y for x, y in zip(p, d))
            p = [x - k * y for x, y in zip(p, d)]
            n = math.sqrt(sum(x * x for x in p))
            e = rnd.choice((0.0, 1e-7, -1e-7, 1e-4, -1e-4, 1e-38, -1e-38))
            s = [x / n + e * y for x, y in zip(p, d)]
        elif r < 0.4:
            s = [-x + rnd.uniform(-0.01, 0.01) for x in d]
        elif r < 0.55:
            s = [x + rnd.uniform(-0.05, 0.05) for x in d]
        elif r < 0.7:
            s = unit()
        elif r < 0.8:
            s = [x * rnd.uniform(0.5, 2.0) for x in unit()]
            d = [x * rnd.uniform(0.5, 2.0) for x in d]
        else:
            return ([fb(x) for x in d] if rnd.random() < 0.5 else vec()), vec()
        dd, ss = [fb(x) for x in d], [fb(x) for x in s]
        if rnd.random() < 0.05:
            dd = rnd.choice(([0, 0, 0, 0], [0x80000000] * 4, [1, 0, 0x80000001, 0], [0x7FC00000, 0, 0, fb(1.0)]))
        return (dd, ss) if rnd.random() < 0.5 else (ss, dd)

    def flag():
        # the type nibble (0 Vector3, 1 Quaternion, 2 Float, 3 and others), 0x10 the 'no value' bit, other high bits
        return rnd.choice((0, 1, 2, 3, 0, 1, 2, rnd.randint(4, 15))) | (0x10 if rnd.random() < 0.2 else 0) | rnd.choice((0, 0, 0, 0x20, 0x40, 0x80, 0xE0))

    def vflag():
        return (0x10 if rnd.random() < 0.25 else 0) | rnd.choice((0, 0, 0, 0x20, 0xC0, 0x0F, 0x01))

    cases = []
    for _ in range(2000):
        n = rnd.choice((1, 2, 3, 4, 6))
        c = ['B', weight(), n]
        for _ in range(n):
            fd, fs = flag(), flag()
            if rnd.random() < 0.5:
                fs = (fs & 0xF0) | (fd & 0x0F)
            t = fd & 0xF
            if t == 1 or rnd.random() < 0.1:
                d, s = quat_pair()
            else:
                d, s = vec(), vec()
            c += [fd] + d + [fs] + s
        cases.append(c)
    for _ in range(1000):
        d, s = quat_pair()
        cases.append(['Q', weight(), scalar(), vflag()] + d + s)
        cases.append(['V', weight(), scalar(), vflag()] + vec() + vec())
        cases.append(['F', weight(), scalar(), vflag()] + vec() + vec())
    for _ in range(400):
        for m in 'vqf':
            r = rnd.choice((scalar(), fb(rnd.uniform(0, 3)), fb(rnd.uniform(0, 0.1)), 0, 0x80000000))
            if m == 'q':
                this = [fb(x) for x in unit()] if rnd.random() < 0.8 else vec()
                if rnd.random() < 0.2:
                    this[3] = rnd.choice((fb(1.0), fb(-1.0), fb(1.0) - 1, fb(-1.0) + 1, fb(1.0) + 1, fb(-1.0) - 1, 0x7FC00000))
            else:
                this = vec()
            cases.append([m, r, vflag()] + this + [vflag()] + vec())
    for _ in range(200):
        for m in 'skmMrRzeEgwW':
            if m in 'mM':
                x = rnd.choice((0, 1, 2, 3, 4, 0xFFFFFFFF))
            else:
                x = scalar()
            cases.append([m, x, vflag()] + vec() + [vflag()] + vec())
    width = max(len(c) for c in cases)
    assert width <= 96
    inp = '\n'.join(' '.join(['%x' % MODES.index(c[0])] + ['%x' % x for x in c[1:]]) for c in cases) + '\n'

    nl = chr(10)
    samp = {('sub_' + f[k]): 'for(int i=0;i<%d;i++)store(ctx.r5.u32+4*i,W[%d+i]);' % (n, o)
            for k, n, o in (('vs', 4, 8), ('qs', 4, 8), ('fs', 1, 8))}
    samp.update(throw)
    src = r'''
static void dof(uint32_t a,uint32_t fl,const uint32_t*v){memset(pointer(a),0,32);*pointer(a+4)=uint8_t(fl);for(int i=0;i<4;i++)store(a+16+4*i,v[i]);}
static void pdof(uint32_t a){printf(" %x",(unsigned)*pointer(a+4));for(int i=0;i<4;i++)printf(" %08x",load(a+16+4*i));}
static double F(uint32_t u){float f;memcpy(&f,&u,4);return f;}
static Context fresh(){Context c{};c.r1.u32=0xF000;return c;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
FILE*in=fopen(argv[2],"r");char line[2048];
while(fgets(line,sizeof line,in)){
 memset(W,0,sizeof W);int n=0;char*p=line;while(*p&&*p!='\n'){W[n++]=strtoul(p,&p,16);while(*p==' ')p++;}
 int m=W[0];memset(objects,0,sizeof(objects));Context c=fresh();
 try{
 if(m==0){
  int k=W[2];store(0x1000+8,0x1F00);store(0x1000+12,0x1100);store16(0x1000+16,k);
  store(0x1040+8,0x1F00);store(0x1040+12,0x1180);store16(0x1040+16,k);
  for(int i=0;i<k;i++){const uint32_t*q=W+3+10*i;dof(0x2000+32*i,q[0],q+1);dof(0x3000+32*i,q[5],q+6);
   store(0x1100+4*i,0x2000+32*i);store(0x1180+4*i,0x3000+32*i);}
  c.r3.u32=0x1000;c.r4.u32=0;c.r5.u32=0x1040;c.r6.u32=0;c.f1.f64=F(W[1]);sub_BLEND(c,nullptr);
  printf("->");for(int i=0;i<k;i++)pdof(0x2000+32*i);printf("\n");continue;}
 if(m<=3){dof(0x2000,W[3],W+4);c.r3.u32=0x2000;c.r5.u32=0x1F00;c.f1.f64=F(W[2]);c.f2.f64=F(W[1]);
  if(m==1)sub_V4(c,nullptr);else if(m==2)sub_Q4(c,nullptr);else sub_F4(c,nullptr);
  printf("->");pdof(0x2000);printf("\n");continue;}
 dof(0x2000,W[2],W+3);dof(0x2040,W[7],W+8);c.r3.u32=0x2000;
 switch(m){
 case 4:case 5:case 6:c.r4.u32=0x2040;c.f1.f64=F(W[1]);if(m==4)sub_V6(c,nullptr);else if(m==5)sub_Q6(c,nullptr);else sub_F6(c,nullptr);break;
 case 7:c.f1.f64=F(W[1]);sub_VSC(c,nullptr);break;
 case 8:c.f1.f64=F(W[1]);sub_FSC(c,nullptr);break;
 case 9:c.r4.u32=W[1];sub_VMI(c,nullptr);break;
 case 10:c.r4.u32=W[1];sub_QMI(c,nullptr);break;
 case 11:sub_VRE(c,nullptr);break;
 case 12:sub_QRE(c,nullptr);break;
 case 13:sub_FRE(c,nullptr);break;
 case 14:case 15:for(int i=0;i<4;i++)store(0x2100+4*i,W[8+i]);c.r4.u32=0x2100;if(m==14)sub_VSE(c,nullptr);else sub_QSE(c,nullptr);break;
 case 16:c.f1.f64=F(W[1]);sub_FSE(c,nullptr);break;
 case 17:c.r4.u32=0x2040;sub_SW(c,nullptr);break;
 case 18:c.r4.u32=0x2040;sub_FSW(c,nullptr);break;}
 printf("->");pdof(0x2000);pdof(0x2040);printf("\n");
 }catch(std::exception&e){printf("-> error %s\n",e.what());}
}}
'''
    for k, v in (('BLEND', 'blend'), ('V4', 'v4'), ('Q4', 'q4'), ('F4', 'f4'), ('V6', 'v6'), ('Q6', 'q6'), ('F6', 'f6'),
                 ('VSC', 'vsc'), ('FSC', 'fsc'), ('VMI', 'vmi'), ('QMI', 'qmi'), ('VRE', 'vre'), ('QRE', 'qre'),
                 ('FRE', 'fre'), ('VSE', 'vse'), ('QSE', 'qse'), ('FSE', 'fse'), ('SW', 'sw'), ('FSW', 'fsw')):
        src = src.replace('sub_%s(' % k, 'sub_%s(' % f[v])
    roots = ['sub_' + f[k] for k in ('blend', 'v4', 'q4', 'f4', 'v6', 'q6', 'f6', 'vsc', 'fsc', 'vmi', 'qmi', 'vre', 'qre',
                                     'fre', 'vse', 'qse', 'fse', 'sw', 'fsw')]
    idx = index(a.recomp)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    exe = build(idx, roots, src, a.out_dir, 'frame_blend', stubs=samp, vector=True, scratch_end=0x10000)
    (a.out_dir / 'frame_blend_cases.txt').write_text(inp)
    out = _run([str(exe), str(a.image), str(a.out_dir / 'frame_blend_cases.txt')], capture_output=True, text=True, check=True).stdout
    out = [x for x in out.split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    assert not any('error' in o for o in out), [o for o in out if 'error' in o][:3]
    lines = []
    for c, o in zip(cases, out):
        ws = [int(x, 16) for x in o[2:].split()]
        lines.append(' '.join([c[0]] + [word(x) for x in c[1:]]) + ' ->' + ''.join(' ' + word(x) for x in ws))
    (a.out_dir / 'frame_blend_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases:', ', '.join('%s %d' % (m, sum(c[0] == m for c in cases)) for m in MODES))


if __name__ == '__main__':
    main()
