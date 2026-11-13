"""The rider's state check (F-94409) run offline: the generated code whole and unchanged on flat guest memory, the
footprint probe F-39379 stubbed (prints that it ran, answers the case's hit). Memory: the behaviour 0x2000 (+272 the
state), the frame context 0x3000 (+4 the world, +8 the car 0x4000 with its byte +500), the world placed so that its
bytes +0x642D1 / +0x642D2 are 0x2001 / 0x2002; the two global bytes read first are written in the image. Every
combination of the states 0 - 10, +500 bit 0x20 (0 / 0x20, other bits set), the two globals, the two world bytes and
the probe's answer.

Writes <out-dir>/state_check_ref.txt, a case a line: state b500 g1 g2 s1 s2 hit '->' the state after, the probe ran
(1 / 0)."""
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
    fn, probe = addr(a.tool, 'F-94409'), addr(a.tool, 'F-39379')
    cs = [[st, b5, g1, g2, s1, s2, h] for st in range(11) for b5 in (0, 0x20, 0xDF, 0xFF) for g1 in (0, 1) for g2 in (0, 7)
          for s1 in (0, 1) for s2 in (0, 0x80) for h in (0, 1)]
    stubs = {'sub_' + probe: 'printf("P ");if(ctx.r3.u32!=0x2000)printf("bad ");ctx.r3.u64=hit;'}
    rows = NL.join('{%s},' % ','.join('0x%02xu' % x for x in c) for c in cs)
    world = (0x2000 - 0x642D0) & 0xFFFFFFFF
    body = ('static const uint32_t C[][7]={' + NL + rows + NL + '};' + NL +
            'for(auto&c:C){memset(objects,0,sizeof(objects));hit=c[6];'
            'store(0x2000+272,c[0]);store(0x3004,0x%08xu);store(0x3008,0x4000);*pointer(0x4000+500)=uint8_t(c[1]);'
            '*pointer(0x8294A564u)=uint8_t(c[2]);*pointer(0x8294A565u)=uint8_t(c[3]);'
            '*pointer(0x2001)=uint8_t(c[4]);*pointer(0x2002)=uint8_t(c[5]);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r3.u32=0x2000;ctx.r4.u32=0x3000;'
            'sub_%s(ctx,nullptr);printf("S%%x\\n",load(0x2000+272));}' % (world, fn)) + NL
    src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + NL + body + '}' + NL
    recomp_closure.EXTRA += 'static uint32_t hit;' + NL
    exe = build(index(a.recomp), ['sub_' + fn], src, a.out_dir, 'state_check', stubs=stubs, scratch_end=0x10000)
    out = [x for x in run(exe, a.image).split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = []
    for c, o in zip(cs, out):
        w = o.split()
        assert 'bad' not in w
        st = next(x[1:] for x in w if x.startswith('S'))
        lines.append('%x %x %x %x %x %x %x -> %s %d' % (*c, st, 'P' in w))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'state_check_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
