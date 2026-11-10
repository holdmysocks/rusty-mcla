"""vehGyro's turn F-81152 run offline: generated code unchanged (vector mode, the recomp's own -msse4.1 -mssse3), the
three virtual calls on the body answered by host bodies that record what they are given:
  vtable +132 (the torque applied as a point): records v1 -> "applied"
  vtable +68  (the body's matrix, r4 = where to write 3 rows): writes the case's 3 rows (the world inverse inertia)
  vtable +128 (the force applied): records v1 -> "force"
Guest layout: gyro object 0x1000 (+8 chassis), chassis 0x1100 (+80 the per-axis scale of the torque in the car's frame, (1, 1, 1) in
play and here - the port has no field for it; +128 body; the frame rows +304 / +320 / +336, the place
+352), body 0x1600 (+0 vtable 0x1700, +192 mass), r4 the torque (0x1800), r5 the offset (0x1810), r6 the ground normal
(0x1820, or 0). Writes <out-dir>/gyro_turn_ref.txt: "torque(3) offset(3) rows(12) at(3) matrix(9) mass normal(3 or none) ->
applied(3) force(3)", hex words. No denormal inputs (flush mode is a no-op in this build, the console flushes)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure
from cite_safe import cite_safe

_vector_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda scratch_end=0x3000: '#include <simde/x86/avx2.h>' + chr(10) + _vector_shim(scratch_end)
recomp_closure.EXTRA += ('#define ctr r2' + chr(10) + 'static void dispatch(Context&,const uint8_t*,uint32_t);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10) +
                         'static uint32_t APPLIED[4], FORCE[4], MATRIX[12]; static int calls[3];' + chr(10))


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def is_denormal(w):
    return (w >> 23) & 0xFF == 0 and (w & 0x7FFFFF) != 0


def word(rnd, scale=10.0):
    r = rnd.random()
    if r < 0.1:
        w = rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x00800000, 0x322bcc77))
    elif r < 0.9:
        w = fb(rnd.uniform(-scale, scale))
    else:
        w = rnd.getrandbits(32)
    return 0 if is_denormal(w) else w


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-81152')
    rnd = random.Random(81152)
    cases = []
    for _ in range(6000):
        torque = [word(rnd, 100) for _ in range(3)]
        offset = [word(rnd, 3) for _ in range(3)]
        rows = [word(rnd, 1) for _ in range(12)]            # three rows and the place (w lanes included), as the chassis holds them
        if rnd.random() < 0.7:
            rows[9:12] = [fb(rnd.uniform(-600, 600)) for _ in range(3)]    # a place some hundred metres out
        matrix = [word(rnd, 1) for _ in range(9)]
        mass = word(rnd, 3000)
        if rnd.random() < 0.7:
            n = [rnd.gauss(0, 1) for _ in range(3)]
            normal = [fb(x) for x in n]
            if rnd.random() < 0.3:
                normal = [fb(0.0), fb(1.0), fb(0.0)]
        else:
            normal = None
        cases.append((torque, offset, rows, matrix, mass, normal))
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = []
    for torque, offset, rows, matrix, mass, normal in cases:
        flat += torque + offset + rows + matrix + [mass, 1 if normal else 0] + (normal or [0, 0, 0])
    W = 3 + 3 + 12 + 9 + 2 + 3
    stubs = {
        'fk_torque': 'for(int k=0;k<4;k++)APPLIED[k]=ctx.v1.u32[3-k];calls[0]++;',
        'fk_matrix': 'for(int i=0;i<9;i++)store(ctx.r4.u32+4*(i/3)*4+4*(i%3),MATRIX[i]);for(int r=0;r<3;r++)store(ctx.r4.u32+16*r+12,0);calls[1]++;',
        'fk_force': 'for(int k=0;k<4;k++)FORCE[k]=ctx.v1.u32[3-k];calls[2]++;',
    }
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x7F000001)fk_torque(ctx,base);else if(a==0x7F000002)fk_matrix(ctx,base);else if(a==0x7F000003)fk_force(ctx,base);'
            'else throw std::runtime_error("unexpected indirect call");}')
    body = ('static const uint32_t C[][%d]={' % W + nl + nl.join('{%s},' % ','.join('0x%x' % x for x in flat[k * W:(k + 1) * W]) for k in range(len(cases))) + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;' +
            'store(0x1000+8,0x1100);store(0x1100+128,0x1600);store(0x1600,0x1700);store(0x1700+132,0x7F000001);store(0x1700+68,0x7F000002);store(0x1700+128,0x7F000003);' +
            'store(0x1600+192,c[27]);' +
            'for(int k=0;k<3;k++)store(0x1100+80+4*k,0x3F800000);' +   # (the chassis' per-axis scale +80: (1, 1, 1) live, bikegyro_pseudo.md)
            'for(int i=0;i<3;i++){store(0x1800+4*i,c[i]);store(0x1810+4*i,c[3+i]);}' +
            'for(int r=0;r<4;r++)for(int k=0;k<3;k++)store(0x1100+304+16*r+4*k,c[6+3*r+k]);' +
            'for(int i=0;i<9;i++)MATRIX[i]=c[18+i];' +
            'if(c[28]){for(int k=0;k<3;k++)store(0x1820+4*k,c[29+k]);}' +
            'ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r5.u32=0x1810;ctx.r6.u32=c[28]?0x1820:0;' +
            'memset(APPLIED,0,sizeof(APPLIED));memset(FORCE,0,sizeof(FORCE));calls[0]=calls[1]=calls[2]=0;' +
            ('sub_%s(ctx,nullptr);' % f) +
            'printf("%u%u%u %08x %08x %08x %08x %08x %08x' + NL + '",calls[0],calls[1],calls[2],APPLIED[0],APPLIED[1],APPLIED[2],FORCE[0],FORCE[1],FORCE[2]);}' + nl)
    consts = ('for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);' +
              'store(0x8286093C,0x3F800000);')
    main_src = disp + nl + ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                            nl + consts + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], main_src, a.out_dir, 'gyro_turn', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for k, (c, o) in enumerate(zip(cases, out)):
        torque, offset, rows, matrix, mass, normal = c
        assert o.split()[0] == '111', o
        lines.append('%s -> %s' % (' '.join('%08x' % x for x in torque + offset + rows + matrix + [mass]) + ' ' +
                                   (' '.join('%08x' % x for x in normal) if normal else 'none'), ' '.join(o.split()[1:])))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'gyro_turn_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print(len(lines), 'cases,', sum(1 for c in cases if c[5]), 'with a normal')


if __name__ == '__main__':
    main()
