"""The vinyl editor's group edit run offline: the bake FinalizeGroupMovement F-88223 (with its helpers F-38353, F-18413, F-94277,
F-85623, the half encoder F-70220, sin F-72580 / cos F-63589, the reset F-90968) and the stick step ChangeLayerAttribute
F-66271 -> F-48118 (per-axis dead zone F-93983, rotation wrap F-43389, the int wrap F-58674), the generated code whole and
unchanged, on random layer records, group records, marks, zones, modes and stick bytes.

Stubbed: F-65049 (returns the test object whose +48 -> +132 is the vinyl data base), F-18285 (pad 0), F-42664 (the
UI_MENU_NAV_UP sound), F-56493 (the dimension read-out to Flash). None of them writes a field compared here.

Writes <out-dir>/vinyl_group_ref.txt, every number led by a letter (halves and words in hex):
  B z<zone> n<count> m<marks, one char per layer 0/1> g<group record of the zone, 20 bytes hex>
    then per layer 'l<20 bytes hex>' ; '->' per layer 'l<20 bytes hex>' then 'r<5 group records, 100 bytes hex>'
  E z<zone> c<cursor> n<count> m<marks> o<mode> b<busy> g<group record hex> l<cursor layer hex>
    then per frame 'f<x byte>,<y byte> -> g<group record hex> l<cursor layer hex>'
Layer / group record: +0 scale x half, +2 scale y half, +4 pos x half, +6 pos y half, +8 colour word, +12 skew half,
+14 rotation half (degrees), +16 shape id (s16, -1 = empty), +18 byte (mode 7 field), +19 byte."""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, 'D:/MCLA/mcla-rs/tools')
import recomp_dispatch as rd  # noqa: E402

ARENA = 0x40000000
GEN = ARENA + 0x10000      # vinyl generator (its global pointer set below): +244 -> P, +248 busy, +4*(z+6) count, +4*(z+417) marks ptr
P = ARENA + 0x20000        # [P+132] = BASE ; also F-65049's stub returns R with [R+48] = P
R = ARENA + 0x28000
BASE = ARENA + 0x30000     # vinyl data: group records at BASE+64+2144+20z, layers at BASE+64+2244+20*(start[z]+i)
MARKS = ARENA + 0x40000    # + 0x100*z
GAR = ARENA + 0x100000     # garage: +57692 cursor, +57696 zone, +57708 mode
STACK = ARENA + 0xF00000
START = (0, 64, 128, 192, 208)
SIZE = (64, 64, 64, 16, 16)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def f2h(x):
    """F-70220 in Python (only used to make inputs)."""
    if (-6.1035156e-05 < x < 6.1035156e-05) or x != x:
        return 0
    x = max(-65504.0, min(65504.0, x))
    u = struct.unpack('>I', struct.pack('>f', x))[0]
    return ((((u >> 23) & 0x1FF) + 16) << 10 & 0xFFFF) | ((u >> 13) & 0x3FF) | ((u >> 16) & 0x8000)


def rec(r, kind):
    h = lambda lo, hi: f2h(r.uniform(lo, hi)) if r.random() > 0.08 else r.choice((0, 0x8000, 0x3C00, 0xBC00, r.randrange(65536)))
    shape = r.randrange(0, 300) if r.random() > 0.1 else 0xFFFF
    if kind == 'G':
        return [h(-5, 5), h(-5, 5), h(-2, 2), h(-2, 2), r.getrandbits(32), h(-1, 1), h(-360, 360), shape, r.randrange(3), r.randrange(2)]
    return [h(-5, 5), h(-5, 5), h(-2, 2), h(-2, 2), r.getrandbits(32), h(-1, 1), h(-360, 360), shape, r.randrange(3), r.randrange(2)]


