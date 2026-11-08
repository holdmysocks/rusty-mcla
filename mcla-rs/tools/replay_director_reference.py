"""The replay director's small routines run offline: unchanged generated code on guest objects.
  shot length F-66204   (+32 = clamp(normal(+160 mean, +164 deviation), +168 least, +172 most), the game's generator)
  zoom timer  F-16218   (+364 = the same over +348 .. +360)
  k-th bit    F-48190   (r3 -> a 32-bit mask word, r4 = k: the k-th set bit ascending, or -1)
  polar stick F-63907   (r3 = the polar camera: +168 lens, +272 pitch, +276 yaw, +280 / +284 lens range; r4 = the pad: bytes 18 / 19
                         right stick x / y, 20 / 21 left / right trigger; the frame time is the game's global; dead zone F-93983)
The generator (multiply-with-carry, normal deviate F-66400 with its spare) is the game's global object: +0 low word, +4 carry, +8 spare flag
(top byte), +12 spare value. A length sequence starts from a random generator with no spare and makes 12 calls; each call's tune is random.
Lines:  LEN <lo> <carry> | <mean> <dev> <min> <max> x12 -> per call: <result> <flag> <spare> <lo> <carry>
        ZOOM <same>                                     (the zoom timer, same layout)
        KTH <mask> <k> -> <bit>                           (bit as a signed decimal)
        POL <fov> <pitch> <yaw> <min> <max> <dt> <b18> <b19> <b20> <b21> -> <fov> <pitch> <yaw>
        FIX <car rows a b c place (12)> <mount (3)> <look back> -> <rows a b c (9)> <eye (3)>   (fixed mount update F-77143)
        PACT <lo> <carry> <default lens> -> <lens> <pitch> <yaw> <lo> <carry>   (the polar camera's activate F-82095: +288 -> +168)
        MNT <state> <mounts 0..5 (18)> -> <fixed +192 x y z> <look back +208>   (the mount setter F-31661: director +28 state, +40 -> the
                                         fixed camera, mounts at +64 + 16 k; +192 starts as 0xA5A5A5A5 and +208 as 0xA5 to show what is written)
(floats as hex bits)
Usage: replay_director_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

IDS = {'LEN': 'F-66204', 'ZOOM': 'F-16218', 'KTH': 'F-48190', 'POL': 'F-63907', 'PACT': 'F-82095', 'FIX': 'F-77143', 'MNT': 'F-31661'}

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t G=uint32_t(-2105147392)+2176, GDT=uint32_t(-2105737216)+29952+88;
 const uint32_t O=0x40010000,P=0x40010400,V=uint32_t(-2105147392);
 while(in>>k){rt_reset();
  if(k=="LEN"||k=="ZOOM"){uint32_t lo,ca;in>>std::hex>>lo>>ca;store(G,lo);store(G+4,ca);store(G+8,0);store(G+12,0);
   int b=k=="LEN"?160:348, r=k=="LEN"?32:364;
   for(int i=0;i<12;i++){uint32_t m,d,mn,mx;in>>m>>d>>mn>>mx;for(int j=0;j<0x200;j+=4)store(O+j,0);
    store(O+b,m);store(O+b+4,d);store(O+b+8,mn);store(O+b+12,mx);
    Context c=fresh();c.r3.u32=O;if(k=="LEN")sub_LEN(c,nullptr);else sub_ZOOM(c,nullptr);
    printf("%08x %u %08x %08x %08x\n",load(O+r),load(G+8)>>24,load(G+12),load(G),load(G+4));}
   in>>std::dec;}
  else if(k=="KTH"){uint32_t m,n;in>>std::hex>>m>>n>>std::dec;store(O,m);Context c=fresh();c.r3.u32=O;c.r4.u32=n;sub_KTH(c,nullptr);printf("%d\n",int(c.r3.u32));}
  else if(k=="FIX"){uint32_t r[15],bk;in>>std::hex;for(int i=0;i<15;i++)in>>r[i];in>>bk>>std::dec;
   for(int i=0;i<3;i++){store(V+2384+4*i,0xFFFFFFFFu);store(V+2352+4*i,0);}store(V+2384+12,0);store(V+2352+12,0x3F800000u);
   for(int i=0;i<4;i++){store(V+2240+4*i,0);store(V+2528+4*i,0x7F800000u);}
   for(int j=0;j<0x200;j+=4){store(O+j,0);store(P+j,0);}
   for(int row=0;row<4;row++){for(int i=0;i<3;i++)store(P+16*row+4*i,r[3*row+i]);store(P+16*row+12,row==3?0x3F800000u:0);}
   store(O+144,P);for(int i=0;i<3;i++)store(O+192+4*i,r[12+i]);store(O+208,bk<<24);
   Context c=fresh();c.r3.u32=O;sub_FIX(c,nullptr);for(int j=16;j<80;j+=4)if((j&15)!=12)printf("%08x ",load(O+j));printf("\n");}
  else if(k=="PACT"){uint32_t lo,ca,df;in>>std::hex>>lo>>ca>>df>>std::dec;store(G,lo);store(G+4,ca);for(int j=0;j<0x200;j+=4)store(O+j,0xA5A5A5A5u);
   store(O+288,df);Context c=fresh();c.r3.u32=O;sub_PACT(c,nullptr);printf("%08x %08x %08x %08x %08x\n",load(O+168),load(O+272),load(O+276),load(G),load(G+4));}
  else if(k=="MNT"){uint32_t st,m[18];in>>std::hex>>st;for(int i=0;i<18;i++)in>>m[i];in>>std::dec;
   for(int j=0;j<0x200;j+=4){store(O+j,0);store(P+j,0xA5A5A5A5u);}store(O+28,st);store(O+40,P);
   for(int q=0;q<6;q++){for(int i=0;i<3;i++)store(O+64+16*q+4*i,m[3*q+i]);store(O+64+16*q+12,0x3F800000u);}
   Context c=fresh();c.r3.u32=O;sub_MNT(c,nullptr);
   printf("%08x %08x %08x %u\n",load(P+192),load(P+196),load(P+200),load(P+208)>>24);}
  else{uint32_t fov,pi,ya,mn,mx,dt,b[4];in>>std::hex>>fov>>pi>>ya>>mn>>mx>>dt>>b[0]>>b[1]>>b[2]>>b[3]>>std::dec;
   for(int j=0;j<0x200;j+=4)store(O+j,0);for(int j=0;j<32;j+=4)store(P+j,0);
   store(O+168,fov);store(O+272,pi);store(O+276,ya);store(O+280,mn);store(O+284,mx);store(GDT,dt);
   store(P+16,(b[0]<<8)|b[1]);store(P+20,(b[2]<<24)|(b[3]<<16));
   Context c=fresh();c.r3.u32=O;c.r4.u32=P;sub_POL(c,nullptr);printf("%08x %08x %08x\n",load(O+168),load(O+272),load(O+276));}
 }
}
'''


def f32bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    names = {k: addr(a.tool, v) for k, v in IDS.items()}
    src = MAIN
    for k, v in names.items():
        src = src.replace('sub_' + k + '(', v + '(')
    rd.build(idx, list(names.values()), src, a.work, 'replay_director', image=rd.load_image(a.image))
    # (built again with -msse4.1 like the recomp: vmsum*fp as dpps, whose sum order the port follows)
    exe = a.work / 'replay_director_sse.exe'
    r = subprocess.run(['g++', '-std=c++17', '-O2', '-msse4.1', '-ffp-contract=off', '-static', '-I' + str(rc.SIMDE), str(a.work / 'replay_director.cpp'), '-o', str(exe)],
                       capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:1500])
    rnd = random.Random(2041)
    fb = f32bits
    lines = []
    for kind in ('LEN', 'ZOOM'):
        for _ in range(150):
            calls = []
            for _ in range(12):
                m, d, lo, hi = rnd.choice([(5.0, 3.0, 1.0, 10.0), (15.0, 5.0, 5.0, 30.0),
                                           (rnd.uniform(0, 20), rnd.uniform(0, 8), rnd.uniform(0, 5), rnd.uniform(5, 40))])
                calls.append(' '.join(f'{fb(x):08x}' for x in (m, d, lo, hi)))
            lines.append(f'{kind} {rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x} ' + ' '.join(calls))
    for _ in range(4000):
        m = rnd.choice([rnd.getrandbits(20), rnd.getrandbits(32), rnd.getrandbits(32) & rnd.getrandbits(32), 0, 0xFFFFFFFF])
        lines.append(f'KTH {m:08x} {rnd.choice([rnd.randrange(40), rnd.randrange(bin(m).count("1") + 1)]):x}')
    for _ in range(6000):
        lo, hi = rnd.choice([(25.0, 60.0), (rnd.uniform(5, 40), rnd.uniform(40, 90))])
        fov = rnd.choice([47.0, lo, hi, rnd.uniform(lo - 5, hi + 5)])
        pitch = rnd.choice([0.0, 1.2566371, rnd.uniform(-0.2, 1.4)])
        yaw = rnd.uniform(-20, 20)
        dt = rnd.choice([1 / 30, 1 / 60, rnd.uniform(0.001, 0.2)])
        b = [rnd.choice([0, 127, 128, 255, rnd.randrange(256)]) for _ in range(4)]
        lines.append('POL ' + ' '.join(f'{fb(x):08x}' for x in (fov, pitch, yaw, lo, hi, dt)) + ' ' + ' '.join(f'{x:x}' for x in b))
    import math
    for _ in range(3000):
        q = [rnd.gauss(0, 1) for _ in range(4)]
        n = math.sqrt(sum(x * x for x in q))
        x, y, z, w = (v / n for v in q)
        m = [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)], [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
             [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]
        if rnd.random() < 0.3:
            m = [m[0], [0.0, 1.0, 0.0] if rnd.random() < 0.5 else m[1], [0.0, rnd.choice([1.0, -1.0]), 0.0] if rnd.random() < 0.3 else m[2]]
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 100), rnd.uniform(-3000, 3000)]
        mount = [rnd.uniform(-1.5, 1.5), rnd.uniform(0, 2), rnd.uniform(-3, 3)]
        vals = [v for r in m for v in r] + place + mount
        lines.append('FIX ' + ' '.join(f'{fb(v):08x}' for v in vals) + f' {rnd.choice([0, 0, 1])}')
    for _ in range(3000):
        lines.append(f'PACT {rnd.getrandbits(32):08x} {rnd.getrandbits(32):08x} {fb(rnd.choice([47.0, rnd.uniform(10, 90)])):08x}')
    for _ in range(3000):
        st = rnd.choice([rnd.randrange(6, 18), rnd.randrange(6, 18), rnd.randrange(0, 22)])
        ms = [rnd.choice([0.0, -0.0, rnd.uniform(-3, 3)]) for _ in range(18)]
        lines.append(f'MNT {st:x} ' + ' '.join(f'{fb(v):08x}' for v in ms))
    cases = a.work / 'replay_director_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = iter(p.stdout.splitlines())
    out = []
    for l in lines:
        k = l.split(' ', 1)[0]
        if k in ('LEN', 'ZOOM'):
            w = l.split(' ')
            res = [next(rows) for _ in range(12)]
            out.append(f'{k} {w[1]} {w[2]} | ' + ' '.join(w[3:]) + ' -> ' + ' | '.join(res))
        else:
            out.append(l + ' -> ' + next(rows))
    keep = [l for l in out if not re.search(r'82[0-9a-f]{6}', l)]      # tracked data must not hold raw-address-looking words
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(keep), 'of', len(out), 'lines')


if __name__ == '__main__':
    main()
