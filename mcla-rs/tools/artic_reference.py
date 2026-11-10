"""The thrown rider's articulated body, small routines run offline (the generated code unchanged, vector mode, the recomp's
x86 flags; the callees of a closure included). Cases are random (some words zero, -0, NaN, infinities, denormals) and the
original's outputs go into text files the Rust tests (`physics::artic*::offline`) read:

  artic_island_ref.txt  F-78078 (-> F-67069), F-41014 (-> F-19400), F-18964, F-48604 (-> F-10668, walk), F-96841,
                        F-79699, F-47802
  artic_ref.txt         F-63136, F-91185, F-38496, F-21563 (-> F-69646), F-53726, F-25250, F-93538
  artic_build_ref.txt   F-15442, F-77258, F-60898, F-33001
  artic_run_ref.txt     F-78981

One case a line: kind | registers | memory in | memory out | r3. Registers `code:words` (3..10 = r3..r10 one word, 11..13 =
v1..v3 four words in lane order x y z w, 14 = f1 as a float's bits); memory `addr:words` hex, words big endian. The guest
memory of a case is flat at 0x1000 .. 0x6000 (the harness's scratch), the body laid out as in `Mem` of
`physics/artic_offline.rs`: collider, body, 4 links, 3 joints."""
import argparse
import math
import random
import struct
import subprocess
import sys
from pathlib import Path

from recomp_closure import index, build, run  # noqa: E402  (PYTHONPATH=mcla-rs/tools of the main checkout)
import recomp_closure  # noqa: E402

_vshim = recomp_closure.vector_shim


def _shim(scratch_end=0x3000):
    # the SDK's own helpers (include/rex/ppc/intrinsics.h): the unsigned convert (vcuxwfp128), vperm, the saturating signed convert
    # (vcfpsxws128)
    cvt = ('namespace rex { namespace ppc { inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {'
           'simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));'
           'simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8); xmm0 = simde_mm_srli_epi32(xmm0, 31);'
           'xmm0 = simde_mm_add_epi32(xmm0, xmm1); xmm0 = simde_mm_srai_epi32(xmm0, 8);'
           'xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000)); simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);'
           'return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1)); }'
           'inline simde__m128i simde_mm_perm_epi8_(simde__m128i a, simde__m128i b, simde__m128i c) {'
           'simde__m128i d = simde_mm_set1_epi8(0xF); simde__m128i e = simde_mm_sub_epi8(d, simde_mm_and_si128(c, d));'
           'return simde_mm_blendv_epi8(simde_mm_shuffle_epi8(a, e), simde_mm_shuffle_epi8(b, e), simde_mm_slli_epi32(c, 3)); }'
           'inline simde__m128i simde_mm_vctsxs(simde__m128 src1) {'
           'simde__m128 xmm2 = simde_mm_cmpunord_ps(src1, src1); simde__m128i xmm0 = simde_mm_cvttps_epi32(src1);'
           'simde__m128i xmm1 = simde_mm_cmpeq_epi32(xmm0, simde_mm_set1_epi32(INT_MIN));'
           'xmm1 = simde_mm_andnot_si128(simde_mm_castps_si128(src1), xmm1);'
           'simde__m128 dest = simde_mm_blendv_ps(simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(simde_mm_set1_epi32(INT_MAX)), simde_mm_castsi128_ps(xmm1));'
           'return simde_mm_andnot_si128(simde_mm_castps_si128(xmm2), simde_mm_castps_si128(dest)); } } }'
           'static inline simde__m128i simde_mm_sllv_epi32(simde__m128i a, simde__m128i n) {'
           'alignas(16) uint32_t x[4], k[4]; simde_mm_store_si128((simde__m128i*)x, a); simde_mm_store_si128((simde__m128i*)k, n);'
           'for (int i = 0; i < 4; i++) x[i] = k[i] > 31 ? 0 : x[i] << k[i]; return simde_mm_load_si128((simde__m128i*)x); }')
    s = '#include <climits>' + chr(10) + '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx.h>' + chr(10) + cvt + chr(10) + _vshim(scratch_end)
    return s


