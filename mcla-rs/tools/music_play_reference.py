"""The music manager's Play and suspend run offline: the original's Play F-33831 (from the musicplay cine event, the scripts'
mcMusicManager_Play, the playlist's PLAY and the HUD's music buttons), the race scene's suspend F-64824 (racer_enablemusic)
and PlaySilently F-60016, with what they reach, the song start F-62155, the song reset F-14624 and the song change
F-17609, the generated code whole and unchanged, on random manager states. Laid out as the original's manager: +20 the tune
object (MUSIC_0_MANAGER from game.dat, or a random one: +10 the jingle, +14 / +26 / +32 the song's predelay behind the
jingle / the +508 / the +512 sound), +24 console mode (the +392 player's vtable: +20, +32, +52), +28 / +396 the console
entries, +36 / +40, +48 / +52, +72 .. +84, +76 the start offset, +376 / +380 the state handler, +384 the song, +388, +408
MUSIC_USER (its resolved volume in the resolved category table, entry +82 x 36), +496 the state, +500 the jingle, +508 / +512,
+516 the MUSIC_SUSPEND scene, +520 the song change callback, +526 / +527 / +528 (background play) bytes.
Stubbed, as inputs: the scene start F-10644 (records the mixer, name, strength and slot; fills the slot or not) and stop
F-55770, the sound stop F-31667, the jingle's create F-39054 (records hash, slot, level and the request words; fills the
slot or not), the voice release F-63886 (records; empties the slot that holds it), the song pick F-19761 (returns the case's
hash), the song's start F-21756 (records hash, slots, the request; fills +384 or not), F-77135 (records), the console
player's three slots and the callback (record; +52 returns the case's word).
Writes <out-dir>/music_play_ref.txt (each number led by a letter, so no column reads as an address): per case 'C' and its
inputs, the calls in order, then 'A' and the fields after."""
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
TUNE = 0x40110000
CAT = 0x40120000     # the MUSIC_USER category object (+82 its index)
RES = 0x40130000     # the resolved category table
CON = 0x40140000     # the console player (+392)
CVT = 0x40141000     # its vtable
ENTS = 0x40150000    # +396
MIXER = 0x40160000
VOICES = 0x40170000
IN = 0x40180000      # the case's stub inputs: +0 scene ok, +4 jingle ok, +8 song ok, +12 pick hash, +16 console +52 word
STACK = 0x40F00000
F52, F20, F32, FCB = 0x7F000010, 0x7F000020, 0x7F000030, 0x7F000040
SC_OLD, SC_NEW, J_OLD, J_NEW, S_OLD, S_NEW, V388 = 0x41000010, 0x41000020, 0x41000030, 0x41000040, 0x41000050, 0x41000060, 0x41000070
CATIDX = 5


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def gamedat(path, want):
    d = open(path, 'rb').read()
    size = struct.unpack_from('>I', d, 4)[0]
    p = 8 + size
    p += 4 + struct.unpack_from('>I', d, p)[0]
    count = struct.unpack_from('>I', d, p)[0]
    p += 8
    for _ in range(count):
        n = d[p]
        name = d[p + 1:p + 1 + n].decode('latin1')
        p += 1 + n
        off, sz = struct.unpack_from('>II', d, p)
        p += 8
        if name == want:
            return d[8 + off:8 + off + sz]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--game-dat', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    play, susp, silent, start, reset, change = (addr(a.tool, f) for f in ('F-33831', 'F-64824', 'F-60016', 'F-62155', 'F-14624', 'F-17609'))
    sc_start, sc_stop, snd_stop, jmake, release, pick, sstart, place = (addr(a.tool, f) for f in (
        'F-10644', 'F-55770', 'F-31667', 'F-39054', 'F-63886', 'F-19761', 'F-21756', 'F-77135'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    real = gamedat(a.game_dat, 'MUSIC_0_MANAGER')
    r = random.Random(33831)
    dbs = (0.0, -95.0, -94.99, -95.01, -100.0, -48.0, 3.0, -60.0)
    cases = []
    for k in range(1500):
        own = k % 3 != 0
        j_hash = struct.unpack_from('>I', real, 10)[0] if own else r.getrandbits(32)
        pre = [struct.unpack_from('>h', real, o)[0] for o in (14, 26, 32)] if own else [r.choice((0, 900, 75, 1, -5, 3000)) for _ in range(3)]
        op = r.choice((0, 0, 0, 0, 1, 1, 2))
        cases.append((
            op, int(r.random() < 0.3),                          # op, Play's argument
            int(r.random() < 0.15),                             # +24 console
            int(r.random() < 0.5), int(r.random() < 0.3),       # +384 song, +388
            int(r.random() < 0.3), int(r.random() < 0.2), int(r.random() < 0.2),   # +500, +508, +512
            int(r.random() < 0.4), int(r.random() < 0.3),       # +516, +520 callback
            int(r.random() < 0.3), int(r.random() < 0.3),       # +526, +528
            r.choice((0, 1, 2)),                                # +496
            r.choice((0, 1500, 30000, 7)),                      # +76
            r.randrange(0, 4),                                  # +28
            fb(r.choice(dbs)),                                  # MUSIC_USER's resolved volume
            int(r.random() < 0.85), int(r.random() < 0.85), int(r.random() < 0.85),   # scene / jingle / song ok
            r.getrandbits(32), int(r.random() < 0.5),           # pick hash, console +52 word
            j_hash, pre[0] & 0xffff, pre[1] & 0xffff, pre[2] & 0xffff,
            fb(r.choice((0.0, 2.5, -1.0))), fb(r.choice((0.0, 10.0, 0.25))),   # +36, +40
        ))
    data = ','.join(','.join('%du' % v for v in c) for c in cases)
    NW = len(cases[0])
    stubs = {
        'sub_' + sc_start: 'printf(" M%c %s f%08x s%u",ctx.r3.u32==@MIXER@u?\'m\':\'x\',(const char*)pointer(ctx.r4.u32),fbits(ctx.f1.f64),ctx.r5.u32-@MGR@u);'
                           'store(ctx.r5.u32,load(@IN@u)?@SC_NEW@u:0);',
        'sub_' + sc_stop: 'printf(" S%08x e%u",ctx.r3.u32,ctx.r4.u32);',
        'sub_' + snd_stop: 'printf(" K%08x e%u",ctx.r3.u32,ctx.r4.u32);',
        'sub_' + jmake: 'printf(" J%08x s%u f%08x g%u g%u g%u g%u g%u g%u q%u q%u q%u q%u q%u q%u",ctx.r4.u32,ctx.r5.u32-@MGR@u,'
                        'fbits(ctx.f1.f64),ctx.r6.u32,ctx.r7.u32,ctx.r8.u32,ctx.r9.u32,ctx.r10.u32,ctx.r3.u32-@MGR@u,'
                        'load(ctx.r1.u32+84),load(ctx.r1.u32+100),(unsigned)*pointer(ctx.r1.u32+111),load(ctx.r1.u32+116),'
                        '(unsigned)*pointer(ctx.r1.u32+127),load(ctx.r1.u32+132));'
                        'store(ctx.r5.u32,load(@IN@u+4)?@J_NEW@u:0);',
        'sub_' + release: 'printf(" R%08x e%u",ctx.r3.u32,ctx.r4.u32);'
                          'if(load(@MGR@u+384)==ctx.r3.u32)store(@MGR@u+384,0);if(load(@MGR@u+388)==ctx.r3.u32)store(@MGR@u+388,0);',
        'sub_' + pick: 'printf(" H");ctx.r3.u32=load(@IN@u+12);',
        'sub_' + sstart: '{uint32_t q=ctx.r7.u32;printf(" P%08x s%u s%u v%c b%d n%u n%u w%08x w%u w%u w%u w%d w%08x w%d w%u w%u w%u",ctx.r4.u32,'
                         'ctx.r5.u32-@MGR@u,ctx.r6.u32-@MGR@u,ctx.r3.u32==@VOICES@u?\'v\':\'x\',(int)ctx.r8.s32,ctx.r9.u32,ctx.r10.u32,'
                         'load(q),load(q+36),load(q+40),load(q+44),(int)load(q+56),load(q+60),(int)load(q+64),'
                         '(unsigned)*pointer(q+68),(unsigned)*pointer(q+69),(unsigned)*pointer(q+70));'
                         'store(ctx.r5.u32,load(@IN@u+8)?@S_NEW@u:0);}',
        'sub_' + place: 'printf(" G%u %u",ctx.r4.u32,ctx.r5.u32);ctx.r3.u32=0;',
    }
    fakes = {
        F52: 'printf(" C52");ctx.r3.u32=load(@IN@u+16);',
        F20: 'printf(" C20 i%u",(ctx.r4.u32-@ENTS@u)/820);',
        F32: 'printf(" C32");',
        FCB: 'printf(" B");',
    }
    c = r"""
static const uint32_t C[]={@DATA@};
static char hnd(uint32_t h){return h==0?'n':h==@HPLAY@u?'p':h==@HIDLE@u?'i':'x';}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@GMIX@u,@MIXER@u);store(@GRES@u,@RES@u);store(@GVOI@u,@VOICES@u);store(@GVB@u,@VOICES@u+0x100);
*pointer(@VOICES@u+0x100+15)=0xFE;
store(@CVT@u+52,@F52@u);store(@CVT@u+20,@F20@u);store(@CVT@u+32,@F32@u);store(@CON@u,@CVT@u);
for(size_t i=0;i<sizeof(C)/4;i+=@NW@){const uint32_t*w=C+i;
 memset(pointer(@MGR@u),0,0x400);memset(pointer(@TUNE@u),0,0x100);memset(pointer(@RES@u),0,0x400);
 printf("C");for(int k=0;k<@NW@;k++)printf(" i%u",w[k]);printf("\n");
 store(@GMGR@u,@MGR@u);
 store(@MGR@u+20,@TUNE@u);store(@TUNE@u+10,w[21]);store16(@TUNE@u+14,w[22]);store16(@TUNE@u+26,w[23]);store16(@TUNE@u+32,w[24]);
 store(@MGR@u+24,w[2]);store(@MGR@u+392,@CON@u);store(@MGR@u+396,@ENTS@u);store(@MGR@u+28,w[14]);
 store(@MGR@u+384,w[3]?@S_OLD@u:0);store(@MGR@u+388,w[4]?@V388@u:0);store(@MGR@u+500,w[5]?@J_OLD@u:0);
 store(@MGR@u+508,w[6]);store(@MGR@u+512,w[7]);store(@MGR@u+516,w[8]?@SC_OLD@u:0);store(@MGR@u+520,w[9]?@FCB@u:0);
 *pointer(@MGR@u+526)=(uint8_t)w[10];*pointer(@MGR@u+528)=(uint8_t)w[11];store(@MGR@u+496,w[12]);store(@MGR@u+76,w[13]);
 store(@MGR@u+408,@CAT@u);store16(@CAT@u+82,@CATIDX@);store(@RES@u+36*@CATIDX@,w[15]);
 store(@MGR@u+36,w[25]);store(@MGR@u+40,w[26]);store(@MGR@u+48,3);store(@MGR@u+52,4);
 store(@IN@u,w[16]);store(@IN@u+4,w[17]);store(@IN@u+8,w[18]);store(@IN@u+12,w[19]);store(@IN@u+16,w[20]);
 Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@MGR@u;ctx.r4.u32=w[1];printf("L");
 if(w[0]==0)sub_@PLAY@(ctx,nullptr);else if(w[0]==1)sub_@SUSP@(ctx,nullptr);else{ctx.r3.u32=0;sub_@SILENT@(ctx,nullptr);}
 printf("\nA h%c h%08x t%u j%08x m%08x s%08x v%08x b%u b%u b%u x%08x x%08x x%08x x%08x x%08x\n",hnd(load(@MGR@u+376)),load(@MGR@u+380),
  load(@MGR@u+496),load(@MGR@u+500),load(@MGR@u+516),load(@MGR@u+384),load(@MGR@u+388),(unsigned)*pointer(@MGR@u+526),
  (unsigned)*pointer(@MGR@u+527),(unsigned)*pointer(@MGR@u+528),load(@MGR@u+72),load(@MGR@u+76),load(@MGR@u+80),load(@MGR@u+84),load(@MGR@u+36));}}
"""
    G = lambda hi, lo: ((hi << 16) + lo) & 0xffffffff     # the globals as the code builds them (lis / addi)
    glob = (('GMIX', G(-32121, -12252)), ('GRES', G(-32125, 26652)), ('GVOI', G(-32121, -12548)), ('GVB', G(-32121, -12512)),
            ('GMGR', G(-32121, -12216)), ('HPLAY', int(addr(a.tool, 'F-61726'), 16)), ('HIDLE', G(-32237, 8)))
    for k, v in glob + (('DATA', data), ('NW', NW), ('MGR', MGR), ('TUNE', TUNE), ('CAT', CAT), ('RES', RES), ('CON', CON), ('CVT', CVT),
                 ('ENTS', ENTS), ('MIXER', MIXER), ('VOICES', VOICES), ('IN', IN), ('STACK', STACK), ('F52', F52), ('F20', F20),
                 ('F32', F32), ('FCB', FCB), ('SC_NEW', SC_NEW), ('J_OLD', J_OLD), ('J_NEW', J_NEW), ('S_OLD', S_OLD),
                 ('S_NEW', S_NEW), ('V388', V388), ('SC_OLD', SC_OLD), ('CATIDX', CATIDX), ('PLAY', play), ('SUSP', susp),
                 ('SILENT', silent)):
        c = c.replace('@%s@' % k, str(v))
        stubs = {n: b.replace('@%s@' % k, str(v)) for n, b in stubs.items()}
        fakes = {n: b.replace('@%s@' % k, str(v)) for n, b in fakes.items()}
    exe = rd.build(idx, ['sub_' + play, 'sub_' + susp, 'sub_' + silent], c, a.out_dir, 'music_play', stubs=stubs, image=img, fakes=fakes)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'music_play_ref.txt').write_text(out)
    ls = out.split(chr(10))
    calls = [l for l in ls if l.startswith('L')]
    print(len(cases), 'cases,', sum(' J' in l for l in calls), 'jingles,', sum(' P' in l for l in calls), 'song starts,',
          sum(' M' in l for l in calls), 'suspends,', sum(' S' in l for l in calls), 'scene stops')


if __name__ == '__main__':
    main()
