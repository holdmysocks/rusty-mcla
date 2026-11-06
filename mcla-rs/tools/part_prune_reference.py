"""The rail sweep's step into a composite bound's part (F-35029, type 12): the query moved into the part's frame (F-66128)
and the part dropped when the capsule misses its box (F-16177 -> F-81407 -> F-18932) and moved back after it (F-15412), run offline on
the original's code whole - nothing stubbed.

A case: the part's matrix (three rows and the place, the part bound's frame in the composite), the capsule's ends (the
query's +176 / +192) and radius (+208), the part bound's box (+16 the corner of the larger values, +32 the smaller).
Boxes about the capsule, across it, by an end, far off; matrices turned at random or not at all; capsules without
length. Out per case: the query's +176 and +192 after F-66128 (four lanes each), its box +96 / +112 (four lanes each)
and F-16177's result (1: the part is dropped), then the same four after the query is moved back (F-15412 -> F-53488;
the start-up constants the image holds as zero filled in: the xyz mask and (0, 0, 0, 1)). Writes <out-dir>/part_prune_ref.txt. Test
`camprobe::tests::part_prune_matches_the_original`."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


_run = subprocess.run


def _gpp(cmd, *a, **k):
    # the recomp's own x86 flag: vmsum3fp is dpps, (lanes 0 + 1) + (2 + 3)
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


subprocess.run = _gpp
_vs = rd.vector_shim
# (vslw128 is the AVX2 per-lane shift in the recomp's intrinsics; simde's portable form)
rd.vector_shim = lambda: '#include <simde/x86/avx2.h>' + chr(10) + _vs()
N_IN = 12 + 3 + 3 + 1 + 3 + 3
MAIN = r'''
static const uint32_t Q=0x40800000, M=0x40801000, P=0x40801400;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}

int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 FILE*f=fopen(argv[2],"r"); unsigned w[NIN_];
 while(1){ int n=0; for(int i=0;i<NIN_;i++) n+=fscanf(f,"%x",&w[i]); if(n!=NIN_)break;
  rt_reset(); for(uint32_t a=Q;a<P+0x100;a+=4) store(a,0);
  for(int k=0;k<4;k++){ for(int j=0;j<3;j++) store(M+16*k+4*j,w[3*k+j]); store(M+16*k+12,k==3?0x3F800000u:0u); }
  for(int j=0;j<3;j++){ store(Q+176+4*j,w[12+j]); store(Q+192+4*j,w[15+j]); }
  store(Q+208,w[18]);
  for(int j=0;j<3;j++){ store(P+16+4*j,w[19+j]); store(P+32+4*j,w[22+j]); }
  Context c=fresh(); c.r3.u64=Q; c.r4.u64=M;
  try{ sub_FRAME_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  for(int o: {176,192,96,112}) for(int j=0;j<4;j++) printf("%08x ",load(Q+o+4*j));
  c=fresh(); c.r3.u64=Q; c.r4.u64=P;
  try{ sub_PRUNE_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  printf("%d",c.r3.u32&0xFF);
  c=fresh(); c.r3.u64=Q; c.r4.u64=M;
  try{ sub_BACK_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  for(int o: {176,192,96,112}) for(int j=0;j<4;j++) printf(" %08x",load(Q+o+4*j));
  printf("\n"); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    ap.add_argument('--seed', type=int, default=16177)
    a = ap.parse_args()
    frame = addr(a.tool, 'F-66128')
    prune = addr(a.tool, 'F-16177')
    unframe = addr(a.tool, 'F-15412')
    rnd = random.Random(a.seed)
    cases = []
    while len(cases) < a.cases:
        if rnd.random() < 0.25:
            rows = [[1.0, 0, 0], [0, 1.0, 0], [0, 0, 1.0]]
        else:
            q = [rnd.gauss(0, 1) for _ in range(4)]
            s = math.sqrt(sum(x * x for x in q))
            w, x, y, z = (c / s for c in q)
            rows = [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)],
                    [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
                    [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]
        pos = [rnd.uniform(-2, 2) for _ in range(3)]
        sc = rnd.choice([0.3, 1.0, 3.0])
        half = [rnd.uniform(0.05, 1.5) * sc for _ in range(3)]
        cen = [rnd.uniform(-0.5, 0.5) * sc for _ in range(3)]
        mx = [cen[j] + half[j] for j in range(3)]
        mn = [cen[j] - half[j] for j in range(3)]
        r = rnd.choice([0.1, 0.5, 1.0, rnd.uniform(0.0, 2.0)])
        # the capsule in the part's frame: through the box, by it, far off; then back into the composite's frame
        how = rnd.random()
        loc = [cen[j] + rnd.uniform(-1, 1) * (half[j] + r) * rnd.choice([0.5, 1.0, 1.3, 2.5]) for j in range(3)]
        d = [rnd.gauss(0, 1) for _ in range(3)]
        ln = rnd.choice([0.0, 0.3, 1.0, 4.0]) * sc if how < 0.9 else 0.0
        dn = math.sqrt(sum(x * x for x in d)) or 1.0
        la = [loc[j] - d[j] / dn * ln * rnd.random() for j in range(3)]
        lb = [la[j] + d[j] / dn * ln for j in range(3)]
        back = lambda p: [pos[k] + sum(rows[i][k] * p[i] for i in range(3)) for k in range(3)]
        A, B = back(la), back(lb)
        row = ([fb(f32(v)) for rr in rows for v in rr] + [fb(f32(v)) for v in pos] + [fb(f32(v)) for v in A + B] + [fb(f32(r))] +
               [fb(f32(v)) for v in mx + mn])
        assert len(row) == N_IN
        cases.append(row)
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%08x' % x for x in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    main_src = MAIN.replace('NIN_', str(N_IN)).replace('FRAME_', frame).replace('PRUNE_', prune).replace('BACK_', unframe)
    exe = rd.build(idx, ['sub_' + frame, 'sub_' + prune, 'sub_' + unframe], main_src, work, 'prune', image=img)
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-500:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = [' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    nl = chr(10)
    (a.out_dir / 'part_prune_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    res = [o.split()[16] for o in out]
    print(len(cases), 'cases;', sum(x == '1' for x in res), 'dropped')


if __name__ == '__main__':
    main()
