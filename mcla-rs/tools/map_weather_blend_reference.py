"""The weather manager's colour filter F-94045 and fog step F-82205 run offline as generated (cameras2-map5), written to
crates/game/src/testdata/map_weather_blend_ref.txt for the port's tests. recomp_closure imports tools/memcap.py, so every process of
this script (g++, the harness) runs in a 4 GB job.

Both read the hour from the clock getter F-22302 (stubbed: answers the case's hour), pick the weight row from one of two words (the
render thread's copy when the thread block's byte +72 is set), and for each of the four weathers sample that weather's track (the
24 track objects of the weather manager: +24 the 48-byte keys, +28 their count) and add value x weight / row sum. The tracks hold
the keys the running original holds after its loader (testdata/map_weather_keys_ref.txt, read from the recomp's memory); the weight
table (12 words at 0x827E1B10, the row sums right after it at 0x827E1B40, which the fourth row also reads), the row and the hour
are random per case.
One line per case: "B <hour> <tls byte> <row> <table 16 words> -> <colour 3> | <min fog 4> | <max fog 4> | <height x> <height y>
<near z> <far z>", all words hex.
Usage: map_weather_blend_reference.py --recomp D:/MCLA-static --image IMG --work DIR --keys FILE --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

HI = 0x82880000
BASES = [HI - 11712, HI - 11372, HI - 11212, HI - 10572, HI - 10412, HI - 10092]
KINDS = {'ColorCorrect': 0, 'MaxFogColor': 1, 'FogNearZ': 2, 'FogHeight': 3, 'FogFarZ': 4, 'MinFogColor': 5}
SUF = ['Nice', 'Cloudy', 'Stormy', 'Foggy']

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);
 // the tracks: words [n tracks] then per track: object address, key count, then 12 words a key
 size_t at=0;uint32_t nt=in[at++];std::vector<std::pair<uint32_t,std::vector<uint32_t>>> tr;
 for(uint32_t t=0;t<nt;t++){uint32_t obj=in[at++],n=in[at++];std::vector<uint32_t> k(in.begin()+at,in.begin()+at+12*n);at+=12*n;tr.push_back({obj,k});}
 uint32_t nc=in[at++];
 for(uint32_t c=0;c<nc;c++,at+=19){const uint32_t*q=in.data()+at;
  rt_reset();
  uint32_t keys=0x40100000u;
  for(auto&t:tr){uint32_t n=t.second.size()/12;store(t.first+24,keys);store(t.first+28,(n<<16)|n);
   for(size_t i=0;i<t.second.size();i++)store(keys+4*i,t.second[i]);keys+=16*((4*t.second.size()+15)/16);}
  for(int i=0;i<16;i++)store(0x827E1B10u+4*i,q[3+i]);
  store(0x827E5640u,q[2]);store(0x8288B9F0u,q[2]^1u);
  Context cx=fresh();*pointer(load(cx.r13.u32)+72)=uint8_t(q[1]);
  if(q[1]==0){store(0x8288B9F0u,q[2]);store(0x827E5640u,q[2]^1u);}
  std::memcpy(&hour_,&q[0],4);
  cx.r3.u32=0;sub_COLOR(cx,nullptr);
  Context cy=fresh();sub_FOG(cy,nullptr);
  printf("%08x %08x %08x |",load(0x827E1550u),load(0x827E1554u),load(0x827E1558u));
  for(int i=0;i<4;i++)printf(" %08x",load(0x827E1770u+4*i));printf(" |");
  for(int i=0;i<4;i++)printf(" %08x",load(0x827E1780u+4*i));printf(" |");
  printf(" %08x %08x %08x %08x\n",load(0x827E1660u),load(0x827E1664u),load(0x8287D390u),load(0x827E1668u));
 }
}
'''


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--keys', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    a.work.mkdir(parents=True, exist_ok=True)

    def name(fid):
        return 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    tracks, cur = [], None
    for line in a.keys.read_text().splitlines():
        f = line.split()
        if f[0] == 'TRK':
            nm = f[1][len('sm_Key'):]
            kind, suf = nm.split('Default')
            cur = (BASES[KINDS[kind]] + 40 * SUF.index(suf), [])
            tracks.append(cur)
        else:
            w = [int(x, 16) for x in f[2:]]
            cur[1].extend([w[0], 0, 0, 0] + w[1:5] + w[5:9])
    rnd = random.Random(94045)
    times = sorted({k[0] for _, ks in tracks for k in [ks[i:i + 12] for i in range(0, len(ks), 12)]})
    cases = []
    for c in range(a.cases):
        m = rnd.random()
        if m < 0.15:
            h = struct.unpack('>f', struct.pack('>I', rnd.choice(times)))[0]
        elif m < 0.2:
            h = rnd.choice((-1.5, -0.001, 24.0, 24.5, 30.0, 0.0, -0.0))
        else:
            h = rnd.uniform(0.0, 24.0)
        row = rnd.randrange(4)
        tab = []
        for r in range(4):
            if rnd.random() < 0.3:
                one = rnd.randrange(4)
                tab += [1.0 if k == one else 0.0 for k in range(4)]
            else:
                tab += [rnd.choice((0.0, rnd.uniform(0, 1), rnd.uniform(0, 5))) for _ in range(4)]
        if rnd.random() < 0.7:
            # the row sums the weather manager keeps (the fourth row reads them as its weights)
            for r in range(3):
                tab[12 + r] = sum(struct.unpack('>f', struct.pack('>f', x))[0] for x in tab[4 * r:4 * r + 4]) or rnd.choice((0.0, 1.0))
        cases.append([fb(h), rnd.choice((0, 1)), row] + [fb(x) for x in tab])
    blob = [len(tracks)]
    for obj, ks in tracks:
        blob += [obj, len(ks) // 12] + ks
    blob.append(len(cases))
    for c in cases:
        blob += c
    inp = a.work / 'blend_in.bin'
    inp.write_bytes(struct.pack('<%dI' % len(blob), *blob))
    rc.EXTRA += 'static float hour_;' + chr(10)
    clock = name('F-22302')
    stubs = {clock: 'ctx.f1.f64=double(hour_);'}
    src = MAIN.replace('sub_COLOR', name('F-94045')).replace('sub_FOG', name('F-82205'))
    exe = rd.build(idx, [name('F-94045'), name('F-82205')], src, a.work, 'map_weather_blend', image=img, stubs=stubs)
    p = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['B %08x %d %d %s -> %s' % (c[0], c[1], c[2], ' '.join('%08x' % x for x in c[3:]), o) for c, o in zip(cases, out)]
    a.out.write_text('\n'.join(lines) + '\n')
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