def pack(v):
    return struct.pack('>HHHHIHHHBB', *v)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('--out-dir', type=Path, default=Path(__file__).resolve().parent.parent / 'crates/game/src/testdata')
    ap.add_argument('--tool', default='D:/MCLA/work/ui2/target/release/mcla-tool.exe')
    a = ap.parse_args()
    bake, change, getobj, pad, snd, show = (addr(a.tool, f) for f in ('F-88223', 'F-66271', 'F-65049', 'F-18285', 'F-42664', 'F-56493'))
    img = rd.load_image(a.image)
    idx = rd.index(a.recomp)
    r = random.Random(88223)
    words = []          # flat u32 stream read by MAIN

    def put_bytes(b):
        b = b + b'\0' * (-len(b) % 4)
        words.append(len(b) // 4)
        words.extend(struct.unpack('>%dI' % (len(b) // 4), b))

    nb = 0
    for k in range(3000):           # bake cases
        z = r.randrange(5)
        n = r.choice((1, 2, 3, 4, 5, 8, SIZE[z])) if k > 20 else 2
        marks = [int(r.random() < 0.6) for _ in range(n)]
        if k < 20:
            g = [0x3C00, 0x3C00, 0, 0, 0, 0, 0, 0, 0, 0] if k < 5 else rec(r, 'G')
            if 5 <= k < 10:
                g[1] = 0x3C00
        else:
            g = rec(r, 'G')
        others = [rec(r, 'G') for _ in range(5)]
        others[z] = g
        layers = [rec(r, 'L') for _ in range(n)]
        words.append(0)              # kind 0 = bake
        words += [z, n] + marks
        put_bytes(b''.join(pack(o) for o in others))
        put_bytes(b''.join(pack(l) for l in layers))
        nb += 1
    ne = 0
    for k in range(3000):           # edit cases
        z = r.randrange(5)
        n = r.choice((1, 2, 3, 5, 8))
        marks = [int(r.random() < 0.6) for _ in range(n)]
        cur = r.randrange(n)
        mode = r.choice((1, 1, 2, 2, 3, 3, 4, 5, 6, 7))
        busy = int(r.random() < 0.05)
        g = rec(r, 'G')
        lay = [rec(r, 'L') for _ in range(n)]
        frames = []
        for _ in range(r.randrange(1, 12)):
            sx = r.choice((0, 255, 127, 128, 160, 96, 200, 30, 250, 5, 140, 115, r.randrange(256)))
            sy = r.choice((0, 255, 127, 128, 160, 96, 200, 30, r.randrange(256)))
            frames += [sx, sy]
        words.append(1)
        words += [z, n, cur, mode, busy] + marks
        put_bytes(pack(g))
        put_bytes(b''.join(pack(l) for l in lay))
        words.append(len(frames) // 2)
        words += frames
        ne += 1
    data = ','.join('%du' % w for w in words)
    stubs = {
        'sub_' + getobj: 'ctx.r3.u64=%du;' % R,
        'sub_' + pad: 'ctx.r3.u64=0;',
        'sub_' + snd: '',
        'sub_' + show: '',
    }
    c = r"""
static const uint32_t D[]={@DATA@};
static size_t at=0;
static void hexrec(uint32_t a,int n){for(int i=0;i<n;i++)printf("%02x",*pointer(a+i));}
static void readbytes(uint32_t dst){uint32_t n=D[at++];for(uint32_t i=0;i<n;i++)store(dst+4*i,D[at++]);}
static void setup(uint32_t z,uint32_t n,const uint32_t*marks){
 memset(pointer(@GEN@u),0,0x10000);memset(pointer(@BASE@u),0,0x2000);memset(pointer(@MARKS@u),0,0x600);
 store(@GEN@u+244,@P@u);store(@P@u+132,@BASE@u);store(@R@u+48,@P@u);store(0x8288DFC0u,@GEN@u);
 for(int q=0;q<6;q++){store(@GEN@u+4*(q+6),0);store(@GEN@u+4*(q+417),@MARKS@u+0x100*q);}
 store(@GEN@u+4*(z+6),n);for(uint32_t i=0;i<n;i++)*pointer(@MARKS@u+0x100*z+i)=uint8_t(marks[i]);}
static const uint32_t ST[5]={0,64,128,192,208};
static uint32_t layer(uint32_t z,uint32_t i){return @BASE@u+64+2244+20*(ST[z]+i);}
static uint32_t grp(uint32_t z){return @BASE@u+64+2144+20*z;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
while(at<sizeof(D)/4){uint32_t kind=D[at++];
 if(kind==0){uint32_t z=D[at++],n=D[at++];const uint32_t*m=D+at;at+=n;setup(z,n,m);
  readbytes(grp(0));readbytes(layer(z,0));
  printf("B z%u n%u m",z,n);for(uint32_t i=0;i<n;i++)printf("%u",m[i]);printf(" g");hexrec(grp(z),20);
  for(uint32_t i=0;i<n;i++){printf(" l");hexrec(layer(z,i),20);}
  Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@GEN@u;ctx.r4.u32=z;sub_@BAKE@(ctx,nullptr);
  printf(" ->");for(uint32_t i=0;i<n;i++){printf(" l");hexrec(layer(z,i),20);}printf(" r");hexrec(grp(0),100);printf("\n");}
 else{uint32_t z=D[at++],n=D[at++],cur=D[at++],mode=D[at++],busy=D[at++];const uint32_t*m=D+at;at+=n;setup(z,n,m);
  *pointer(@GEN@u+248)=uint8_t(busy);
  memset(pointer(@GAR@u),0,0x10000);store(@GAR@u+57692,cur);store(@GAR@u+57696,z);store(@GAR@u+57708,mode);
  readbytes(grp(z));readbytes(layer(z,0));
  printf("E z%u c%u n%u m",z,cur,n);for(uint32_t i=0;i<n;i++)printf("%u",m[i]);printf(" o%u b%u g",mode,busy);hexrec(grp(z),20);
  printf(" l");hexrec(layer(z,cur),20);printf("\n");
  uint32_t nf=D[at++];for(uint32_t f=0;f<nf;f++){uint32_t sx=D[at++],sy=D[at++];
   *pointer(0x82905388u+16)=uint8_t(sx);*pointer(0x82905388u+17)=uint8_t(sy);
   Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@GAR@u;ctx.r4.u32=2;sub_@CHANGE@(ctx,nullptr);
   printf("f%u,%u -> g",sx,sy);hexrec(grp(z),20);printf(" l");hexrec(layer(z,cur),20);printf("\n");}
  printf("=\n");}}}
"""
    for k, v in (('DATA', data), ('GEN', GEN), ('P', P), ('R', R), ('BASE', BASE), ('MARKS', MARKS), ('GAR', GAR),
                 ('STACK', STACK), ('BAKE', bake), ('CHANGE', change)):
        c = c.replace('@%s@' % k, str(v))
    exe = rd.build(idx, ['sub_' + bake, 'sub_' + change], c, a.out_dir / 'build', 'group_bake', stubs=stubs, image=img)
    out = rd.run(exe, a.image)
    (a.out_dir / 'vinyl_group_ref.txt').write_text(out)
    print(nb, 'bake cases,', ne, 'edit cases,', out.count('\nf'), 'frames')


if __name__ == '__main__':
    main()