recomp_closure.vector_shim = _shim

# the layout (mirrored by physics/artic_offline.rs)
COLL, BODY, LINK0, JOINT0 = 0x1000, 0x1300, 0x1510, 0x2500
LSZ, JSZ = 1008, 1408
CHAIN, TABLE, VT = 0x3600, 0x3700, 0x3740   # collider's +16 chain objects, the bone table, a vtable
BUF = 0x3800                                  # argument buffers (16 aligned)
OUT = 0x3900
STACK = 0x5E00
FILL = 0xCD
GRAV = 0x82000000 + 0x812800                  # the world's gravity vector (an image global)
MEMSET_BODY = 'for (uint32_t i = 0; i < ctx.r5.u32; i++) *pointer(ctx.r3.u32 + i) = uint8_t(ctx.r4.u32);'  # the library memset (r3 dest, r4 byte, r5 length)
NL = 4                                        # links of a test body


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:], 16)


def fb(x):
    try:
        return struct.unpack('>I', struct.pack('>f', x))[0]
    except OverflowError:
        return 0x7F800000 if x > 0 else 0xFF800000


def bf(w):
    return struct.unpack('>f', struct.pack('>I', w))[0]


class Rng(random.Random):
    def f(self, spec=0.06):
        """a float word: mostly moderate, some wide, some special"""
        r = self.random()
        if r < spec:
            return self.choice((0x00000000, 0x80000000, 0x7FC00000, 0x7F800000, 0xFF800000, 0x00000400, 0x80000400, 0x3F800000, 0xBF800000, 0x7F7FFFFF))
        if r < 0.22:
            return fb(self.choice((-1, 1)) * 10 ** self.uniform(-5, 5))
        return fb(self.uniform(-3, 3))

    def fin(self):
        """a finite float word"""
        r = self.random()
        return fb(self.choice((-1, 1)) * 10 ** self.uniform(-4, 3)) if r < 0.3 else fb(self.uniform(-3, 3))

    def v(self, n=4, spec=0.06):
        return [self.f(spec) for _ in range(n)]


class Mem:
    def __init__(self):
        self.w = {}

    def put(self, a, words):
        for i, x in enumerate(words):
            self.w[a + 4 * i] = x & 0xFFFFFFFF

    def segs(self):
        out, cur = [], None
        for a in sorted(self.w):
            if cur and cur[0] + 4 * len(cur[1]) == a:
                cur[1].append(self.w[a])
            else:
                cur = [a, [self.w[a]]]
                out.append(cur)
        return out


class Case:
    def __init__(self, kind):
        self.kind, self.regs, self.mem, self.outs = kind, [], Mem(), []

    def reg(self, code, *words):
        self.regs.append((code, list(words)))
        return self

    def out(self, a, n):
        self.outs.append((a, n))
        return self


def harness_text(cases, kinds):
    t = []
    for c in cases:
        t.append('%x %x' % (kinds[c.kind], len(c.regs)))
        for code, ws in c.regs:
            t.append('%x %x ' % (code, len(ws)) + ' '.join('%x' % w for w in ws))
        segs = c.mem.segs()
        t.append('%x' % len(segs))
        for a, ws in segs:
            t.append('%x %x ' % (a, len(ws)) + ' '.join('%x' % w for w in ws))
        t.append('%x' % len(c.outs))
        for a, n in c.outs:
            t.append('%x %x' % (a, n))
    return '\n'.join(t) + '\n'


def data_line(c, res):
    regs = ';'.join('%d:%s' % (code, ','.join('%x' % w for w in ws)) for code, ws in c.regs)
    mem = ';'.join('%x:%s' % (a, ','.join('%x' % w for w in ws)) for a, ws in c.mem.segs())
    outs, o = [], 1
    for a, n in c.outs:
        outs.append('%x:%s' % (a, ','.join(res[o:o + n])))
        o += n
    return '|'.join((c.kind, regs, mem, ';'.join(outs), res[0]))


