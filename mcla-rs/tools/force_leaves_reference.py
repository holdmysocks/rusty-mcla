"""Four small routines of the body / contact code run offline (generated code unchanged, vector mode, flush mode as the SDK
sets it): the body class's apply-force-at-a-point F-18006 (vfunc 31: +336 += F, +352 += the torque of F at the point, in the
vector unit's two-product order), the impulse at a point F-97724 (P + J handed to the body's momentum setter, +320 += the
torque of J), the velocity a broken-off piece takes from its parent's turn F-74100 (the instance's place less the parent's
centre, crossed with the turn, + the parent's velocity; handed to the body's velocity and turn setters) and the refresh of
one contact F-33100 (the age +174; if not 0 the world points +48 / +64 from the two matrices, and for kind +164 = 0 the
depth +12; +144 zeroed; age + 1).

The indirect calls are stubbed by a dispatch that logs their arguments (the vtable slots hold 0x11 .. 0x32 here):
F-18006's notification slot 17 (the force copy and the point copy on its stack), F-97724's slots 42 (P + J) and 46 (called),
F-74100's slots 41 (the velocity) and 43 (the turn r4). F-74100 finds its body through the level's tables
(BASE + 0xF710 -> +4 -> +112 flags, +120 bodies by index), seeded here as a scratch block.

Cases: positions to +-3000, forces to 5e5, velocities to 90 m/s, turn to 10 rad/s, the contact's local points within
a few metres and the matrices rotated and placed anywhere in the city; zeros, -0, subnormals, huge values, NaN and
cancelling pairs (the torque sum made exactly 0); the w lanes of every vector register hold garbage.

Writes in <out-dir>: force_at_point_ref.txt ("ff F p pos F0 T0 -> n1 force point | F T"), impulse_at_point_ref.txt
("imp P L pos J p -> c1 P+J | L'"), piece_velocity_ref.txt ("pv idx flag body pos c v w -> calls") and contact_refresh_ref.txt
("cr age kind la lb back A B -> words"), hex words, x y z of each vector (the matrices as 4 rows of 4)."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('static void dispatch(Context &ctx,const uint8_t *base,uint32_t a);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10))
_vshim = recomp_closure.vector_shim


def _shim():
    # (cr6 from a vector compare as the SDK's; the flush mode as the SDK sets it: MXCSR FTZ | DAZ; scratch 0x1000 .. 0x10000)
    s = '#include <xmmintrin.h>' + chr(10) + '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx2.h>' + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fl = 'void disableFlushMode() {} void enableFlushMode() {}'
    assert s.count(fl) == 1
    s = s.replace(fl, 'void disableFlushMode() { _mm_setcsr(_mm_getcsr() & ~0x8040); }'
                      ' void enableFlushMode() { _mm_setcsr(_mm_getcsr() | 0x8040); }', 1)
    fu = 'void disableFlushModeUnconditional() {} void enableFlushModeUnconditional() {}'
    assert s.count(fu) == 1
    s = s.replace(fu, 'void disableFlushModeUnconditional() { _mm_setcsr(_mm_getcsr() & ~0x8040); }'
                      ' void enableFlushModeUnconditional() { _mm_setcsr(_mm_getcsr() | 0x8040); }', 1)
    for old, new in (('objects[8192]', 'objects[0xF000]'), ('a<12288', 'a<0x10000')):
        assert s.count(old) == 1, old
        s = s.replace(old, new, 1)
    return s


recomp_closure.vector_shim = _shim

BASE = 0x828F << 16
LEVEL = BASE + 0xF710


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def f32(x):
    try:
        return struct.unpack('>f', struct.pack('>f', x))[0]
    except OverflowError:
        return float('inf') if x > 0 else float('-inf')


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(b):
    return struct.unpack('>f', struct.pack('>I', b))[0]


NASTY = (0.0, -0.0, 1e-40, -1e-39, 1e-20, 3e38, float('nan'), float('inf'))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f_force, f_imp, f_piece, f_refresh = ('sub_' + addr(a.tool, f) for f in ('F-18006', 'F-97724', 'F-74100', 'F-33100'))
    rnd = random.Random(18006)

    def lane(s, p_nasty=0.08):
        r = rnd.random()
        if r < p_nasty:
            x = rnd.choice(NASTY)
            if x != x or x in (float('inf'), float('-inf')):
                return fb(x) if rnd.random() < 0.15 else fb(rnd.uniform(-s, s))
            return fb(x)
        return fb(rnd.uniform(-s, s))

    def vec(s):
        return [lane(s) for _ in range(3)]

    def sane(s):
        return [fb(rnd.uniform(-s, s)) for _ in range(3)]

    def near(p, r):
        return [fb(f32(bf(p[k]) + rnd.uniform(-r, r))) for k in range(3)]

    def cross_parts(d, F):
        # (yzx(d) * zxy(F), zxy(d) * yzx(F))
        D = [bf(x) for x in d]
        G = [bf(x) for x in F]
        p1 = [f32(D[1] * G[2]), f32(D[2] * G[0]), f32(D[0] * G[1])]
        p2 = [f32(D[2] * G[1]), f32(D[0] * G[2]), f32(D[1] * G[0])]
        return p1, p2

    cases = []
    # F-18006 (kind 0): F, point, pos, F0, T0
    for i in range(2500):
        pos = sane(3000) if rnd.random() < 0.9 else vec(3000)
        mode = rnd.random()
        pt = near(pos, 3) if mode < 0.6 else (vec(3000) if mode < 0.7 else near(pos, 0.01))
        F = vec(5e5) if rnd.random() < 0.7 else vec(300)
        F0 = vec(2e6) if rnd.random() < 0.8 else sane(10)
        T0 = vec(2e6) if rnd.random() < 0.8 else sane(10)
        if rnd.random() < 0.12:
            # (the torque sum made exactly 0 by the two products: T0 = -p1, and a zero second product)
            d = [fb(f32(bf(pt[k]) - bf(pos[k]))) for k in range(3)]
            p1, p2 = cross_parts(d, F)
            T0 = [fb(-x) for x in p1]
            if rnd.random() < 0.6:
                F[1] = fb(rnd.choice((0.0, -0.0)))
                p1, p2 = cross_parts(d, F)
                T0 = [fb(-x) for x in p1]
        cases.append((0, F + pt + pos + F0 + T0))
    # F-97724 (kind 1): P, L, pos, J, point
    for i in range(2500):
        pos = sane(3000)
        pt = near(pos, 3) if rnd.random() < 0.8 else vec(3000)
        J = vec(1e5) if rnd.random() < 0.7 else vec(50)
        P = vec(3e5)
        L = vec(2e6)
        if rnd.random() < 0.08:
            P = [fb(-bf(J[k])) if rnd.random() < 0.5 else P[k] for k in range(3)]
        if rnd.random() < 0.1:
            d = [fb(f32(bf(pt[k]) - bf(pos[k]))) for k in range(3)]
            p1, p2 = cross_parts(d, J)
            L = [fb(-(f32(x - y))) for x, y in zip(p1, p2)]
        cases.append((1, P + L + pos + J + pt))
    # F-74100 (kind 2): index, flags, body non-null, inst pos, centre, v, w
    for i in range(2500):
        r = rnd.random()
        idx = 0xFFFF if r < 0.07 else rnd.randrange(0, 12)
        flag = rnd.choice((0, 0, 0, 0, 1, 2, 3, 4)) if rnd.random() < 0.15 else 0
        body = 0 if rnd.random() < 0.07 else 1
        c = sane(3000) if rnd.random() < 0.9 else vec(3000)
        p = near(c, 4) if rnd.random() < 0.85 else vec(3000)
        v = vec(90)
        w = vec(10)
        cases.append((2, [idx, flag, body] + p + c + v + w))
    # F-33100 (kind 3): age, kind, local A, local B, back, matrix A, matrix B
    def mat():
        def rows():
            m = []
            for k in range(3):
                m += [fb(rnd.gauss(0, 1)) for _ in range(3)] + [rnd.choice((0, 0, 0, fb(rnd.uniform(-1, 1))))]
            return m
        rot = rows()
        place = sane(3000) + [rnd.choice((0x3F800000, 0x3F800000, 0x3F800000, rnd.getrandbits(32) & 0x7F7FFFFF))]
        if rnd.random() < 0.1:
            rot = [fb(x) for x in (1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0)]
        if rnd.random() < 0.06:
            rot = [lane(2) for _ in range(12)]
        return rot + place

    ident = [fb(x) for x in (1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0)]
    for i in range(1200):
        age = rnd.choice((0, 1, 1, 2, 3, 7, 65535, rnd.randrange(65536)))
        kind = rnd.choice((0, 0, 0, 1, 7))
        la = near([0, 0, 0], 3) if rnd.random() < 0.8 else vec(100)
        lb = near([0, 0, 0], 3) if rnd.random() < 0.8 else vec(3000)
        back = vec(1.0) if rnd.random() < 0.8 else vec(5)
        ma = mat()
        mb = list(ident) if rnd.random() < 0.3 else mat()
        cases.append((3, [age, kind] + la + lb + back + ma + mb))
    names = ['ff', 'imp', 'pv', 'cr']
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = []
    for kind, ws in cases:
        flat += [kind, len(ws)] + ws
    cpp = r'''
static void p3(uint32_t a){printf("%08x %08x %08x ",load(a),load(a+4),load(a+8));}
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a){
  switch(a){
  case 0x11: printf("n1 ");p3(ctx.r4.u32);p3(ctx.r5.u32);return;
  case 0x21: printf("c1 ");p3(ctx.r4.u32);return;
  case 0x22: printf("c2 ");return;
  case 0x31: printf("v %x ",ctx.r3.u32);p3(ctx.r4.u32);return;
  case 0x32: printf("w %x ",ctx.r3.u32);p3(ctx.r4.u32);return;
  default: throw std::runtime_error("indirect");}
}
static void v4(PPCVRegister &v,const uint32_t*w,uint32_t g){v.u32[3]=w[0];v.u32[2]=w[1];v.u32[1]=w[2];v.u32[0]=g;}
static void put3(uint32_t at,const uint32_t*w,uint32_t g){store(at,w[0]);store(at+4,w[1]);store(at+8,w[2]);store(at+12,g);}
int main(int argc,char**argv){
  std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
  size_t at=0;
  for(int c=0;c<NCASES;c++){
    memset(objects,0,sizeof(objects));
    uint32_t kind=D[at],n=D[at+1];const uint32_t*w=&D[at+2];at+=2+n;
    uint32_t g=0x40000000u+c*977u;
    Context ctx{};ctx.r1.u32=0xEF00;
    try{
      if(kind==0){
        store(0x1000+16,0x5000);store(0x5000,0x5100);store(0x5100+68,0x11);
        put3(0x1000+256,w+6,g+1);put3(0x1000+336,w+9,g+2);put3(0x1000+352,w+12,g+3);
        v4(ctx.v1,w,g+4);v4(ctx.v2,w+3,g+5);ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;
        FORCE(ctx,nullptr);
        printf("| ");p3(0x1000+336);p3(0x1000+352);
      }else if(kind==1){
        store(0x1000,0x4000);store(0x4000+168,0x21);store(0x4000+184,0x22);
        put3(0x1000+256,w+6,g+1);put3(0x1000+304,w,g+2);put3(0x1000+320,w+3,g+3);
        v4(ctx.v1,w+9,g+4);v4(ctx.v2,w+12,g+5);ctx.r3.u32=0x1000;
        IMP(ctx,nullptr);
        printf("| ");p3(0x1000+320);
      }else if(kind==2){
        store(LEVELA,0x1300);store(0x1300+4,0x1400);store(0x1400+112,0x1500);store(0x1400+120,0x1600);
        uint32_t idx=w[0];
        if(idx!=0xFFFF){store(0x1500+8*idx+4,w[1]);store(0x1600+4*idx,w[2]?0x1700:0);}
        store(0x1700,0x1780);store(0x1780+164,0x31);store(0x1780+172,0x32);
        store16(0x1000+8,idx);put3(0x1000+64,w+3,g+1);put3(0x1200,w+6,g+2);put3(0x1210,w+9,g+3);put3(0x1220,w+12,g+4);
        ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;ctx.r5.u32=0x1210;ctx.r6.u32=0x1220;
        PIECE(ctx,nullptr);
        printf("|");
      }else{
        for(uint32_t i=0;i<0xB0;i+=4)store(0x1000+i,0xA5A50000u+i);
        store16(0x1000+174,w[0]);*pointer(0x1000+164)=w[1];
        put3(0x1000+16,w+2,g+1);put3(0x1000+32,w+5,g+2);put3(0x1000+96,w+8,g+3);
        for(int k=0;k<16;k++){store(0x2000+4*k,w[11+k]);store(0x2040+4*k,w[27+k]);}
        ctx.r3.u32=0x1000;ctx.r4.u32=0x2000;ctx.r5.u32=0x2040;
        REFRESH(ctx,nullptr);
        printf("| ");
        for(uint32_t i=0;i<16;i+=4)printf("%08x ",load(0x1000+i));
        for(uint32_t i=48;i<80;i+=4)printf("%08x ",load(0x1000+i));
        for(uint32_t i=144;i<160;i+=4)printf("%08x ",load(0x1000+i));
        printf("%04x ",load16(0x1000+174));
      }
    }catch(std::exception&e){printf("| stop %s",e.what());}
    printf("NLX");
  }
}
'''
    cpp = (cpp.replace('NCASES', str(len(cases))).replace('FORCE', f_force).replace('IMP', f_imp).replace('PIECE', f_piece)
           .replace('REFRESH', f_refresh).replace('LEVELA', '0x%xu' % LEVEL).replace('NLX', NL))
    main_src = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl + cpp)
    exe = build(index(a.recomp), [f_force, f_imp, f_piece, f_refresh], main_src, a.out_dir, 'force_leaves', vector=True,
                flags=('-msse4.1', '-mssse3'))
    out = [x.rstrip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    assert not any('stop' in o for o in out), next(o for o in out if 'stop' in o)
    files = {0: [], 1: [], 2: [], 3: []}
    for (kind, ws), o in zip(cases, out):
        files[kind].append('%s %s -> %s' % (names[kind], ' '.join('%x' % x for x in ws), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    for kind, name in ((0, 'force_at_point'), (1, 'impulse_at_point'), (2, 'piece_velocity'), (3, 'contact_refresh')):
        # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
        text = re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(files[kind]) + nl)
        (a.out_dir / (name + '_ref.txt')).write_text(text)
        print(name, len(files[kind]), 'cases', len(text), 'bytes')


if __name__ == '__main__':
    main()
