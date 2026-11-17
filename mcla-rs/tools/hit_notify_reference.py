"""The vehicle-hit notify of the racers' drivers run offline (the generated code whole and unchanged): the contact walker
F-83713 calls slot 40 (vt+160) of both cars' drivers, `hit(byte aggressor, speed, other car)`. Each case dispatches that slot
through a real driver vtable of the image (mcOpponent, mcAiRacer, mcPolice, mcPlayer):

- mcOpponent's slot 40 is F-30284: on byte 0 (the victim), when its brain (+68) answers IsA(mcBrainRacing) (brain vt+0,
  the class ids in the image's data) and the other car's driver ([[other+80]+128]) answers slot 10 (vt+40) true, the brain's
  collision timer +296 = 1.5 (F-37791); then the base F-33876 (stubbed here, counted).
- mcAiRacer / mcPolice / mcPlayer's slot 40 is F-33876 itself.

Brains: none, mcBrainRacing, mcBrainCruising (derives from mcBrainRacing), mcBrainAnimated. The other car's driver: each of
the four classes. The class-id words the IsA functions compare are given distinct values (they are run-time data).

Writes <ref>: "case byte victim brain other -> timer_hex base_calls"."""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

DRIVERS = {'mcOpponent': 0, 'mcAiRacer': 1, 'mcPolice': 2, 'mcPlayer': 3}
BRAINS = ['mcBrainRacing', 'mcBrainCruising', 'mcBrainAnimated']


def isa_ids(img, vts):
    """The class-id words the brains' IsA chains (vt+0) compare: each IsA opens `lis r11,hi; lwz r11,lo(r11)` and, when not
    the base, ends at +24 in `b <the base class's IsA>`."""
    ids = []
    for vt in vts:
        f = rd.image_u32(img, vt)
        while True:
            i0, i1 = rd.image_u32(img, f), rd.image_u32(img, f + 4)
            assert i0 >> 26 == 15 and i1 >> 26 == 32, hex(f)
            a = ((i0 & 0xFFFF) << 16) + ((i1 & 0xFFFF) ^ 0x8000) - 0x8000 & 0xFFFFFFFF
            if a not in ids:
                ids.append(a)
            b = rd.image_u32(img, f + 24)
            if b >> 26 != 18:
                break
            f = f + 24 + (((b & 0x3FFFFFC) ^ 0x2000000) - 0x2000000)
    return ids

B = 0x40100000

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});
static const uint32_t DV[4]={DVTS};static const uint32_t BV[3]={BVTS};static const uint32_t ID[NID]={IDS};
static const uint32_t BY[4]={0u,1u,0x100u,0x101u};
for(int i=0;i<NID;i++)store(ID[i],0x51000000u+i);
int n=0;
for(int by=0;by<4;by++)for(int v=0;v<4;v++)for(int br=0;br<4;br++)for(int o=0;o<4;o++){
 rt_reset();for(uint32_t a=B;a<B+0x8000;a+=4)store(a,0);
 uint32_t drv=B,brain=B+0x400,other=B+0x800,odrv=B+0xC00,ocar=B+0x1000;
 store(drv,DV[v]);store(drv+68,br?brain:0);if(br)store(brain,BV[br-1]);store(brain+296,0xDEADBEEFu);
 store(other+80,ocar);store(ocar+128,odrv);store(odrv,DV[o]);
 Context c=fresh();c.r3.u32=drv;c.r4.u32=BY[by];c.r5.u32=other;c.f1.f64=7.5;
 indirect(c,nullptr,load(DV[v]+160));
 printf("case %d %u %d %d %d -> %08x %d\n",n++,BY[by],v,br,o,load(brain+296),load(0x40104000u));}}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    r = subprocess.run([a.tool, 'registry', 'addr', 'F-30284', 'F-33876'], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    img = rd.load_image(a.image)
    dv = [rd.vtables_of(img, k)[0][0] for k in DRIVERS]
    bv = [rd.vtables_of(img, k)[0][0] for k in BRAINS]
    ids = isa_ids(img, bv)
    print('class ids', [hex(i) for i in ids])
    slot40 = [rd.image_u32(img, v + 160) for v in dv]
    assert slot40 == [addr['F-30284']] + [addr['F-33876']] * 3, [hex(s) for s in slot40]
    src = (MAIN.replace('DVTS', ','.join('0x%08Xu' % v for v in dv)).replace('BVTS', ','.join('0x%08Xu' % v for v in bv))
           .replace('IDS', ','.join('0x%08Xu' % v for v in ids)).replace('NID', str(len(ids))))
    src = 'static const uint32_t B=0x%08Xu;\n' % B + src
    base = 'sub_%08X' % addr['F-33876']
    exe = rd.build(rd.index(a.recomp), ['sub_%08X' % addr['F-30284']], src, a.out, 'hit_notify', image=img,
                   vtables=[(v, [10, 40]) for v in dv] + [(v, [0]) for v in bv], stubs={base: 'store(0x40104000u,load(0x40104000u)+1);'}, limit=40)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, timeout=300)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.splitlines()
    assert len(lines) == 256, len(lines)
    a.ref.write_text('\n'.join(lines) + '\n')
    print(len(lines), 'cases ->', a.ref)


if __name__ == '__main__':
    main()