MAIN_HEAD = r'''
static void dispatch(Context &ctx, const uint8_t *base, uint32_t a) {
    DISPATCH_BODY
}
int main(int argc, char **argv) {
    std::ifstream bin(argv[1], std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin), {});
    for (int k = 0; k < 3; k++) { store(0x82860950 + 4 * k, 0xFFFFFFFF); store(0x82860930 + 4 * k, 0); }
    store(0x8286095C, 0); store(0x8286093C, 0x3F800000);
    FILE *f = fopen(argv[2], "r");
    unsigned kind, nr;
    while (fscanf(f, "%x %x", &kind, &nr) == 2) {
        memset(objects, 0, sizeof(objects));
        Context ctx{};
        ctx.r1.u32 = STACKV;
        for (unsigned i = 0; i < nr; i++) {
            unsigned code, n, w[4] = {0, 0, 0, 0};
            fscanf(f, "%x %x", &code, &n);
            for (unsigned k = 0; k < n; k++) fscanf(f, "%x", &w[k]);
            PPCRegister *R[8] = {&ctx.r3, &ctx.r4, &ctx.r5, &ctx.r6, &ctx.r7, &ctx.r8, &ctx.r9, &ctx.r10};
            if (code >= 3 && code <= 10) R[code - 3]->u64 = w[0];
            else if (code >= 11 && code <= 13) { VReg *v = code == 11 ? &ctx.v1 : code == 12 ? &ctx.v2 : &ctx.v3; for (int k = 0; k < 4; k++) v->u32[3 - k] = w[k]; }
            else if (code == 14) { float x; memcpy(&x, &w[0], 4); ctx.f1.f64 = x; }
        }
        unsigned ns, a, n;
        fscanf(f, "%x", &ns);
        for (unsigned i = 0; i < ns; i++) {
            fscanf(f, "%x %x", &a, &n);
            for (unsigned k = 0; k < n; k++) { unsigned w; fscanf(f, "%x", &w); store(a + 4 * k, w); }
        }
        unsigned no;
        fscanf(f, "%x", &no);
        unsigned oa[16], on[16];
        for (unsigned i = 0; i < no; i++) fscanf(f, "%x %x", &oa[i], &on[i]);
        switch (kind) {
CALLS
        }
        printf("%x", ctx.r3.u32);
        for (unsigned i = 0; i < no; i++) for (unsigned k = 0; k < on[i]; k++) printf(" %x", load(oa[i] + 4 * k));
        printf("\n");
    }
}
'''


def run_cases(args, cases, calls, dispatch_body, names, tag, flags=('-msse4.1', '-mssse3'), stubs=()):
    """calls: kind -> C++ statement; names: the sub_ names the closure needs"""
    import recomp_closure
    kinds = {k: i for i, k in enumerate(calls)}
    body = ''.join('case %d: %s break;\n' % (kinds[k], calls[k]) for k in calls)
    main_src = MAIN_HEAD.replace('CALLS', body).replace('DISPATCH_BODY', dispatch_body).replace('STACKV', hex(STACK))
    recomp_closure.EXTRA += '\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
    exe = build(index(args.recomp), names, main_src, args.out_dir, tag, stubs=stubs, vector=True, scratch_end=0x6000, flags=flags)
    inp = args.out_dir / (tag + '_cases.txt')
    inp.write_text(harness_text(cases, kinds))
    import subprocess as sp
    r = sp.run([str(exe), str(args.image), str(inp)], capture_output=True, text=True, check=True)
    lines = [x for x in r.stdout.split('\n') if x]
    assert len(lines) == len(cases), (len(lines), len(cases))
    return [l.split(' ') for l in lines]


# ------------------------------------------------------------------------------------------------------------
# case builders

