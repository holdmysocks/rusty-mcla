"""The car's suspension parts (vehSuspension: the arms, shocks and guides the car builder F-70909 makes from the bones
"arm" / "shock" / "guide" + fl / fr / rl / rr + "0") run offline on the generated code unchanged: the constructor
F-18804, the set-up of the next-to-last builder step (the class's Init F-66627) and the
update (vtable slot 2) over a run of ticks. The wheel's axle displacement F-48387 is stubbed: it returns the
float at the wheel's +600, which the case sets each tick. The partner (the rear axle: +84 / +88 its two wheels) is
given for the cases that have one.

Memory: chassis 0x1000 (+240.. its matrix), wheel 0x2000 (+4 its tune 0x2800, tune +40 the steering limit; +384 the
rest place), partner axle 0x3000 (+84 = 0x3400, +88 = 0x3800: two wheels with their own +600), the part 0x4000,
the local matrix M 0x4800 (the copy the part owns), the global matrix 0x4C00.

Writes <out-dir>/susp_parts_ref.txt: one case a line, "in -> out" (hex words): in = partner(0/1) steer rest(3)
local(16) global(16) then per tick dw dl dr (the wheel's, the partner's two), then f1 for the width setter F-12544
(+88 = fma(+92, f1, 1.0); 0 = not called); out = mode(+96 as i16) inv(+92) then
per tick the 16 words of M. Test `steer_parts::tests::susp_parts_match_the_original`."""
import argparse
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs


