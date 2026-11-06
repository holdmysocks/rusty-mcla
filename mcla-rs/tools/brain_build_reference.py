"""Build the original's cop-brain state machine offline (static recompilation, native, flat guest memory) and show which of
its constructors' constants survive into the started machine (R21 test 2 for the brain's machine and state constructors).

The brain's attach F-43897 runs unchanged on a brain object: it allocates the 24-byte machine (ctor F-88578, vtable
820C8E84), stores it at +560 and calls its reset (slot 1, F-61449): +12 / +16 = -1, the builder (slot 7, F-84338: owner +20,
nine states - the base state ctor F-16738 with vtables 820C8EC4 / 8F3C / 8FBC / 903C / 90BC / 913C, Stopping F-91346,
AlignWithTarget F-54763, Search F-60172), each state's attach (slot 1, its machine at +4, slot 11 F-34717: +12 = +4), then
start (slot 3) with no state: each state's reset (slot 3: +8 = 0.0, then slot 13 - Search's F-97098, Stopping's and
AlignWithTarget's own). The racing brain's Clear F-76382 (attach's tail call) is stubbed: not this machine.

Two runs: as the game, and with the constructors F-88578, F-16738, F-91346, F-54763, F-60172 replaced by copies that store
A5A5A5A5 wherever the original stores a constant (vtables and base calls kept). Every word of the machine, its state table
and the nine states is printed after the attach; a word that differs between the two runs is a constructor constant that
survives the attach (and must then be shown dead or compared).

  machine <24 bytes as words>
  state <k> <vtable> <words>
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN_IDS = ['F-43897']
CTORS = ['F-88578', 'F-16738', 'F-91346', 'F-54763', 'F-60172']
OTHER = ['F-76382', 'F-51414', 'F-38433']
SIZES = [16, 16, 16, 20, 16, 16, 24, 352, 16]
MACHINE_VT = 0x820C8E84
STATE_VT = [0x820C8EC4, 0x820C8F3C, 0x820C8FBC, 0x820C9EA4, 0x820C903C, 0x820C90BC, 0x820C9D04, 0x820C9B34, 0x820C913C]

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 rt_reset(); const uint32_t B=0x40A00000u;
 Context c=fresh(); c.r3.u32=B; c.r4.u32=0; c.r5.u32=0; ATTACH(c,nullptr);
 uint32_t M=load(B+560); printf("machine"); for(int i=0;i<24;i+=4) printf(" %08x",i==0||i==4||i==20?0u:load(M+i)); printf("\n");
 static const int sz[9]={SIZES};
 uint32_t T=load(M+4); for(int k=0;k<(int)(load(M+8)>>16);k++){ uint32_t S=load(T+4*k); printf("state %d %08x",k,load(S));
  for(int i=4;i<sz[k];i+=4) printf(" %08x",i==4||i==12?(load(S+i)==M?1u:0xBADu):load(S+i)); printf("\n"); }
 printf("end\n");
}
'''
# (pointers are printed as 0 - the machine's own vtable / table / owner - or as 1 when a state's +4 / +12 is its machine)

POISON = 0xA5A5A5A5


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, help='write both runs here')
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = MAIN_IDS + CTORS + OTHER
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    vtabs = [(MACHINE_VT, [1, 3, 7, 9])] + [(v, [1, 3, 11, 13]) for v in STATE_VT]
    main_src = MAIN.replace('ATTACH', name('F-43897')).replace('SIZES', ','.join(map(str, SIZES)))
    p = '0x%08Xu' % POISON
    # the constructors as the original's, every constant store replaced by the marker (asm: docs/reference/hookmen/brain_build.md)
    poison = {
        name('F-88578'): 'uint32_t t=ctx.r3.u32; %s(ctx,base); store(t,0x820C8E84u); store(t+20,%s); ctx.r3.u32=t;' % (name('F-51414'), p),
        name('F-16738'): 'uint32_t t=ctx.r3.u32; %s(ctx,base); store(t,0x820C92F4u); store(t+12,%s); ctx.r3.u32=t;' % (name('F-38433'), p),
        name('F-91346'): 'uint32_t t=ctx.r3.u32; %s(ctx,base); store(t,0x820C9EA4u); store(t+16,%s); ctx.r3.u32=t;' % (name('F-16738'), p),
        name('F-54763'): ('uint32_t t=ctx.r3.u32; %s(ctx,base); store(t,0x820C9D04u); store(t+16,%s); store(t+20,(load(t+20)&0x00ffffffu)|0xA5000000u);'
                          ' ctx.r3.u32=t;') % (name('F-16738'), p),
        name('F-60172'): ('uint32_t t=ctx.r3.u32; %s(ctx,base); store(t,0x820C9B34u); store(t+344,%s); store(t+336,%s); '
                          'store(t+340,(load(t+340)&0x00ffffffu)|0xA5000000u); for(int i=0;i<10;i++){ store(t+32+32*i,%s); '
                          'for(int j=0;j<4;j++) store(t+16+32*i+4*j,%s);} ctx.r3.u32=t;') % (name('F-16738'), p, p, p, p),
    }
    outs = []
    for tag, stubs in (('brain_build', {}), ('brain_build_poison', poison)):
        st = {name('F-76382'): ''}
        st.update(stubs)
        exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in MAIN_IDS + ['F-51414', 'F-38433']], main_src, a.out, tag, image=img, vtables=vtabs, limit=800,
                          stubs=st)
        r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
        if r.returncode:
            sys.exit(r.stdout[-500:] + r.stderr[-1500:])
        outs.append(r.stdout.splitlines())
    words = diff = 0
    for x, y in zip(*outs):
        fx, fy = x.split(), y.split()
        for i, (u, v) in enumerate(zip(fx, fy)):
            words += 1
            if u != v:
                diff += 1
                print('differs:', fx[:2], 'word', i, u, v)
    print('words', words, 'differing', diff)
    if a.ref:
        a.ref.write_text('\n'.join(['# as the game'] + outs[0] + ['# constructors poisoned'] + outs[1]) + '\n')


if __name__ == '__main__':
    main()
