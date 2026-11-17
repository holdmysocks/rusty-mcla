"""The garage stat panel's fitted figures run offline: the original's F-17130 (the cur_ speed, acceleration and handling of the
live car), the generated code whole and unchanged, on random cases. Stubs: the player lookup F-65049 hands a player whose
+48 is the case's car, the record's vehicle index F-25618 hands the case's catalogue row, the motorcycle test F-54299 hands
the case's flag. Not stubbed: the part-state getter F-37057 (reads the record's CarCfgPerf bytes +17 .. +20 = record +37 .. +40)
and the engine getter F-49211 (the live engine's tune +4).
Memory: car +76 the type record, whose +104 / +108 are the base / mods mcCarSimTune sets (+112 Mass, +144 the engine tune
(+4 MaxHorsePower), +148 the transmission tune (+12 HighGearMPH)); car +8 the vehicle: +16 the engine (+4 its live tune,
+4 MaxHorsePower), +20 the transmission (+4 its live tune, +12 HighGearMPH), +128 the body (+192 mass). The catalogue row
(104 bytes, table pointer the image word 0x828CD0A8): +64 BaseAccel, +68 ModsAccel, +72 BaseSpeed, +76 ModsSpeed,
+80 BaseHndlg, +84 ModsHndlg.
Writes <out-dir>/garage_cur_meters_ref.txt, one case a line: the case's 20 words (catalogue six f32 bits; base hp, mass, high
gear; mods hp, mass, high gear; live hp, mass, high gear; the four part bytes; the motorcycle flag) '->' the ints of modes
1 (speed), 2 (acceleration) and 3 (handling)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

THIS = 0x40100000
PLAYER = 0x40108000
CAR = 0x40108400
TYPE = 0x40108800
SETB = 0x40108C00
SETM = 0x40109000
ENGB = 0x40109400
ENGM = 0x40109600
TRB = 0x40109800
TRM = 0x40109A00
VEH = 0x4010A000
LENG = 0x4010A400
LENGT = 0x4010A600
LTR = 0x4010A800
LTRT = 0x4010AA00
BODY = 0x4010AC00
REC = 0x40110000
TABLE = 0x40120000
TABLE_PTR = 0x828CD0A8
STACK = 0x40F00000
IDX = 0x40107000
BIKE = 0x40107004
OUT = 0x40107010


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def case(rnd):
    ca = lambda: rnd.choice((0.0, 1.0, rnd.uniform(0, 1), round(rnd.uniform(0, 1), 2), k / 100 if (k := rnd.randrange(101)) else 0.0))
    sp = lambda: rnd.choice((100.0, 160.0, rnd.uniform(100, 260), float(rnd.randrange(100, 260)), round(rnd.uniform(100, 260), 1)))
    hp_b, hp_m = rnd.uniform(100, 600), rnd.uniform(100, 900)
    ms_b = rnd.uniform(150, 2500)
    ms_m = rnd.choice((ms_b, ms_b - rnd.uniform(0, 300), rnd.uniform(150, 2500)))
    hg_b = rnd.uniform(90, 220)
    hg_m = rnd.choice((hg_b, hg_b + rnd.uniform(0, 60), rnd.uniform(90, 260)))
    if rnd.random() < 0.05:
        hp_m, ms_m = hp_b, ms_b
    t = rnd.choice((0.0, 1.0, rnd.uniform(0, 1), rnd.uniform(-0.3, 1.3), round(rnd.uniform(0, 1), 2)))
    lerp = lambda a, b: a + (b - a) * t
    live = [lerp(hp_b, hp_m), lerp(ms_b, ms_m), lerp(hg_b, hg_m)]
    if rnd.random() < 0.3:
        live = [rnd.uniform(100, 900), rnd.uniform(150, 2500), rnd.uniform(90, 260)]
    parts = [rnd.choice((0, 0, 1, 2, 3, rnd.randrange(256))) for _ in range(4)]
    return [ca(), ca(), sp(), sp(), ca(), ca(), hp_b, ms_b, hg_b, hp_m, ms_m, hg_m] + live, parts, rnd.randrange(2)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    st = lambda k: 'sub_' + addr(a.tool, k)
    rnd = random.Random(17130)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    words = ','.join(','.join(['%du' % fb(x) for x in f] + ['%du' % p for p in parts] + ['%du' % bike]) for f, parts, bike in cases)
    stubs = {
        st('F-65049'): 'ctx.r3.u32=%du;' % PLAYER,
        st('F-25618'): 'ctx.r3.u32=load(%du);' % IDX,
        st('F-54299'): 'ctx.r3.u32=load(%du);' % BIKE,
    }
    links = [(PLAYER + 48, CAR), (CAR + 76, TYPE), (TYPE + 104, SETB), (TYPE + 108, SETM), (SETB + 144, ENGB), (SETB + 148, TRB),
             (SETM + 144, ENGM), (SETM + 148, TRM), (CAR + 8, VEH), (VEH + 16, LENG), (LENG + 4, LENGT), (VEH + 20, LTR),
             (LTR + 4, LTRT), (VEH + 128, BODY), (TABLE_PTR, TABLE)]
    # (each case's 20 words: 0..5 the catalogue row, then base hp / mass / high gear, mods, live, the four part bytes, the flag)
    places = [ENGB + 4, SETB + 112, TRB + 12, ENGM + 4, SETM + 112, TRM + 12, LENGT + 4, BODY + 192, LTRT + 12]
    main_src = (
        'static const uint32_t C[]={' + words + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        ''.join('store(%du,%du);' % l for l in links) + nl +
        'for(size_t i=0;i<sizeof(C)/80;i++){const uint32_t*c=C+20*i;uint32_t n=uint32_t(i%%7);store(%du,n);uint32_t r=%du+104*n;' % (IDX, TABLE) +
        'for(int k=0;k<6;k++)store(r+64+4*k,c[k]);' +
        ''.join('store(%du,c[%d]);' % (p, 6 + k) for k, p in enumerate(places)) +
        'for(int k=0;k<4;k++)*pointer(%du+k)=uint8_t(c[15+k]);store(%du,c[19]);' % (REC + 37, BIKE) +
        'for(int k=0;k<20;k++)printf("%s%08x",k?" ":"",c[k]);printf(" ->");' + nl +
        'for(uint32_t m=1;m<=3;m++){store(%du,0x7fffffffu);Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;ctx.r4.u32=%du;ctx.r5.u32=m;ctx.r6.u32=%du;' % (OUT, STACK, THIS, REC, OUT) +
        '%s(ctx,nullptr);printf(" %%d",(int)load(%du));}' % (st('F-17130'), OUT) +
        'printf("\\n");}}' + nl)
    idx = index(a.recomp)
    exe = rd.build(idx, [st('F-17130')], main_src, a.out_dir, 'garage_cur_meters', image=rd.load_image(a.image), stubs=stubs)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'garage_cur_meters_ref.txt').write_text(out)
    lines = [x for x in out.split(nl) if x]
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
