"""Runs two of the racer's getters offline, whole and unchanged: mcRacer::GetMatrix F-35658 and the brain's drift ratio
F-40927 (the aiv's f3: the gyro tune's DriftTorque +180 / the engine's torque at peak power +20). Writes
<out-dir>/racer_getters_ref.txt, one line a case:
  "m <car present> <thread byte> -> <offset of the returned matrix from its source object>"  (source: car +272 when the
  thread byte is set, the car's instance [+28] +16 when not, the racer +288 without a car)
  "d <car present> <drift torque> <engine torque> -> <result>"  (hex float words)
docs/reference/ai/verify_racer_update.md; test brain::tests::racer_getters_match_the_original.
"""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb

R, CAR, SIM, TLS, INST, ENG, ENGT, GY = (0x40100000 + 0x1000 * k for k in range(8))


def ratio_case(rnd, k):
    d = rnd.choice((0.0, rnd.uniform(0, 3), rnd.uniform(0, 3), rnd.uniform(0, 1e6), -rnd.uniform(0, 3)))
    s = rnd.choice((rnd.uniform(1, 900), rnd.uniform(1, 900), rnd.uniform(0, 1e-3), 295.305237))
    return [int(k % 97 != 5), fb(d), fb(s)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fm, fd = addr(a.tool, 'F-35658'), addr(a.tool, 'F-40927')
    rnd = random.Random(40927)
    nl = chr(10)
    cases = [ratio_case(rnd, k) for k in range(a.cases)]
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    src = (
        'static const uint32_t CS[][3]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(int k=0;k<6;k++){rt_reset();memset(pointer(@R@),0,0x8000);int car=k<4,flag=k&1;'
        'store(@R@+48,car?@CAR@:0u);store(@CAR@+28,@INST@);store(@TLS@,@TLS@+0x100);*pointer(@TLS@+0x100+72)=flag;'
        'Context ctx=fresh();ctx.r13.u32=@TLS@;ctx.r3.u32=@R@;sub_@FM@(ctx,nullptr);uint32_t p=ctx.r3.u32;'
        'uint32_t base=!car?@R@:(flag?@CAR@:@INST@);printf("m %d %d -> %u\\n",car,flag,p-base);}' + nl +
        'for(auto&s:CS){rt_reset();memset(pointer(@R@),0,0x8000);'
        'store(@R@+4,@CAR@);store(@CAR@+48,s[0]?@SIM@:0u);store(@SIM@+8,@ENG@);store(@SIM@+96,@GY@);store(@ENG@+16,@ENGT@);'
        'store(@GY@+180,s[1]);store(@ENGT@+20,s[2]);'
        'Context ctx=fresh();ctx.r3.u32=@R@;sub_@FD@(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t q;memcpy(&q,&r,4);'
        'printf("d %u %08x %08x -> %08x\\n",s[0],s[1],s[2],q);}}' + nl)
    for k, v in (('R', R), ('CAR', CAR), ('SIM', SIM), ('TLS', TLS), ('INST', INST), ('ENG', ENG), ('ENGT', ENGT), ('GY', GY)):
        src = src.replace('@%s@' % k, '%du' % v)
    src = src.replace('@FM@', fm).replace('@FD@', fd)
    exe = rd.build(rd.index(a.recomp), ['sub_' + fm, 'sub_' + fd], src, a.out_dir, 'racergetters', image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases + 6, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'racer_getters_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'lines')


if __name__ == '__main__':
    main()