def body_frame(c, rnd, n=NL, parents=None, flags=False, pivot=None, joints=True):
    """collider chain / table, body header, link pointers, joint pointers and parents (the words every routine reads)"""
    m = c.mem
    m.put(COLL + 672, [BODY])
    m.put(BODY, [n])
    m.put(BODY + 4, [LINK0 + LSZ * i for i in range(n)])
    if joints:
        m.put(BODY + 132, [JOINT0 + JSZ * j for j in range(n - 1)])
        m.put(BODY + 252, [0] + (parents or [0] * (n - 1))[:n - 1])
    if flags:
        m.put(BODY + 384, [rnd.randrange(0, 4) for _ in range(n)])
    if pivot is not None:
        m.put(BODY + 512, [pivot & 0xFFFFFFFF])


def chain(c, nbones, table):
    m = c.mem
    m.put(COLL + 16, [CHAIN])
    m.put(CHAIN + 4, [CHAIN + 0x40])
    m.put(CHAIN + 0x40 + 12, [CHAIN + 0x80])
    m.put(CHAIN + 0x80 + 144, [nbones])
    m.put(COLL + 716, [TABLE])
    m.put(TABLE, table)


def rand_tree(rnd, n):
    return [rnd.randrange(0, i + 1) for i in range(n - 1)]   # parent of link i+1 (< i+1)


