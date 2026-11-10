"""Small routines of the loose-object (prop / body) simulation run offline: the generated code unchanged (vector mode, the
recomp's own -msse4.1 -mssse3), on flat guest memory. Each case line of <out-dir>/loose_small_ref.txt is
"<tag> <input words> -> <output words>" (hex words, memory words in the guest's big-endian reading; a word that reads
82xxxxxx is written with an h in front). Sections (the tag, the row, what is compared):

  other    F-19844  a contact's other side: the manifold's side B if side A is the instance, else side A
  append   F-59304  list append: the node goes after the tail, the header's tail and count updated
  unlink   F-68557  list unlink: the node's neighbours joined, its own links cleared, the count less one; gives the next
  move     F-51875  a node's parent: leave the old list (parent's children or the root list), set +28, append to the new
                    parent's children, or (no parent) put at the head of the root list unless it is in it (F-69979)
  subtree  F-45377  a group and all its sub-groups marked in a bit array, the sub-tree of a skipped group left out
  matrixA  F-21340  side A's matrix of a manifold: the instance's rows, times the composite bound's part matrix (type 12)
  matrixB  F-17349  the same for side B
  terms    F-66163  the five still-test terms of a body (the body's real slots 17 and 40 and 15 answer)
  still    F-61306  the verdict of those terms against the sleep object's tolerances (+4, +8, +12)
  sleep    F-43494  the sleep object's update (counters A and B, state; slot 2 answers "still", slot 1 is counted)
  endtick  F-21091  pending impulses into the momenta and velocities (the real slots 45 and 46)
  ctor     F-33655  a body's constructor (its slot 4 is counted)
  constraint F-31401  a constraint's defaults
  islands  F-50834  does an island rest (with F-82734 in its closure: the manager's flag of a collider)
  spread   F-14324  the share of an impact through the fragment groups (tuning object absent, 1 and 1, or other numbers)
  mass     F-49276  an entry's mass with the damaged forms (health at most 0)
  contact  F-43430  a contact's refresh: friction, elasticity and limit from the materials (manager slot 5, F-11848 as a stub)
                    and the instances (slot 9)
  centre   F-99875  the volume-weighted centre of a type's parts (F-83058 as a stub that records the centre)
  damp     F-66303  a body's damping: force and torque added to the sums (the real slots 32 and 33)

Where a table is needed, the body's is at 0x2800 with every slot 0x7F0000nn; `dispatch` maps the slots it needs to the real
functions, and the others to host bodies that answer from the case. Node codes in the list sections: 0 = null, 1..6 = node k at 0x1100 + 0x40 (k-1), 7 = the root header at 0x1000. The
guest vectors: ctx.vN.u32[3-k] holds lane k. No denormal is generated for a vector input (the SDK's flush mode is a
no-op in this build, the console flushes)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure
from cite_safe import cite_safe

_vector_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda scratch_end=0x3000: '#include <simde/x86/avx2.h>' + chr(10) + _vector_shim(scratch_end).replace('throw std::runtime_error("unexpected guest access")', '{fprintf(stderr,"bad address %08x ",a);fflush(stderr);throw std::runtime_error("unexpected guest access");}')
recomp_closure.EXTRA += ('namespace rex { namespace ppc { static simde__m128 simde_mm_cvtepu32_ps_(simde__m128i v) { alignas(16) uint32_t a[4]; alignas(16) float f[4];' +
                         ' simde_mm_store_si128((simde__m128i*)a, v); for (int i = 0; i < 4; i++) f[i] = float(a[i]); return simde_mm_load_ps(f); } } }' + chr(10) +
                         '#define ctr r2' + chr(10) + 'static void dispatch(Context&,const uint8_t*,uint32_t);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10) +
                         'static uint32_t REC[16][16], STILLV, IDS[2], FS[2]; static int ncalls;' + chr(10) +
                         'static uint32_t A(uint32_t k){return k==0?0:k==7?0x1000:0x1100+0x40*(k-1);}' + chr(10))

NL = chr(92) + 'n'
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def is_denormal(w):
    return (w >> 23) & 0xFF == 0 and (w & 0x7FFFFF) != 0


def word(rnd, scale=10.0):
    r = rnd.random()
    if r < 0.12:
        w = rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x00800000))
    elif r < 0.9:
        w = fb(rnd.uniform(-scale, scale))
    else:
        w = rnd.getrandbits(32)
    return 0 if is_denormal(w) else w


def fw(rnd, scale=10.0):
    """A float word for the vector routines: special values, a uniform number, or a power of two between 2^-30 and 2^30
    (products of three of those stay normal, which the console's flush mode and the host agree on)."""
    r = rnd.random()
    if r < 0.1:
        return rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x00800000 << 8))
    if r < 0.88:
        return fb(rnd.uniform(-scale, scale))
    return fb(rnd.choice((-1, 1)) * 2.0 ** rnd.uniform(-30, 30))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = {k: addr(a.tool, v) for k, v in dict(other='F-19844', append='F-59304', unlink='F-68557', move='F-51875', subtree='F-45377',
                                              matrixA='F-21340', matrixB='F-17349',
                                              terms='F-66163', still='F-61306', sleep='F-43494', endtick='F-21091', ctor='F-33655',
                                              constraint='F-31401', islands='F-50834', spread='F-14324', mass='F-49276',
                                              vt15='F-68457', vt17='F-72147', vt40='F-24169', vt45='F-72870', vt46='F-80680', contact='F-43430', side_id='F-11848',
                                              centre='F-99875', set_centre='F-83058', damp='F-66303', vt32='F-17506', vt33='F-93816').items()}
    rnd = random.Random(19844)
    secs = []   # (tag, cases, setup, fn, nout, outcode)

    # other: inst, side A, side B (the manifold at +160)
    cs = []
    for _ in range(2000):
        x = rnd.getrandbits(32)
        cs.append([rnd.choice((x, x, 0, 0x1100, rnd.getrandbits(32))), rnd.choice((x, 0, 0x1100, rnd.getrandbits(32))), rnd.choice((x, 0, rnd.getrandbits(32)))])
    secs.append(('other', cs, 'store(0x1000+160,0x1100);store(0x1100,c[1]);store(0x1104,c[2]);ctx.r3.u32=0x1000;ctx.r4.u32=c[0];',
                 f['other'], 1, 'ctx.r3.u32'))

    # lists: head tail count, six nodes (prev next), the node
    def list_state():
        wellformed = rnd.random() < 0.5
        if wellformed:
            n = rnd.randrange(0, 7)
            members = rnd.sample(range(1, 7), n)
            head = members[0] if members else 0
            tail = members[-1] if members else 0
            nodes = {k: [0, 0] for k in range(1, 7)}
            for i, m in enumerate(members):
                nodes[m] = [members[i - 1] if i else 0, members[i + 1] if i + 1 < n else 0]
            return [head, tail, n] + [x for k in range(1, 7) for x in nodes[k]]
        # (a list with a head has a tail: the original reads the tail's links)
        h, t = rnd.choice((0, 0, 1, 2, 3, 4, 5, 6)), rnd.randrange(1, 7)
        return [h, t if h else rnd.choice((0, t)), rnd.choice((0, 1, 2, 3, 0xFFFFFFFF))] + \
            [rnd.choice((0, 0, 1, 2, 3, 4, 5, 6, 7)) for _ in range(12)]

    list_setup = ('store(0x1000,A(c[0]));store(0x1004,A(c[1]));store(0x1008,c[2]);for(int k=0;k<6;k++){store(0x1100+0x40*k,A(c[3+2*k]));' +
                  'store(0x1104+0x40*k,A(c[4+2*k]));}ctx.r3.u32=0x1000;ctx.r4.u32=A(c[15]);')
    list_out = '(i<3?load(0x1000+4*i):load(0x1100+0x40*((i-3)/2)+4*((i-3)%2)))'
    cs = [list_state() + [rnd.randrange(1, 7)] for _ in range(2000)]
    secs.append(('append', cs, list_setup, f['append'], 15, list_out))
    cs = [list_state() + [rnd.randrange(1, 7)] for _ in range(2000)]
    secs.append(('unlink', cs, list_setup, f['unlink'], 16, '(i==15?ctx.r3.u32:' + list_out + ')'))

    # move: root (head tail count), six nodes (prev next head tail count parent), node, new parent
    def move_state():
        if rnd.random() < 0.5:
            owner = {}
            lists = {0: []}
            for k in range(1, 7):
                p = rnd.choice((0, 0, 1, 2, 3, 4, 5, 6))
                if p == k:
                    p = 0
                owner[k] = p
                lists.setdefault(p, []).append(k)
            rows = {k: [0, 0, 0, 0, rnd.choice((0, 0, 1, 2)), owner[k]] for k in range(1, 7)}
            root = [0, 0, 0]
            for p, ms in lists.items():
                for i, m in enumerate(ms):
                    rows[m][0] = ms[i - 1] if i else 0
                    rows[m][1] = ms[i + 1] if i + 1 < len(ms) else 0
                if ms:
                    if p == 0:
                        root = [ms[0], ms[-1], len(ms)]
                    else:
                        rows[p][2:5] = [ms[0], ms[-1], len(ms)]
            return root + [x for k in range(1, 7) for x in rows[k]]
        ptr = lambda: rnd.choice((0, 0, 1, 2, 3, 4, 5, 6))
        def pair():
            h, t = ptr(), rnd.randrange(1, 7)
            return h, (t if h else rnd.choice((0, t)))

        h, t = pair()
        return [h, t, rnd.choice((0, 1, 2, 3))] + [x for _ in range(6) for x in (ptr(), ptr(), *pair(), rnd.choice((0, 1, 2, 3)), ptr())]

    cs = [move_state() + [rnd.randrange(1, 7), rnd.choice((0, 0, 1, 2, 3, 4, 5, 6))] for _ in range(2500)]
    secs.append(('move', cs, 'store(0x1000,A(c[0]));store(0x1004,A(c[1]));store(0x1008,c[2]);for(int k=0;k<6;k++){uint32_t b=0x1100+0x40*k;' +
                 'store(b,A(c[3+6*k]));store(b+4,A(c[4+6*k]));store(b+16,A(c[5+6*k]));store(b+20,A(c[6+6*k]));store(b+24,c[7+6*k]);' +
                 'store(b+28,A(c[8+6*k]));}ctx.r3.u32=0x1000;ctx.r4.u32=A(c[39]);ctx.r5.u32=A(c[40]);',
                 f['move'], 39, '(i<3?load(0x1000+4*i):load(0x1100+0x40*((i-3)/6)+(((i-3)%6)<2?4*((i-3)%6):16+4*(((i-3)%6)-2))))'))

    # subtree: group, skip, count, the two bit words, then (first, count) of each group (children after their parent)
    cs = []
    for _ in range(2000):
        n = rnd.choice((1, 2, 3, 5, 8, 12, 20, 33, 40))
        groups = []
        for i in range(n):
            if i < n - 1 and rnd.random() < 0.6:
                first = rnd.randrange(i + 1, n)
                groups.append((first, rnd.randrange(0, min(4, n - first) + 1)))
            else:
                groups.append((rnd.choice((0, 0, 255)) if rnd.random() < 0.3 else rnd.randrange(0, n), 0))
        c = [rnd.randrange(0, n), rnd.choice((255, 255, rnd.randrange(0, n + 2))), n, rnd.getrandbits(32), rnd.getrandbits(32)]
        for g in groups:
            c += list(g)
        cs.append(c)
    secs.append(('subtree', cs, 'store(0x1000+208,0x1400);for(int i=0;i<c[2];i++){store(0x1400+4*i,0x1600+0x60*i);*pointer(0x1600+0x60*i+68)=c[5+2*i];' +
                 '*pointer(0x1600+0x60*i+72)=c[6+2*i];}store(0x2C00,c[3]);store(0x2C04,c[4]);ctx.r4.u32=0x1000;ctx.r5.u32=c[0];ctx.r6.u32=0x2C00;' +
                 'ctx.r7.u32=c[1];', f['subtree'], 2, 'load(0x2C00+4*i)'))

    # matrix of a side: instance present, its 16 matrix words, the bound type, the part index, four part matrices
    for tag, off, eoff in (('matrixA', 0, 28), ('matrixB', 4, 36)):
        cs = []
        for _ in range(1200):
            c = [rnd.choice((0, 1, 1, 1)), rnd.choice((12, 12, 12, 4, 13, 0, 255)), rnd.randrange(0, 4)]
            c += [word(rnd) for _ in range(16)]
            c += [word(rnd) for _ in range(64)]
            cs.append(c)
        secs.append((tag, cs, ('store(0x1000+%d,c[0]?0x1100:0);store(0x1000+%d,c[2]);' % (off, eoff) +
                               'for(int i=0;i<16;i++){store(0x1100+16+4*i,c[3+i]);store(0x1500+4*i,c[19+i]);store(0x1500+4*(16+i),c[35+i]);' +
                               'store(0x1500+4*(32+i),c[51+i]);store(0x1500+4*(48+i),c[67+i]);store(0x1600+4*i,0xA5A5A5A5);}' +
                               'store(0x1100+4,0x1200);store(0x1200+12,0x1300);*pointer(0x1300+4)=c[1];store16(0x1300+146,4);' +
                               'store(0x1300+128,0x1400);store(0x1300+132,0x1500);for(int i=0;i<4;i++)store(0x1400+4*i,0x1700+i);' +
                               'ctx.r3.u32=0x1000;ctx.r4.u32=0x1600;'), f[tag], 16, 'load(0x1600+4*i)'))

    # ---- the bodies: virtual calls answered by the real vtable functions (slots 15 17 40 45 46 of the body's table at 0x2800) ----
    VT = 'for(int k=0;k<64;k++)store(0x2800+4*k,0x7F000000+k);'
    VF = 'auto V=[&](uint32_t a,int i){store(a,c[i]);store(a+4,c[i+1]);store(a+8,c[i+2]);store(a+12,0);};'
    FP = '{float x;memcpy(&x,&c[%d],4);ctx.f%d.f64=x;}'

    def dt_word():
        return rnd.choice((0, 0x80000000, 0x3d088889, 0x3c888889, 0x3d800000, 0x7fc00000, 0xbd088889, fb(rnd.uniform(0.001, 0.2)), fb(rnd.uniform(-1, 1)), fb(2.0 ** rnd.uniform(-20, 5))))

    def scaled_vec(scale):
        return [fw(rnd, scale) for _ in range(3)]

    def terms_case():
        scale = rnd.choice((10.0, 0.5, 0.05, 0.01))
        c = [dt_word(), rnd.choice((dt_word(), 0, 0x3d088889))]
        for _ in range(8):
            c += scaled_vec(scale)
        c += [fw(rnd, 2.0) for _ in range(9)] + [fw(rnd, 5.0) for _ in range(3)] + [fw(rnd, 3.0)]
        return c

    terms_setup = ('store(0x1000+28,0x1100);store(0x1100,0x2800);' + VT + VF +
                   'V(0x1100+272,2);V(0x1100+288,5);V(0x1100+336,8);V(0x1100+352,11);V(0x1100+368,14);V(0x1100+384,17);V(0x1100+400,20);V(0x1100+416,23);' +
                   'for(int r=0;r<3;r++)V(0x1100+208+16*r,26+3*r);V(0x1100+176,35);store(0x1100+196,c[38]);' + FP % (0, 1) + FP % (1, 2) +
                   'ctx.r3.u32=0x1000;ctx.r6.u32=0x1800;ctx.r7.u32=0x1804;ctx.r8.u32=0x1808;ctx.r9.u32=0x180C;ctx.r10.u32=0x1810;')
    secs.append(('terms', [terms_case() for _ in range(2500)], terms_setup, f['terms'], 5, 'load(0x1800+4*i)'))
    cs = []
    for k in range(2500):
        c = terms_case()
        if k % 2:
            c += [0x3d4ccccd, 0x3d4ccccd, 0x3e4ccccd]
        else:
            c += [rnd.choice((0x3d4ccccd, 0x3e4ccccd, 0, 0x7fc00000, fb(rnd.uniform(0, 1)), 0xbf800000)) for _ in range(3)]
        cs.append(c)
    secs.append(('still', cs, terms_setup + 'store(0x1000+4,c[39]);store(0x1000+8,c[40]);store(0x1000+12,c[41]);', f['still'], 1, 'ctx.r3.u32'))

    # sleep object update: still?, state, counters A and B, their limits (non-negative: the port counts in unsigned)
    near = lambda: rnd.choice((0, 1, 2, 13, 14, 15, 16, 18, 19, 20, 21, 0x7FFFFFFE, rnd.randrange(0, 40)))
    cs = [[rnd.choice((0, 1, 1, 2, 0x100, 0xFF, 0xFFFFFF00)), rnd.choice((0, 1, 1, 2, 0, 3, 255)), near(), near(), rnd.choice((15, 15, 15, 0, 1, 2, 16, near())),
           rnd.choice((20, 20, 20, 0, 1, 2, 21, near()))] for _ in range(4000)]
    secs.append(('sleep', cs, 'store(0x1000,0x1100);store(0x1104,0x7F000101);store(0x1108,0x7F000102);store(0x1000+16,c[4]);store(0x1000+20,c[5]);' +
                 'store(0x1000+24,c[1]);store(0x1000+32,c[2]);store(0x1000+36,c[3]);STILLV=c[0];ctx.r3.u32=0x1000;ctx.f1.f64=0.0625;',
                 f['sleep'], 4, '(i==0?load(0x1018):i==1?load(0x1020):i==2?load(0x1024):uint32_t(ncalls))'))

    # end of tick: the impulses into the momenta and the velocities (slots 45 and 46)
    cs = []
    for _ in range(2500):
        scale = rnd.choice((10.0, 1.0, 0.01))
        c = []
        for k in range(6):
            c += scaled_vec(scale)
        c += [fw(rnd, 2.0) for _ in range(9)] + [fw(rnd, 5.0) for _ in range(3)] + [fw(rnd, 3.0)]
        c += [rnd.choice((fb(500.0), fb(1.0), fb(0.5), fb(rnd.uniform(0, 20)), 0, 0x7f800000, fw(rnd, 10.0))) for _ in range(2)]
        c += [rnd.choice((0, 0, 0, 0x3f800000, 0x80000000, 0x7fc00000)) for _ in range(2)]
        if rnd.random() < 0.3:      # an impulse that is zero in some lanes, or both
            for v in (6, 9):
                if rnd.random() < 0.5:
                    for k in range(3):
                        c[v + k] = rnd.choice((0, 0x80000000))
        cs.append(c)
    secs.append(('endtick', cs, 'store(0x1100,0x2800);' + VT + VF + 'V(0x1100+304,0);V(0x1100+320,3);V(0x1100+368,6);V(0x1100+384,9);V(0x1100+272,12);V(0x1100+288,15);' +
                 'store(0x1100+368+12,c[33]);store(0x1100+384+12,c[34]);' +
                 'for(int r=0;r<3;r++)V(0x1100+208+16*r,18+3*r);V(0x1100+176,27);store(0x1100+196,c[30]);store(0x1100+200,c[31]);store(0x1100+204,c[32]);' +
                 'ctx.r3.u32=0x1100;', f['endtick'], 18,
                 'load(0x1100+(i<3?304+4*i:i<6?320+4*(i-3):i<9?368+4*(i-6):i<12?384+4*(i-9):i<15?272+4*(i-12):288+4*(i-15)))'))

    # constructors: every word of the object, prefilled
    cs = [[rnd.getrandbits(32) for _ in range(172)] for _ in range(1000)]
    secs.append(('ctor', cs, 'for(int i=0;i<172;i++)store(0x1000+4*i,c[i]);ctx.r3.u32=0x1000;', f['ctor'], 174,
                 '(i<172?load(0x1000+4*i):i==172?uint32_t(ncalls):ctx.r3.u32)'))
    cs = [[rnd.getrandbits(32) for _ in range(84)] for _ in range(1000)]
    secs.append(('constraint', cs, 'for(int i=0;i<84;i++)store(0x1000+4*i,c[i]);ctx.r3.u32=0x1000;', f['constraint'], 84, 'load(0x1000+4*i)'))

    # an island at rest: the manager (count, base of its 672-byte colliders, flags), the rest objects, the constraint and contact lists
    cs = []
    for _ in range(2500):
        c = [rnd.choice((0, 1, 2, 3, 4, 5, 5, 8))] + [rnd.choice((0, 0, 0, 1, 2, 0x80)) for _ in range(8)]
        for _ in range(4):
            c += [rnd.choice((2, 2, 2, 2, 0, 1, 3, 0xFFFFFFFF, 0x102)), rnd.choice((0, 0, 0, 0, 1, 0xFF, 0x80))]
        c += [rnd.choice((0, 1, 2, 3, 4)) for _ in range(5)]
        c += [rnd.choice((0, 1, 2, 3, 3))] + [rnd.choice((0, 1, 2, 3, 4, 4)) for _ in range(3)]
        c += [rnd.choice((0, 1, 2, 3, 3))] + [rnd.choice((0, 1, 2, 3, 4, 5, 5)) for _ in range(6)]
        cs.append(c)
    # codes: 0..3 collider k of the manager, 4 a collider outside it, 5 none; a rest reference 0..3 = rest object, 4 = none
    secs.append(('islands', cs,
                 'auto K=[&](uint32_t k){return k==4?0x2C00u:k==5?0u:0x2000u+672u*k;};store(0x1000+4,0x1100);store(0x1000+12,0x1200);store(0x1000+20,c[22]);' +
                 'store(0x1000+52,c[26]);store(0x1800+24,c[0]);store(0x1800+32,0x2000);store(0x1800+36,0x1580);store((0x828fu<<16)|0xf710u,0x1800);' +
                 'for(int i=0;i<8;i++)*pointer(0x1580+i)=c[1+i];for(int j=0;j<4;j++){store(0x1600+0x40*j+24,c[9+2*j]);*pointer(0x1600+0x40*j+40)=c[10+2*j];}' +
                 'for(int k=0;k<4;k++)store(0x2000+672*k+24,c[17+k]==4?0:0x1600+0x40*c[17+k]);store(0x2C00+24,c[21]==4?0:0x1600+0x40*c[21]);' +
                 'for(int i=0;i<3;i++)store(0x1200+4*i,K(c[23+i]));for(int k=0;k<3;k++){store(0x1100+4*k,0x1300+0xC0*k);store(0x1300+0xC0*k+160,0x1300+0xC0*k+176);' +
                 'store(0x1300+0xC0*k+176+8,K(c[27+2*k]));store(0x1300+0xC0*k+176+12,K(c[28+2*k]));}ctx.r3.u32=0x1000;',
                 f['islands'], 1, 'ctx.r3.u32&0xFF'))

    # the share of an impact through the groups: the strength m into group g in direction dir, added to gi
    def group_tree(n):
        parent = [255] * n
        first = [0] * n
        count = [0] * n
        nxt = 1
        for i in range(n):
            nxt = max(nxt, i + 1)
            if nxt < n:
                k = min(rnd.choice((0, 1, 2, 3)), n - nxt)
                first[i], count[i] = nxt, k
                for j in range(nxt, nxt + k):
                    parent[j] = i
                nxt += k
        return first, count, parent

    cs = []
    for _ in range(2500):
        n = rnd.choice((1, 2, 3, 5, 8, 12, 20))
        first, count, parent = group_tree(n)
        mode = rnd.choice((0, 0, 1, 2))
        c = [fb(rnd.uniform(0.1, 50)) if rnd.random() < 0.8 else fw(rnd, 10.0), rnd.randrange(0, n), rnd.choice((0, 0, 0, 0xFFFFFFFF, 1, 0xFFFFFFFE, 2)), n,
             rnd.choice((0, 1, 1, 1)), rnd.choice((0, 0, rnd.getrandbits(20), rnd.getrandbits(20))), 0, mode,
             0x3f800000 if mode == 1 else fb(rnd.uniform(0.1, 3)), 0x3f800000 if mode == 1 else fb(rnd.uniform(0.1, 3))]
        c += [fw(rnd, 5.0) for _ in range(n)]
        for i in range(n):
            c += [fb(rnd.uniform(0, 2)) if rnd.random() < 0.85 else fw(rnd, 3.0), fb(rnd.uniform(0, 2)) if rnd.random() < 0.85 else fw(rnd, 3.0), first[i], parent[i], count[i]]
        cs.append(c)
    secs.append(('spread', cs,
                 'store(0x1000+92,0x1100);store(0x1000+84,c[4]?0x2000:0);store(0x1100+208,0x1300);for(int i=0;i<c[3];i++){store(0x1300+4*i,0x1400+0x60*i);}' +
                 'for(int i=0;i<c[3];i++){uint32_t b=0x1400+0x60*i,o=10+c[3]+5*i;store(b+12,c[o]);store(b+16,c[o+1]);' +
                 '*pointer(b+68)=c[o+2];*pointer(b+69)=c[o+3];*pointer(b+72)=c[o+4];}store(0x2000+256+12,0x2200);store(0x2200,0x2300);store(0x2300,c[5]);store(0x2304,c[6]);' +
                 'for(int i=0;i<c[3];i++)store(0x2400+4*i,c[10+i]);if(c[7]){store(0x2500+12,c[8]);store(0x2500+16,c[9]);store((0x828du<<16)|0x4ce4u,0x2500);}else store((0x828du<<16)|0x4ce4u,0);' +
                 'ctx.r3.u32=0x1000;ctx.r5.u32=0x2400;ctx.r6.u32=c[1];ctx.r7.u32=c[2];' + FP % (0, 1), f['spread'], 20,
                 '(i<c[3]?load(0x2400+4*i):0)'))

    # the mass of an entry: each child not absent, its mass or (health at most 0 and a damaged form) the damaged mass
    cs = []
    for _ in range(1500):
        n = rnd.randrange(0, 12)
        c = [n, rnd.getrandbits(32), rnd.getrandbits(32)]
        c += [rnd.choice((fb(rnd.uniform(-5, 5)), 0, 0x3f800000, 0x80000000, 0xbf800000, 0x7fc00000, 0x7f800000)) for _ in range(16)]
        for _ in range(n):
            c += [fw(rnd, 50.0), fw(rnd, 50.0), rnd.randrange(0, 16), rnd.choice((0, 0, 1, 0x1234))]
        cs.append(c)
    secs.append(('mass', cs, 'store(0x1000+212,0x1100);*pointer(0x1000+303)=c[0];store(0x2000+4,0x2200);store(0x2000+12,0x2100);store(0x2100,0x2300);store(0x2300,c[1]);' +
                 'store(0x2304,c[2]);for(int g=0;g<16;g++)store(0x2200+4*g,c[3+g]);for(int i=0;i<c[0];i++){uint32_t b=0x1200+0x100*i,o=19+4*i;store(0x1100+4*i,b);' +
                 'store(b+4,c[o]);store(b+8,c[o+1]);*pointer(b+12)=c[o+2];store(b+148,c[o+3]);}ctx.r4.u32=0x1000;ctx.r5.u32=0x2000;ctx.f1.f64=0.0;',
                 f['mass'], 1, '[&]{float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);return u;}()'))

    # a contact's per-tick refresh: the impulse cleared; for kind 0 the friction and the elasticity from the two materials
    # (the manager's slot 5 answers a material by an id; the ids by the stub of the side-id function) and the limit from the
    # two instances' slot 9
    cs = []
    for _ in range(2500):
        c = [rnd.choice((0, 0, 0, 1, 2, 0x80)), rnd.randrange(0, 4), rnd.randrange(0, 4)]
        for _ in range(4):
            c += [rnd.choice((fw(rnd, 2.0), fb(rnd.uniform(0, 2)), fb(rnd.uniform(0, 2)), 0, 0x80000000, 0x7fc00000, 0xbf800000)),
                  rnd.choice((fw(rnd, 2.0), fb(rnd.uniform(0, 1.5)), fb(rnd.uniform(0, 1.5)), 0x3f800000, 0, 0x7fc00000, 0x7f800000))]
        c += [rnd.choice((fw(rnd, 50.0), fb(rnd.uniform(0, 30)), 0x7f800000, 0x7fc00000)) for _ in range(2)]
        c += [rnd.choice((0, 1, 1)), rnd.choice((0, 1, 1))]
        cs.append(c)
    secs.append(('contact', cs,
                 'for(int i=0;i<4;i++){store(0x1000+112+4*i,0xA5A5A5A5);}store(0x1000+28,0xA5A5A5A5);store(0x1000+44,0xA5A5A5A5);store(0x1000+60,0xA5A5A5A5);' +
                 'store(0x1000+160,0x1100);*pointer(0x1000+164)=c[0];store(0x1100,c[13]?0x1200:0);store(0x1104,c[14]?0x1300:0);store(0x1100+28,7);store(0x1100+36,9);' +
                 'store(0x1200,0x1400);store(0x1300,0x1400);store(0x1400+36,0x7F000209);store(0x1800,0x1900);store(0x1900+20,0x7F000205);' +
                 'store((0x828du<<16)|0xa458u,0x1800);for(int k=0;k<4;k++){store(0x1A00+0x20*k+12,c[3+2*k]);store(0x1A00+0x20*k+16,c[4+2*k]);}' +
                 'IDS[0]=c[1];IDS[1]=c[2];FS[0]=c[11];FS[1]=c[12];ctx.r3.u32=0x1000;',
                 f['contact'], 8, '(i<4?load(0x1000+112+4*i):i==4?load(0x1000+28):i==5?load(0x1000+44):i==6?load(0x1000+60):uint32_t(ncalls))'))

    # the volume-weighted centre of a type's parts: n, from a volume list?, then per part: bound?, flag byte, cg (4 words), volume, matrix (16)
    cs = []
    for _ in range(2500):
        n = rnd.randrange(0, 5)
        c = [n, rnd.choice((0, 1))]
        for k in range(4):
            c += [rnd.choice((0, 1, 1, 1)), rnd.choice((0, 1, 1, 2)), fw(rnd, 3.0), fw(rnd, 3.0), fw(rnd, 3.0), fw(rnd, 3.0),
                  rnd.choice((fb(rnd.uniform(0, 5)), fb(rnd.uniform(0, 0.001)), 0, 0x80000000, fw(rnd, 5.0), fb(rnd.uniform(-1, 1)))),
                  *[(fw(rnd, 4.0) if j % 4 != 3 else (0x3f800000 if j == 15 else 0)) for j in range(16)]]
        cs.append(c)
    secs.append(('centre', cs,
                 # (the point's w = 1 and the xyz mask are filled at start-up in the running original; zero in the image)
                 'for(int k=0;k<4;k++){store((0x8286u<<16)|(0x0950u+4*k),k<3?0xFFFFFFFFu:0);store((0x8286u<<16)|(0x0930u+4*k),k<3?0:0x3F800000u);}' +
                 'store(0x1000+128,0x1100);store(0x1000+132,0x1200);store16(0x1000+146,c[0]);for(int k=0;k<4;k++){uint32_t o=2+23*k;store(0x1100+4*k,c[o]?0x1400+0x80*k:0);' +
                 'store(0x1400+0x80*k,0x1900);*pointer(0x1400+0x80*k+6)=c[o+1];for(int j=0;j<4;j++)store(0x1400+0x80*k+80+4*j,c[o+2+j]);store(0x1A00+4*k,c[o+6]);' +
                 'for(int j=0;j<16;j++)store(0x1200+64*k+4*j,c[o+7+j]);}store(0x1900+32,0x7F000308);ctx.r3.u32=0x1000;ctx.r4.u32=c[1]?0x1A00:0;',
                 f['centre'], 5, '(i<4?REC[0][i]:uint32_t(ncalls))'))

    # damping of a body (the archetype asks for it): the force and the torque added to the sums (slots 32 and 33)
    cs = []
    for _ in range(3000):
        scale = rnd.choice((10.0, 1.0, 0.1, 0.001))
        c = [rnd.choice((0x3d088889, 0x3c888889, 0x3d800000, fb(rnd.uniform(0.001, 0.2)), fb(2.0 ** rnd.uniform(-12, 3)), 0, 0x7fc00000))]
        c += [fw(rnd, 2.0) for _ in range(9)] + scaled_vec(scale) + scaled_vec(scale) + [fw(rnd, 50.0), *[fb(rnd.uniform(0.1, 50)) for _ in range(3)]]
        c += [rnd.choice((fb(rnd.uniform(0, 1)), fb(rnd.uniform(0, 0.05)), fw(rnd, 1.0))) for _ in range(18)]
        c += [fw(rnd, 20.0) for _ in range(6)]
        cs.append(c)
    secs.append(('damp', cs,
                 'store(0x1000,0x2800);' + VT + VF + 'store(0x1000+16,0x1100);store(0x1100+4,0x1200);store(0x1200,0x1300);store(0x1300+68,0x7F000411);' +
                 'for(int r=0;r<3;r++)V(0x1000+208+16*r,1+3*r);V(0x1000+272,10);V(0x1000+288,13);store(0x1000+192,c[16]);V(0x1000+160,17);' +
                 'for(int k=0;k<6;k++)V(0x1200+96+16*k,20+3*k);V(0x1000+336,38);V(0x1000+352,41);' +
                 '{for(int k=0;k<4;k++)ctx.v1.u32[k]=c[0];}ctx.r3.u32=0x1000;', f['damp'], 6, 'load(0x1000+(i<3?336+4*i:352+4*(i-3)))'))

    sl = {15: 'vt15', 17: 'vt17', 32: 'vt32', 33: 'vt33', 40: 'vt40', 45: 'vt45', 46: 'vt46'}
    cases_sw = ''.join('case 0x7F0000%02X:sub_%s(ctx,base);return;' % (k, f[v]) for k, v in sl.items())
    stubs = {'sub_' + f['side_id']: 'ctx.r3.u32=(ctx.r4.u32&0xFF)?IDS[0]:IDS[1];',
             'sub_' + f['set_centre']: 'for(int k=0;k<4;k++)REC[0][k]=load(ctx.r4.u32+4*k);ncalls++;'}
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){' + cases_sw +
            'case 0x7F000101:ncalls++;return;case 0x7F000102:ctx.r3.u32=STILLV;return;case 0x7F000411:ctx.r3.u32=1;return;' +
            'case 0x7F000205:ctx.r3.u32=0x1A00+0x20*(ctx.r4.u32&3);return;' +
            'case 0x7F000209:{float x;memcpy(&x,&FS[ctx.r3.u32==0x1200?0:1],4);ctx.f1.f64=x;}return;' +
            'case 0x7F000308:{float x;memcpy(&x,pointer(0x1A00+4*((ctx.r3.u32-0x1400)/0x80)),4);uint32_t b=load(0x1A00+4*((ctx.r3.u32-0x1400)/0x80));memcpy(&x,&b,4);ctx.f1.f64=x;}return;' +
            'default:if((a>>24)==0x82){ncalls++;return;}throw std::runtime_error("unexpected indirect call");}}')
    body = ''
    for tag, cs, setup, fn, nout, outc in secs:
        w = max(len(c) for c in cs)
        rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c + [0] * (w - len(c))) for c in cs)
        body += ('{static const uint32_t C[][%d]={%s%s%s};' % (w, nl, rows, nl) +
                 'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x7F00;ncalls=0;' + setup +
                 'sub_%s(ctx,nullptr);' % fn +
                 'for(int i=0;i<%d;i++)printf("%%08x ",uint32_t(%s));printf("%s");fflush(stdout);}}' % (nout, outc, NL) + nl)
    consts = ''
    main_src = disp + nl + ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                            nl + consts + nl + body + '}' + nl)
    names = sorted({'sub_' + x for x in f.values()})
    exe = build(index(a.recomp), names, main_src, a.out_dir, 'loose_small', stubs=stubs, vector=True, scratch_end=0x8000, flags=('-msse4.1', '-mssse3'))
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == sum(len(s[1]) for s in secs), (len(out), sum(len(s[1]) for s in secs))
    lines = []
    k = 0
    for tag, cs, *_ in secs:
        for c in cs:
            lines.append('%s %s -> %s' % (tag, ' '.join('%08x' % x for x in c), out[k].strip()))
            k += 1
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'loose_small_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print({s[0]: len(s[1]) for s in secs})


if __name__ == '__main__':
    main()
