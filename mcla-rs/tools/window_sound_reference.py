"""A side window's controller and its motor sounds run offline: the original's open F-85820, close F-82826 and update
F-27851 on a window tracker (vtable 0x82046958), with the sound players they call (open start F-35283, open end F-78633, close
start F-45082, close end F-56861: the generated code whole and unchanged), in vector mode, on random runs of requests and frames.
The tracker: +8 state (1 closed), +16 the game object VEH_ANIM_WINDOW_LEFT/RIGHT_DEFAULT (its 61 bytes from game.dat),
+20 the loop handle, +24..+32 the prepared handles, +48 the sound's place, +64 time, +68 rate 0.5, +80 the clip player
(vt slot 2 F-12590 returns it: +64 -> +24 the clip, +12 its duration; the player's time is written to +4), +84 the clip's
duration. The car (r4): +16 the sound entity, +28 the instance (its matrix at +16); [r13]+72 = 0. The frame step is the
image's word at lis -32131 / 29952 + 96. The start-up vector constants the image holds as zero are filled in as the
running game has them: 0x82860930 = (0,0,0,1), 0x82860950 the xyz mask (docs/reference/ai/raceline.md).
Stubbed (the voices): the one-shot create F-55796 prints O<hash>, the persistent create F-30971 prints L<hash> and hands
a new handle into its slot, the stop F-31667 prints S<handle> and empties the slot that holds it (the voice clears its
owner's slot), the prepare F-60794 / play-prepared F-87666 / stop-prepared F-63886 calls print P / Y / X (none are reached: both window objects
have flags 0).
Writes <out-dir>/window_sound_ref.txt: the two objects' bytes, then one case a line: side, duration, the matrix's 12 words '->' per step the op
(u<dt> / o / c), the calls, then '=' state, time, the clip player's time and the place (hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

T = 0x40100000
OBJ = 0x40100100
VEH = 0x40101000
ENT = 0x40101100
INST = 0x40101200
ANIM = 0x40101300
APL = 0x40101400
CLIP = 0x40101500
TLS = 0x40101600
TLSB = 0x40101700
STACK = 0x40F00000
DT = ((-32131 << 16) + 29952 + 96) & 0xFFFFFFFF
VTABLE = 0x82046958
STEPS = 40


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def game_objects(path, names):
    d = path.read_bytes()
    size = struct.unpack_from('>I', d, 4)[0]
    data = d[8:8 + size]
    o = 8 + size
    o += 4 + struct.unpack_from('>I', d, o)[0]
    n = struct.unpack_from('>I', d, o)[0]
    o += 8
    out = {}
    for _ in range(n):
        ln = d[o]
        name = d[o + 1:o + 1 + ln].decode('latin1')
        o += 1 + ln
        off, sz = struct.unpack_from('>II', d, o)
        o += 8
        if name in names:
            out[name] = data[off:off + sz]
    return [out[n] for n in names]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--game-dat', type=Path, default=Path('D:/MCLA/extracted/cache/audio/X360/config/game.dat'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    st = lambda k: 'sub_' + addr(a.tool, k)
    objs = game_objects(a.game_dat, ['VEH_ANIM_WINDOW_LEFT_DEFAULT', 'VEH_ANIM_WINDOW_RIGHT_DEFAULT'])
    assert all(len(o) == 61 for o in objs)
    rnd = random.Random(27851)
    cases = []
    for _ in range(1500):
        side = rnd.randrange(2)
        dur = rnd.choice((0.6666667, 0.5, 1.0, rnd.uniform(0.05, 2.0), 1 / 30, 0.0))
        m = [rnd.uniform(-1, 1) for _ in range(9)] + [rnd.uniform(-2000, 2000), rnd.uniform(-50, 200), rnd.uniform(-2000, 2000)]
        steps = []
        for _ in range(STEPS):
            r = rnd.random()
            if r < 0.12:
                steps.append((1, 0.0))
            elif r < 0.24:
                steps.append((2, 0.0))
            else:
                steps.append((0, rnd.choice((1 / 30, 1 / 60, 0.0, rnd.uniform(0, 0.2), rnd.uniform(0, 3)))))
        cases.append((side, dur, m, steps))
    nl = chr(10)
    words = []
    for side, dur, m, steps in cases:
        words += [side, fb(dur)] + [fb(x) for x in m]
        for op, dt in steps:
            words += [op, fb(dt)]
    per = 2 + 12 + 2 * STEPS
    objw = ','.join('%du' % b for o in objs for b in o)
    stubs = {
        st('F-55796'): 'printf(" O%08x",ctx.r4.u32);',
        st('F-30971'): '{uint32_t n=load(0x40101800u)+1;store(0x40101800u,n);store(ctx.r5.u32,0x50000000u+n);}'
                       'printf(" L%08x",ctx.r4.u32);',
        st('F-31667'): 'printf(" S%08x",ctx.r3.u32);for(uint32_t s=0x40100014u;s<=0x40100020u;s+=4)if(load(s)==ctx.r3.u32)store(s,0);',
        st('F-60794'): 'printf(" P%08x",ctx.r4.u32);',
        st('F-87666'): 'printf(" Y%08x",ctx.r3.u32);',
        st('F-63886'): 'printf(" X%08x",ctx.r3.u32);',
    }
    main_src = (
        'static const uint32_t C[]={' + ','.join('%du' % w for w in words) + '};' + nl +
        'static const uint8_t OB[]={' + objw + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'printf("objects");for(int k=0;k<122;k++)printf("%s%02x",k%61?"":" ",OB[k]);printf("\\n");' + nl +
        'for(int k=0;k<3;k++)store(0x82860950u+4*k,0xFFFFFFFFu);store(0x8286095Cu,0);for(int k=0;k<3;k++)store(0x82860930u+4*k,0);store(0x8286093Cu,0x3f800000u);' + nl +
        'for(size_t i=0;i<sizeof(C)/4/%d;i++){const uint32_t*c=C+%d*i;store(0x40101800u,0);' % (per, per) +
        'for(uint32_t k=0;k<0x100;k+=4)store(%du+k,0);' % T +
        'for(int k=0;k<61;k++)pointer(%du+k)[0]=OB[61*c[0]+k];' % OBJ +
        'store(%du,%du);store(%du+8,1);store(%du+12,1);store(%du+16,%du);store(%du+68,%du);store(%du+80,%du);store(%du+84,c[1]);'
        % (T, VTABLE, T, T, T, OBJ, T, fb(0.5), T, APL, T) +
        'store(%du+64,%du);store(%du+24,%du);store(%du+12,c[1]);store(%du+4,0);' % (APL, ANIM, ANIM, CLIP, CLIP, ANIM) +
        'store(%du+16,%du);store(%du+28,%du);store(%du+96,0x1234u);store(%du,%du);pointer(%du+72)[0]=0;' % (VEH, ENT, VEH, INST, ENT, TLS, TLSB, TLSB) +
        'for(int r=0;r<4;r++){for(int k=0;k<3;k++)store(%du+16+16*r+4*k,c[2+3*r+k]);store(%du+16+16*r+12,r==3?0x3f800000u:0);}' % (INST, INST) +
        'printf("%d %08x",c[0],c[1]);for(int k=0;k<12;k++)printf(" %08x",c[2+k]);printf(" ->");' +
        'for(int s=0;s<%d;s++){uint32_t op=c[14+2*s],dt=c[15+2*s];Context ctx{};ctx.r1.u32=%du;ctx.r13.u32=%du;ctx.r3.u32=%du;ctx.r4.u32=%du;'
        % (STEPS, STACK, TLS, T, VEH) +
        'if(op==0){printf(" u%%08x",dt);store(%du,dt);%s(ctx,nullptr);}else if(op==1){printf(" o");%s(ctx,nullptr);}else{printf(" c");%s(ctx,nullptr);}'
        % (DT, st('F-27851'), st('F-85820'), st('F-82826')) +
        'printf(" = %%d %%08x %%08x %%08x %%08x %%08x",load(%du+8),load(%du+64),load(%du+4),load(%du+48),load(%du+52),load(%du+56));}'
        % (T, T, ANIM, T, T, T) +
        'printf("\\n");}}' + nl)
    idx = index(a.recomp)
    exe = rd.build(idx, [st('F-27851'), st('F-85820'), st('F-82826')], main_src, a.out_dir, 'window_sound', image=rd.load_image(a.image),
                   stubs=stubs, targets=[int(addr(a.tool, 'F-12590'), 16)])
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'window_sound_ref.txt').write_text(out)
    lines = [x for x in out.split(nl) if x]
    print(len(lines), 'cases', sum(x.count(' O') for x in lines), 'one-shots', sum(x.count(' L') for x in lines), 'loops')


if __name__ == '__main__':
    main()
