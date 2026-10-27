"""The camera time warper (mcTimeWarper: start F-85638, release F-79714, per-frame update F-25863 with its ramp F-73069 and tail F-22575,
constructor F-30992, tune defaults F-49780) run offline: unchanged generated code on a guest object; the game's time scale of the moment
and the frame time are guest globals set before each operation (before an update the scale is the warper's own current value: the game applies it
each frame, and the automatic release at the end of the hold reads the global). The sound-handle branches (object +40 / +44 stay 0) are not reached.
Object: +0 state (0 in, 1 out, 2 idle, 3 hold), +4 enabled, +8 normal scale, +12 from, +16 to, +20 current, +24 t, +28 since, +32 rate, +36 -> tune
(+12 duration, +16 slow-motion scale, +20 rise rate, +24 fall rate).
Lines:  SEQ <dur> <slow> <rise> <fall> | <ops> -> one 7-word state (state from to cur t since rate) per op      (floats as hex bits)
        ops: S<now>,<dt> start | R<now>,<dt> release | U<dt> update
        CTOR -> the 12 words of the constructed object (+0 .. +44), then DEFAULTS -> the four tune floats of the tune constructor
Usage: camera_warper_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

SOUND_STUBS = ['F-55770', 'F-81206']       # the sound-handle calls of release / start (object +40 / +44 stay 0: not reached)
IDS = {'START': 'F-85638', 'REL': 'F-79714', 'UPD': 'F-25863', 'CTOR': 'F-30992', 'DEF': 'F-49780'}

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string t;
 const uint32_t GNOW=uint32_t(-2105671680)+7220, GDT=uint32_t(-2105737216)+29952+88;
 const uint32_t O=0x40010000,TU=0x40010200;
 auto dump=[&](){printf("S");for(int o:{0,12,16,20,24,28,32})printf(" %08x",load(O+o));printf("\n");};
 {rt_reset();for(int i=0;i<12;i++)store(O+4*i,0xdeadbeefu);Context c=fresh();c.r3.u32=O;sub_CTOR(c,nullptr);printf("C");for(int i=0;i<12;i++)printf(" %08x",load(O+4*i));printf("\n");}
 {rt_reset();for(int i=0;i<12;i++)store(O+4*i,0xdeadbeefu);Context c=fresh();c.r3.u32=O;sub_DEF(c,nullptr);printf("D");for(int o:{12,16,20,24})printf(" %08x",load(O+o));printf("\n");}
 uint32_t d,s,r,f;
 while(in>>std::hex>>d>>s>>r>>f>>std::dec){rt_reset();
  for(int i=0;i<12;i++)store(O+4*i,0);store(TU+12,d);store(TU+16,s);store(TU+20,r);store(TU+24,f);
  {Context c=fresh();c.r3.u32=O;sub_CTOR(c,nullptr);}store(O+36,TU);
  int n;in>>n;
  for(int i=0;i<n;i++){std::string op;in>>op;char k=op[0];uint32_t a=0,b=0;
   if(k=='U'){a=std::stoul(op.substr(1),nullptr,16);store(GDT,a);store(GNOW,load(O+20));Context c=fresh();c.r3.u32=O;sub_UPD(c,nullptr);}
   else{size_t cm=op.find(',');a=std::stoul(op.substr(1,cm-1),nullptr,16);b=std::stoul(op.substr(cm+1),nullptr,16);store(GNOW,a);store(GDT,b);
    Context c=fresh();c.r3.u32=O;if(k=='S')sub_START(c,nullptr);else sub_REL(c,nullptr);}
   dump();}
 }
}
'''


def f32bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    addr = {k: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', v], capture_output=True, text=True, check=True).stdout.split()[1] for k, v in IDS.items()}
    src = MAIN
    for k, v in addr.items():
        src = src.replace('sub_' + k, v)
    tail = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', 'F-22575'], capture_output=True, text=True, check=True).stdout.split()[1]
    exe = rd.build(idx, list(addr.values()) + [tail], src, a.work, 'camera_warper', image=rd.load_image(a.image), stubs=['sub_' + subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[1] for f in SOUND_STUBS])
    rnd = random.Random(2030)
    fb = f32bits
    seqs = []
    for _ in range(300):
        dur = rnd.choice([0.0, 0.5, 1.25, rnd.uniform(0, 3)])
        slow = rnd.choice([0.1, 0.239, rnd.uniform(0.02, 1.0)])
        rise = rnd.choice([0.0, -1.0, 2.009, 5.0, rnd.uniform(0.1, 6)])
        fall = rnd.choice([0.0, 3.009, 1.0, 10.0, rnd.uniform(0.1, 6)])
        dts = [1 / 60, 1 / 30, rnd.uniform(0.001, 0.25)]
        ops = []
        for _ in range(80):
            x = rnd.random()
            dt = fb(rnd.choice(dts))
            now = fb(rnd.choice([1.0, rnd.uniform(0.05, 1.4)]))
            if x < 0.05:
                ops.append(f'S{now:x},{dt:x}')
            elif x < 0.10:
                ops.append(f'R{now:x},{dt:x}')
            else:
                ops.append(f'U{dt:x}')
        seqs.append(f'{fb(dur):x} {fb(slow):x} {fb(rise):x} {fb(fall):x} 80 ' + ' '.join(ops))
    cases = a.work / 'camera_warper_cases.txt'
    cases.write_text('\n'.join(seqs) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    out = ['CTOR -> ' + rows[0][2:], 'DEFAULTS -> ' + rows[1][2:]]
    rows = [r[2:] for r in rows[2:]]
    assert len(rows) == 300 * 80
    for i, s in enumerate(seqs):
        out.append('SEQ ' + s + ' -> ' + ' | '.join(rows[i * 80:(i + 1) * 80]))
    a.out.write_text('\n'.join(out) + '\n')
    print(len(seqs), 'sequences')


if __name__ == '__main__':
    main()
