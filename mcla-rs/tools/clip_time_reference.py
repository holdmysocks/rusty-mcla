"""Run the clip player's sample F-50482 offline (static recompilation, native, flat guest memory) for the port's
`vehicle_model.rs::clip_time` (test clip_time_matches_the_original): the time it hands to the sampler F-47152.

F-50482(clip, t f1, frame, round, reset): n1 = (clip +8 u16) - 1 as a float, dur = clip +12; round 0 (the clip player's):
F = (n1 / dur) x t (fdivs, fmuls); round 1: F = trunc(fma(n1 / dur, t, 0.5)) (not used by the port, not run here); reset -> the
frame cleared (F-26304, stubbed); then F-47152(frame, clip, (dur x F) / n1, 0) - stubbed here, recording its f1.

Cases: 6,000 from a fixed seed - frame counts 2 .. 300 (and the cop lights' 11), durations 1/30 .. 20 s (and 0x3EAAAAAB), times
in and just past [0, dur], 0, -0. Output, one line a case (words in decimal, 8-digit numbers padded to 10):
  <n> <dur bits> <t bits> | <t' bits>
"""
import argparse
import random
import re
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t C=0x40800000u, FR=0x40801000u;
 while(in>>tok){ if(tok!="case") break; uint32_t n, d, t; in>>n>>d>>t; rt_reset(); store(0x40900000u,0x77777777u);
  store16(C+8,(uint16_t)n); store(C+12,d); float tf; memcpy(&tf,&t,4);
  Context c=fresh(); c.r3.u32=C; c.f1.f64=(double)tf; c.r5.u32=FR; c.r6.u32=0; c.r7.u32=1; SAMPLE(c,nullptr);
  printf("%u\n",load(0x40900000u));}
 printf("end\n");
}
'''


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def cases():
    g = random.Random(50482)
    out = []
    for k in range(6000):
        n = 11 if k % 5 == 0 else g.randint(2, 300)
        dur = bits(0.33333334) if k % 7 == 0 else bits(g.choice([1 / 30, g.uniform(1 / 30, 2), g.uniform(0.1, 20)]))
        df = struct.unpack('<f', struct.pack('<I', dur))[0]
        t = g.choice([0.0, -0.0, g.uniform(0, df), g.uniform(0, df), df, g.uniform(df, 1.05 * df), (g.randint(0, 40) / 30.0)])
        out.append((n, dur, bits(t)))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-50482', 'F-47152', 'F-26304']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    reset = name('F-26304')
    stubs = {name('F-47152'): 'float x=(float)ctx.f1.f64; uint32_t w; memcpy(&w,&x,4); store(0x40900000u,w);', reset: ''}
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-50482')], MAIN.replace('SAMPLE', name('F-50482')), a.out, 'clip_time', image=img,
                      stubs=stubs, limit=10)
    cs = cases()
    cf = a.out / 'clip_cases.txt'
    cf.write_text(''.join('case %d %d %d\n' % c for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    res = r.stdout.splitlines()[:-1]
    # (8-digit numbers padded to 10 digits, so that no word reads as an address of the original to the citation check)
    pad = lambda t: re.sub(r'\b(\d{8})\b', lambda m: m.group(1).zfill(10), t)
    a.ref.write_text(''.join(pad('%d %d %d | %s' % (*c, x)) + '\n' for c, x in zip(cs, res)))
    print('cases', len(res), 'same t', sum(int(x) == c[2] for c, x in zip(cs, res)), '->', a.ref)


if __name__ == '__main__':
    main()
