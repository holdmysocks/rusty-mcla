"""The cruise assist F-43388 (mcPlayerInput) run offline on the generated code: the smoothed speed +108 (0.2 of the way to
chassis +224), the road's limit +104 (F-87157 x 2.237 when the player manager's road object [+640] is there and F-86691
gives a road; else +104 kept), on / off +101 (driving forward - the chassis' slot 12, F-80284 > 0, no nitrous [+664] +32,
the gyro's +18 at 0 -, a limit, faster than 4.47 m/s; on within CruiseActivationSpeedRange of the limit while the
transmission's modulator +148 is under 0.1, off over 0.9), and while on the throttle +12 remade and +102 / +40's 0x2000.
Stubbed: F-65049 (the player manager), F-35658, F-86691, F-87157, F-80284 and the chassis' slot 12, all answering from the
case and logged. The port's decode: physics::pad::cruise (with its caller's +104 rule).

The first line holds the pointers every case has; a case is "address value" pairs stored after them (0xF0000000 + k: the
stubs' answers; 0x40000000 | a: a byte; 0x20000000 | a: a halfword). Writes <out-dir>/cruise_ref.txt:
"pairs -> +12 +40 +101 +102 +104 +108 | calls"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the recomp's x86 flags)

addr = bs.addr

S, TUNE, CAR, GY, CH, VT, TR, NB, PM = 0x1000, 0x1100, 0x1200, 0x1280, 0x1300, 0x1700, 0x1800, 0x1880, 0x1900
RET = 0xF0000000


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def f(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


HARNESS = r'''
#include <sstream>
#include <string>
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    if(a==0x0F000048) { g_log+=" V"; ctx.r3.u64=g_ret[3]; }
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
        std::memset(objects,0,sizeof(objects)); g_log.clear(); std::memset(g_ret,0,sizeof g_ret);
        for(size_t i=0;i+1<basew.size();i+=2) store(basew[i],basew[i+1]);
        for(size_t i=0;i+1<w.size();i+=2) {
            if(w[i]>=0xF0000000u) g_ret[w[i]-0xF0000000u]=w[i+1];
            else if(w[i]>=0x80000000u) store(w[i],w[i+1]);
            else if(w[i]&0x40000000u) *pointer(w[i]&0x3FFFFFFFu)=uint8_t(w[i+1]);
            else if(w[i]&0x20000000u) store16(w[i]&0x1FFFFFFFu,uint16_t(w[i+1]));
            else store(w[i],w[i+1]);
        }
        Context ctx{}; ctx.r1.u32=0x4F00; ctx.r13.u32=0x1200; ctx.r3.u32=0x1000;
        FN(ctx,nullptr);
        std::printf("%08x %08x %x %x %08x %08x |%s\n",load(0x100Cu),load(0x1028u),*pointer(0x1065u),*pointer(0x1066u),
                    load(0x1068u),load(0x106Cu),g_log.c_str());
    }
}
'''


def base():
    p = []
    w = lambda a, v: p.extend([a, v])
    w(S + 48, TUNE); w(S + 72, CAR); w(CAR + 8, CH); w(CAR + 48, GY)
    w(CH + 0, VT); w(VT + 48, 0x0F000048); w(CH + 20, TR); w(CH + 664, NB)
    w(0x82874374, 0x1A00)
    return p


def case(rnd):
    p = []
    w = lambda a, v: p.extend([a, v & 0xFFFFFFFF])
    ch = rnd.choice
    U = rnd.uniform
    lo = ch([0.1, 0.2, 0.3, U(0.05, 0.5)])
    hi = ch([0.8, 0.9, 0.95, U(0.5, 0.99)])
    rng = ch([0.1, 0.15, 0.2, U(0.02, 0.3)])
    w(TUNE + 100, bits(lo)); w(TUNE + 104, bits(hi)); w(TUNE + 108, bits(rng))
    road = ch([40.0, 55.0, 25.0, 35.0, 65.0]) * 0.44704
    limit_old = ch([0.0, 40.0, 55.0, f(road * 2.237), U(10, 70)])
    w(S + 104, bits(limit_old))
    spd = ch([road, road * U(0.8, 1.2), U(0, 40), 4.47, f(4.47), U(3, 6)])
    w(CH + 224, bits(spd))
    held = ch([spd, f(spd * U(0.9, 1.1)), road * U(0.85, 1.15), U(0, 40)])
    w(S + 108, bits(held))
    w(0x40000000 | (S + 101), ch([0, 1, 1]))
    w(0x40000000 | (S + 102), ch([0, 1]))
    w(S + 40, rnd.getrandbits(31))  # (top bit clear: no word that reads as an address)
    w(S + 12, bits(ch([0.0, 1.0, lo, hi, f(lo + 1e-6), U(0, 1), U(lo, hi), U(hi, 1), U(0, lo)])))
    w(TR + 148, bits(ch([0.0, 0.0, 0.05, f(0.1), 0.5, f(0.9), 0.95, 1.0, U(0, 1)])))
    w(0x40000000 | (NB + 32), ch([0, 0, 0, 0, 1]))
    w(0x20000000 | (GY + 18), ch([0, 0, 0, 0, 1, 0xFFFF]))
    w(RET + 0, ch([PM, PM, PM, 0x1A80]))  # (F-65049: a manager with or without a road object)
    w(PM + 640, ch([0x1B00, 0x1B00, 0]))
    w(RET + 1, ch([0x1C00, 0x1C00, 0]))  # (F-86691)
    w(RET + 2, bits(ch([road, road, U(5, 30), 0.0])))  # (F-87157)
    w(RET + 3, ch([1, 1, 1, 0, 0x100]))  # (slot 12: the byte only)
    w(RET + 4, ch([1, 1, 2, 0, -1]))  # (F-80284)
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
    fn = sub('F-43388')
    rnd = random.Random(43388)
    cases = [case(rnd) for _ in range(a.cases)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'cruise_cases.txt'
    nl = chr(10)
    head = ' '.join('%x' % x for x in base())
    inp.write_text(head + nl + nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             '#include <string>\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                             'static uint32_t g_ret[16]; static std::string g_log;\n'
                             'static double retf(int k) { float x; std::memcpy(&x,&g_ret[k],4); return x; }\n')
    stubs = {
        sub('F-65049'): '{ char s[32]; std::snprintf(s,sizeof s," P%x",ctx.r3.u32); g_log+=s; ctx.r3.u64=g_ret[0]; }',
        sub('F-35658'): 'g_log+=" Q";',
        sub('F-86691'): '{ char s[32]; std::snprintf(s,sizeof s," R%x",ctx.r3.u32); g_log+=s; ctx.r3.u64=g_ret[1]; }',
        sub('F-87157'): '{ char s[32]; std::snprintf(s,sizeof s," L%x",ctx.r3.u32); g_log+=s; ctx.f1.f64=retf(2); }',
        sub('F-80284'): '{ char s[32]; std::snprintf(s,sizeof s," W%x",ctx.r3.u32); g_log+=s; ctx.r3.u64=int64_t(int32_t(g_ret[4])); }',
    }
    idx = recomp_closure.index(a.recomp)
    harness = HARNESS.replace('FN(', fn + '(')
    exe = recomp_closure.build(idx, [fn], harness, a.out_dir, 'cru', stubs=stubs, scratch_end=0x5000)
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'cruise_ref.txt').write_text(head + nl + nl.join(lines) + nl)
    on = sum(1 for o in out if o.split()[2] == '1')
    quiet = sum(1 for o in out if o.split()[3] == '1')
    print(len(lines), 'cases;', on, 'on,', quiet, 'holding')


if __name__ == '__main__':
    main()