fb, addr = bs.fb, bs.addr
# (the update reads the mode as a signed halfword: the shim's register gets an s16 view)
_vs = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda *k: _vs(*k).replace('int32_t s32;', 'int32_t s32; int16_t s16;', 1)
TICKS = 6


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    ctor, init, upd, disp, width = ('sub_' + addr(a.tool, k) for k in ('F-18804', 'F-66627', 'F-17019', 'F-48387', 'F-12544'))
    rnd = random.Random(29)

    def unit(v):
        n = sum(x * x for x in v) ** 0.5
        return [x / n for x in v]

    def frame(kind):
        """a bone matrix: rows 0..2 and the place, w words 0 (the 4th word of the place 1)"""
        if kind == 0:
            r2 = unit([rnd.uniform(-0.2, 0.2), rnd.uniform(-1, 1), rnd.uniform(-1, 1)])
        elif kind == 1:
            r2 = unit([rnd.uniform(-0.05, 0.05), rnd.choice((-1, 1)) * rnd.uniform(0.75, 1), rnd.uniform(-0.6, 0.6)])
        else:
            r2 = unit([rnd.uniform(-1, 1) for _ in range(3)])
        r0 = unit([r2[2], 0.0, -r2[0]] if abs(r2[0]) + abs(r2[2]) > 1e-3 else [1.0, 0.0, 0.0])
        r1 = [r2[1] * r0[2] - r2[2] * r0[1], r2[2] * r0[0] - r2[0] * r0[2], r2[0] * r0[1] - r2[1] * r0[0]]
        pos = [0.0, 0.0, 0.0] if rnd.random() < 0.25 else [rnd.uniform(-1, 1), rnd.uniform(-0.2, 0.6), rnd.uniform(-1.6, 1.6)]
        if rnd.random() < 0.3:
            pos[0] = rnd.choice((-1, 1)) * rnd.uniform(0.11, 0.9)
        if rnd.random() < 0.2:
            r2[0] = rnd.uniform(-0.09, 0.09)
        return r0 + [0.0] + r1 + [0.0] + r2 + [0.0] + pos + [1.0]

    cases = []
    for k in range(a.cases):
        partner = 1 if rnd.random() < 0.3 else 0
        steer = rnd.choice((0.0, 0.0, 0.35, 0.5, -0.1))
        rest = [rnd.choice((rnd.uniform(-0.09, 0.09), rnd.uniform(-0.95, -0.6), rnd.uniform(0.6, 0.95))), rnd.uniform(0.2, 0.4), rnd.uniform(-1.6, 1.6)]
        local = frame(rnd.choice((0, 1, 2)))
        glob = local[:] if rnd.random() < 0.5 else frame(rnd.choice((0, 1, 2)))
        ticks = []
        for t in range(TICKS):
            ticks += [rnd.choice((0.0, rnd.uniform(-0.15, 0.15), rnd.uniform(-0.02, 0.02))) for _ in range(3)]
        # (F-67116's f1 = 0.2 - width - fender shift; 0: F-12544 not called)
        f1 = 0.0 if rnd.random() < 0.3 else rnd.choice((rnd.uniform(-0.2, 0.05), rnd.uniform(-1.5, 1.5)))
        cases.append(([partner], [steer] + rest + local + glob + ticks + [f1]))
    n = 1 + 1 + 3 + 32 + 3 * TICKS + 1
    rows = ',\n'.join('{%s}' % ','.join(['%d' % x for x in ints] + ['0x%08x' % fb(float(x)) for x in f]) for ints, f in cases)
    nl = chr(10)
    setup = ('memset(objects,0,0x7000);'
             'store(0x2000+4,0x2800);store(0x2800+40,c[1]);for(int k=0;k<3;k++)store(0x2000+384+4*k,c[2+k]);store(0x2000+384+12,0);'
             'store(0x3000+84,0x3400);store(0x3000+88,0x3800);'
             'for(int k=0;k<16;k++){store(0x4800+4*k,c[5+k]);store(0x4C00+4*k,c[21+k]);}'
             'for(int k=0;k<4;k++){store(0x1000+240+4*k,k==0?0x3f800000:0);store(0x1000+256+4*k,k==1?0x3f800000:0);'
             'store(0x1000+272+4*k,k==2?0x3f800000:0);store(0x1000+288+4*k,k==3?0x3f800000:0);}')
    call = lambda f, r: 'Context ctx{};' + r + 'ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + f + '(ctx,nullptr);'
    body = ('static const uint32_t C[][%d]={' % n + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            '{' + call(ctor, 'ctx.r3.u32=0x4000;') + '}' +
            '{' + call(init, 'ctx.r3.u32=0x4000;ctx.r4.u32=0x1000;ctx.r5.u32=0x2000;ctx.r6.u32=0x4800;ctx.r7.u32=0x4C00;ctx.r8.u32=c[0]?0x3000:0;') + '}' +
            'if(c[37+3*' + str(TICKS) + ']){uint32_t u=c[37+3*' + str(TICKS) + '];float f;memcpy(&f,&u,4);Context ctx{};ctx.r3.u32=0x4000;ctx.f1.f64=f;'
            'ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + width + '(ctx,nullptr);}'
            'printf("%d %08x",(int)(int16_t)(load(0x4000+96)>>16),load(0x4000+92));'
            'for(int t=0;t<' + str(TICKS) + ';t++){store(0x2000+600,c[37+3*t]);store(0x3400+600,c[38+3*t]);store(0x3800+600,c[39+3*t]);'
            '{' + call(upd, 'ctx.r3.u32=0x4000;') + '}'
            'for(int o=0;o<64;o+=4)printf(" %08x",load(0x4800+o));}printf("' + chr(92) + 'n");}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    stubs = {disp: '{uint32_t u=load(ctx.r3.u32+600);float f;memcpy(&f,&u,4);ctx.f1.f64=f;}'}
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [ctor, init, upd, width], main_src, a.out_dir, 'susp_parts', stubs=stubs, vector=True)
    # (the xyz compare mask the set-up ands the bone's place with is zero in the image and xyz all ones in the running
    # original - docs/MAP.md, 2026-11-10: filled in a copy)
    img = bytearray(a.image.read_bytes())
    o = int('8286' + '0950', 16) - int('82' + '000000', 16)
    img[o:o + 16] = bytes.fromhex('ffffffff' * 3 + '00000000')
    a.out_dir.mkdir(parents=True, exist_ok=True)
    image = a.out_dir / 'image_mask.bin'
    image.write_bytes(bytes(img))
    out = [x.strip() for x in recomp_closure.run(exe, image).split(nl) if x.strip()]
    image.unlink()
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%d' % x for x in ints) + ' ' + ' '.join('%08x' % fb(float(x)) for x in f) + ' -> ' + o for (ints, f), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'susp_parts_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
