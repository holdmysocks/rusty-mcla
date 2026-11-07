"""The menu slider's event handler run offline (recomp_dispatch): F-59022 on a slider object whose vtable is faked (the two
gates vt+72 / vt+60 answer the case's `live`), the sound play F-42664 stubbed to record the sound it is given, the change
delegate F-57833 stubbed to record that it was called. The step sounds +248 / +252 are what the slider ctor F-39683 stores
there (the menu sound table's words, read from the image), the refusal sound is the table's own word. Each case: event id,
lo (+208), hi (+212), value (+216), step (+220), wrap (+224), live. Writes ui_slider_ref.txt:
  <ev> <lo> <hi> <value> <step> <wrap> <live> -> <returned> <+216 after> <sound name or -> <delegate called>
Usage: ui_slider_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
FAKE_ON = 0x7F000010
FAKE_OFF = 0x7F000020
# the menu sound table (a `const char*` table in the image): the slider ctor's +248 / +252 words and the refusal word,
# as the ctor and the handler address them (hi part -32126, lo parts 24600, 24596, 24612)
TABLE = (-32126 << 16) & 0xFFFFFFFF
DOWN_W, UP_W, ERR_W = TABLE + 24600, TABLE + 24596, TABLE + 24612


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
uint32_t SOUND=0,DELEG=0;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t ev,wrap,live;int32_t lo,hi,v,st;
 while(scanf("%u %d %d %d %d %u %u",&ev,&lo,&hi,&v,&st,&wrap,&live)==7){
  if(ev>100||wrap>1||live>1){printf("BAD");putchar(10);fflush(stdout);continue;}
  rt_reset();
  uint32_t S=rt_alloc(512,16),V=rt_alloc(512,16);
  for(int i=0;i<512;i+=4){store(S+i,0);store(V+i,live?FAKEON:FAKEOFF);}
  store(S,V);store(S+208,uint32_t(lo));store(S+212,uint32_t(hi));store(S+216,uint32_t(v));store(S+220,uint32_t(st));
  *pointer(S+224)=uint8_t(wrap);
  store(S+248,load(DOWNW));store(S+252,load(UPW));
  SOUND=0;DELEG=0;
  Context c=fresh();c.r3.u32=S;c.r4.u32=ev;
  sub_EVT(c,nullptr);
  printf("OUT %u %d %u %u",c.r3.u32&255,int32_t(load(S+216)),SOUND,DELEG);putchar(10);fflush(stdout);
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
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 6000
    assert 0 < count <= 100000, count
    r = random.Random(59022)
    cases = []
    for i in range(count):
        ev = r.choice([51, 52, 53, 54, 51, 52, 53, 54, 50, 55, 0, 47])
        lo = r.choice([0, -10, 1, r.randint(-100000, 100000)])
        hi = lo + r.choice([0, 1, 10, 100, r.randint(0, 200000)])
        st = r.choice([1, 5, 200, 875, r.randint(1, 5000)])
        v = r.choice([lo, hi, lo + st, hi - st, r.randint(lo, hi), lo - 3, hi + 3])
        cases.append((ev, lo, hi, v, st, r.choice([0, 1]), r.choice([1, 1, 1, 0])))
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_slider'
    work.mkdir(exist_ok=True)
    evt, play, deleg = ('sub_' + addr(x) for x in ('F-59022', 'F-42664', 'F-57833'))
    src = (MAIN.replace('sub_EVT', evt).replace('FAKEON', '%uu' % FAKE_ON).replace('FAKEOFF', '%uu' % FAKE_OFF)
           .replace('DOWNW', '%uu' % DOWN_W).replace('UPW', '%uu' % UP_W))
    exe = rd.build(idx, [evt], src, work, 'slider', image=img,
                   stubs={play: '{extern uint32_t SOUND;SOUND=ctx.r3.u32;}', deleg: '{extern uint32_t DELEG;DELEG=1;}'},
                   fakes={FAKE_ON: '{ctx.r3.u64=1;}', FAKE_OFF: '{ctx.r3.u64=0;}'})
    p = subprocess.run([str(exe), str(image)], input=''.join('%u %d %d %d %d %u %u\n' % c for c in cases), capture_output=True, text=True)
    res = [l[4:].split() for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    names = {0: '-'}
    lines = []
    for c, (ret, val, snd, dl) in zip(cases, res):
        snd = int(snd)
        if snd not in names:
            names[snd] = cstr(img, snd)
        lines.append('%u %d %d %d %d %u %u -> %s %s %s %s\n' % (c + (ret, val, names[snd], dl)))
    Path(out).write_text(''.join(lines))
    print(len(res), 'cases', sorted(set(names.values())))


if __name__ == '__main__':
    main()
