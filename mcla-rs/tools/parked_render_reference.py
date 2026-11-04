"""Whether a car's render update runs (F-37359, car slot 6's last step) and the draw object's update count (F-14577)
run offline: the generated code whole and unchanged on flat guest memory. F-37359's tail call of the draw object's
slot 5 (the render update F-37033) is dispatched to a stub that records the call; F-14577 is run on its own with the
count as the case's.
Memory: the car 0x1000 (+448 its type 0x1600: +112 the trailer byte; +452 the behaviour 0x1700 or 0; +456 the render
instance 0x1800 or 0; +500 the case's byte), the instance +24 the draw object 0x1A00 (+0 vtable 0x1B00, slot 5 at
+20; +32 the count).
Writes <out-dir>/parked_render_ref.txt, one case a line: behaviour, instance, +500 byte, count, trailer byte (hex), then
'->', 1 when the render update was called else 0, and the count F-14577 leaves (hex)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic int called;\n')


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    gate, tick = (addr(a.tool, f) for f in ('F-37359', 'F-14577'))
    rnd = random.Random(37359)
    cases = []
    for _ in range(3000):
        beh = int(rnd.random() < 0.4)
        inst = int(rnd.random() < 0.9)
        b500 = rnd.choice((0, 0, 0x10, 0x20, 0x30, 0xEF, 0xFF, rnd.getrandbits(8)))
        count = rnd.choice((0, 1, 2, 3, 3, 4, 0x7FFFFFFF, 0xFFFFFFFF, 0x80000000))
        trailer = rnd.choice((0, 0, 1, 0xFF))
        cases.append([beh, inst, b500, count, trailer])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%x' % x for x in c) for c in cases)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x9100&&ctx.r3.u32==0x1A00)called=1;else throw std::runtime_error("unexpected indirect call");}')
    setup = ('memset(objects,0,sizeof(objects));called=0;'
             'store(0x1000+448,0x1600);objects[0x1600-0x1000+112]=c[4];store(0x1000+452,c[0]?0x1700:0);'
             'store(0x1000+456,c[1]?0x1800:0);objects[0x1000-0x1000+500]=c[2];'
             'store(0x1800+24,0x1A00);store(0x1A00,0x1B00);store(0x1B00+20,0x9100);store(0x1A00+32,c[3]);')
    body = ('static const uint32_t C[][5]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r5.u32=0;'
            'sub_%s(ctx,nullptr);int k=called;' % gate +
            'store(0x1A00+32,c[3]);Context t{};t.r1.u32=0x7F000;t.r3.u32=0x1A00;sub_%s(t,nullptr);' % tick +
            ('printf("%%d %%x%s",k,load(0x1A00+32));}' % NL) + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + gate, 'sub_' + tick], main_src, a.out_dir, 'parked_render', stubs={}, scratch_end=0x80000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'parked_render_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] == '1' for o in out), 'called')


if __name__ == '__main__':
    main()
