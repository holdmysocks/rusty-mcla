"""The menu choice row's (combo's) event handler run offline (recomp_dispatch): F-47154 on a combo object whose vtable is
faked (the gates vt+72 / vt+60 answer the case's `live`), with no list (+200 = 0) so the count is the u16 at +192, the
sound play F-42664 stubbed to record the sound, and the selection setter F-11092 stubbed to record the index it is given
(-1 when not called). Each case: event id, index (+208), count, wrap (+232, the ctor F-70207's last argument), live.
Writes ui_combo_ref.txt:  <ev> <index> <count> <wrap> <live> -> <returned> <index set or -1> <sound name or ->
Usage: ui_combo_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
FAKE_ON = 0x7F000010
FAKE_OFF = 0x7F000020


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
uint32_t SOUND=0;int32_t SET=-1;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t ev,cnt,wrap,live;int32_t i;
 while(scanf("%u %d %u %u %u",&ev,&i,&cnt,&wrap,&live)==5){
  if(ev>100||cnt==0||cnt>1000||wrap>1||live>1){printf("BAD");putchar(10);fflush(stdout);continue;}
  rt_reset();
  uint32_t S=rt_alloc(512,16),V=rt_alloc(512,16);
  for(int k=0;k<512;k+=4){store(S+k,0);store(V+k,live?FAKEON:FAKEOFF);}
  store(S,V);store(S+208,uint32_t(i));
  *pointer(S+192)=uint8_t(cnt>>8);*pointer(S+193)=uint8_t(cnt);
  *pointer(S+232)=uint8_t(wrap);
  SOUND=0;SET=-1;
  Context c=fresh();c.r3.u32=S;c.r4.u32=ev;
  sub_EVT(c,nullptr);
  printf("OUT %u %d %u",c.r3.u32&255,SET,SOUND);putchar(10);fflush(stdout);
 }
}
'''


def cstr(img, a):
    s = bytearray()
    while True:
        b = (rd.image_u32(img, a & ~3) >> (8 * (3 - (a & 3)))) & 255
        if b == 0 or len(s) > 64:
            return s.decode('latin-1')
        s.append(b)
        a += 1


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 3000
    assert 0 < count <= 100000, count
    r = random.Random(47154)
    cases = []
    for _ in range(count):
        ev = r.choice([51, 52, 53, 54, 51, 52, 53, 54, 50, 55, 0])
        n = r.choice([1, 2, 3, 4, 4, 4, 7, r.randint(1, 300)])
        i = r.choice([0, n - 1, r.randint(0, n - 1), n, -1, n + 3])
        cases.append((ev, i, n, r.choice([0, 1]), r.choice([1, 1, 1, 0])))
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_combo'
    work.mkdir(exist_ok=True)
    evt, play, sel = ('sub_' + addr(x) for x in ('F-47154', 'F-42664', 'F-11092'))
    src = MAIN.replace('sub_EVT', evt).replace('FAKEON', '%uu' % FAKE_ON).replace('FAKEOFF', '%uu' % FAKE_OFF)
    exe = rd.build(idx, [evt], src, work, 'combo', image=img,
                   stubs={play: '{extern uint32_t SOUND;SOUND=ctx.r3.u32;}', sel: '{extern int32_t SET;SET=int32_t(ctx.r4.u32);}'},
                   fakes={FAKE_ON: '{ctx.r3.u64=1;}', FAKE_OFF: '{ctx.r3.u64=0;}'})
    p = subprocess.run([str(exe), str(image)], input=''.join('%u %d %u %u %u\n' % c for c in cases), capture_output=True, text=True)
    res = [l[4:].split() for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    names = {0: '-'}
    lines = []
    for c, (ret, st, snd) in zip(cases, res):
        snd = int(snd)
        if snd not in names:
            names[snd] = cstr(img, snd)
        lines.append('%u %d %u %u %u -> %s %s %s\n' % (c + (ret, st, names[snd])))
    Path(out).write_text(''.join(lines))
    print(len(res), 'cases', sorted(set(names.values())))


if __name__ == '__main__':
    main()
