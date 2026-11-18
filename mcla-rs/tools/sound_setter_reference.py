"""The request entry's setters and the voice filter's parameter set run offline: the generated code whole and unchanged.
F-45018 pitch (r4, s16 at +80), F-46088 volume (f1, f32 at +64), F-78611 Doppler (f1, f32 at +84), F-62727 LPF (r4, u16 at
+88), F-49040 HPF (r4, u16 at +90): each writes the half of the request block that the buffer table (0x82836814 + 8 x
(+199 >> 5)) selects, 32 bytes a half. F-30281 the source filter's parameter (r4 = 0 LPF -> +80, 1 HPF -> +76, the float at
r6 truncated; anything else returns E_FAIL and writes nothing).
Inputs: random blocks (+199 random, the table word 0 or 1) and values (cents over and under the s16 range, dB and Doppler
floats with their edge values, cutoffs over and under 16 bits, filter Hz 0 .. 72,000 with fractions).
Writes <out-dir>/sound_setter_ref.txt: 'S <setter 0-4> <+199> <table word> <argument bits> -> <the block's 128 bytes>' and
'F <param> <float bits> -> <r3> <+76> <+80>' (hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

BLK = 0x40100000
VOICE = 0x40110000
ARG = 0x40120000
STACK = 0x40F00000
TABLE = 0x82836814
SETTERS = ('F-45018', 'F-46088', 'F-78611', 'F-62727', 'F-49040')
FLOAT = (False, True, True, False, False)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = [addr(a.tool, k) for k in SETTERS]
    flt = addr(a.tool, 'F-30281')
    r = random.Random(45018)
    f = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
    rows = []
    for _ in range(4000):
        k = r.randrange(5)
        if FLOAT[k]:
            v = f(r.choice((0.0, -0.0, 1.0, -100.0, 6.0, 1e-30, -3e30, r.uniform(-120, 24), r.uniform(-2, 2))))
        elif k == 0:
            v = r.choice((0, 1, -1, 1200, -2400, 32767, 32768, -32768, -32769, 65535, 70000, r.randrange(-40000, 40000), r.getrandbits(32))) & 0xffffffff
        else:
            v = r.choice((0, 23900, 24000, 65535, 65536, 70000, r.randrange(0, 24000), r.getrandbits(32))) & 0xffffffff
        rows.append((k, r.getrandbits(8), r.randrange(2), v))
    frows = []
    for _ in range(2000):
        p = r.choice((0, 0, 1, 1, 2, 255))
        hz = r.choice((0.0, 23900.0, 26290.0, 65535.0, 72088.5, 0.999, r.uniform(0, 72100), float(r.randrange(0, 72100))))
        frows.append((p, f(hz)))
    data = ','.join('%du,%du,%du,%du' % x for x in rows)
    fdata = ','.join('%du,%du' % x for x in frows)
    calls = '\n'.join(' if(k==%d)sub_%s(c,nullptr);' % (i, x) for i, x in enumerate(fn))
    c = r"""
static const uint32_t C[]={@DATA@};
static const uint32_t FD[]={@FDATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(size_t i=0;i<sizeof(C)/4;i+=4){uint32_t k=C[i],b=C[i+1],t=C[i+2],v=C[i+3];
 for(uint32_t o=0;o<256;o++)pointer(@BLK@u+o)[0]=(uint8_t)(0xA5^o);
 pointer(@BLK@u+199)[0]=(uint8_t)b;for(int j=0;j<8;j++)store(@TABLE@u+8*j,0x7777u);store(@TABLE@u+8*(b>>5),t);
 Context c{};c.r1.u32=@STACK@u;c.r3.u32=@BLK@u;c.r4.u64=v;float fv;memcpy(&fv,&v,4);c.f1.f64=fv;
@CALLS@
 printf("S %x %02x %x %08x ->",k,b,t,v);for(uint32_t o=0;o<128;o++)printf("%s%02x",o?"":" ",pointer(@BLK@u+o)[0]);printf("\n");}
for(size_t i=0;i<sizeof(FD)/4;i+=2){uint32_t p=FD[i],v=FD[i+1];
 store(@VOICE@u+76,0x11111111u);store(@VOICE@u+80,0x22222222u);store(@ARG@u,v);
 Context c{};c.r1.u32=@STACK@u;c.r3.u32=@VOICE@u;c.r4.u64=p;c.r6.u32=@ARG@u;sub_@FLT@(c,nullptr);
 printf("F %x %08x -> %08x %08x %08x\n",p,v,c.r3.u32,load(@VOICE@u+76),load(@VOICE@u+80));}}
"""
    for k, v in (('DATA', data), ('FDATA', fdata), ('BLK', BLK), ('VOICE', VOICE), ('ARG', ARG), ('STACK', STACK), ('TABLE', TABLE),
                 ('CALLS', calls), ('FLT', flt)):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(index(a.recomp), ['sub_' + x for x in fn + [flt]], c, a.out_dir, 'sound_setter', image=img)
    out = [x for x in run(exe, a.image).split(chr(10)) if x]
    assert len(out) == len(rows) + len(frows), (len(out), len(rows), len(frows))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sound_setter_ref.txt').write_text(chr(10).join(out) + chr(10))
    print(len(rows), 'setter calls,', len(frows), 'filter calls')


if __name__ == '__main__':
    main()
