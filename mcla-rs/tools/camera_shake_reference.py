"""The camera shake's setters run offline (unchanged generated code): reset of the five blocks (F-29203), a continuous block's strength (F-54070),
the one-shot on block 0 (F-48972) and the manager's trigger in front of it (F-68650, scales by 0.4 when the camera's view is 0 or 1).
FX object: blocks at +288 + 224 i: +0 timer, +4 magnitude, +8 scale, +12 / +16 (1.0), +20 state. Camera object: +24 -> FX, +80 the view.
Line:  SEQ <5 x (timer magnitude scale state)> | <ops> -> after every op the 5 blocks (timer magnitude scale state) as hex bits / state
       ops: R reset | S<i>,<scale> set | J<view>,<duration>,<scale> jolt
Usage: camera_shake_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = {'RESET': 'F-29203', 'SET': 'F-54070', 'JOLT': 'F-68650'}

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);const uint32_t FX=0x40010000,CAM=0x40020000;
 auto dump=[&](){printf("S");for(int i=0;i<5;i++){uint32_t b=FX+288+224*i;printf(" %08x %08x %08x %u",load(b),load(b+4),load(b+8),load(b+20));}printf("\n");};
 int n;
 while(in>>n){rt_reset();
  store(CAM+24,FX);
  for(int i=0;i<5;i++){uint32_t b=FX+288+224*i,t,m,s,st;in>>std::hex>>t>>m>>s>>std::dec>>st;store(b,t);store(b+4,m);store(b+8,s);store(b+20,st);store(b+12,0);store(b+16,0);}
  int k;in>>k;
  for(int j=0;j<k;j++){std::string op;in>>op;Context c=fresh();
   if(op[0]=='R'){c.r3.u32=FX;sub_RESET(c,nullptr);}
   else if(op[0]=='S'){size_t cm=op.find(',');c.r3.u32=FX;c.r5.u32=std::stoul(op.substr(1,cm-1));uint32_t b=std::stoul(op.substr(cm+1),nullptr,16);float f;std::memcpy(&f,&b,4);c.f1.f64=f;sub_SET(c,nullptr);}
   else{size_t c1=op.find(','),c2=op.find(',',c1+1);store(CAM+80,std::stoul(op.substr(1,c1-1)));uint32_t d=std::stoul(op.substr(c1+1,c2-c1-1),nullptr,16),s=std::stoul(op.substr(c2+1),nullptr,16);
    float df,sf;std::memcpy(&df,&d,4);std::memcpy(&sf,&s,4);c.r3.u32=CAM;c.f1.f64=df;c.f2.f64=sf;sub_JOLT(c,nullptr);}
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
    exe = rd.build(idx, list(addr.values()), src, a.work, 'camera_shake', image=rd.load_image(a.image))
    rnd = random.Random(2031)
    fb = f32bits
    nice = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 0.4, 0.8, 4.0]
    seqs = []
    for _ in range(400):
        blocks = []
        for _ in range(5):
            blocks += [fb(rnd.choice([0.0, 1.0, rnd.choice(nice), rnd.uniform(0, 3)])), fb(rnd.choice([0.01, 3.0, rnd.uniform(0, 5)])), fb(rnd.choice([2.0, rnd.choice(nice), rnd.uniform(0, 4)])), rnd.randint(0, 2)]
        ops = []
        for _ in range(30):
            x = rnd.random()
            if x < 0.1:
                ops.append('R')
            elif x < 0.4:
                ops.append(f'S{rnd.randint(0, 4)},{fb(rnd.choice([rnd.choice(nice), rnd.uniform(0, 3)])):x}')
            else:
                ops.append(f'J{rnd.randint(0, 5)},{fb(rnd.choice([rnd.choice(nice), rnd.uniform(0.05, 3)])):x},{fb(rnd.choice([rnd.choice(nice), rnd.uniform(0.05, 4)])):x}')
        seqs.append(' '.join(f'{blocks[i]:x} {blocks[i+1]:x} {blocks[i+2]:x} {blocks[i+3]}' for i in range(0, 20, 4)) + ' 30 ' + ' '.join(ops))
    cases = a.work / 'camera_shake_cases.txt'
    cases.write_text('\n'.join('5 ' + s for s in seqs) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = [r[2:] for r in p.stdout.splitlines()]
    assert len(rows) == 400 * 30
    out = [f'SEQ {s} -> ' + ' | '.join(rows[i * 30:(i + 1) * 30]) for i, s in enumerate(seqs)]
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'sequences')


if __name__ == '__main__':
    main()
