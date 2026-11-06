"""The walker's remark on the player's car run offline: F-27362 (the gates, the kind, the timer drawn again) with F-47522
(the expensive-car test) and F-77233 (the timer from the shared MWC) as the recomp has them, unchanged, over random walker /
car states with values set near every gate.
Stubbed, as inputs of the case: the player test F-37636 (the case's byte), the first player F-55479 (the player object),
the velocity F-98427, the matrix F-35658 and the walker's place F-58860 (pointers to the case's vectors), the vehicle
table record F-45718 (+44 the price) and the options object F-82698 (+1344 the current slot, +0x3C3C2 + slot x 84 the
purchase records' +82 words); the speech F-39461 records its kind and object.
Writes <out-dir>/walker_remark_ref.txt: one line a case, its words (flags, timer, has player, has car, has damage, vel
xyz, car xyz, ped xyz, points, max, unpainted bytes x6, price, slot, record word, MWC lo, carry) '->' the kind (0 = none),
the speech's object word is the walker (1) or not (0), the timer after, the MWC after; words as hex."""
import argparse
import random
import struct
from pathlib import Path
from car_dodge_reference import addr, fb, f32, word
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += 'static uint32_t KIND,WHO;' + chr(10)

NAMES = ('flags', 'timer', 'has', 'car', 'dmg', 'vx', 'vy', 'vz', 'cx', 'cy', 'cz', 'px', 'py', 'pz', 'points', 'max',
         'u0', 'u1', 'u2', 'u3', 'u4', 'u5', 'price', 'slot', 'rec', 'lo', 'carry')
P = {n: i for i, n in enumerate(NAMES)}
W = len(NAMES)

