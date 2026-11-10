"""The speech manager's scenario start F-95157 run offline: the generated code whole and unchanged, with the audio's
generator F-62480 (its state the two words F-62480 keeps), on random manager and scenario states. Laid out as the
original's: the manager (+2180 the pending requests' count, +2184 / +2192 handed to the first line, +2200 / +2204 the
sender's and receiver's voices, +2244 the clock, +2252 the sender entity, +2256 the first line, +2260 the scenario
playing), the scenario (+0 its scripts, 8 bytes: the steps and their count; +4 the scripts' count, +8 the name, +12
Priority, +16 Delay, +20 Frequency, +24 the last start, +28 the last script, +32 / +36 its own voices, +64 Probability),
the steps (24 bytes: +0 the type, 0 PlayAsset, 2 WaitTime, +4 the WaitTime's text), the sender entity (+36 its rank).
Arguments: the manager, the scenario, the sender entity, the sender's / receiver's names (none), whether forced, the byte
the first line keeps (+467) and whether the request may wait (queued while a scenario plays).
Stubbed, as inputs or recorders: the online check F-24338 (the case's value), the log F-47121 (the message's address),
the pending push F-13869, PlayPreparedScenario F-34685, the stop F-73277 (records; clears +2252 / +2256 / +2260 as it
does), the line pool's reset F-41074, the line build F-62806 (records the step and the delay it gets; returns the next
line), the line check F-86996 (ready), the text-to-number F-53123 (the step's number), the name copy F-32585. Writes <out-dir>/speech_start_ref.txt: per case 'C' and its
inputs, 'L' and the calls in order, 'A' and what is after (the result, +24, +28, the generator, +2200, +2204, +2252,
+2256, +2260, the first line's +440 / +444 / +467)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

MGR = 0x40100000
SCN = 0x40200000
OTH = 0x40210000     # another scenario playing
SCR = 0x40220000     # the scripts (8 bytes each)
STEP = 0x40230000    # script s's steps at STEP + 0x100 s, 24 bytes each (+16 the WaitTime's number)
LINE = 0x40300000    # the lines the build hands out, 0x200 apart
OLDL = 0x402F0000    # the first line of the scenario playing
SND = 0x40280000     # the sender entity
SND2 = 0x40290000    # another sender entity
NAME = 0x402A0000
V32, V0 = 0x402B0000, 0x402B0100
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    start = addr(a.tool, 'F-95157')
    sub = {f: 'sub_' + addr(a.tool, f) for f in ('F-24338', 'F-47121', 'F-13869', 'F-34685', 'F-73277', 'F-41074', 'F-62806',
                                                  'F-86996', 'F-53123', 'F-32585')}
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    r = random.Random(95157)
    cases = []
    for _ in range(4000):
        n = r.choice((0, 1, 2, 2, 3))
        scripts = []
        for _s in range(3):
            st = [r.choice((0, 0, 2)) for _j in range(r.choice((0, 1, 2, 3, 4)) if r.random() > 0.05 else 0)]
            if r.random() < 0.85 and 0 not in st:
                st.append(0)
            scripts.append(st + [9] * (4 - len(st)) if len(st) < 4 else st[:4])
        cur = r.choice((0, 0, 0, 1, 2, 2))
        cases.append([
            int(r.random() < 0.04),                             # 0 online
            int(r.random() < 0.15),                             # 1 may wait
            int(r.random() < 0.12),                             # 2 forced
            int(r.random() < 0.1),                              # 3 the first line's byte
            cur,                                                # 4 playing: 0 none, 1 this scenario, 2 another
            int(cur != 0 and r.random() < 0.25),                # 5 the playing line's +467
            r.choice((0, 1, 1, 2)) if cur else 0,               # 6 +2252: 0 none, 1 this sender, 2 another
            r.choice((0, 0, 8)),                                # 7 the pending count
            fb(r.choice((0.0, 5.0, 130.0, r.uniform(0, 500)))),            # 8 the clock
            fb(r.choice((-3.4028234663852886e38, 0.0, 4.0, r.uniform(0, 400)))),   # 9 the last start
            fb(r.choice((0.0, 10.0, 120.0))),                   # 10 Frequency
            r.choice((0, 30, 50, 100)),                         # 11 Probability
            n,                                                  # 12 the scripts
            r.choice((-1, 0, 1, 2)) & 0xffffffff,               # 13 the last script
            r.choice((1, 2, 3, 3, 5)),                          # 14 Priority
            r.choice((1, 2, 3, 4, 5)),                          # 15 the playing one's Priority
            fb(r.choice((0.0, 0.0, 1.5))),                      # 16 Delay
            int(r.random() < 0.85),                             # 17 +32 set
            int(r.random() < 0.5),                              # 18 +2200 set before
            r.choice((0, 1, 2, 3)), r.choice((0, 1, 2, 3)),     # 19 / 20 the two senders' +36
            r.getrandbits(32), r.getrandbits(32),               # 21 / 22 the generator
        ] + [t for s in scripts for t in s] + [fb(r.choice((0.5, 1.0, 2.25))) for _ in range(12)])
    NW = len(cases[0])
    data = ','.join(','.join('%du' % v for v in c) for c in cases)
    stubs = {
        sub['F-24338']: 'ctx.r3.u32=load(0x40180000u);',
        sub['F-47121']: 'printf(" g%04x",ctx.r3.u32&0xffff);',
        sub['F-13869']: 'printf(" U");ctx.r3.u32=1;',
        sub['F-34685']: 'printf(" A");',
        sub['F-73277']: 'printf(" S");store(@MGR@u+2252,0);store(@MGR@u+2256,0);store(@MGR@u+2260,0);',
        sub['F-41074']: 'printf(" F");',
        sub['F-62806']: 'printf(" L%u %u d%08x %c",(ctx.r4.u32-@STEP@u)/256,((ctx.r4.u32-@STEP@u)%256)/24,fbits(ctx.f1.f64),'
                        'ctx.r5.u32==@SND@u?109:120);ctx.r3.u32=@LINE@u+0x200*nl++;',
        sub['F-86996']: 'ctx.r3.u32=1;',
        sub['F-53123']: '{uint32_t b=load(ctx.r3.u32);float f;memcpy(&f,&b,4);ctx.f1.f64=f;}',
        sub['F-32585']: 'printf(" !copy");',
    }
    recomp_closure.EXTRA += 'static uint32_t fbits(double d){float f=(float)d;uint32_t b;memcpy(&b,&f,4);return b;}' + chr(10) + \
        'static uint32_t nl=0;' + chr(10)
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(size_t i=0;i<sizeof(C)/4;i+=@NW@){const uint32_t*w=C+i;nl=0;
 memset(pointer(@MGR@u),0,0x1000);memset(pointer(@SCN@u),0,0x100);memset(pointer(@OTH@u),0,0x100);memset(pointer(@LINE@u),0,0x2000);
 memset(pointer(@OLDL@u),0,0x200);memset(pointer(@STEP@u),0,0x400);
 printf("C");for(int k=0;k<@NW@;k++)printf(" i%u",w[k]);printf("\n");
 store(0x40180000u,w[0]);store(@RNG@u,w[21]);store(@RNG@u+4,w[22]);
 store(@MGR@u+2180,w[7]);store(@MGR@u+2184,0x1234u);store(@MGR@u+2192,0x5678u);store(@MGR@u+2244,w[8]);
 store(@MGR@u+2200,w[18]?@V0@u:0);
 store(@MGR@u+2260,w[4]==1?@SCN@u:w[4]==2?@OTH@u:0);store(@MGR@u+2256,w[4]?@OLDL@u:0);*pointer(@OLDL@u+467)=(uint8_t)w[5];
 store(@MGR@u+2252,w[6]==1?@SND@u:w[6]==2?@SND2@u:0);store(@SND@u+36,w[19]);store(@SND2@u+36,w[20]);
 store(@SCN@u+0,@SCR@u);store16(@SCN@u+4,w[12]);store(@SCN@u+8,@NAME@u);store(@SCN@u+12,w[14]);store(@SCN@u+16,w[16]);
 store(@SCN@u+20,w[10]);store(@SCN@u+24,w[9]);store(@SCN@u+28,w[13]);store(@SCN@u+32,w[17]?@V32@u:0);store(@SCN@u+64,w[11]);
 store(@OTH@u+12,w[15]);store(@OTH@u+8,@NAME@u);
 for(int s=0;s<3;s++){int n=0;for(int j=0;j<4;j++){uint32_t t=w[23+4*s+j];if(t==9)break;uint32_t at=@STEP@u+256*s+24*j;
  store(at,t);store(at+4,at+16);store(at+16,w[35+4*s+j]);n++;}
  store(@SCR@u+8*s,@STEP@u+256*s);store16(@SCR@u+8*s+4,n);}
 Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@MGR@u;ctx.r4.u32=@SCN@u;ctx.r5.u32=@SND@u;ctx.r6.u32=0;ctx.r7.u32=0;
 ctx.r8.u32=w[2];ctx.r9.u32=w[3];ctx.r10.u32=w[1];printf("L");
 sub_@START@(ctx,nullptr);
 uint32_t h=load(@MGR@u+2256);
 printf("\nA r%u x%08x x%08x x%08x x%08x x%08x x%08x x%08x x%08x x%08x",ctx.r3.u32&0xff,load(@SCN@u+24),load(@SCN@u+28),
  load(@RNG@u),load(@RNG@u+4),load(@MGR@u+2200),load(@MGR@u+2204),load(@MGR@u+2252),h,load(@MGR@u+2260));
 if(h)printf(" x%08x x%08x b%u",load(h+440),load(h+444),(unsigned)*pointer(h+467));
 printf("\n");}}
"""
    for k, v in (('DATA', data), ('NW', NW), ('MGR', MGR), ('SCN', SCN), ('OTH', OTH), ('SCR', SCR), ('STEP', STEP), ('LINE', LINE),
                 ('OLDL', OLDL), ('SND', SND), ('SND2', SND2), ('NAME', NAME), ('V32', V32), ('V0', V0), ('STACK', STACK),
                 ('RNG', ((-32125 << 16) + 26624) & 0xffffffff), ('START', start)):    # (F-62480's state, as its lis / addi build it)
        c = c.replace('@%s@' % k, str(v))
        stubs = {n: b.replace('@%s@' % k, str(v)) for n, b in stubs.items()}
    exe = rd.build(idx, ['sub_' + start], c, a.out_dir, 'speech_start', stubs=stubs, image=img)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'speech_start_ref.txt').write_text(out)
    ls = [l for l in out.split(chr(10)) if l.startswith('A')]
    print(len(cases), 'cases,', sum(l.startswith('A r1') for l in ls), 'started or held,', out.count(' L'), 'lines,', out.count(' S'), 'stops')


if __name__ == '__main__':
    main()
