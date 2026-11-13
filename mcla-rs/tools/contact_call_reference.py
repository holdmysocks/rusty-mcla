"""mcCarDamage's contact callback F-11688 (vtable slot 3, one call per run of a car's contacts) run offline on the
generated code: the gate (+36 points under MaxDamage), the other object and mcCarStuck's slot 4, the first enabled
contact (pair +165 & 0x80) with its own / other sides, the hit-by masks +84 / +86 / +89, the two velocities at the point
(slot 21), dv (F-19931), the jolt (JoltThreshold to ImpactThreshold over 5 .. 30 m/s; the racing wheel's effect, the
camera's jolt F-68650, the pad's rumble 1), the normals summed, the LandingDampFactor set on a roof-side contact, the
point of a car lying over, the scrape (over 3 m/s, along the surface: rumble 2, a draw of the generator 0x82860880), the
zone damage kind 0 and kind 1 (F-74840) with their rumbles, the largest sqrt dv (0x8288DCC4), and F-96323.
Real: F-45769, F-83617, F-38702, F-58287. Stubbed and logged: F-19931 (the case's dv), F-31409, F-14919, F-55794,
F-35412, F-17707, F-86040, F-68650, F-74840, F-96323 and the virtual calls. The port's decode:
physics::contact_call::run.

The first line holds the pointers every case has; a case is "address value" pairs stored after them (0xF0000000 + k: the
stubs' answers; 0x40000000 | a: a byte; 0x20000000 | a: a halfword; image addresses as they are). Writes
<out-dir>/contact_call_ref.txt: "pairs -> +84 +86 +89 aero+4 stat gen0 gen1 | calls" (the generator's words in 16-bit
halves)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the recomp's x86 flags)

addr = bs.addr

THIS, DT, INST, CH, VCH, AERO, BODY, ST = 0x1000, 0x1080, 0x1100, 0x1300, 0x1480, 0x14C0, 0x1500, 0x1640
STUCK, VSTK, RUMB, VRUM, ZG, I = 0x1780, 0x1790, 0x17C0, 0x17D0, 0x1800, 0x1840
IT, ARR, PAIR, AB, Q = 0x1880, 0x18A0, 0x1900, 0x1E00, 0x1F40
K2, K3, VK = 0x2000, 0x2200, 0x2600
ENTS, VE, XE = (0x2700, 0x2780, 0x2800), 0x2880, (0x28A0, 0x28C0, 0x28E0)
LT, TBL, PM = 0x2900, 0x2980, 0x2A00
RAC, CAM, X1, X2, CT = 0x3000, 0x3400, 0x3480, 0x3500, 0x3580
WARR, WE = 0x3600, 0x3800
CARS = (0x3C00, 0x3C40, 0x3C80)
GEN, STAT, CHEAT = 0x82860880, 0x8288DCC4, 0x8288E5A7
RET = 0xF0000000
NP = 6


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def f(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


HELPERS = r'''
static int g_vel;
static void fl(const char *t,double x) { float y=float(x); uint32_t b; std::memcpy(&b,&y,4); char s[32]; std::snprintf(s,sizeof s,"%s%x",t,b); g_log+=s; }
static void vecw(uint32_t a) { char s[48]; std::snprintf(s,sizeof s,",%x,%x,%x",load(a),load(a+4),load(a+8)); g_log+=s; }
'''

HARNESS = r'''
#include <sstream>
#include <string>
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    uint32_t r3=ctx.r3.u32; char s[96];
    if(a==0x0F000010) { std::snprintf(s,sizeof s," K,%x,%x,%x,%x",load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r4.u32+12)); g_log+=s; }
    else if(a==0x0F100004) { int k=int((r3-0x2700u)/0x80u); std::snprintf(s,sizeof s," Y%d",k); g_log+=s; ctx.r3.u64=int64_t(int32_t(g_ret[1+k])); }
    else if(a==0x0F200054) {
        std::snprintf(s,sizeof s," U,%x,%x,%x,%x,%x",r3,ctx.r5.u32,ctx.v1.u32[3],ctx.v1.u32[2],ctx.v1.u32[1]); g_log+=s;
        for(int i=0;i<3;i++) store(ctx.r4.u32+4*i,g_ret[8+3*g_vel+i]); g_vel++;
    }
    else if(a==0x0F30001C) { std::snprintf(s,sizeof s," R%d",ctx.r4.u32); g_log+=s; }
    else if(a==0x0F40002C) { g_log+=" N"; ctx.r3.u64=int64_t(int32_t(g_ret[4])); }
    else if(a==0x0F400030) { g_log+=" M"; ctx.r3.u64=g_ret[5]; }
    else { std::printf("indirect %08x\n",a); std::exit(5); }
}
int main(int argc,char **argv) {
    std::ifstream bin(argv[2],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    std::ifstream cases(argv[1]); std::string line;
    std::vector<uint32_t> basew; { std::getline(cases,line); std::istringstream in(line); std::string tok; while(in>>tok) basew.push_back(uint32_t(std::stoul(tok,nullptr,16))); }
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        std::memset(objects,0,sizeof(objects)); g_log.clear(); std::memset(g_ret,0,sizeof g_ret); g_vel=0;
        auto put=[&](uint32_t a,uint32_t v) {
            if(a>=0xF0000000u) g_ret[a-0xF0000000u]=v;
            else if(a&0x80000000u) store(a,v);
            else if(a&0x40000000u) *pointer(a&0x3FFFFFFFu)=uint8_t(v);
            else if(a&0x20000000u) store16(a&0x1FFFFFFFu,uint16_t(v));
            else store(a,v);
        };
        for(size_t i=0;i+1<basew.size();i+=2) put(basew[i],basew[i+1]);
        for(size_t i=0;i+1<w.size();i+=2) put(w[i],w[i+1]);
        Context ctx{}; ctx.r1.u32=0x5F00; ctx.r13.u32=0x5F80; ctx.r3.u32=0x1000; ctx.r4.u32=0x1880;
        FN(ctx,nullptr);
        uint32_t g0=load(0x82860880u),g1=load(0x82860884u);  // (the generator in halves: no word reads as an address)
        std::printf("%x %x %x %08x %08x %04x:%04x %04x:%04x |%s\n",(uint32_t(pointer(0x1054u)[0])<<8)|pointer(0x1055u)[0],(uint32_t(pointer(0x1056u)[0])<<8)|pointer(0x1057u)[0],*pointer(0x1059u),load(0x14C4u),
                    load(0x8288DCC4u),g0>>16,g0&0xFFFF,g1>>16,g1&0xFFFF,g_log.c_str());
    }
}
'''


def base():
    p = []
    w = lambda a, v: p.extend([a, v])
    w(THIS + 4, INST); w(THIS + 8, DT)
    w(INST + 8, CH); w(INST + 12, ZG); w(INST + 24, RUMB); w(INST + 28, I); w(INST + 44, STUCK)
    w(CH, VCH); w(VCH + 44, 0x0F40002C); w(VCH + 48, 0x0F400030); w(CH + 8, AERO); w(CH + 128, BODY); w(CH + 368, ST)
    w(CH + 28, WARR)
    for k in range(3):
        w(WARR + 4 * k, WE)
    w(STUCK, VSTK); w(VSTK + 16, 0x0F000010); w(RUMB, VRUM); w(VRUM + 28, 0x0F30001C)
    w(IT, ARR); w(IT + 4, I)
    for k in range(NP):
        pr, ab = PAIR + 0xC0 * k, AB + 0x30 * k
        w(ARR + 4 * k, pr); w(pr + 160, ab)
        w(0x20000000 | (Q + 0x10 * k + 8), k)
    for o in (BODY, K2, K3):
        w(o, VK)
    w(VK + 84, 0x0F200054)
    for k, e in enumerate(ENTS):
        w(e, VE); w(e + 4, XE[k]); w(e + 80, CARS[k])
    w(VE + 4, 0x0F100004)
    w(0x828DA464, LT); w(LT + 112, TBL); w(0x828CCEA0, PM)
    w(RAC + 880, CAM); w(RAC + 48, X1); w(X1 + 76, X2); w(X2 + 100, CT)
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
    mx = f(ch([100.0, 250.0, U(50, 600)]))
    w(DT + 24, bits(mx))
    w(THIS + 36, bits(ch([0.0, U(0, mx), U(0, mx), mx, f(mx - 1), U(mx, mx * 1.2)])))
    jolt, imp = f(U(5, 15)), f(U(1, 4))
    w(DT + 48, bits(jolt)); w(DT + 32, bits(imp)); w(DT + 52, bits(f(U(5, 30))))
    w(0x40000000 | (THIS + 33), ch([0] * 10 + [1, 2]))
    w(0x20000000 | (THIS + 84), ch([0, 0, rnd.getrandbits(15)])); w(0x20000000 | (THIS + 86), ch([0, 0, rnd.getrandbits(15)]))
    w(0x40000000 | (THIS + 89), ch([0, 0, 1]))
    w(CHEAT - 3, ch([0] * 12 + [1]))  # (the word holding the byte: an image address takes no byte flag)
    w(0x40000000 | (PM + 6), ch([0] * 8 + [1]))
    speed = ch([5.0, 30.0, f(U(0, 40)), U(0, 10), U(25, 60), 0.0])
    w(CH + 224, bits(speed))
    # dv: about the jolt's threshold, about 0.1, big, or nothing
    t = min(max((speed - 5.0) * 0.04, 0.0), 1.0)
    thr = jolt + t * (imp - jolt)
    dd = unit(rnd)
    mag = ch([thr * U(0.9, 1.1), thr, U(0, 0.2), 0.1, U(0, 40), U(0, 3), 0.0])
    for i in range(3):
        w(RET + 20 + i, bits(dd[i] * mag))
    w(RET + 23, 0)
    w(RET + 6, ch([1, 1, 0])); w(RET + 7, ch([0, 1])); w(RET + 16, ch([1, 1, 0]))  # (F-31409 own / other car, F-14919)
    w(RET + 4, ch([0, 1, 2, 4, 4, -1])); w(RET + 5, ch([1, 1, 1, 0, 0x100]))  # (slots 11 and 12)
    w(RET + 18, ch([0, 0, 0, 1, 0x100]))  # (the racing wheel took it)
    w(ST + 300, bits(ch([0.0, 1.0, f(U(1, 3)), U(0, 1), 1.5])))
    w(AERO + 4, bits(1.0))
    # the racer's camera
    w(0x40000000 | (CAM + 89), ch([1, 1, 0, 2]))
    if rnd.random() < 0.1:
        w(RAC + 880, 0)
    for k in range(4):
        w(CT + 36 + 4 * k, bits(U(0.05, 2.0)))
    # the collider's rows (row 1 sometimes lying over)
    r0 = unit(rnd)
    r1 = ch([[0.0, 1.0, 0.0], tilt(rnd, [0.0, 1.0, 0.0], U(0, 1)), unit(rnd), [0.0, f(0.1), 0.0], [0.0, 0.0999, 0.995]])
    wv(I + 16, r0 + [0.0]); wv(I + 32, r1 + [0.0])
    wv(INST + 400, [U(-30, 30) for _ in range(3)] + [0.0])
    w(GEN, rnd.getrandbits(31)); w(GEN + 4, rnd.getrandbits(31))  # (top bits clear: no word reads as an address)
    w(STAT, bits(ch([0.0, U(0, 6), 2.0])))
    w(CH + 24, ch([0, 1, 3])); w(0x40000000 | (WE + 529), ch([0, 1]))
    wv(BODY + 272, [U(-30, 30) for _ in range(3)] + [0.0])
    for o in (K2, K3):
        wv(o + 272, ch([[U(-30, 30) for _ in range(3)], [0.0, 0.0, 0.0]]) + [0.0])
    w(CH + 128, ch([BODY, BODY, BODY, K3]))
    w(INST + 28, ch([I, I, I, 0x1234]))
    for k, e in enumerate(ENTS):
        w(RET + 1 + k, ch([8, 8, 6, 3]))
        w(XE[k] + 16, ch([0, 4, 8, 0xC, 0x10]))
        w(RET + 17 + k, ch([0, 1, 2, 5, 31, 40, -1]))
    for k in range(6):  # (the two velocities at the point: slot 21's answers, in call order)
        w(RET + 8 + k, bits(U(-40, 40)))
    w(IT + 12, ch([1, 1, 2, 3, 4, 6]))
    w(IT + 8, ch([0, 0, 1, 2]))
    for k in range(NP):
        pr, ab, q = PAIR + 0xC0 * k, AB + 0x30 * k, Q + 0x10 * k
        w(0x40000000 | (pr + 165), ch([0x80, 0x80, 0x81, 0xC0, 0, 0x7F]))
        own_a = rnd.random() < 0.6
        w(ab + 0, I if own_a else q); w(ab + 4, q if own_a else I)
        if rnd.random() < 0.05:
            w(ab + (4 if own_a else 0), 0)  # (no other object: the callback returns)
        insts = [ch([BODY, BODY, K2, K3, 0]), ch([K2, K3, 0, BODY])]
        w(ab + 8, insts[0] if own_a else insts[1]); w(ab + 12, insts[1] if own_a else insts[0])
        w(ab + 28, rnd.getrandbits(12)); w(ab + 36, rnd.getrandbits(12))
        for o in (BODY, K2, K3):
            pass
        w(TBL + 8 * k + 4, ch([0, 0, 1, 2, 3, 4]))
        nn = ch([unit(rnd), [0.0, 1.0, 0.0], tilt(rnd, [0.0, 1.0, 0.0], U(0, 0.5)), [x * U(0, 0.001) for x in unit(rnd)]])
        wv(pr + 80, nn + [0.0])
        wv(pr + 48, [U(-500, 500) for _ in range(3)] + [0.0])
        wv(pr + 64, [U(-500, 500) for _ in range(3)] + [0.0])
    for k, o in enumerate((BODY, K2, K3)):
        w(o + 16, ch([ENTS[k], ENTS[k], 0]))
    w(0x20000000 | (Q + 0x10 * 5 + 8), ch([5, 0xFFFF]))
    return p


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    fn = sub('F-11688')
    rnd = random.Random(11688)
    cases = [case(rnd) for _ in range(a.cases)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'contact_call_cases.txt'
    nl = chr(10)
    head = ' '.join('%x' % x for x in base())
    inp.write_text(head + nl + nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             '#include <string>\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                             'static uint32_t g_ret[32]; static std::string g_log;\n' + HELPERS)
    stubs = {
        sub('F-19931'): ('{ char s[48]; std::snprintf(s,sizeof s," V,%x,%x",ctx.r4.u32,load(ctx.r5.u32+8)); g_log+=s;'
                         ' for(int i=0;i<4;i++) store(ctx.r3.u32+4*i,g_ret[20+i]); }'),
        sub('F-31409'): ('{ char s[32]; std::snprintf(s,sizeof s," H,%x",ctx.r3.u32); g_log+=s;'
                         ' ctx.r3.u64=g_ret[ctx.r3.u32==0x1100u?6:7]; }'),
        sub('F-14919'): '{ g_log+=" F"; ctx.r3.u64=g_ret[16]; }',
        sub('F-55794'): ('{ int k=int((ctx.r3.u32-0x3C00u)/0x40u); char s[16]; std::snprintf(s,sizeof s," X%d",k); g_log+=s;'
                         ' ctx.r3.u64=int64_t(int32_t(g_ret[17+k])); }'),
        sub('F-35412'): '{ char s[32]; std::snprintf(s,sizeof s," G,%x",ctx.r3.u32); g_log+=s; ctx.r3.u64=0x3000; }',
        sub('F-17707'): ('{ char s[48]; std::snprintf(s,sizeof s," J,%x,%x,%x",ctx.r3.u32,ctx.r4.u32,ctx.r5.u32); g_log+=s;'
                         ' ctx.r3.u64=g_ret[18]; }'),
        sub('F-86040'): ('{ char s[48]; std::snprintf(s,sizeof s," J,%x,%x,0",ctx.r3.u32,ctx.r4.u32); g_log+=s;'
                         ' ctx.r3.u64=g_ret[18]; }'),
        sub('F-68650'): '{ char s[32]; std::snprintf(s,sizeof s," C,%x",ctx.r3.u32); g_log+=s; fl(",",ctx.f1.f64); fl(",",ctx.f2.f64); }',
        sub('F-74840'): ('{ uint32_t m=ctx.r4.u32; for(int i=0;i<12;i++) if(load(m+4*i)!=((i%5==0)?0x3F800000u:0u)) { std::printf("rows\\n"); std::exit(6); }'
                         ' char s[48]; std::snprintf(s,sizeof s," Z,%x,%x",ctx.r3.u32,ctx.r5.u32); g_log+=s; fl(",",ctx.f1.f64);'
                         ' vecw(m+48); vecw(ctx.r7.u32); vecw(ctx.r8.u32); }'),
        sub('F-96323'): ('{ char s[64]; std::snprintf(s,sizeof s," T,%x,%x,%x,%x",ctx.r7.u32,ctx.r8.u32,ctx.r9.u32,ctx.r10.u32); g_log+=s;'
                         ' vecw(ctx.r5.u32); vecw(ctx.r6.u32); }'),
    }
    idx = recomp_closure.index(a.recomp)
    harness = HARNESS.replace('FN(', fn + '(')
    exe = recomp_closure.build(idx, [fn], harness, a.out_dir, 'ccall', stubs=stubs, vector=True, scratch_end=0x6000,
                               flags=('-msse4.1', '-mssse3'))
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'contact_call_ref.txt').write_text(head + nl + nl.join(lines) + nl)
    cnt = lambda t: sum(o.count(t) for o in out)
    print(len(lines), 'cases;', cnt(' C,'), 'jolts,', cnt(' J,'), 'wheel calls,', cnt(' Z,'), 'zone calls,', cnt(' T,'), 'hits,',
          cnt(' K,'), 'stuck calls,', sum(1 for o in out if o.split()[3] != '3f800000'), 'aero sets')


if __name__ == '__main__':
    main()
