"""An ambient car's quarter weight F-60791 run offline (generated code unchanged): r3 the car (+448 -> its instance,
whose +104 is the archetype or 0), the archetype's mass call (vtable slot 7: phArchetypePhys / phArchetypeDamp read +40,
the base phArchetype returns a constant) dispatched to the generated getters by the vtable the harness gives the
archetype; f1 out. The constants 9.8 / 0.25 / 500 are read from the image.

Cases: masses round the ambient tunes' (800 .. 40,000 kg), tiny / huge / zero / negative, each archetype class, and no
archetype.

Writes <out-dir>/quarter_weight_ref.txt, one case a line: "kind mass -> f1" (kind 0 no archetype, 1 Phys, 2 Damp,
3 base; hex float bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure
import recomp_dispatch as rd

recomp_closure.EXTRA += ('void indirect(Context &ctx,const uint8_t *base,uint32_t a);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) indirect(ctx,base,(a))\n')


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fq = addr(a.tool, 'F-60791')
    img = rd.load_image(a.image)
    vts = [rd.vtables_of(img, c)[0][0] for c in ('phArchetypePhys@rage', 'phArchetypeDamp@rage', 'phArchetype@rage')]
    getters = sorted({rd.image_u32(img, vt + 28) for vt in vts})
    rnd = random.Random(60791)
    cases = []
    for i in range(6000):
        m = rnd.choice((rnd.uniform(800, 3000), rnd.uniform(3000, 40000), 10 ** rnd.uniform(-30, 30), 0.0, -rnd.uniform(0, 5000),
                        1500.0, 2000.0, 500.0))
        cases.append((rnd.choice((0, 1, 1, 1, 2, 2, 3)), m))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%d,0x%08x},' % (k, fb(m)) for k, m in cases)
    disp = ('void indirect(Context &ctx,const uint8_t *base,uint32_t a){switch(a){' +
            ''.join('case 0x%08X:sub_%08X(ctx,base);return;' % (g, g) for g in getters) +
            'default:throw std::runtime_error("indirect");}}' + nl)
    body = ('static const uint32_t C[][2]={' + nl + rows + nl + '};' + nl +
            'static const uint32_t VT[3]={' + ','.join('0x%08x' % v for v in vts) + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000+448,0x1100);' +
            'if(c[0]){store(0x1100+104,0x1200);store(0x1200,VT[c[0]-1]);store(0x1200+40,c[1]);}' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);' % fq) +
            'float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);printf("%08x' + NL + '",u);}' + nl)
    main_src = (disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + fq] + ['sub_%08X' % g for g in getters], main_src, a.out_dir, 'quarter_weight')
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%d %08x -> %s' % (k, fb(m), o) for (k, m), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'quarter_weight_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases; getters', ' '.join('%08X' % g for g in getters))


if __name__ == '__main__':
    main()
