"""The revs machine run offline: the original's F-74751 (the car audio's update callback +120 while the revs mode byte
+1950 is set and until the start boost +152 is back at 1) with its ramp F-90102, the generated code whole and unchanged,
on random sequences of updates. Stubbed, as inputs of the case: the audio clock F-84285 (the case's `now`), the
one-shot's make F-60794 (puts the slot object in +1464, state 1), play F-87666 (state 5), release F-63886 (empties the
slot) and the Duration F-73664 (the case's value). The revs mode setter F-17772 is done by the runner (+120 set to the
machine when the mode goes on); between updates the runner ends the one-shot at random (the slot emptied, as F-30709
does when its sound has gone) or puts its slot object in another state. The real frame time is the image word the
machine reads.
Writes <out-dir>/revs_machine_ref.txt: per update the mode, the slot's state before (0 empty), the time since played and
the stored Duration, the frame time and the boost (f32 bits), then '->' the boost after and make / play / stop / done."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static uint32_t NOW, DUR, MK, PL, ST;' + chr(10)

THIS = 0x40100000
WRAP = 0x40110000
ONE = 0x40120000
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    machine, clock, make, play, release, dur = (addr(a.tool, f) for f in (
        'F-74751', 'F-84285', 'F-60794', 'F-87666', 'F-63886', 'F-73664'))
    idx = index(a.recomp)
    m = idx['sub_' + machine].read_text(errors='replace')
    m = m[m.index('DEFINE_REX_FUNC(sub_%s)' % machine):]
    m = m[:m.index('\n}\n')]
    # the real frame time: lis rX,hi / addi rY,rX,lo / lfs f,92(rY)
    hi, lo = re.search(r'// lis (r\d+),(-?\d+)\s*\n(?:.*\n){0,4}?\s*// addi (r\d+),\1,(-?\d+)', m).group(2, 4)
    step = (((int(hi) << 16) + int(lo)) & 0xffffffff) + 92
    assert step == 0x827D755C, hex(step)
    done_fn = int(addr(a.tool, 'F-47121'), 16)  # the empty callback the machine leaves in +120
    rnd = random.Random(74751)
    rows = []
    for _ in range(400):
        n = rnd.randrange(10, 90)
        boost = rnd.choice((1.0, 1.0, 1.995262, 0.5, 0.0, 0.25))
        frames = []
        mode = 1
        for k in range(n):
            if k and rnd.random() < 0.08:
                mode ^= 1
            dt = rnd.choice((1 / 30, 1 / 60, 0.16, 0.3, rnd.uniform(0.001, 0.5)))
            d = rnd.choice((-1, 0, 1500, 2000, 2001, 2500, 5000, 12000))
            # between updates: 1 the one-shot ends (slot emptied), 2 its slot object is put in state 3
            ev = 1 if rnd.random() < 0.06 else 2 if rnd.random() < 0.03 else 0
            frames.append((mode, fbits(dt), d, ev, int(dt * 1000) + rnd.randrange(0, 3)))
        rows.append((fbits(boost), frames))
    nl = chr(10)
    data = []
    for b, frames in rows:
        data.append('%d,%du,%s,' % (len(frames), b, ','.join('%d,%du,%d,%d,%d' % f for f in frames)))
    stubs = {
        'sub_' + clock: 'ctx.r3.u32=NOW;',
        'sub_' + make: 'store(ctx.r5.u32,%du);store(%du+16,1);MK=1;' % (WRAP, WRAP),
        'sub_' + play: 'store(ctx.r3.u32+16,5);PL=1;',
        'sub_' + release: 'store(%du+1464,0);ST=1;' % THIS,
        'sub_' + dur: 'ctx.r3.u32=DUR;',
    }
    main_src = (
        'static const int32_t C[]={' + nl + nl.join(data) + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(size_t i=0;i<sizeof(C)/4;){int32_t n=C[i];uint32_t b=C[i+1];const int32_t*w=C+i+2;i+=2+5*n;'
        'memset(pointer(%du),0,0x800);memset(pointer(%du),0,0x40);store(%du+1228,%du);store(%du+152,b);NOW=100000;int on=0,prev=0;'
        % (THIS, WRAP, THIS, ONE, THIS) +
        'for(int32_t k=0;k<n;k++){int32_t mode=w[5*k],d=w[5*k+2],ev=w[5*k+3],adv=w[5*k+4];uint32_t dt=w[5*k+1];'
        'NOW+=adv;DUR=d;'
        'if(load(%du+1464)){if(ev==1)store(%du+1464,0);else if(ev==2)store(%du+16,3);}' % (THIS, THIS, WRAP) +
        '*pointer(%du+1950)=mode;if(mode&&!prev){store(%du+120,0x%xu);store(%du+124,0);}prev=mode;' % (THIS, THIS, int(machine, 16), THIS) +
        'if(load(%du+120)!=0x%xu){printf("-\\n");continue;}' % (THIS, int(machine, 16)) +
        'uint32_t s=load(%du+1464)?load(%du+16):0;uint32_t el=NOW-load(%du+1468);int32_t ds=load(%du+1472);'
        % (THIS, WRAP, THIS, THIS) +
        'store(0x%xu,dt);uint32_t bi=load(%du+152);MK=PL=ST=0;' % (step, THIS) +
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;sub_%s(ctx,nullptr);}' % (STACK, THIS, machine) +
        'int dn=load(%du+120)==0x%xu;' % (THIS, done_fn) +
        'printf("%%d %%u %%u %%d %%u %%u -> %%u %%d %%d %%d %%d\\n",mode,s,el,ds,dt,bi,load(%du+152),MK,PL,ST,dn);' % THIS +
        '}printf("=\\n");}}' + nl)
    exe = rd.build(idx, ['sub_' + machine], main_src, a.out_dir, 'revs_machine', stubs=stubs, image=rd.load_image(a.image))
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'revs_machine_ref.txt').write_text(out)
    lines = [x for x in out.split(nl) if x and x not in ('-', '=')]
    print(len(rows), 'sequences,', len(lines), 'updates,', sum(x.split()[1] == '5' for x in lines), 'while playing,',
          sum(x.split()[-1] == '1' for x in lines), 'ends')


if __name__ == '__main__':
    main()
