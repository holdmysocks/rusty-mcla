"""The slipstream turbo's leader search F-69277 run offline on the generated code: the SST object's own gates (its tune
[+4] and car [+8], the car's chassis +224 against the tune's +32), then over the racer list ([0x828745E4] +0 / +400, or
+404 / +804 with the thread's byte +72) every racer whose car is not this one, has an SST object with its +4 set
([[car+8]+668]+4), is not a motorcycle (F-54299) and has no gyro or one with +18 at 0: the strongest slipstream
(F-22877), strictly greater, the first of equals. Stubbed: F-27863 (the case's answer, asked once a racer), F-54299 and
F-22877 (the case's per racer). The port's decode: physics::sst::find_leader.

The first line holds the pointers every case has; a case is "address value" pairs stored after them (0xF0000000 + k:
the stubs' answers). Writes <out-dir>/sst_leader_ref.txt: "pairs -> r3 out-float out-index | calls"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the recomp's x86 flags)

addr = bs.addr

S, TUNE, MEW, MECH, TLS, TB, OUTF, OUTI, L = 0x1000, 0x1040, 0x1080, 0x1100, 0x1200, 0x1240, 0x1300, 0x1310, 0x1400
ENT, CAR, CH = 0x1800, 0x2000, 0x2800  # (+0x40, +0x80, +0x300 a racer)
MESO = 0x4000
RET = 0xF0000000
N = 8


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


HARNESS = r'''
#include <sstream>
#include <string>
static uint32_t g_ret[32]; static std::string g_log;
static int racer_of(uint32_t car) { for(int k=0;k<8;k++) if(car==0x2000u+0x80u*k) return k; return -1; }
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
            else if(w[i]&0x40000000u) *pointer(w[i]&0x3FFFFFFFu)=uint8_t(w[i+1]);
            else if(w[i]&0x20000000u) store16(w[i]&0x1FFFFFFFu,uint16_t(w[i+1]));
            else store(w[i],w[i+1]);
        }
        store(OUTF_,0x11111111u); store(OUTI_,0x22222222u);
        Context ctx{}; ctx.r1.u32=0x4F00; ctx.r13.u32=0x1200; ctx.r3.u32=0x1000; ctx.r4.u32=OUTF_; ctx.r5.u32=OUTI_;
        FN(ctx,nullptr);
        std::printf("%x %08x %08x |%s\n",ctx.r3.u32,load(OUTF_),load(OUTI_),g_log.c_str());
    }
}
'''


def base():
    p = []
    w = lambda a, v: p.extend([a, v])
    w(S + 8, MEW); w(MEW + 8, MECH); w(MECH + 668, MESO)
    w(TLS, TB); w(0x828745E4, L)
    for k in range(N):
        w(L + 4 * k, ENT + 0x40 * k); w(L + 404 + 4 * k, ENT + 0x40 * k)
    return p


def case(rnd):
    p = []
    w = lambda a, v: p.extend([a, v & 0xFFFFFFFF])
    ch = rnd.choice
    U = rnd.uniform
    if rnd.random() < 0.95:
        w(S + 4, TUNE)
    if rnd.random() < 0.03:
        w(S + 8, 0)
    mn = U(5, 20)
    w(TUNE + 32, bits(mn))
    w(MECH + 224, bits(ch([U(0, 60), mn, U(0, mn)])))
    w(MESO + 4, 1)
    side = ch([0, 1])
    w(0x40000000 | (TB + 72), side)
    n = ch([0, 1, 2, 3, 5, 8, 8])
    w(L + (804 if side else 400), n)
    w(L + (400 if side else 804), ch([0, n, 8]))  # (the other list's count: not read)
    me = rnd.randrange(N) if rnd.random() < 0.7 else -1
    for k in range(N):
        e, c, h = ENT + 0x40 * k, CAR + 0x80 * k, CH + 0x300 * k
        if rnd.random() < 0.05:
            w(L + (404 if side else 0) + 4 * k, 0)
            continue
        if k == me:
            w(e + 48, MEW)
            continue
        if rnd.random() < 0.05:
            continue
        w(e + 48, c); w(c + 8, h)
        if rnd.random() < 0.9:
            w(h + 668, h + 0x2C0); w(h + 0x2C0 + 4, ch([1, 1, 1, 0]))
        if rnd.random() < 0.5:
            w(c + 48, c + 0x40); w(0x20000000 | (c + 0x40 + 18), ch([0, 0, 0, 1, 0xFFFF]))
        w(RET + 1 + k, ch([0, 0, 0, 1]))
        w(RET + 9 + k, bits(ch([0.0, U(0, 1), U(0, 1), -0.0, U(-1, 0), 0.5])))
    w(RET + 0, ch([1, 1, 1, 0]))
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
    fn = sub('F-69277')
    rnd = random.Random(69277)
    cases = [case(rnd) for _ in range(a.cases)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'sst_leader_cases.txt'
    nl = chr(10)
    head = ' '.join('%x' % x for x in base())
    inp.write_text(head + nl + nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#include <string>\nstatic uint32_t g_ret[32]; static std::string g_log;\n'
                             'static int racer_of(uint32_t);\n')
    stubs = {
        sub('F-27863'): 'g_log+=" K"; ctx.r3.u64=g_ret[0];',
        sub('F-54299'): '{ int k=racer_of(ctx.r3.u32); char s[32]; std::snprintf(s,sizeof s," M%d",k); g_log+=s; ctx.r3.u64=k>=0?g_ret[1+k]:0; }',
        sub('F-22877'): '{ int k=racer_of(ctx.r4.u32); char s[32]; std::snprintf(s,sizeof s," S%d,%x",k,ctx.r3.u32); g_log+=s;'
                        ' float x; std::memcpy(&x,&g_ret[9+(k<0?0:k)],4); ctx.f1.f64=x; }',
    }
    idx = recomp_closure.index(a.recomp)
    harness = (HARNESS.replace('FN(', fn + '(').replace('OUTF_', '0x1300u').replace('OUTI_', '0x1310u')
               .replace('static uint32_t g_ret[32]; static std::string g_log;\n', '', 1))
    exe = recomp_closure.build(idx, [fn], harness, a.out_dir, 'sref', stubs=stubs, scratch_end=0x5000)
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'sst_leader_ref.txt').write_text(head + nl + nl.join(lines) + nl)
    found = sum(1 for o in out if o.startswith('1 '))
    print(len(lines), 'cases;', found, 'found a leader')


if __name__ == '__main__':
    main()
