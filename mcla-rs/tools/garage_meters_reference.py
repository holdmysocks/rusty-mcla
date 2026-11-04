"""The garage stat panel run offline: the original's F-13938 (the GARAGEMOVIE globals base_ / max_ speed, acceleration and
handling from the vehicle catalogue's record), the generated code whole and unchanged, on random catalogue records. Stubs:
the panel's set-up F-20823 and the part record copy F-78475 do nothing, the player lookup F-65049 hands a player whose
car record leads to the case's vehicle index (F-25618), the movie's global setter F-98200 prints the name and the int it
is given, and the part-level menu lookup F-77906 ends the case (what follows are the 13 part rows).
The catalogue record (104 bytes, filled by F-87336): +64 BaseAccel, +68 ModsAccel, +72 BaseSpeed, +76 ModsSpeed,
+80 BaseHndlg, +84 ModsHndlg; the table's pointer is the image word 0x828CD0A8.
Writes <out-dir>/garage_meters_ref.txt, one record a line: the six fields (f32 bits) '->' the six ints in the order the
panel sets them (base_speed max_speed base_acceleration max_acceleration base_handling max_handling)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

THIS = 0x40100000
PLAYER = 0x40108000
CARREC = 0x40109000
PARTS = 0x4010A000
TABLE = 0x40120000
TABLE_PTR = 0x828CD0A8
STACK = 0x40F00000
IDX = 0x40107000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def value(rnd, kind):
    if kind == 'speed':
        return rnd.choice((100.0, 99.5, 0.0, 160.0, 260.0, rnd.uniform(100, 260), rnd.uniform(0, 400), float(rnd.randrange(100, 260)),
                           round(rnd.uniform(100, 260), 1), round(rnd.uniform(100, 260), 2)))
    return rnd.choice((0.0, 1.0, rnd.uniform(0, 1), round(rnd.uniform(0, 1), 2), round(rnd.uniform(0, 1), 3), rnd.uniform(-0.5, 2),
                       k / 100 if (k := rnd.randrange(101)) else 0.0, k / 130 if (k := rnd.randrange(131)) else 0.0))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    st = lambda k: 'sub_' + addr(a.tool, k)
    rnd = random.Random(13938)
    cases = []
    for _ in range(4000):
        cases.append([value(rnd, 'accel'), value(rnd, 'accel'), value(rnd, 'speed'), value(rnd, 'speed'), value(rnd, 'hand'), value(rnd, 'hand')])
    nl = chr(10)
    words = ','.join(','.join('%du' % fb(x) for x in c) for c in cases)
    stubs = {
        st('F-20823'): '',
        st('F-78475'): '',
        st('F-65049'): 'ctx.r3.u32=%du;' % PLAYER,
        st('F-25618'): 'ctx.r3.u32=load(%du);' % IDX,
        st('F-98200'): 'printf(" %d",(int)ctx.r6.u32);',
        st('F-77906'): 'throw 1;',
    }
    main_src = (
        'static const uint32_t C[]={' + words + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'store(%du+48,%du);store(%du+132,%du);store(%du,%du);' % (PLAYER, CARREC, CARREC, PARTS, TABLE_PTR, TABLE) +
        'for(size_t i=0;i<sizeof(C)/24;i++){uint32_t n=uint32_t(i%%7);store(%du,n);uint32_t r=%du+104*n;' % (IDX, TABLE) +
        'for(int k=0;k<6;k++)store(r+64+4*k,C[6*i+k]);'
        'for(int k=0;k<6;k++)printf("%%s%%08x",k?" ":"",C[6*i+k]);printf(" ->");'
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;ctx.r4.u32=1;try{%s(ctx,nullptr);}catch(int){}}' % (STACK, THIS, st('F-13938')) +
        'printf("\\n");}}' + nl)
    idx = index(a.recomp)
    exe = rd.build(idx, [st('F-13938')], main_src, a.out_dir, 'garage_meters', image=rd.load_image(a.image), stubs=stubs)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'garage_meters_ref.txt').write_text(out)
    lines = [x for x in out.split(nl) if x]
    print(len(lines), 'records')


if __name__ == '__main__':
    main()
