"""The tyre effects' per-car update F-67671 run offline on the generated code: over the car's four tyre-effect states
(this +1012, 144 bytes each) it calls the per-wheel rule F-42446 with (state, the car's matrix [chassis +128] +272, wheel,
mirror, burnout flag). The flag: chassis +580 > 0 or the melt [chassis +660] +24 > 0. A motorcycle (chassis +24 == 2): the
front wheel ([+28] +0) twice then the rear ([+28] +4) twice, mirror 1 / 0, the flag only on the rear. A car: the list +404
then the list +400 (count +20, wheels +24 ..), the state index running on, mirror = the place in its list is even, the flag
only on the +400 list. Stubbed: F-42446 (logged with its five arguments). The port's decode: tyrefx::order.

The first line holds the pointers every case has; a case is "address value" pairs stored after them. Writes
<out-dir>/tyrefx_order_ref.txt: "pairs -> | calls"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the recomp's x86 flags)

addr = bs.addr

S, W, CH, MELT, BW, MAT, FL, PL, ST = 0x1000, 0x1040, 0x1100, 0x1400, 0x1440, 0x1500, 0x1600, 0x1680, 0x2000
WH = 0x3000  # (wheels: +0x40 each)


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


HARNESS = r'''
#include <sstream>
#include <string>
int main(int argc,char **argv) {
    std::ifstream bin(argv[2],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    std::ifstream cases(argv[1]); std::string line;
    std::vector<uint32_t> basew; { std::getline(cases,line); std::istringstream in(line); std::string tok; while(in>>tok) basew.push_back(uint32_t(std::stoul(tok,nullptr,16))); }
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        std::memset(objects,0,sizeof(objects)); g_log.clear();
        for(size_t i=0;i+1<basew.size();i+=2) store(basew[i],basew[i+1]);
        for(size_t i=0;i+1<w.size();i+=2) store(w[i],w[i+1]);
        Context ctx{}; ctx.r1.u32=0x4F00; ctx.r13.u32=0x1200; ctx.r3.u32=0x1000;
        FN(ctx,nullptr);
        std::printf("|%s\n",g_log.c_str());
    }
}
'''


def base():
    p = []
    w = lambda a, v: p.extend([a, v])
    w(S + 20, W); w(W + 8, CH); w(CH + 660, MELT); w(CH + 28, BW); w(CH + 128, MAT)
    w(CH + 404, FL); w(CH + 400, PL)
    for k in range(4):
        w(FL + 24 + 4 * k, WH + 0x40 * k); w(PL + 24 + 4 * k, WH + 0x40 * (4 + k))
    w(BW, WH + 0x40 * 8); w(BW + 4, WH + 0x40 * 9)
    return p


def case(rnd):
    p = []
    w = lambda a, v: p.extend([a, v & 0xFFFFFFFF])
    ch = rnd.choice
    if rnd.random() < 0.97:
        w(S + 1012, ST)
    w(CH + 24, ch([2, 2, 1, 0, 3]))
    w(CH + 580, bits(ch([0.0, 0.0, -0.0, rnd.uniform(0, 1), rnd.uniform(-1, 0), 1e-30])))
    w(MELT + 24, bits(ch([0.0, 0.0, -0.0, rnd.uniform(0, 1), rnd.uniform(-1, 0)])))
    nf = ch([0, 2, 2, 2, 1, 3, 4])
    npw = ch([2, 2, 4, 4, 0, 1, 3]) if nf <= 2 else ch([0, 1, 2])
    w(FL + 20, nf); w(PL + 20, npw)
    for k in range(4):  # (shuffled wheels: the call gets the list's entry)
        if rnd.random() < 0.5:
            w(FL + 24 + 4 * k, WH + 0x40 * rnd.randrange(10))
        if rnd.random() < 0.5:
            w(PL + 24 + 4 * k, WH + 0x40 * rnd.randrange(10))
    return p


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    fn = sub('F-67671')
    rnd = random.Random(67671)
    cases = [case(rnd) for _ in range(a.cases)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'tyrefx_order_cases.txt'
    nl = chr(10)
    head = ' '.join('%x' % x for x in base())
    inp.write_text(head + nl + nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += '\n#include <string>\nstatic std::string g_log;\n'
    stubs = {
        sub('F-42446'): '{ char s[80]; std::snprintf(s,sizeof s," %x,%x,%x,%x,%x",ctx.r3.u32,ctx.r4.u32,ctx.r5.u32,ctx.r6.u32,ctx.r7.u32); g_log+=s; }',
    }
    idx = recomp_closure.index(a.recomp)
    harness = HARNESS.replace('FN(', fn + '(')
    exe = recomp_closure.build(idx, [fn], harness, a.out_dir, 'tref', stubs=stubs, scratch_end=0x5000)
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'tyrefx_order_ref.txt').write_text(head + nl + nl.join(lines) + nl)
    print(len(lines), 'cases;', sum(o.count(' ') for o in out), 'calls')


if __name__ == '__main__':
    main()
