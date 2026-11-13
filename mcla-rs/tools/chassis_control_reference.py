"""The chassis control update F-17997 (the car's controls into the wheels, the engine and the transmission, each tick)
run offline on the generated code: the function whole, every callee stubbed and recorded, over random chassis states,
so that its own pieces (P1..P36 of docs/reference/vehicle/chassis_control_pieces.md) can be compared with the port's
decode (physics::chassis_control). The stubs answer what the case gives (the chassis' slots 11 / 12 / 13, the two
other objects' slots, the callees with a result) and write nothing; the diagnostic text object's slot 12 answers 0.
The start-up constants the image holds as zero are set as the running game has them (0x828609E0 = +inf, 0x828608C0 = 0).

The first line holds the pointers every case has; a case is a list of "address value" pairs stored after them, before the call (addresses at 0xF0000000 + k: the stubs' answers).
Writes <out-dir>/chassis_control_ref.txt: "pairs -> changed words (address=value) | calls" (hex; a call is
"tag,r3,r4,f1,f2,f3" with the floats as bits, the vector call "R,r3,lanes")."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)
import artic_reference  # noqa: F401 (its shim: the SDK helpers the vector code needs)

addr = bs.addr

# the layout (the Rust test reads the same addresses)
C, TUNE, V, INP, V8, V28, V28O, S, X72, A76, B92 = 0x1000, 0x1400, 0x1580, 0x1600, 0x1640, 0x1660, 0x1680, 0x16A0, 0x16C0, 0x1700, 0x1780
R8O, R7O, DEST, P0O, P1O, C8, C12, E16, T20, O128 = 0x1820, 0x18C0, 0x1960, 0x1990, 0x19C0, 0x1A00, 0x1A40, 0x1A80, 0x1B00, 0x1B80
G400, G404, WARR, Q660, Q664, Y668, P676, R29, OEA, DIAG = 0x1D00, 0x1D40, 0x1D80, 0x1DA0, 0x1DC0, 0x1DE0, 0x1E40, 0x1E60, 0x1E80, 0x1F20
VTC, VTV28, VTV8, VTD, O16, EL, WH = 0x1F40, 0x1F80, 0x1FC0, 0x2000, 0x2100, 0x2900, 0x3200
RET = 0xF0000000


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def f(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


HARNESS = r'''
#include <sstream>
#include <string>
static uint32_t g_ret[16]; static std::string g_log;
static void logc(char t,uint32_t r3,uint32_t r4,double a,double b,double c) {
    char s[96]; float x=float(a),y=float(b),z=float(c); uint32_t xa,xb,xc; std::memcpy(&xa,&x,4); std::memcpy(&xb,&y,4); std::memcpy(&xc,&z,4);
    std::snprintf(s,sizeof s," %c,%x,%x,%x,%x,%x",t,r3,r4,xa,xb,xc); g_log+=s; }
static double retf(int k) { float x; std::memcpy(&x,&g_ret[k],4); return x; }
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    uint32_t r3=ctx.r3.u32;
    if(a==0x0F000044) { logc('a',r3,0,0,0,0); ctx.r3.u64=int64_t(int32_t(g_ret[0])); }
    else if(a==0x0F000048) { logc('b',r3,0,0,0,0); ctx.r3.u64=g_ret[1]; }
    else if(a==0x0F000052) { logc('c',r3,0,0,0,0); ctx.f1.f64=retf(2); }
    else if(a==0x0F100044) { logc('d',r3,0,0,0,0); ctx.f1.f64=retf(3); }
    else if(a==0x0F200052) { logc('e',r3,0,0,0,0); ctx.f1.f64=retf(4); }
    else if(a==0x0F300048) { logc('f',r3,0,0,0,0); ctx.r3.u64=0; }
    else { std::printf("indirect %08x\n",a); std::exit(5); }
}
int main(int argc,char **argv) {
    std::ifstream bin(argv[2],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    for(int k=0;k<4;k++){store(0x828609E0u+4*k,0x7F800000u);store(0x828608C0u+4*k,0);}
    std::ifstream cases(argv[1]); std::string line;
    static uint8_t before[0x4000];
    std::vector<uint32_t> basew; { std::getline(cases,line); std::istringstream in(line); std::string tok; while(in>>tok) basew.push_back(uint32_t(std::stoul(tok,nullptr,16))); }
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        std::memset(objects,0,sizeof(objects)); g_log.clear(); std::memset(g_ret,0,sizeof g_ret);
        for(size_t i=0;i+1<basew.size();i+=2) store(basew[i],basew[i+1]);
        for(size_t i=0;i+1<w.size();i+=2) {
            if(w[i]>=0xF0000000u) g_ret[w[i]-0xF0000000u]=w[i+1];
            else if(w[i]&0x80000000u) store(w[i],w[i+1]);  // (the image's globals: dt, G)
            else if(w[i]&0x40000000u) *pointer(w[i]&0x3FFFFFFFu)=uint8_t(w[i+1]);
            else if(w[i]&0x20000000u) store16(w[i]&0x1FFFFFFFu,uint16_t(w[i+1]));
            else store(w[i],w[i+1]);
        }
        std::memcpy(before,pointer(0x1000),0x4000);
        Context ctx{}; ctx.r1.u32=0x5F00; ctx.r13.u32=0x5F80; ctx.r3.u32=0x1000;
        FN(ctx,nullptr);
        const uint8_t *now=pointer(0x1000);
        for(uint32_t o=0;o<0x4000;o+=4) if(std::memcmp(before+o,now+o,4)) std::printf("%x=%08x ",0x1000+o,load(0x1000+o));
        std::printf("|%s\n",g_log.c_str());
    }
}
'''

# stubs: tag, F-ID, the body after the log
STUBS = [
    ('A', 'F-90102', 'logc(\'A\',ctx.r3.u32,0,ctx.f1.f64,ctx.f2.f64,ctx.f3.f64); store(ctx.r3.u32,g_ret[5]);'),
    ('B', 'F-86323', 'logc(\'B\',0,0,ctx.f1.f64,0,0); ctx.f1.f64=retf(6);'),
    ('C', 'F-40603', 'logc(\'C\',ctx.r3.u32,0,ctx.f1.f64,0,0); ctx.f1.f64=retf(7);'),
    ('D', 'F-16736', 'logc(\'D\',ctx.r3.u32,0,0,0,0);'),
    ('E', 'F-14919', 'logc(\'E\',ctx.r3.u32,0,0,0,0); ctx.r3.u64=g_ret[8];'),
    ('F', 'F-74215', 'logc(\'F\',ctx.r3.u32,0,0,0,0);'),
    ('G', 'F-74056', 'logc(\'G\',ctx.r3.u32,0,0,0,0);'),
    ('H', 'F-57353', 'logc(\'H\',ctx.r3.u32,0,ctx.f1.f64,ctx.f2.f64,ctx.f3.f64);'),
    ('I', 'F-19727', 'logc(\'I\',ctx.r3.u32,0,0,0,0);'),
    ('J', 'F-50158', 'logc(\'J\',ctx.r3.u32,0,ctx.f1.f64,0,0);'),
    ('K', 'F-57868', 'logc(\'K\',ctx.r3.u32,0,0,0,0); ctx.r3.u64=g_ret[9];'),
    ('L', 'F-80284', 'logc(\'L\',ctx.r3.u32,0,0,0,0); ctx.r3.u64=int64_t(int32_t(g_ret[10]));'),
    ('M', 'F-59013', 'logc(\'M\',ctx.r3.u32,ctx.r4.u32,0,0,0);'),
    ('N', 'F-48988', 'logc(\'N\',ctx.r3.u32,0,ctx.f1.f64,0,0);'),
    ('O', 'F-51861', 'logc(\'O\',ctx.r3.u32,0,ctx.f1.f64,0,0);'),
    ('P', 'F-52728', 'logc(\'P\',ctx.r3.u32,0,0,0,0);'),
    ('Q', 'F-82657', 'logc(\'Q\',ctx.r3.u32,ctx.r4.u32,0,0,0);'),
    ('R', 'F-14399', 'char s[80]; std::snprintf(s,sizeof s," R,%x,%x,%x,%x,%x",ctx.r3.u32,ctx.v1.u32[3],ctx.v1.u32[2],ctx.v1.u32[1],ctx.v1.u32[0]); g_log+=s;'),
    ('S', 'F-12574', 'logc(\'S\',ctx.r3.u32,0,0,0,0); ctx.r3.u64=g_ret[11];'),
    ('T', 'F-19385', 'logc(\'T\',ctx.r3.u32,0,0,0,0); ctx.r3.u64=int64_t(int32_t(g_ret[12]));'),
    ('U', 'F-75805', 'logc(\'U\',ctx.r3.u32,ctx.r4.u32,0,0,0); ctx.r3.u64=g_ret[13];'),
    ('V', 'F-71293', 'logc(\'V\',ctx.r3.u32,0,0,0,0); ctx.r3.u64=0x1E80;'),
    ('W', 'F-48205', 'logc(\'W\',ctx.r3.u32,0,ctx.f1.f64,0,0);'),
    ('X', 'F-55794', 'logc(\'X\',ctx.r3.u32,0,0,0,0); ctx.r3.u64=0;'),
    ('Y', 'F-65049', 'logc(\'Y\',ctx.r3.u32,ctx.r4.u32,0,0,0); ctx.r3.u64=0x1F20;'),
    ('Z', 'F-16038', 'logc(\'Z\',ctx.r3.u32,ctx.r4.u32,0,0,0);'),
]


def base():
    """The pointers every case has (the ref file's first line)."""
    p = []
    w = lambda a, v: p.extend([a, v])
    w(C + 0, VTC); w(VTC + 44, 0x0F000044); w(VTC + 48, 0x0F000048); w(VTC + 52, 0x0F000052)
    w(C + 4, V); w(C + 8, C8); w(C + 12, C12); w(C + 16, E16); w(C + 20, T20); w(C + 28, WARR); w(C + 128, O128); w(C + 368, TUNE)
    w(C + 400, G400); w(C + 404, G404); w(C + 660, Q660)
    for i in range(4):
        w(WARR + 4 * i, WH + 0x220 * i)
    w(C + 412, WH); w(C + 416, WH + 0x220)
    w(C + 664, Q664)  # (P11 reads it without a test: always there)
    w(V + 4, INP)  # (P6 writes through it without a test)
    w(V + 8, V8); w(V8, VTV8); w(VTV8 + 52, 0x0F200052)
    w(V + 28, V28); w(V28 + 4, V28O); w(V28O, VTV28); w(VTV28 + 44, 0x0F100044)
    w(V + 76, A76); w(V + 92, B92); w(A76 + 104, R8O); w(A76 + 108, R7O); w(B92 + 136, DEST); w(R8O + 136, P0O); w(R7O + 136, P1O)
    w(DIAG, VTD); w(VTD + 48, 0x0F300048)
    return p


def case(rnd):
    p = []
    w = lambda a, v: p.extend([a, v & 0xFFFFFFFF])
    fl = lambda a, x: w(a, bits(x))
    b8 = lambda a, v: w(0x40000000 | a, v)
    h16 = lambda a, v: w(0x20000000 | a, v)
    ch = rnd.choice
    U = rnd.uniform
    # globals: dt, G
    w(0x827D7508, bits(ch([1 / 60, 1 / 30, U(0, 0.05)])))
    w(0x8288E614, ch([0, 0, 0, 1]))
    nw = ch([4, 4, 4, 2, 2, 0, 1, 3])
    w(C + 24, nw)
    for i in range(6):
        fl(WH + 0x220 * i + 392, U(0.2, 0.5))
        fl(WH + 0x220 * i + 524, U(-1, 1))
    for k, o in enumerate((420, 424, 428, 432)):
        if rnd.random() < 0.6:
            w(C + o, WH + 0x220 * (2 + k))
    b8(Q664 + 32, ch([0, 1]))
    if rnd.random() < 0.6:
        w(C + 668, Y668); w(Y668 + 72, ch([-1, 0, 2, -5]))
    if rnd.random() < 0.5:
        w(C + 676, P676); w(P676 + 16, ch([0, 0, 1])); w(P676 + 20, ch([0, 0, 1]))
    # the vehicle's
    h16(INP + 8, ch([0, 3, 0xFFFF]))
    for o in (12, 16, 20, 24, 28):
        fl(INP + o, ch([0.0, 1.0, U(-1, 1), U(0, 1)]))
    fl(INP + 32, ch([0.0, -1.0, -0.4]))
    if rnd.random() < 0.4:
        w(V + 16, O16); w(O16 + 1928, ch([0, 3, 4, 7]))
    if rnd.random() < 0.6:
        w(V + 48, S); h16(S + 16, ch([0, 0, 1, 0xFFFF, 2])); h16(S + 18, ch([0, 0, 1]))
    if rnd.random() < 0.5:
        w(V + 72, X72); w(X72 + 48, ch([3, 3, 1])); w(X72 + 52, ch([1, 1, 0]))
    fl(P0O + 40, U(-1, 1)); fl(P1O + 40, U(-1, 1)); fl(DEST + 40, U(-1, 1))
    # the chassis' own fields
    for o in (132, 136, 140, 144, 148):
        fl(C + o, ch([0.0, 1.0, U(-1, 1), U(0, 1)]))
    w(C + 624, ch([0, 0, rnd.getrandbits(14), 1 << rnd.randrange(14), 0x80]))
    flags = ch([0, 0, 0, 1, 2, 8, 4, 0x10, 0x12, 2 | 0x10, rnd.getrandbits(5)])
    w(C + 592, flags)
    h16(C + 576, ch([0, 0, 4, rnd.getrandbits(16)]))
    fl(C + 580, ch([0.0, 0.0, U(0, 1.2), 1.0, U(0.9, 1.0)]))
    b8(C + 578, ch([0, 0, 1, 2, 3, 0xFF]))
    b8(C + 579, ch([0, 1]))
    fl(C + 656, U(-1, 1)); fl(C + 260, U(-1, 1)); fl(C + 224, ch([U(0, 60), U(0, 5), 0.0, U(0, 2)]))
    for o in (272, 276, 280):
        fl(C + o, U(-30, 30))
        fl(O128 + o, U(-1, 1))
    fl(O128 + 296, U(-2, 2))
    fl(C + 396, ch([0.0, 0.0, U(0, 1), 1.0]))
    for o in (816, 817, 818, 152):
        b8(C + o, ch([0, 0, 1]))
    for o in (112, 116, 120, 124):
        fl(C + o, U(-1, 1))
    w(C + 372, ch([0, 1, 2, 3]))
    fl(C + 736, ch([0.0, U(0, 1)])); fl(C + 740, ch([0.0, U(-0.2, 1.5)])); b8(C + 744, ch([0, 1]))
    # the tune
    fl(TUNE + 52, U(-0.5, 0.5)); fl(TUNE + 56, ch([U(-0.5, 0.5), 0.0]))
    if rnd.random() < 0.3:  # (c+120 at the tune's for P21's m == -1)
        p.extend([C + 120, p[p.index(TUNE + 56) + 1]])
    fl(TUNE + 112, U(500, 2000)); w(TUNE + 132, ch([0, 1, 2, 3]))
    for o in (224, 228, 232, 236, 240, 248):
        fl(TUNE + o, U(0, 40) if o in (224, 228) else U(0, 2))
    fl(TUNE + 252, U(0, 0.5)); fl(TUNE + 256, U(0.5, 1)); fl(TUNE + 260, U(0, 1)); fl(TUNE + 264, ch([U(-1, 1), -0.5, 0.3]))
    fl(TUNE + 276, ch([0.0, U(0, 2)])); fl(TUNE + 292, U(0, 3)); fl(TUNE + 296, U(0, 3)); fl(TUNE + 300, U(0, 1))
    fl(TUNE + 304, ch([1.0, U(0, 1)])); fl(TUNE + 308, ch([1.0, U(0, 1)]))
    # the objects
    fl(C8 + 4, ch([U(0, 1.5), 1.0])); b8(C8 + 40, ch([0, 1]))
    fl(E16 + 8, U(0, 8000)); h16(E16 + 46, ch([0, 0, 1])); fl(E16 + 68, U(6000, 9000)); fl(E16 + 72, U(500, 1000)); fl(E16 + 76, U(0.2, 0.5))
    gear = ch([0, 1, 2, 3, 4])
    w(T20 + 20, gear); w(T20 + 24, ch([0, 1])); b8(T20 + 32, ch([0, 1])); b8(T20 + 33, ch([0, 1]))
    for g in range(8):
        fl(T20 + 36 + 4 * g, ch([U(-4, 4), 0.0]) if g == gear else U(-4, 4))
    for G, n in ((G400, ch([0, 1, 2, 2, 4])), (G404, ch([0, 1, 2, 2]))):
        w(G + 20, n)
        for i in range(n):
            e = EL + 0x240 * i
            w(G + 24 + 4 * i, e)
            fl(e + 420, U(0.2, 0.4))
            for j in range(4):
                fl(e + 352 + 4 * j, U(-1000, 1000))
            b8(e + 529, ch([0, 1, 1]))
    fl(G400 + 48, U(0, 100))
    fl(Q660 + 20, U(-1, 1)); fl(Q660 + 24, ch([0.0, U(0, 1), 0.005]))
    h16(R29 + 18, ch([0, 1, 0xFFFF, 3, 0xFFFD]))
    for o in (128, 132, 136):
        fl(OEA + o, U(0.5, 2))
    # the stubs' answers
    w(RET + 0, ch([0, 1, 2, -1])); w(RET + 1, ch([0, 0, 1])); fl(RET + 2, ch([0.0, U(-1, 1)]))
    fl(RET + 3, U(0, 2)); fl(RET + 4, U(0, 1)); fl(RET + 5, U(-1, 1)); fl(RET + 6, U(0, 2)); fl(RET + 7, U(0, 2))
    w(RET + 8, ch([0, 1])); w(RET + 9, ch([0, 1])); w(RET + 10, ch([-1, 0, 1, 2])); w(RET + 11, ch([0, 1]))
    w(RET + 12, ch([0, 1])); w(RET + 13, ch([0, R29]))
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
    fn = sub('F-17997')
    rnd = random.Random(17997)
    cases = [case(rnd) for _ in range(a.cases)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'chassis_control_cases.txt'
    nl = chr(10)
    head = ' '.join('%x' % x for x in base())
    inp.write_text(head + nl + nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             'static void dispatch(Context&,const uint8_t*,uint32_t);\n'
                             'static void logc(char,uint32_t,uint32_t,double,double,double);\nstatic double retf(int);\n'
                             'static uint32_t g_ret[16]; static std::string g_log;\n')
    stubs = {sub(fid): body for _, fid, body in STUBS}
    idx = recomp_closure.index(a.recomp)
    harness = HARNESS.replace('FN(', fn + '(').replace('static uint32_t g_ret[16]; static std::string g_log;\n', '', 1)
    exe = recomp_closure.build(idx, [fn], harness, a.out_dir, 'cref', stubs=stubs, vector=True, scratch_end=0x6000,
                               flags=('-msse4.1', '-mssse3'))
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), out[:3])
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'chassis_control_ref.txt').write_text(head + nl + nl.join(lines) + nl)
    tags = {}
    for o in out:
        for t in o.split('|')[1].split():
            tags[t[0]] = tags.get(t[0], 0) + 1
    print(len(lines), 'cases; calls', dict(sorted(tags.items())))


if __name__ == '__main__':
    main()
