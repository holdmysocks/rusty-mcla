"""Three small leaves of the car's sim run offline (generated code unchanged, scalar mode):

- the player's shift down F-31773 (r3 +20 the gearbox, its +20 the gear index, 0 reverse, 1 neutral, 2.. forward): the
  gearbox's down shift F-67455 (stubbed: logged) only above index 2;
- first gear out of reverse or neutral F-16736 (r3 the gearbox): SetGear F-59013 (stubbed: logs r4) at index 0 or 1;
- the engine's boost started afresh F-50158 (r3 the engine, its +4 the tune, f1): +28 = f1 above 0, else the tune's +40.

Cases: indices 0 .. 12 and odd ones (negative, huge); f1 0 / -0 / tiny / subnormal / random / negative / NaN / inf,
the tune's +40 random and odd.

Writes <out-dir>/gear_leaves_ref.txt, one case a line, hex: "down gear -> called", "first gear -> called r4",
"boost f1 dur -> +28"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10)


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
    fdown, ffirst, fboost, fdown2, fset = ('sub_' + addr(a.tool, f) for f in ('F-31773', 'F-16736', 'F-50158', 'F-67455', 'F-59013'))
    rnd = random.Random(31773)
    gear = lambda: rnd.choice((0, 1, 2, 3, 4, rnd.randrange(0, 13), rnd.randrange(0, 13), 0xFFFFFFFF, 0x80000000, 0x7FFFFFFF))
    real = lambda: fb(rnd.choice((0.0, -0.0, 1e-40, 1e-30, rnd.uniform(0, 5), rnd.uniform(0, 5), 2.0, -rnd.uniform(0, 5),
                                  float('nan'), float('inf'), float('-inf'), -1e-40)))
    cases = []
    for _ in range(1500):
        cases.append(('down', [gear()]))
    for _ in range(1500):
        cases.append(('first', [gear()]))
    for _ in range(3000):
        cases.append(('boost', [real() if rnd.random() < 0.6 else fb(0.0), real()]))
    nl = chr(10)
    NL = chr(92) + 'n'
    kinds = {'down': 0, 'first': 1, 'boost': 2}
    flat = []
    for kind, ws in cases:
        flat += [kinds[kind], len(ws)] + [int(x) for x in ws]
    stubs = {fdown2: 'printf("called ");', fset: 'printf("called %x ",ctx.r4.u32);'}
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'size_t at=0;for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));' +
            'uint32_t kind=D[at],n=D[at+1];const uint32_t*w=&D[at+2];at+=2+n;' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            f'if(kind==0){{store(0x1000+20,0x1400);store(0x1400+20,w[0]);{fdown}(ctx,nullptr);printf("-");}}' +
            f'else if(kind==1){{store(0x1000+20,w[0]);{ffirst}(ctx,nullptr);printf("-");}}' +
            'else{store(0x1000+4,0x1400);store(0x1400+40,w[1]);float f;memcpy(&f,&w[0],4);ctx.f1.f64=f;' +
            f'{fboost}(ctx,nullptr);printf("%08x",load(0x1000+28));}}' +
            'printf("' + NL + '");}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fdown, ffirst, fboost], main_src, a.out_dir, 'gear_leaves', stubs=stubs)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s %s -> %s' % (kind, ' '.join('%x' % int(x) for x in ws), o) for (kind, ws), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'gear_leaves_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