def link_words(c, rnd, i, spans):
    a = LINK0 + LSZ * i
    for lo, hi in spans:
        c.mem.put(a + lo, [rnd.f() for _ in range((hi - lo) // 4)])


def joint_fill(c, rnd, j, kind, parent, child, spec=0.03):
    """a joint block with the words the scratch maps read (type, links, +304 pair, the kind's maps)"""
    a = JOINT0 + JSZ * j
    m = c.mem
    m.put(a, [kind, rnd.f(), LINK0 + LSZ * parent, LINK0 + LSZ * child])
    m.put(a + 304, [rnd.f(spec) for _ in range(16)])
    if kind == 0:
        m.put(a + 640, [rnd.f(spec) for _ in range(8)])       # the spatial axis
        m.put(a + 704, [rnd.f(spec) for _ in range(16)])      # U, U'
        m.put(a + 768, [rnd.f(spec) for _ in range(3)])       # d, 1/d, 1/d'
    else:
        m.put(a + 896, [rnd.f(spec) for _ in range(4)])       # the point
        m.put(a + 1200, [rnd.f(spec) for _ in range(48)])     # G', G, H', H


def island_state(c, rnd, with_pivot=True, n=NL):
    parents = rand_tree(rnd, n)
    body_frame(c, rnd, n, parents, flags=True, pivot=(rnd.randrange(-1, n) if with_pivot and rnd.random() < 0.8 else -1))
    for i in range(n):
        link_words(c, rnd, i, [(0, 128), (736, 880)])
    for j in range(n - 1):
        joint_fill(c, rnd, j, rnd.choice((0, 1)), parents[j], j + 1)
    return parents


def island_outs(c, n=NL):
    for i in range(n):
        c.out(LINK0 + LSZ * i, 32)
    for j in range(n - 1):
        c.out(JOINT0 + JSZ * j + 304, 16)
    c.out(BODY + 384, n)
    c.out(BODY + 512, 1)


def gen_island(rnd, tool):
    cases = []
    # F-78078 -> F-67069
    for _ in range(300):
        c = Case('pr')
        nb = rnd.randrange(1, 10)
        chain(c, nb, [rnd.choice((-1, 0, 1, 2, 3, 4, 5)) & 0xFFFFFFFF for _ in range(10)])
        body_frame(c, rnd, joints=False)
        c.mem.put(COLL + 256, rnd.v())
        for i in range(NL):
            link_words(c, rnd, i, [(736, 880)])
        bone = rnd.choice((-1, nb, nb + 3, rnd.randrange(0, 10))) if rnd.random() < 0.4 else rnd.randrange(0, nb)
        c.reg(3, COLL).reg(4, bone & 0xFFFFFFFF).reg(5, OUT).reg(11, *rnd.v(4, 0.04))
        c.out(OUT, 12)
        cases.append(c)
    # F-41014 -> F-19400
    for _ in range(200):
        c = Case('am')
        nb = rnd.randrange(1, 10)
        chain(c, nb, [rnd.choice((-1, 0, 1, 2, 3, 4, 5)) & 0xFFFFFFFF for _ in range(10)])
        body_frame(c, rnd, joints=False)
        c.mem.put(COLL, [VT])
        c.mem.put(VT + 136, [tool['F-19400']])
        c.mem.put(COLL + 256, rnd.v())
        for i in range(NL):
            c.mem.put(LINK0 + LSZ * i + 704, rnd.v(8))
        bone = rnd.choice((-1, nb, nb + 3, rnd.randrange(0, 10))) if rnd.random() < 0.4 else rnd.randrange(0, nb)
        c.reg(3, COLL).reg(4, bone & 0xFFFFFFFF).reg(11, *rnd.v()).reg(12, *rnd.v(4, 0.04)).reg(13, *rnd.v(4, 0.04)).reg(14, 0x3F800000)
        for i in range(NL):
            c.out(LINK0 + LSZ * i + 704, 8)
        cases.append(c)
    # F-18964
    for _ in range(300):
        c = Case('cp')
        nb = rnd.randrange(1, 10)
        chain(c, nb, [rnd.choice((-1, 0, 1, 2, 3, 4, 5)) & 0xFFFFFFFF for _ in range(10)])
        body_frame(c, rnd, parents=rand_tree(rnd, NL))
        for j in range(NL - 1):
            c.mem.put(JOINT0 + JSZ * j, [rnd.choice((0, 0, 1, 2)), 0, LINK0, LINK0 + LSZ * (j + 1)])
            c.mem.put(JOINT0 + JSZ * j + 532, [rnd.choice((0, 0x01000000, 0xAB000000, 0x00123456))])
        part = rnd.choice((-1, nb, nb + 2, rnd.randrange(0, 10))) if rnd.random() < 0.3 else rnd.randrange(0, nb)
        c.reg(3, COLL).reg(4, part & 0xFFFFFFFF)
        for j in range(NL - 1):
            c.out(JOINT0 + JSZ * j + 532, 1)
        cases.append(c)
    # F-48604 (closure: F-10668, the walk and the joints' maps)
    for _ in range(250):
        c = Case('ar')
        island_state(c, rnd)
        piv = rnd.choice((-1, 0, 1, 2, 3, rnd.randrange(0, NL)))
        c.mem.put(BUF, rnd.v(8, 0.04))
        c.reg(3, BODY).reg(4, rnd.randrange(0, NL)).reg(5, BUF).reg(6, piv & 0xFFFFFFFF)
        island_outs(c)
        cases.append(c)
    # F-96841
    for _ in range(250):
        c = Case('pi')
        island_state(c, rnd)
        c.mem.put(BUF, rnd.v(4, 0.04))
        c.mem.put(BUF + 16, rnd.v(4, 0.04))
        c.reg(3, BODY).reg(4, rnd.randrange(0, NL)).reg(5, BUF).reg(6, BUF + 16)
        island_outs(c)
        cases.append(c)
    # F-79699: a swing-twist joint between links 0 and 1
    for _ in range(700):
        c = Case('bc')
        a = JOINT0
        ell = rnd.random() < 0.5
        w = []
        c.mem.put(a, [1, 0, LINK0, LINK0 + LSZ])
        c.mem.put(a + 608, [rnd.fin() if rnd.random() < 0.9 else rnd.f(), rnd.f(), rnd.fin() if rnd.random() < 0.9 else rnd.f()])
        c.mem.put(a + 632, [(0x01000000 if ell else 0)])
        c.mem.put(a + 636, [rnd.f()])
        c.mem.put(a + 652, [rnd.f()])
        c.mem.put(a + 660, [rnd.f() if rnd.random() < 0.7 else fb(abs(bf(rnd.fin())) * 2)])
        c.mem.put(a + 768, rnd.v(4, 0.03))
        c.mem.put(a + 832, rnd.v(12, 0.03))
        c.mem.put(LINK0 + 624, rnd.v())
        c.mem.put(LINK0 + LSZ + 624, rnd.v())
        # make the swing test often active: theta past the limit, or the ellipse >= 1
        if rnd.random() < 0.6:
            lim = abs(bf(rnd.fin())) + 1e-3
            c.mem.put(a + 608, [fb(lim)])
            c.mem.put(a + 616, [fb(abs(bf(rnd.fin())) + 1e-3)])
            c.mem.put(a + 660, [fb(lim * rnd.uniform(0.5, 4))])
        c.reg(3, a)
        c.out(a + 636, 1).out(a + 652, 1)
        cases.append(c)
    # F-47802
    for _ in range(300):
        c = Case('hd')
        a = JOINT0
        c.mem.put(a, [0, 0, LINK0, LINK0 + LSZ])
        c.mem.put(a + 608, [rnd.f(), rnd.f()])
        c.mem.put(a + 624, [rnd.f(), rnd.f(), rnd.choice((0xFFFFFFFF, 1, 0, 0xFFFFFFFE, 7))])
        c.mem.put(BUF, [0xDEADBEEF] * 4)
        c.reg(3, a).reg(5, BUF).reg(6, BUF + 4).reg(7, BUF + 8).reg(8, BUF + 12)
        c.out(BUF, 4)
        cases.append(c)
    return cases


def gen_artic(rnd, tool):
    cases = []
    # F-63136 (the body's reset part): garbage everywhere in the ranges
    for _ in range(80):
        c = Case('rs')
        parents = rand_tree(rnd, NL)
        body_frame(c, rnd, parents=parents)
        for i in range(NL):
            link_words(c, rnd, i, [(608, 1008)])
        for j in range(NL - 1):
            c.mem.put(JOINT0 + JSZ * j + 304, [rnd.f(0.0) for _ in range(40)])
            c.mem.put(JOINT0 + JSZ * j, [rnd.choice((0, 1)), 0, LINK0, LINK0 + LSZ * (j + 1)])
        c.reg(3, BODY)
        for i in range(NL):
            c.out(LINK0 + LSZ * i + 672, 16).out(LINK0 + LSZ * i + 880, 8).out(LINK0 + LSZ * i + 976, 8)
        for j in range(NL - 1):
            c.out(JOINT0 + JSZ * j + 368, 8)
        cases.append(c)
    # F-91185
    for _ in range(300):
        c = Case('sv')
        body_frame(c, rnd, joints=False)
        for i in range(NL):
            c.mem.put(LINK0 + LSZ * i + 464, rnd.v(4))
            c.mem.put(LINK0 + LSZ * i + 608, rnd.v(4))
            c.mem.put(LINK0 + LSZ * i + 624, rnd.v(4))
        c.mem.put(BUF, rnd.v(4, 0.04))
        c.reg(3, BODY).reg(4, BUF)
        for i in range(NL):
            c.out(LINK0 + LSZ * i + 608, 16)
        cases.append(c)
    # F-38496
    for _ in range(3000):
        c = Case('tr')
        r = rnd.random()
        ang = (fb(rnd.uniform(-7, 7)) if r < 0.4 else fb(rnd.uniform(-1000, 1000)) if r < 0.55 else fb(rnd.uniform(-1e-3, 1e-3)) if r < 0.65 else
               rnd.choice((0, 0x80000000, 0x7FC00000, 0x7F800000, 0xFF800000, 0x3FC90FDB, 0x40490FDB, 0x4B800000, 0x00000400)) if r < 0.75 else rnd.f())
        axis = rnd.v(4, 0.04) if rnd.random() < 0.5 else _unit(rnd)
        c.reg(3, OUT).reg(11, *axis).reg(12, ang, ang, ang, ang)
        c.out(OUT, 12)
        cases.append(c)
    # F-21563 -> F-69646
    for _ in range(150):
        c = Case('gr')
        body_frame(c, rnd, joints=False)
        c.mem.put(BODY + 516, [rnd.f()])
        for i in range(NL):
            c.mem.put(LINK0 + LSZ * i + 464, rnd.v())
            c.mem.put(LINK0 + LSZ * i + 480, rnd.v())
            c.mem.put(LINK0 + LSZ * i + 672, rnd.v(8))
        c.mem.put(GRAV, rnd.v())
        c.reg(3, COLL)
        for i in range(NL):
            c.out(LINK0 + LSZ * i + 672, 8)
        cases.append(c)
    # F-53726
    for _ in range(800):
        c = Case('si')
        a = LINK0
        c.mem.put(a + 416, rnd.v(12, 0.03) if rnd.random() < 0.3 else _frame(rnd))
        c.mem.put(a + 464, rnd.v(4, 0.03))
        c.mem.put(a + 480, rnd.v(4, 0.03))
        c.mem.put(a + 496, rnd.v(4, 0.03))
        c.reg(3, a)
        c.out(a + 128, 36)
        cases.append(c)
    # F-25250 / F-93538: a hinge joint's outer product taken off its inertia
    for kind, rows_at, u_at, s_at in (('m1', 160, 704, 772), ('m2', 16, 736, 776)):
        for _ in range(800):
            c = Case(kind)
            a = JOINT0
            c.mem.put(a + rows_at, rnd.v(36, 0.03))
            c.mem.put(a + u_at, rnd.v(8, 0.03))
            c.mem.put(a + s_at, [rnd.f(0.03)])
            c.reg(3, a).reg(4, BUF)
            c.out(BUF, 36)
            cases.append(c)
    return cases


def _unit(rnd):
    v = [rnd.gauss(0, 1) for _ in range(3)]
    l = math.sqrt(sum(x * x for x in v)) or 1
    return [fb(x / l) for x in v] + [rnd.f(0.0)]


def _frame(rnd):
    a, b, g = (rnd.uniform(-math.pi, math.pi) for _ in range(3))
    ca, sa, cb, sb, cc, sc = math.cos(a), math.sin(a), math.cos(b), math.sin(b), math.cos(g), math.sin(g)
    r = [[cb * cc, cb * sc, -sb], [sa * sb * cc - ca * sc, sa * sb * sc + ca * cc, sa * cb], [ca * sb * cc + sa * sc, ca * sb * sc - sa * cc, ca * cb]]
    return [fb(x) for row in r for x in row + [0.0]]


def gen_build(rnd, tool):
    cases = []
    # the constructors on a block of the heap fill (the recording's 0xCD): the whole block out
    fill = FILL * 0x01010101
    for kind, size, reg in (('jb', 784, 1), ('jb', 1408, 1), ('hj', 784, 1), ('lb', LSZ, 1)):
        c = Case(kind)
        a = JOINT0 if kind != 'lb' else LINK0
        c.mem.put(a, [fill] * (size // 4))
        c.reg(3, a).out(a, size // 4)
        cases.append(c)
    # F-33001: the rest frame becomes the current frame (the link constructor zeroed +576 / +592)
    for _ in range(1500):
        c = Case('rc')
        a = LINK0
        c.mem.put(a + 416, [fill] * 16)
        c.mem.put(a + 464, rnd.v(4, 0.03))
        c.mem.put(a + 512, rnd.v(16, 0.03))
        c.mem.put(a + 576, [0] * 4)
        c.mem.put(a + 592, [0] * 4)
        c.mem.put(a + 608, [fill] * 4)
        c.mem.put(a + 624, [fill] * 4)
        c.mem.put(a + 640, [fill] * 4)
        c.mem.put(a + 656, [fill] * 4)
        c.reg(3, a)
        c.out(a + 416, 16).out(a + 608, 16)
        cases.append(c)
    return cases


def gen_run(rnd, tool):
    cases = []
    # F-78981: the iterator's record (+0 the list, +4 the run's instance, +8 the index), the entry's +160 -> (A, B)
    for _ in range(1500):
        c = Case('ol')
        n = rnd.randrange(1, 5)
        idx = rnd.randrange(0, n)
        run_inst = rnd.randrange(0, 5)
        c.mem.put(BUF, [BUF + 0x40, run_inst, idx])
        c.mem.put(BUF + 0x40, [BUF + 0x80 + 0x20 * i for i in range(n)])
        for i in range(n):
            c.mem.put(BUF + 0x80 + 0x20 * i + 160, [BUF + 0x300 + 8 * i])
            c.mem.put(BUF + 0x300 + 8 * i, [rnd.randrange(0, 5), rnd.randrange(0, 5)])
        c.reg(3, BUF)
        cases.append(c)
    return cases


def write(out, name, cases, res):
    (out / name).write_text('\n'.join(data_line(c, r) for c, r in zip(cases, res)) + '\n')
    print(name, len(cases), 'cases')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--data-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--group', default='all')
    a = ap.parse_args()
    a.out_dir.mkdir(parents=True, exist_ok=True)
    fid = {f: addr(a.tool, f) for f in (
        'F-78078', 'F-67069', 'F-41014', 'F-19400', 'F-18964', 'F-48604', 'F-96841', 'F-79699', 'F-47802', 'F-63136', 'F-91185', 'F-38496',
        'F-21563', 'F-29294', 'F-53726', 'F-25250', 'F-93538', 'F-15442', 'F-77258', 'F-60898', 'F-33001', 'F-78981')}
    sub = {f: 'sub_%08X' % x for f, x in fid.items()}
    rnd = Rng(20261109)
    call = lambda f: '%s(ctx,nullptr);' % sub[f]
    if a.group in ('all', 'island'):
        cases = gen_island(rnd, fid)
        calls = {'pr': call('F-78078'), 'am': call('F-41014'), 'cp': call('F-18964'), 'ar': call('F-48604'), 'pi': call('F-96841'),
                 'bc': call('F-79699'), 'hd': call('F-47802')}
        disp = 'if (a == 0x%08X) %s(ctx, base); else throw std::runtime_error("indirect");' % (fid['F-19400'], sub['F-19400'])
        res = run_cases(a, cases, calls, disp, [sub[f] for f in ('F-78078', 'F-41014', 'F-19400', 'F-18964', 'F-48604', 'F-96841', 'F-79699', 'F-47802')], 'artic_island')
        write(a.data_dir, 'artic_island_ref.txt', cases, res)
    if a.group in ('all', 'artic'):
        cases = gen_artic(rnd, fid)
        calls = {'rs': call('F-63136'), 'sv': call('F-91185'), 'tr': call('F-38496'), 'gr': call('F-21563'), 'si': call('F-53726'),
                 'm1': call('F-25250'), 'm2': call('F-93538')}
        res = run_cases(a, cases, calls, 'throw std::runtime_error("indirect");', [sub[f] for f in ('F-63136', 'F-91185', 'F-38496', 'F-21563', 'F-53726', 'F-25250', 'F-93538')], 'artic')
        write(a.data_dir, 'artic_ref.txt', cases, res)
    if a.group in ('all', 'build'):
        cases = gen_build(rnd, fid)
        calls = {'jb': call('F-15442'), 'hj': call('F-77258'), 'lb': call('F-60898'), 'rc': call('F-33001')}
        res = run_cases(a, cases, calls, 'throw std::runtime_error("indirect");', [sub[f] for f in ('F-15442', 'F-77258', 'F-60898', 'F-33001')], 'artic_build', stubs={sub['F-29294']: MEMSET_BODY})
        write(a.data_dir, 'artic_build_ref.txt', cases, res)
    if a.group in ('all', 'run'):
        cases = gen_run(rnd, fid)
        calls = {'ol': call('F-78981')}
        res = run_cases(a, cases, calls, 'throw std::runtime_error("indirect");', [sub['F-78981']], 'artic_run')
        write(a.data_dir, 'artic_run_ref.txt', cases, res)


if __name__ == '__main__':
    main()
