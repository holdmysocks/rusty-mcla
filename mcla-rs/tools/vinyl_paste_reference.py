"""The vinyl editor's PASTE run offline: the original's F-64893 (the clipboard of layer records pasted into a zone at an
index, mirrored by the pair of source and target zone), the generated code whole and unchanged with its callees (the
used-layer count F-48156, the half-float writer F-70220 and scaler F-50377, the insert of a record F-16364).
Guest memory: the editor 0x40100000 (+244 the car 0x40110000, whose +132 is the livery block 0x40120000; the records
at block +64 +2244 + 20 x (the zone's start in the image table 0x827E9770 + index); the clipboard at +380, its count
+1660, its source zone +1664, the zone's room +24 + 4 zone).
Writes <out-dir>/vinyl_paste_ref.txt, one paste a line: source zone, target zone, index, room, the clip count, the
clip's records (40 hex digits each), '|', the target zone's used records before (the rest of the zone is empty, all
ff), then '->' the zone's first min(capacity, used + count) records after."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

E = 0x40100000
CAR = 0x40110000
LIV = 0x40120000
STACK = 0x40F00000
START = [0, 64, 128, 192, 208]
CAP = [64, 64, 64, 16, 16]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def h(rnd):
    # (a half float of the kinds the records hold: 0, small and large of both signs, the original's odd ones)
    return rnd.choice((0, 0x8000, rnd.randrange(0, 0x10000), rnd.randrange(0x2000, 0x3C00), rnd.randrange(0xA000, 0xBC00),
                       rnd.randrange(0x3C00, 0x5000), rnd.randrange(0x0001, 0x0400), 0x7C00, 0xFC00))


def record(rnd):
    r = struct.pack('>HHHH', h(rnd), h(rnd), h(rnd), h(rnd)) + bytes(rnd.randrange(256) for _ in range(4))
    r += struct.pack('>HHH', h(rnd), h(rnd), rnd.randrange(0, 5000)) + bytes((rnd.randrange(3), 0))
    return r


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-64893')
    rnd = random.Random(64893)
    cases = []
    for _ in range(1500):
        src, dst = rnd.randrange(5), rnd.randrange(5)
        n = rnd.choice((1, 1, 1, 2, 3, rnd.randrange(1, 17)))
        used = rnd.choice((0, 1, rnd.randrange(CAP[dst] + 1), CAP[dst] - n, CAP[dst] - n + 1))
        used = max(0, min(CAP[dst], used))
        room = rnd.choice((CAP[dst], CAP[dst], rnd.randrange(CAP[dst] + 1)))
        at = rnd.randrange(0, used + 1)
        zone = [record(rnd) for _ in range(used)] + [b'\xff' * 20] * (CAP[dst] - used)
        cases.append((src, dst, at, room, [record(rnd) for _ in range(n)], zone))
    nl = chr(10)
    words = []
    for src, dst, at, room, clip, zone in cases:
        blob = b''.join(clip) + b''.join(zone)
        used = sum(r[16:18] != bytes((255, 255)) for r in zone)
        w = [src, dst, at, room, len(clip), used] + [struct.unpack('>I', blob[i:i + 4])[0] for i in range(0, len(blob), 4)]
        words.append(','.join('%du' % x for x in w) + ',')
    main_src = (
        'static const uint32_t C[]={' + nl + nl.join(words) + nl + '};' + nl +
        'static const int S[5]={0,64,128,192,208},K[5]={64,64,64,16,16};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(size_t i=0;i<sizeof(C)/4;){uint32_t src=C[i],dst=C[i+1],at=C[i+2],room=C[i+3],n=C[i+4],used=C[i+5];i+=6;'
        'memset(pointer(%du),0,0x800);memset(pointer(%du),0,0x200);memset(pointer(%du),0xff,0x8000);' % (E, CAR, LIV) +
        'store(%du+244,%du);store(%du+132,%du);' % (E, CAR, CAR, LIV) +
        'for(uint32_t k=0;k<n*5;k++)store(%du+380+4*k,C[i+k]);i+=n*5;' % E +
        'store(%du+1660,n);store(%du+1664,src);store(%du+24+4*dst,room);' % (E, E, E) +
        'uint32_t z=%du+64+2244+20*S[dst];for(int k=0;k<K[dst]*5;k++)store(z+4*k,C[i+k]);' % LIV +
        'printf("%%u %%u %%u %%u %%u",src,dst,at,room,n);'
        'for(uint32_t k=0;k<n*5;k++)printf("%%s%%08x",k%%5?"":" ",load(%du+380+4*k));' % E +
        'printf(" |");for(uint32_t k=0;k<used*5;k++)printf("%%s%%08x",k%%5?"":" ",C[i+k]);i+=K[dst]*5;'
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;ctx.r4.u32=dst;ctx.r5.u32=at;sub_%s(ctx,nullptr);}' % (STACK, E, fn) +
        'uint32_t m=used+n<(uint32_t)K[dst]?used+n:K[dst];printf(" ->");for(uint32_t k=0;k<m*5;k++)printf("%s%08x",k%5?"":" ",load(z+4*k));printf("\\n");}}' + nl)
    idx = index(a.recomp)
    exe = rd.build(idx, ['sub_' + fn], main_src, a.out_dir, 'vinyl_paste', image=rd.load_image(a.image))
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'vinyl_paste_ref.txt').write_text(out)
    lines = [x for x in out.split(nl) if x]
    print(len(lines), 'pastes')


if __name__ == '__main__':
    main()
