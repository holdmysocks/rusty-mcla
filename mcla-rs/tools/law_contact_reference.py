"""The driver's contact walker F-83713 (mcRacer and its kin) run offline on the generated code over a run of contacts: per
contact the other instance (F-78981) and its class (slot 1): a prop (3 with +80 bit 2, an entry +84 whose +420 is 0, keyed by
the root of F-19317's chain; 4, 5, 9, 10, 11) not in the 8-slot table +492: law word bit 1 and F-60290; an ambient car (6) not
in the table, the own speed over 6.705 m/s: bit 8 and F-60290; a racer's car (8) not in the table, either speed over 4.47: the
aggressor (the other when the own car stands under 0.2235, the own when the other stands, else the larger (v . (other - own)) /
speed), its word bit 2 (victim +465), 4 (victim slot 10) or 8, then slot 40 of both. Real: F-45769, F-19317, F-35658 (the
racer's matrix). Stubbed and logged: F-97778 (the case's instance per cursor), F-60290 and the virtual calls. The port's decode:
physics::law_contact.

The first line holds the pointers every case has; a case is "address value" pairs stored after them (0xF0000000 + k: the
stubs' answers; 0x40000000 | a: a byte). Writes <out-dir>/law_contact_ref.txt: "pairs -> the law words (own, the five others') | calls"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the recomp's x86 flags)

addr = bs.addr

R, CAR, CH, BODY, COLL, TLS, TLSP, LAWW, IT, VO, VR = 0x1000, 0x1700, 0x1880, 0x1980, 0x1AC0, 0x1B00, 0x1B80, 0x1BA0, 0x1BC0, 0x1D00, 0x1D80
O = [0x2000 + 0x100 * k for k in range(6)]
ENT = [0x2600 + 0x200 * k for k in range(6)]
OC = [0x3400 + 0x200 * k for k in range(5)]
OCH = [0x4000 + 0x100 * k for k in range(5)]
OB = [0x4600 + 0x200 * k for k in range(5)]
OR = [0x5200 + 0x200 * k for k in range(5)]
RET = 0xF0000000
NP = 6


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def f(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


HELPERS = r'''
static void fl(const char *t,double x) { float y=float(x); uint32_t b; std::memcpy(&b,&y,4); char s[32]; std::snprintf(s,sizeof s,"%s%x",t,b); g_log+=s; }
'''

HARNESS = r'''
#include <sstream>
#include <string>
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    uint32_t r3=ctx.r3.u32; char s[96];
    if(a==0x0F000004) { int k=int((r3-0x2000u)/0x100u); ctx.r3.u64=int64_t(int32_t(g_ret[8+k])); }
    else if(a==0x0F000028) { std::snprintf(s,sizeof s," S,%x",r3); g_log+=s; ctx.r3.u64=g_ret[r3==0x1000u?16:(17+int((r3-0x5200u)/0x200u))]; }
    else if(a==0x0F0000A0) { std::snprintf(s,sizeof s," A,%x,%x,%x",r3,ctx.r4.u32,ctx.r5.u32); g_log+=s; fl(",",ctx.f1.f64); }
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
        auto put=[&](uint32_t a,uint32_t v) {
            if(a>=0xF0000000u) g_ret[a-0xF0000000u]=v;
            else if(a&0x80000000u) store(a,v);
            else if(a&0x40000000u) *pointer(a&0x3FFFFFFFu)=uint8_t(v);
            else store(a,v);
        };
        for(size_t i=0;i+1<basew.size();i+=2) put(basew[i],basew[i+1]);
        for(size_t i=0;i+1<w.size();i+=2) put(w[i],w[i+1]);
        Context ctx{}; ctx.r1.u32=0x7F00; ctx.r13.u32=0x1B80; ctx.r3.u32=0x1000; ctx.r4.u32=0x1BC0;
        FN(ctx,nullptr);
        std::printf("%x %x %x %x %x %x |%s\n",load(0x1BA0u),load(0x53F0u),load(0x55F0u),load(0x57F0u),load(0x59F0u),load(0x5BF0u),g_log.c_str());
    }
}
'''


def base():
    p = []
    w = lambda a, v: p.extend([a, v])
    w(R, VR); w(R + 484, LAWW); w(TLSP, TLS)
    w(VR + 40, 0x0F000028); w(VR + 160, 0x0F0000A0); w(VO + 4, 0x0F000004)
    w(CH + 128, BODY)
    for k in range(NP):
        w(O[k], VO)
    for k in range(5):
        w(OC[k] + 8, OCH[k]); w(OC[k] + 128, OR[k]); w(OC[k] + 28, OC[k] + 0x180)
        w(OCH[k] + 128, OB[k]); w(OR[k], VR); w(OR[k] + 484, OR[k] + 0x1F0)
    return p


def case(rnd):
    p = []
    w = lambda a, v: p.extend([a, v & 0xFFFFFFFF])
    wv = lambda a, v: [w(a + 4 * i, bits(x)) for i, x in enumerate(v)]
    ch = rnd.choice
    U = rnd.uniform
    if rnd.random() < 0.93:
        w(R + 48, CAR)
    w(CAR + 8, ch([CH] * 15 + [0])); w(CAR + 28, ch([COLL] * 15 + [0]))
    w(0x40000000 | (TLS + 72), ch([0, 1]))
    sp = ch([0.0, 0.1, f(0.2235), 0.3, f(4.47), 4.5, f(6.705), 6.71, U(0, 40), U(0, 8)])
    w(CH + 224, bits(sp))
    wv(BODY + 272, [U(-30, 30) for _ in range(3)])
    pos = [U(-200, 200), U(-5, 20), U(-200, 200)]
    wv(CAR + 320, pos); wv(COLL + 64, [x + U(-0.1, 0.1) for x in pos]); wv(R + 336, [x + U(-1, 1) for x in pos])
    w(LAWW, ch([0, 0, rnd.getrandbits(4)]))
    n = ch([1, 1, 2, 3, 4, 6])
    w(IT + 12, n)
    insts = []
    for k in range(NP):
        o = ch([O[k]] * 6 + [0])
        insts.append(o)
        w(RET + k, o)
        cls = ch([3, 3, 4, 5, 6, 6, 7, 8, 8, 8, 9, 10, 11, 2, 12])
        w(RET + 8 + k, cls)
        # (a frag: +80 bit 2, the entry, its +420, its parent chain)
        w(O[k] + 80, ch([2, 2, 0, 3, 6]) if cls == 3 else 0)
        if cls == 3:
            w(O[k] + 84, ch([ENT[k], ENT[k], 0]))
            w(ENT[k] + 420, bits(ch([0.0, 0.0, 1.0])))
            par = ch([0, 0, O[(k + 1) % NP], O[k]])
            w(ENT[k] + 256, par)
        if cls == 8 and k < 5:
            w(O[k] + 80, ch([OC[k]] * 8 + [0]))
            w(OC[k] + 8, ch([OCH[k]] * 10 + [0])); w(OC[k] + 128, ch([OR[k]] * 10 + [0]))
            osp = ch([0.0, 0.1, f(0.2235), 0.3, f(4.47), 4.5, U(0, 40), U(0, 8)])
            w(OCH[k] + 224, bits(osp))
            wv(OB[k] + 272, [U(-30, 30) for _ in range(3)])
            opos = [x + U(-5, 5) for x in pos]
            w(OR[k] + 48, ch([OC[k], OC[k], 0]))
            wv(OC[k] + 320, opos); wv(OC[k] + 0x180 + 64, [x + U(-0.1, 0.1) for x in opos]); wv(OR[k] + 336, [x + U(-1, 1) for x in opos])
            w(0x40000000 | (OR[k] + 465), ch([0, 0, 1]))
            w(RET + 17 + k, ch([0, 0, 1, 0x100]))
    w(RET + 16, ch([0, 1]))
    # (the table: some of the instances, a frag's parent, holes)
    for j in range(8):
        if rnd.random() < 0.25:
            w(R + 492 + 12 * j, ch([x for x in insts if x] + [O[0], 0x1234]))
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
    fn = sub('F-83713')
    rnd = random.Random(83713)
    cases = [case(rnd) for _ in range(a.cases)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'law_contact_cases.txt'
    nl = chr(10)
    head = ' '.join('%x' % x for x in base())
    inp.write_text(head + nl + nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             '#include <string>\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                             'static uint32_t g_ret[32]; static std::string g_log;\n' + HELPERS)
    stubs = {
        sub('F-78981'): '{ ctx.r3.u64=g_ret[load(ctx.r3.u32+8)]; }',
        sub('F-60290'): '{ char s[48]; std::snprintf(s,sizeof s," T,%x,%x",ctx.r3.u32,ctx.r4.u32); g_log+=s; fl(",",ctx.f1.f64); }',
    }
    idx = recomp_closure.index(a.recomp)
    harness = HARNESS.replace('FN(', fn + '(')
    exe = recomp_closure.build(idx, [fn], harness, a.out_dir, 'law', stubs=stubs, vector=True, scratch_end=0x8000,
                               flags=('-msse4.1', '-mssse3'))
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'law_contact_ref.txt').write_text(head + nl + nl.join(lines) + nl)
    cnt = lambda t: sum(o.count(t) for o in out)
    print(len(lines), 'cases;', cnt(' T,'), 'table adds,', cnt(' A,'), 'slot 40 calls,', cnt(' S,'), 'slot 10 calls')


if __name__ == '__main__':
    main()
