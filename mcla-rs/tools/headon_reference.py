"""The head-on wipeout F-43509 (mcCarInstance slot 25) run offline on the generated code over a run of contacts: the gates
(mcCarDamage +33, the player's Agro), |dv| (F-19931) against WipeoutThreshold, then per enabled contact (pair +165 & 0x80) by
the partner's class (slot 1: 6, or 8 with a player driver) or level state 2: the ambient rule (speed, the normal against the
other's back axis, the way against it, the generator 0x82860890, the soft lever arm through F-21955, the angular impulse to
the own collider's slot 38) or the fixed rule (the back axis against the normal, F-63589's cosine; the CG copy F-58408); each
hit: AddDamage (slot 4) with MaxDamage x 0.1, F-69183, the melt store, F-46437. Real: the iterator's getters (F-45769,
F-78981, F-97778, F-83617), F-38702, F-58408, F-63589, F-21955. Stubbed and logged: F-19931 (the case's dv), F-69183,
F-46437 and the virtual calls. The port's decode: physics::headon::run_with.

The first line holds the pointers every case has; a case is "address value" pairs stored after them (0xF0000000 + k: the
stubs' answers; 0x40000000 | a: a byte; 0x20000000 | a: a halfword; image addresses as they are). Writes
<out-dir>/headon_ref.txt: "pairs -> CH+112..+124 melt rng0 rng1 | calls" (the generator's words in 16-bit halves)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the recomp's x86 flags)

addr = bs.addr

THIS, C, D, DT, A, P, CH, ST, TF, I = 0x1000, 0x1040, 0x1100, 0x1140, 0x1180, 0x11C0, 0x1200, 0x1380, 0x13C0, 0x1400
IT, ARR = 0x1500, 0x1540
VD, VP, VQ, VQP, VK = 0x1700, 0x1720, 0x1740, 0x1760, 0x1780
PAIR, AB, Q, QC = 0x1800, 0x1E00, 0x2000, 0x2800
K, K2, LT, TBL, TT = 0x3000, 0x3100, 0x3400, 0x3480, 0x3600
RNG = 0x82860890
RET = 0xF0000000
NP = 6


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def f(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


HARNESS = r'''
#include <sstream>
#include <string>
static void logv(char t,uint32_t r,const Context &ctx) { char s[96]; std::snprintf(s,sizeof s," %c,%x,%x,%x,%x",t,r,ctx.v1.u32[3],ctx.v1.u32[2],ctx.v1.u32[1]); g_log+=s; }
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    uint32_t r3=ctx.r3.u32;
    if(a==0x0F000010) { float x=float(ctx.f1.f64); uint32_t b; std::memcpy(&b,&x,4); char s[48]; std::snprintf(s,sizeof s," D,%x,%x",r3,b); g_log+=s; }
    else if(a==0x0F000028) { g_log+=" P"; ctx.r3.u64=g_ret[0]; }
    else if(a==0x0F100004) { int k=int((r3-0x2000u)/0x80u); char s[16]; std::snprintf(s,sizeof s," C%d",k); g_log+=s; ctx.r3.u64=int64_t(int32_t(g_ret[1+k])); }
    else if(a==0x0F200028) { int k=int((r3-0x2810u)/0x100u); char s[16]; std::snprintf(s,sizeof s," Q%d",k); g_log+=s; ctx.r3.u64=g_ret[8+k]; }
    else if(a==0x0F300098) { logv('A',ctx.r4.u32,ctx); }
    else { std::printf("indirect %08x\n",a); std::exit(5); }
}
int main(int argc,char **argv) {
    std::ifstream bin(argv[2],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    for(int k=0;k<4;k++){store(0x828609E0u+4*k,0x7F800000u);store(0x828608C0u+4*k,0);}
    std::ifstream cases(argv[1]); std::string line;
    std::vector<uint32_t> basew; { std::getline(cases,line); std::istringstream in(line); std::string tok; while(in>>tok) basew.push_back(uint32_t(std::stoul(tok,nullptr,16))); }
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        std::memset(objects,0,sizeof(objects)); g_log.clear(); std::memset(g_ret,0,sizeof g_ret);
        auto put=[&](uint32_t a,uint32_t v) {
            if(a>=0xF0000000u) g_ret[a-0xF0000000u]=v;
            else if(a&0x80000000u) store(a,v);
            else if(a&0x40000000u) *pointer(a&0x3FFFFFFFu)=uint8_t(v);
            else if(a&0x20000000u) store16(a&0x1FFFFFFFu,uint16_t(v));
            else store(a,v);
        };
        for(size_t i=0;i+1<basew.size();i+=2) put(basew[i],basew[i+1]);
        for(size_t i=0;i+1<w.size();i+=2) put(w[i],w[i+1]);
        Context ctx{}; ctx.r1.u32=0x5F00; ctx.r13.u32=0x5F80; ctx.r3.u32=0x1000; ctx.r4.u32=0x1500;
        FN(ctx,nullptr);
        uint32_t g0=load(0x82860890u),g1=load(0x82860894u);  // (the generator in halves: no word reads as an address)
        std::printf("%08x %08x %08x %08x %08x %04x:%04x %04x:%04x |%s\n",load(0x1270u),load(0x1274u),load(0x1278u),load(0x127Cu),
                    load(0x13D8u),g0>>16,g0&0xFFFF,g1>>16,g1&0xFFFF,g_log.c_str());
    }
}
'''


def base():
    p = []
    w = lambda a, v: p.extend([a, v])
    w(THIS + 80, C); w(C + 8, CH); w(C + 20, D); w(C + 28, I); w(C + 128, P)
    w(D, VD); w(VD + 16, 0x0F000010); w(D + 8, DT)
    w(P, VP); w(VP + 40, 0x0F000028)
    w(CH + 368, ST); w(CH + 660, TF)
    w(IT, ARR); w(IT + 4, I)
    w(K, VK); w(VK + 152, 0x0F300098)
    w(0x828DA464, LT); w(LT + 112, TBL); w(0x5F80, TT)
    for k in range(NP):
        pr, ab, q, qc = PAIR + 0xC0 * k, AB + 0x20 * k, Q + 0x80 * k, QC + 0x100 * k
        w(ARR + 4 * k, pr); w(pr + 160, ab)
        w(q, VQ); w(q + 80, qc); w(qc + 128, qc + 0x10); w(qc + 0x10, VQP)
        w(0x20000000 | (q + 8), k)
    w(VQ + 4, 0x0F100004); w(VQP + 40, 0x0F200028)
    return p


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if 0.1 < n <= 1:
            return [x / n for x in v]


def tilt(rnd, v, amount):
    u = unit(rnd)
    r = [a + amount * b for a, b in zip(v, u)]
    n = math.sqrt(sum(x * x for x in r)) or 1.0
    return [x / n for x in r]


def case(rnd):
    p = []
    w = lambda a, v: p.extend([a, v & 0xFFFFFFFF])
    wv = lambda a, v: [w(a + 4 * i, bits(x)) for i, x in enumerate(v)]
    ch = rnd.choice
    U = rnd.uniform
    w(0x40000000 | (D + 33), ch([0] * 12 + [1]))
    w(RET + 0, ch([0, 1, 1]))  # (the player driver)
    if rnd.random() < 0.5:
        w(C + 72, A); w(A + 52, ch([1, 1, 0, 2])); w(A + 48, ch([3, 3, 0, 1]))
    thr = f(U(10, 30))
    w(DT + 36, bits(thr)); w(DT + 24, bits(U(100, 600)))
    dd = unit(rnd)
    mag = ch([thr, thr * U(0.95, 1.05), U(0, 40), thr * 1.5])
    w(RET + 20, bits(dd[0] * mag)); w(RET + 21, bits(dd[1] * mag)); w(RET + 22, bits(dd[2] * mag)); w(RET + 23, 0)
    speed = ch([f(22.35), 22.36, 22.34, U(23, 60), U(0, 40)])
    w(CH + 224, bits(speed))
    w(ST + 284, bits(U(0.5, 3.5)))
    wv(CH + 96, [U(-1, 1), U(-1, 1), U(-1, 1), 0.0])
    wv(CH + 112, [0.0, 0.0, 0.0, 0.0])
    w(TF + 24, bits(U(0, 1)))
    w(RNG, rnd.getrandbits(31)); w(RNG + 4, rnd.getrandbits(31))  # (top bits clear: no word reads as an address)
    ob = unit(rnd)  # (the own back axis)
    wv(I + 48, ob + [0.0])
    vel = ch([[-x * speed * U(0.8, 1.2) + U(-2, 2) for x in ob], [x for x in ob], [U(-30, 30) for _ in range(3)]])
    pos = [U(-500, 500), U(-20, 50), U(-500, 500)]
    wv(K + 256, pos + [0.0]); wv(K + 272, vel + [0.0])
    n = ch([0, 1, 1, 2, 3, 4, 6])
    w(IT + 12, n); w(IT + 8, rnd.randrange(4))
    for k in range(NP):
        pr, ab, q = PAIR + 0xC0 * k, AB + 0x20 * k, Q + 0x80 * k
        w(0x40000000 | (pr + 165), ch([0x80, 0x80, 0x80, 0x81, 0xC0, 0, 0x7F]))
        own_a = rnd.random() < 0.6
        sign = 1.0 if own_a else -1.0
        w(ab + 0, I if own_a else q); w(ab + 4, q if own_a else I)
        w(ab + 8, K if own_a else K2); w(ab + 12, K2 if own_a else K)
        cls = ch([6, 6, 8, 3, 3])
        w(RET + 1 + k, cls); w(RET + 8 + k, ch([0, 1]))
        w(TBL + 8 * k + 4, ch([2, 2, 6, 1, 0, 3]))
        b = ch([tilt(rnd, [-x for x in ob], U(0, 1.2)), unit(rnd)])  # (the other's back axis: often facing the own car)
        wv(q + 48, b + [0.0])
        if cls == 3 or rnd.random() < 0.3:
            nin = ch([tilt(rnd, ob, U(0, 0.8)), unit(rnd)])  # (into the own car: a wall ahead)
        else:
            nin = ch([tilt(rnd, [-x for x in b], U(0, 0.9)), unit(rnd)])
        wv(pr + 80, [sign * x for x in nin] + [0.0])
        imp = [x * U(0, 20000) for x in nin] if rnd.random() < 0.8 else [U(-5000, 5000) for _ in range(3)]
        wv(pr + 112, [sign * x for x in imp] + [0.0])
        pt = [a + ch([U(-3, 3), U(-3, 3), 0.0]) for a in pos]
        wv(pr + (48 if own_a else 64), pt + [0.0])
        wv(pr + (64 if own_a else 48), [U(-500, 500) for _ in range(3)] + [0.0])
    return p


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    fn = sub('F-43509')
    rnd = random.Random(43509)
    cases = [case(rnd) for _ in range(a.cases)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'headon_cases.txt'
    nl = chr(10)
    head = ' '.join('%x' % x for x in base())
    inp.write_text(head + nl + nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             '#include <string>\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                             'static uint32_t g_ret[32]; static std::string g_log;\n')
    stubs = {
        sub('F-19931'): ('{ g_log+=" V"; for(int i=0;i<4;i++) store(ctx.r3.u32+4*i,g_ret[20+i]); }'),
        sub('F-69183'): '{ char s[32]; std::snprintf(s,sizeof s," W,%x",ctx.r3.u32); g_log+=s; }',
        sub('F-46437'): 'g_log+=" S";',
    }
    idx = recomp_closure.index(a.recomp)
    harness = HARNESS.replace('FN(', fn + '(')
    exe = recomp_closure.build(idx, [fn], harness, a.out_dir, 'href', stubs=stubs, vector=True, scratch_end=0x6000,
                               flags=('-msse4.1', '-mssse3'))
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'headon_ref.txt').write_text(head + nl + nl.join(lines) + nl)
    spins = sum(o.count(' A,') for o in out)
    fixed = sum(1 for o in out if o.split()[0] != '00000000' or o.split()[1] != '00000000')
    print(len(lines), 'cases;', spins, 'ambient hits,', sum(o.count(' W,') for o in out), 'hits,', fixed, 'with the CG copied')


if __name__ == '__main__':
    main()