S, PROF = 0x1000, 0x1000  # the manager (+0x64244 timer, +0x64248 min, +0x6424C max) and the options object: disjoint fields
M, PED, PLAYER, CAR, DMG, TUNE, CFG = 0x70000, 0x70400, 0x70800, 0x70C00, 0x71000, 0x71100, 0x72000
VEL, MAT, PEDPOS, TAB = 0x71200, 0x71300, 0x71400, 0x71500


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=20000)
    a = ap.parse_args()
    ids = ('F-27362', 'F-47522', 'F-77233', 'F-37636', 'F-55479', 'F-98427', 'F-35658', 'F-58860', 'F-45718', 'F-82698',
           'F-39461')
    A = {f: addr(a.tool, f) for f in ids}
    rnd = random.Random(27362)

    def near(c, spread):
        return rnd.choice((c, c + rnd.uniform(-spread, spread), rnd.uniform(0, 3 * c), c * (1 + rnd.uniform(-1e-6, 1e-6))))

    cases = []
    for _ in range(a.cases):
        c = [0] * W
        # (half the cases pass every gate before the kind: the kind's tests and F-47522 then decide)
        inner = rnd.random() < 0.5
        c[P['flags']] = rnd.choice((0, 1, 2, 0x10) if inner else (0, 0, 0, 1, 2, 3, 4, 5, 6, 0x10, rnd.getrandbits(8)))
        c[P['timer']] = fb(f32(0.0 if inner else rnd.choice((0.0, 0.0, 0.0, 0.0, -0.0, 1e-30, rnd.uniform(0, 30)))))
        c[P['has']] = inner or rnd.random() < 0.95
        c[P['car']] = inner or rnd.random() < 0.97
        c[P['dmg']] = inner or rnd.random() < 0.97
        # the flat speed^2 near 44.957 (6.705 m/s)
        sp = rnd.uniform(0, 6.6) if inner else near(6.7050, 0.01) if rnd.random() < 0.6 else rnd.uniform(0, 60)
        h = rnd.uniform(-3.2, 3.2)
        v = (sp * __import__('math').cos(h), rnd.uniform(-20, 20), sp * __import__('math').sin(h))
        car = (rnd.uniform(-2000, 2000), rnd.uniform(-5, 60), rnd.uniform(-2000, 2000))
        # the flat distance near 4.572 (4.57^2 = 20.903), the height difference near 3
        d = rnd.uniform(0, 4.5) if inner else near(4.5720, 0.005) if rnd.random() < 0.7 else rnd.uniform(0, 12)
        g = rnd.uniform(-3.2, 3.2)
        dy = rnd.uniform(-2.9, 2.9) if inner else rnd.choice((near(3.0, 0.001), -near(3.0, 0.001), rnd.uniform(-1, 1), rnd.uniform(-6, 6)))
        ped = (car[0] - d * __import__('math').cos(g), car[1] - dy, car[2] - d * __import__('math').sin(g))
        for k, x in zip(('vx', 'vy', 'vz', 'cx', 'cy', 'cz', 'px', 'py', 'pz'), v + car + ped):
            c[P[k]] = fb(f32(x))
        mx = f32(rnd.choice((100.0, 120.0, 150.0, 200.0, rnd.uniform(1, 400))))
        third = f32(mx * f32(0.333333))
        pts = rnd.choice((third, f32(third + 1e-5), f32(third - 1e-5), rnd.uniform(0, mx), 0.0, mx))
        c[P['points']] = fb(f32(pts))
        c[P['max']] = fb(mx)
        for k in range(6):
            c[P['u%d' % k]] = rnd.choice((0,) * 30 + (1, 2, 0xFF))
        price = rnd.choice((rnd.randrange(8000, 300000), rnd.randrange(8000, 40000), 12345, 37500, 37499, 37501, 75000, 75001,
                            rnd.randrange(0, 100), rnd.getrandbits(32), 0x40000000 + rnd.randrange(0, 3)))
        c[P['price']] = price
        c[P['slot']] = rnd.randrange(0, 30)
        sp2 = (2 * price) & 0xFFFFFFFF
        cap = min(sp2 - (1 << 32) if sp2 >> 31 else sp2, 75000) & 0xFFFF
        c[P['rec']] = rnd.choice((1, 0, 0xCDCC, 0xFFFF, cap + 1, cap + 2, cap, price + 1, rnd.randrange(0, 0x10000)))
        c[P['lo']] = rnd.getrandbits(32)
        c[P['carry']] = rnd.choice((rnd.getrandbits(32), rnd.randrange(0, 0x5CDCFAA7), 0, 1))
        cases.append(c)

    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    stubs = {
        'sub_' + A['F-37636']: 'ctx.r3.u64=KC[%d];' % P['has'],
        'sub_' + A['F-55479']: 'ctx.r3.u64=0x%x;' % PLAYER,
        'sub_' + A['F-98427']: 'ctx.r3.u64=0x%x;' % VEL,
        'sub_' + A['F-35658']: 'ctx.r3.u64=0x%x;' % MAT,
        'sub_' + A['F-58860']: 'ctx.r3.u64=0x%x;' % PEDPOS,
        'sub_' + A['F-45718']: 'ctx.r3.u64=0x%x;' % TAB,
        'sub_' + A['F-82698']: 'ctx.r3.u64=0x%x;' % PROF,
        'sub_' + A['F-39461']: 'KIND=ctx.r4.u32;WHO=ctx.r3.u32==0x%x;' % PED,
    }
    w = lambda n: 'c[%d]' % P[n]
    setup = (
        'memset(objects,0,sizeof(objects));KC=c;KIND=0;WHO=0;'
        'store(0x%x+260,%s);store(0x%x+224,0x%x);' % (M, w('flags'), M, PED) +
        'store(0x%x+0x64244,%s);putf(0x%x+0x64248,15.0f);putf(0x%x+0x6424C,30.0f);' % (S, w('timer'), S, S) +
        'store(0x%x+48,%s?0x%x:0);store(0x%x+52,0x%x);' % (PLAYER, w('car'), CAR, PLAYER, CFG) +
        'store(0x%x+20,%s?0x%x:0);store(0x%x+8,0x%x);store(0x%x+36,%s);store(0x%x+24,%s);'
        % (CAR, w('dmg'), DMG, DMG, TUNE, DMG, w('points'), TUNE, w('max')) +
        'for(int k=0;k<3;k++){store(0x%x+4*k,c[%d+k]);store(0x%x+48+4*k,c[%d+k]);store(0x%x+4*k,c[%d+k]);}'
        % (VEL, P['vx'], MAT, P['cx'], PEDPOS, P['px']) +
        'for(int k=0;k<6;k++)*pointer(0x%x+64+1608+48*k)=uint8_t(c[%d+k]);' % (CFG, P['u0']) +
        'store(0x%x+44,%s);store(0x%x+1344,%s);store16(0x%x+0x3C3C2+84*%s,uint16_t(%s));'
        % (TAB, w('price'), PROF, w('slot'), PROF, w('slot'), w('rec')) +
        'put(0x8286D8E4,0x%x);put(0x82874374,0x%x);put(0x82860890,%s);put(0x82860894,%s);'
        % (S, 0x70F00, w('lo'), w('carry'))
    )
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x7FF00;ctx.r13.u32=0x7E000;ctx.r3.u32=0x%x;' % M +
            'try{sub_%s(ctx,nullptr);}catch(std::exception&e){printf(" EXC %%s",e.what());}' % A['F-27362'] +
            'printf(" %u %u %08x %08x %08x' + NL + '",KIND,WHO,load(0x%x+0x64244),get(0x82860890),get(0x82860894));}' % S + nl)
    main_src = ('static void dispatch(Context&,const uint8_t*,uint32_t){throw std::runtime_error("unexpected indirect call");}' + nl +
                'static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}' + nl +
                'static uint32_t get(uint32_t a){uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|uint32_t(p[1])<<16|uint32_t(p[2])<<8|p[3];}' + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + A['F-27362']], main_src, a.out_dir, 'walker_remark', stubs=stubs, vector=True)
    res = [x for x in run(exe, a.image).split(nl) if x]
    assert len(res) == len(cases), (len(res), len(cases))
    assert not [o for o in res if 'EXC' in o], [o for o in res if 'EXC' in o][:2]
    lines = []
    kinds = {}
    for c, o in zip(cases, res):
        k, who, t, lo, carry = o.split()
        assert k == '0' or who == '1', o
        kinds[k] = kinds.get(k, 0) + 1
        lines.append(' '.join(word(x) for x in c) + ' -> ' + ' '.join((k, who, t, word(int(lo, 16)), word(int(carry, 16)))))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'walker_remark_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases; kinds', sorted(kinds.items()))


if __name__ == '__main__':
    main()
