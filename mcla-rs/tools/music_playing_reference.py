"""The music manager's playing handler run offline: F-61726 (the state handler +376 while a song is wanted, every frame:
the song's place kept while it plays, the song stopped below -95 dB and started again at its place, the next song after
the end), the generated code whole and unchanged, on random manager states. Laid out as the original's manager (the same
offsets as tools/music_play_reference.py): +24 console mode, +36 the song-line clock, +44 Play's argument, +72 the song's
place (fraction), +76 / +80 / +84 its position / length / held position (ms), +384 the song (+4 its sound: +6 state, +172
position ms), +388 the one before it, +408 MUSIC_USER (its resolved volume in the resolved category table, entry +82 x 36),
+496 the state, +526 the restart byte; f1 the frame's time (s).
Run with it, unchanged: the stop F-79064, the song reset F-14624 and the next song F-51486 (+48 / +52 the order's place,
+88 its song table). Stubbed, as inputs or recorders: the voice release F-63886 (records), the custom-soundtrack flag
F-15700 (the case's value), the song start F-62155, the song-line callback F-17609, the hard stop F-93800, the order's
next place F-44754 (the case's place), F-31667 / F-39054 (F-51486's song branch, not reached), F-77135 (the song's name for the log) and the log F-47121 (record their
arguments), the Streaming duration F-96939 (records; the case's length).
Writes <out-dir>/music_playing_ref.txt (each number led by a letter): per case 'C' and its inputs, 'L' and the calls in
order, then 'A' and the fields after (+36, +72, +76, +80, +84, +526, +48, +68, +376)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static uint32_t fbits(double d){float f=(float)d;uint32_t b;memcpy(&b,&f,4);return b;}' + chr(10)

MGR = 0x40100000
CAT = 0x40120000     # the MUSIC_USER category object (+82 its index)
RES = 0x40130000     # the resolved category table
SONG = 0x40140000    # the song record (+4 its sound)
SND = 0x40150000     # the song's sound
V388 = 0x40160000
TAB = 0x40170000     # the order's song table (+88)
IN = 0x40180000      # the case's stub inputs: +0 the custom-soundtrack flag, +4 the song's length
STACK = 0x40F00000
CATIDX = 5


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
    handler = addr(a.tool, 'F-61726')
    release, user, start, dur, line, hard, order, name, log, kill, make = (addr(a.tool, f) for f in (
        'F-63886', 'F-15700', 'F-62155', 'F-96939', 'F-17609', 'F-93800', 'F-44754', 'F-77135', 'F-47121', 'F-31667', 'F-39054'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    r = random.Random(61726)
    dbs = (0.0, -95.0, -94.99, -95.01, -100.0, -48.0, 3.0, -17.5)
    cases = []
    for _ in range(3000):
        cases.append((
            int(r.random() < 0.2),                              # +388
            fb(r.choice(dbs)),                                  # MUSIC_USER's resolved volume
            int(r.random() < 0.15),                             # the custom-soundtrack flag
            int(r.random() < 0.6),                              # +384 a song
            r.choice((2, 2, 2, 1, 3, 0)),                       # its sound's state
            r.choice((0, 1, 12345, 199000, 240000, r.randrange(0, 300000))),   # its position (ms)
            r.choice((240000, 200000, 1, -1 & 0xffffffff, r.randrange(1, 400000))),   # its length (ms)
            int(r.random() < 0.15),                             # +24 console
            int(r.random() < 0.3),                              # +526
            r.choice((0, 1, 2, 3, 4)),                          # +496
            fb(r.choice((0.0, 0.5, 0.98999, 0.99, 0.995, 1.0, 0.9900001))),   # +72
            int(r.random() < 0.3),                              # +44
            fb(r.choice((15.0, 0.01, 0.0, -1.0, 0.033, r.uniform(0, 15)))),   # +36
            fb(r.choice((1 / 30, 1 / 60, 0.0, 0.1, r.uniform(0, 0.2)))),       # the frame's time
            r.choice((0, 5000, 123456)), r.choice((0, 7000)), r.choice((0, 240000)),   # +76 / +84 / +80
            r.randrange(0, 4),                                  # the order's next place
        ))
    data = ','.join(','.join('%du' % v for v in c) for c in cases)
    NW = len(cases[0])
    stubs = {
        'sub_' + release: 'printf(" R%c e%u",ctx.r3.u32==@V388@u?\'o\':ctx.r3.u32==@SONG@u?\'s\':\'x\',ctx.r4.u32);',
        'sub_' + user: 'ctx.r3.u32=load(@IN@u);',
        'sub_' + order: 'printf(" O%c",ctx.r3.u32==@MGR@u+160?109:120);ctx.r3.u32=load(@IN@u+8);',
        'sub_' + name: 'printf(" G%c",ctx.r3.u32==@MGR@u?109:120);ctx.r3.u32=0;',
        'sub_' + log: '',
        'sub_' + kill: 'printf(" !K");',      # (F-51486's song branch: not reached, the handler calls it with no song)
        'sub_' + make: 'printf(" !M");',
        'sub_' + start: 'printf(" T e%u",ctx.r4.u32);',
        'sub_' + dur: 'printf(" D%c e%u",ctx.r3.u32==@SND@u?\'s\':\'x\',ctx.r4.u32);ctx.r3.u32=load(@IN@u+4);',
        'sub_' + line: 'printf(" B");',
        'sub_' + hard: 'printf(" Q%c",ctx.r3.u32==@SONG@u?\'s\':\'x\');',
    }
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@GRES@u,@RES@u);
for(size_t i=0;i<sizeof(C)/4;i+=@NW@){const uint32_t*w=C+i;
 memset(pointer(@MGR@u),0,0x400);memset(pointer(@RES@u),0,0x400);memset(pointer(@SND@u),0,0x200);
 printf("C");for(int k=0;k<@NW@;k++)printf(" i%u",w[k]);printf("\n");
 store(@MGR@u+388,w[0]?@V388@u:0);store(@MGR@u+408,@CAT@u);store16(@CAT@u+82,@CATIDX@);store(@RES@u+36*@CATIDX@,w[1]);
 store(@IN@u,w[2]);store(@IN@u+4,w[6]);store(@IN@u+8,w[17]);
 store(@MGR@u+88,@TAB@u);for(int k=0;k<4;k++)store(@TAB@u+4*k,0x5000+k);store(@MGR@u+376,0x11111111u);
 store(@MGR@u+384,w[3]?@SONG@u:0);store(@SONG@u+4,@SND@u);store16(@SND@u+6,w[4]);store(@SND@u+172,w[5]);
 store(@MGR@u+24,w[7]);*pointer(@MGR@u+526)=(uint8_t)w[8];store(@MGR@u+496,w[9]);store(@MGR@u+72,w[10]);
 *pointer(@MGR@u+44)=(uint8_t)w[11];store(@MGR@u+36,w[12]);store(@MGR@u+76,w[14]);store(@MGR@u+84,w[15]);store(@MGR@u+80,w[16]);
 Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@MGR@u;{float f;uint32_t b=w[13];memcpy(&f,&b,4);ctx.f1.f64=f;}printf("L");
 sub_@HANDLER@(ctx,nullptr);
 printf("\nA x%08x x%08x x%08x x%08x x%08x b%u",load(@MGR@u+36),load(@MGR@u+72),load(@MGR@u+76),load(@MGR@u+80),load(@MGR@u+84),
  (unsigned)*pointer(@MGR@u+526));
 printf(" x%08x x%08x x%08x\n",load(@MGR@u+48),load(@MGR@u+68),load(@MGR@u+376));}}
"""
    G = lambda hi, lo: ((hi << 16) + lo) & 0xffffffff     # the globals as the code builds them (lis / addi)
    for k, v in (('GRES', G(-32125, 26652)), ('DATA', data), ('NW', NW), ('MGR', MGR), ('CAT', CAT), ('RES', RES), ('SONG', SONG),
                 ('SND', SND), ('V388', V388), ('IN', IN), ('STACK', STACK), ('TAB', TAB), ('CATIDX', CATIDX), ('HANDLER', handler)):
        c = c.replace('@%s@' % k, str(v))
        stubs = {n: b.replace('@%s@' % k, str(v)) for n, b in stubs.items()}
    exe = rd.build(idx, ['sub_' + handler], c, a.out_dir, 'music_playing', stubs=stubs, image=img)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'music_playing_ref.txt').write_text(out)
    ls = out.split(chr(10))
    calls = [l for l in ls if l.startswith('L')]
    print(len(cases), 'cases,', sum(' Q' in l for l in calls), 'hard stops,', sum(' T' in l for l in calls), 'starts,',
          sum(' O' in l for l in calls), 'next songs,', sum(' B' in l for l in calls), 'song lines')


if __name__ == '__main__':
    main()
