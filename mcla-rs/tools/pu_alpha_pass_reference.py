"""The car model's draw by render phase (F-10884, the vtable slot 2 that the type walker F-72175 calls for every car model
linked in its type's list) run offline: when does it reach the holder's draw F-24071 (and so the base card F-30631)?
Unchanged generated code on flat guest memory (recomp_closure, vector shim, -msse4.1), guest scratch widened to
0x1000 .. 0x8000. Every callee but F-61576 (the racer's +32 byte) is a stub: F-78515 (IsCarModelReady, logs " R") and F-34053
(the 0x180 phases' scene test, logs " S") answer the case's values, F-24071 logs " H:<r3>" (the holder it is given), the
model draw F-73214 logs " M", the others answer 0.
Memory: car model 0x1000 (+20 the vehicle), vehicle 0x3000 (+12, +68 the holder 0x3400, +76 -> 0x3500 -> +96 0x3580,
+120, +128 the racer 0x3600), racer +32 / +656, the viewport global -> 0x4000, the scene global -> 0x4800, r13 -> 0x4C00.
A case: phase, vehicle, racer, ready, racer +656, racer +32, the scene test (F-34053's answer), car model bytes +593 +594
+123 +6478, halfword +116, vehicle +120 (float bits), +12 set, the two globals' words, the scene byte +393, r13's +72 byte.
Out per case: the case words, then the calls in order.
Usage: pu_alpha_pass_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --tool mcla-tool
       --work DIR --out FILE"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

recomp_closure.EXTRA += '#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda: _wide(_shim())


def _wide(s):
    assert s.count('objects[8192]') == 1 and s.count('a<12288') == 1
    return s.replace('objects[8192]', 'objects[0x7000]').replace('a<12288', 'a<0x8000')

NCASE = 18


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--tool', required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    a = ap.parse_args()
    draw, ready, hdraw, entbyte, scene = (addr(a.tool, f) for f in ('F-10884', 'F-78515', 'F-24071', 'F-61576', 'F-34053'))
    model = addr(a.tool, 'F-73214')
    # the callees of F-10884 (its listing), all stubbed but F-61576 and the save / restore helpers
    src = (a.recomp / 'generated/default')
    callees = set()
    import re
    for p in src.glob('mcla_recomp.*.cpp'):
        t = p.read_text(errors='replace')
        m = re.search(r'^DEFINE_REX_FUNC\(sub_%s\) \{(.*?)^\}' % draw, t, re.M | re.S)
        if m:
            callees = set(re.findall(r'\b(sub_[0-9A-F]{8})\(ctx, base\)', m.group(1)))
            break
    assert callees, 'F-10884 not found'
    stubs = {}
    for c in sorted(callees):
        if c == 'sub_' + entbyte:
            continue
        if c == 'sub_' + hdraw:
            stubs[c] = 'printf(" H:%x",ctx.r3.u32);ctx.r3.u32=0;'
        elif c == 'sub_' + ready:
            stubs[c] = 'printf(" R");ctx.r3.u32=load(0x4F00);'
        elif c == 'sub_' + scene:
            stubs[c] = 'printf(" S");ctx.r3.u32=load(0x4F04);'
        elif c == 'sub_' + model:
            stubs[c] = 'printf(" M");ctx.r3.u32=0;'
        else:
            stubs[c] = 'ctx.r3.u32=0;'
    rnd = random.Random(20261107)
    phases = [1 << k for k in range(31)]
    cases = []
    for i in range(3000):
        ph = 0x2000000 if rnd.random() < 0.5 else rnd.choice(phases)
        veh = int(rnd.random() < 0.9)
        rac = int(rnd.random() < 0.9)
        rdy = int(rnd.random() < 0.85)
        st = 3 if rnd.random() < 0.6 else rnd.choice((0, 1, 2, 4, 5, -1))
        obs = int(rnd.random() < 0.2)
        sc = int(rnd.random() < 0.5)
        b = [int(rnd.random() < 0.5) for _ in range(4)]
        hw = rnd.choice((0, 1, 7))
        f120 = fb(rnd.choice((0.0, -1.0, 0.5, 30.0)))
        v12 = int(rnd.random() < 0.5)
        g1, g2, g393, t72 = (int(rnd.random() < 0.3) for _ in range(4))
        cases.append([ph, veh, rac, rdy, st & 0xFFFFFFFF, obs, sc] + b + [hw, f120, v12, g1, g2, g393, t72])
    for c in cases:
        assert len(c) == NCASE
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    disp = 'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){printf(" V:%x",a);ctx.r3.u32=0;}'
    body = ('static const uint32_t C[][%d]={' % NCASE + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'for(int k=0;k<%d;k++)printf(k?" %%08x":"%%08x",c[k]);' % NCASE +
            # the globals F-10884 reads: the scene object, the viewport, two words, r13's block
            'store(0x8287E064,0x4800);store(0x82839704,0x4000);store(0x8288DD20,c[14]);store(0x8288DD5C,c[15]);' +
            'pointer(0x4800+393)[0]=c[16];store(0x4C00,0x4D00);pointer(0x4D00+72)[0]=c[17];' +
            'store(0x4F00,c[3]);store(0x4F04,c[6]);' +
            'store(0x1000+20,c[1]?0x3000:0);store(0x3000+128,c[2]?0x3600:0);store(0x3000+68,0x3400);' +
            'store(0x3000+76,0x3500);store(0x3500+96,0x3580);store(0x3000+120,c[12]);store(0x3000+12,c[13]?0x3010:0);' +
            'store(0x3600+656,c[4]);pointer(0x3600+32)[0]=c[5];' +
            'pointer(0x1000+593)[0]=c[7];pointer(0x1000+594)[0]=c[8];pointer(0x1000+123)[0]=c[9];pointer(0x1000+6478)[0]=c[10];' +
            'store16(0x1000+116,c[11]);store(0x1000+6028,0xFFFFFFFF);' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=c[0];ctx.r1.u32=0x7F00;ctx.r13.u32=0x4C00;try{sub_%s(ctx,nullptr);}catch(std::exception&){printf(" X");}printf("' % draw
            + NL + '");}' + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + draw], main_src, a.work, 'pu_alpha_pass', stubs=stubs, vector=True, flags=('-msse4.1',))
    out = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, check=True).stdout.replace('\r', '')
    lines = out.splitlines()
    assert len(lines) == len(cases), len(lines)
    a.out.write_text(out)
    print(len(lines), 'lines')


if __name__ == '__main__':
    main()
