"""The scripts' three WAIT natives run offline as generated: WAIT F-99099 (the frame step, the timer object's +8, scaled),
WAITUNWARPED F-36273 (its +88, the unscaled step) and WAITUNPAUSED F-77483 (the scaled step, not advanced while the
script manager's paused byte is set). The current thread (the script manager's pointer) holds the state (+12) and the
timer (+36); the native's argument (+8 of r3: the pointer to the arguments) is the wait in ms. Each case calls the
native once a frame with that frame's step (and paused byte) until it is done (state back to 0) or the frames run out.
Line:  <variant 0 WAIT / 1 UNWARPED / 2 UNPAUSED> <ms> <runs k> (<frames> <dt> <paused>) x k -> <done frame, -1 none> <timer bits>
(frame 0 is the call that starts the wait)
Usage: script_wait_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
static float rf(std::istream&in){std::string s;in>>s;return strtof(s.c_str(),nullptr);}
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t TH=0x40010000,CX=0x40010100,ARGS=0x40010200;
 int v,ms,k;
 while(in>>v>>ms>>k){rt_reset();
  for(uint32_t x=TH;x<0x40010300;x+=4)store(x,0);
  store(G_THREAD,TH);store(CX+8,ARGS);store(ARGS,(uint32_t)ms);
  int done=-1,f=0;
  for(int r=0;r<k;r++){int c;in>>c;float dt=rf(in);int p;in>>p;
  for(int j=0;j<c;j++,f++){
   if(done>=0)continue;
   sf(G_DTS,dt);sf(G_DTU,dt);uint32_t w=load(G_PAUSED&~3u);int sh=(3-(G_PAUSED&3))*8;store(G_PAUSED&~3u,(w&~(0xFFu<<sh))|((uint32_t)(p&255)<<sh));
   Context cx=fresh();cx.r3.u32=CX;
   if(v==0)sub_W0(cx,nullptr);else if(v==1)sub_W1(cx,nullptr);else sub_W2(cx,nullptr);
   if(f>0&&load(TH+12)==0)done=f;
  }}
  printf("%d %u\n",done,load(TH+36));
 }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    fns = [addr('F-99099'), addr('F-36273'), addr('F-77483')]
    img = rd.load_image(a.image)
    # (the script manager's current thread and paused byte; the frame steps the timer update copies from the timer
    # object: +8 and +88 - as the natives load them, lis / offset)
    g = {'G_THREAD': (-32114 << 16) - 23620, 'G_PAUSED': (-32114 << 16) - 23616,
         'G_DTS': (-32127 << 16) + 4296, 'G_DTU': (-32127 << 16) + 4300}
    m = MAIN
    for k, v in g.items():
        m = m.replace(k, hex(v & 0xFFFFFFFF) + 'u')
    for i, f in enumerate(fns):
        m = m.replace(f'sub_W{i}(', f + '(')
    exe = rd.build(rd.index(a.recomp), fns, m, a.work, 'script_wait', image=img)
    rnd = random.Random(36273)
    f32 = lambda x: struct.unpack('<f', struct.pack('<f', x))[0]
    lines = []
    steps = [1 / 30, 1 / 60, 1 / 50, 1 / 120, 0.1, 0.0]
    mss = [0, 1, 10, 16, 17, 30, 33, 34, 100, 200, 250, 300, 500, 1000, 1500, 2000, 2200, 3000, 4400, 6000, -5]
    for v in range(3):
        for ms in mss:
            for dt in steps:
                n = min(400, int(ms / 1000 / dt) + 6) if dt > 0 else 8
                lines.append((v, ms, [(dt, 0)] * n))
    while len(lines) < 1600:
        v = rnd.randrange(3)
        base = rnd.choice(steps[:5])
        nf = rnd.randrange(2, 40)
        # (mostly a wait that ends within the frames, some that do not)
        ms = rnd.choice([rnd.randrange(0, int(nf * base * 1000) + 1), rnd.randrange(0, int(nf * base * 1300) + 2), rnd.choice(mss)])
        fr = []
        for _ in range(nf):
            dt = rnd.choice([base, base * rnd.uniform(0.5, 1.5), rnd.uniform(0.0, 0.1), base * 0.1])
            fr.append((dt, 1 if rnd.random() < 0.15 else 0))
        lines.append((v, ms, fr))
    bad = re.compile(r'\b8[0-9A-Fa-f]{7}\b')
    def rle(fr):
        runs = []
        for d, q in fr:
            if runs and runs[-1][1:] == [d, q]:
                runs[-1][0] += 1
            else:
                runs.append([1, d, q])
        return f'{len(runs)} ' + ' '.join('%d %.9g %d' % (c, f32(d), q) for c, d, q in runs)
    text = [f'{v} {ms} ' + rle(fr) for v, ms, fr in lines]
    cases = a.work / 'script_wait_cases.txt'
    cases.write_text('\n'.join(text) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(text), (len(rows), len(text))
    out = [l + ' -> ' + r for l, r in zip(text, rows)]
    keep = [o for o in out if not bad.search(o)]
    a.out.write_text(''.join(o + '\n' for o in keep))
    print(len(keep), 'cases', sum(o.endswith(' -1 ' + o.split()[-1]) for o in keep), 'not done')


if __name__ == '__main__':
    main()
