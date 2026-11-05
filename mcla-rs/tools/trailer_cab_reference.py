"""A trailer's cab check (F-99638) run offline: the generated code whole and unchanged on flat guest memory. The ambient
manager's id lookup F-10061 is stubbed (prints the id asked, answers the case's cab or 0) and so is the removal F-51512
(prints the car and the reason). Memory: the car 0x2000 (+448 the type 0x2800 with its byte +112, +3604 the cab id).
Every combination of the type byte (0, 1, 0x80, 0xFF), the id (-1, INT_MIN, 0, 1, 7, INT_MAX) and the lookup's answer.

Writes <out-dir>/trailer_cab_ref.txt, a case a line: byte id found '->' the answer (0 or 1 = the cab), the id asked
(or -), the removal's reason (or -)."""
import argparse
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x10000)
NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, look, rem = addr(a.tool, 'F-99638'), addr(a.tool, 'F-10061'), addr(a.tool, 'F-51512')
    cs = [[b, i & 0xFFFFFFFF, f] for b in (0, 1, 0x80, 0xFF) for i in (-1, -2 ** 31, 0, 1, 7, 2 ** 31 - 1) for f in (0, 1)]
    stubs = {'sub_' + look: 'printf("L%x ",ctx.r4.u32);ctx.r3.u64=found?0x4000:0;',
             'sub_' + rem: 'printf("R%x ",ctx.r5.u32);if(ctx.r4.u32!=0x2000)printf("badcar ");'}
    rows = NL.join('{%s},' % ','.join('0x%08xu' % x for x in c) for c in cs)
    body = ('static const uint32_t C[][3]={' + NL + rows + NL + '};' + NL +
            'for(auto&c:C){memset(objects,0,sizeof(objects));found=c[2];'
            'store(0x2000+448,0x2800);*pointer(0x2800+112)=uint8_t(c[0]);store(0x2000+3604,c[1]);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r3.u32=0x2000;'
            'sub_%s(ctx,nullptr);printf("A%%x\\n",ctx.r3.u32==0x4000?1:ctx.r3.u32);}' % fn) + NL
    main_src = ('static uint32_t found;' + NL + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + NL + body + '}' + NL)
    src = main_src
    idx = index(a.recomp)
    # the stubs read `found`, declared before them
    recomp_closure.EXTRA += 'static uint32_t found;' + NL
    exe = build(idx, ['sub_' + fn], src.replace('static uint32_t found;' + NL, '', 1), a.out_dir, 'trailer_cab', stubs=stubs, scratch_end=0x10000)
    out = [x for x in run(exe, a.image).split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = []
    for c, o in zip(cs, out):
        w = o.split()
        asked = next((x[1:] for x in w if x.startswith('L')), '-')
        reason = next((x[1:] for x in w if x.startswith('R')), '-')
        assert 'badcar' not in w
        ans = next(x[1:] for x in w if x.startswith('A'))
        lines.append('%x %x %x -> %s %s %s' % (c[0], c[1], c[2], ans, asked, reason))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'trailer_cab_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
