"""The car's contact callback against a prop, F-58598, run offline on the generated code (the iterator's end test F-45769 and
the own-part getter F-49569 as generated, the manifold reset F-12537 as generated): one contact a case in a manifold of the
car with an instance on the other side, its type (3, 4, 5), the contact's +165 bit, the car's speed (chassis +8 -> +224),
the other's type tune +316 (its least move), its "hit" bit (+80 0x2), its entry (+84, age +420) and the car's part.
Evidence: docs/reference/vehicle/prop_break_check.md.

Stubbed: the other instance's getter F-78981 (returns the case's), the type tune getter F-92722 (returns the case's tune),
the virtual calls (the instance's type, the driver's IsPlayer: false - the hit-prop statistics branch is not run).

Writes <out-dir>/prop_break_ref.txt: "type enabled speed min_move hit entry age part -> flags165 count1260 selected1264"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs

addr = bs.addr


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


HARNESS = r'''
#include <sstream>
enum { CAR=0x2000, DRV=0x2100, PL=0x2A00, BODY=0x2200, ITER=0x2400, ARR=0x2480, REC=0x3000, MAN=0x4000,
       OTH=0x2600, ENT=0x2700, TUNE=0x2800, VTP=0x2900, VTO=0x2940, STACK=0x7F00 };
static uint32_t g_type;
static void dispatch(Context &ctx,const uint8_t *base,uint32_t a) {
    if(a==0x0F000040) ctx.r3.u64=0;            // IsPlayer: no
    else if(a==0x0F000004) ctx.r3.u64=g_type;  // the instance's type
    else { std::printf("indirect %08x\n",a); std::exit(5); }
}
int main(int argc,char **argv) {
    std::ifstream bin(argv[2],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    std::ifstream cases(argv[1]); std::string line;
    while(std::getline(cases,line)) {
        std::istringstream in(line); std::vector<uint32_t> w; std::string tok;
        while(in>>tok) w.push_back(uint32_t(std::stoul(tok,nullptr,16)));
        if(w.empty()) continue;
        std::memset(objects,0,sizeof(objects));
        g_type=w[0];
        store(CAR+80,DRV); store(DRV+128,PL); store(PL,VTP); store(VTP+40,0x0F000040); store(DRV+8,BODY); store(BODY+224,w[2]);
        store(ITER,ARR); store(ITER+4,CAR); store(ITER+8,0); store(ITER+12,1); store(ARR,REC);
        *pointer(REC+165)=uint8_t(w[1]?0xC0:0x40); store(REC+160,MAN);
        store(MAN,CAR); store(MAN+28,w[7]); store(MAN+36,99); store(MAN+1260,3); store(MAN+1264,1);
        store(OTH,VTO); store(VTO+4,0x0F000004); store(TUNE+316,w[3]); store(OTH+80,w[4]?2:0); store(OTH+84,w[5]?ENT:0); store(ENT+420,w[6]);
        Context ctx{}; ctx.r1.u32=STACK; ctx.r3.u32=CAR; ctx.r4.u32=ITER;
        FN(ctx,nullptr);
        std::printf("%02x %d %d\n",*pointer(REC+165),int(load(MAN+1260)),int(load(MAN+1264)));
    }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    fn = sub('F-58598')
    rnd = random.Random(58598)
    cases = []
    for _ in range(4000):
        speed = rnd.choice([rnd.uniform(0, 40), 20.0, 19.999998, 20.000002])
        mm = rnd.choice([rnd.uniform(-2, 2), 0.0, -0.0, -1e-7])
        age = rnd.choice([rnd.uniform(0, 5), 2.0, 1.9999999, 2.0000002])
        c = [rnd.choice([3, 4, 5, 5, 4]), int(rnd.random() < 0.9), bits(speed), bits(mm), int(rnd.random() < 0.8),
             int(rnd.random() < 0.85), bits(age), rnd.choice([0, 0, 0, 1, 2])]
        if rnd.random() < 0.5:
            # (every test passed but at most one, that one on its edge or failed)
            c = [rnd.choice([4, 5]), 1, bits(rnd.uniform(20.01, 60)), bits(rnd.uniform(-2, -0.01)), 1, 1, bits(rnd.uniform(2.01, 9)), 0]
            k = rnd.randrange(9)
            edge = {0: (0, 3), 1: (1, 0), 2: (2, bits(rnd.choice([20.0, 19.999998, 20.000002]))), 3: (3, bits(rnd.choice([0.0, -0.0, -1e-7, 1e-7]))),
                    4: (4, 0), 5: (5, 0), 6: (6, bits(rnd.choice([2.0, 1.9999999, 2.0000002]))), 7: (7, rnd.choice([1, 2]))}
            if k in edge:
                c[edge[k][0]] = edge[k][1]
        cases.append(c)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'prop_break_cases.txt'
    nl = chr(10)
    inp.write_text(nl.join(' '.join('%x' % x for x in c) for c in cases) + nl)
    recomp_closure.EXTRA += ('\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                             'static void dispatch(Context&,const uint8_t*,uint32_t);\n')
    stubs = {sub('F-78981'): 'ctx.r3.u64=0x2600;', sub('F-92722'): 'ctx.r3.u64=0x2800;'}
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], HARNESS.replace('FN(', fn + '('), a.out_dir, 'pref', stubs=stubs, vector=True)
    r = subprocess.run([str(exe), str(inp), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    out = r.stdout.splitlines()
    assert len(out) == len(cases)
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    (a.out_dir / 'prop_break_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[1] == '0' for o in out), 'reset')


if __name__ == '__main__':
    main()
