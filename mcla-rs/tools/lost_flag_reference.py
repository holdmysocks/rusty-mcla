"""Run the original's track timer and the brains' lost flag offline and record them for the port's `police::track_step`
(crates/game/src/testdata/lost_flag_ref.txt; test police::tests::lost_flag_matches_the_original).

The tactic object as tools/arrest_reference.py builds it (Pursuit ctor F-55508, reset F-20606, begin F-89627, 1 - 3 cops with a
brain each, the target a player; the same stubs), its vtable copied and replaced by one of four classes: the Pursuit, the Double
Pursuit, the Arrow Trap and the pull-over (their vt+28: the first three byte +29, the pull-over 0). Then, per update, the track
set F-79519 (f1 = the case's value) when the case asks for it, and the common update F-63286 with the case's dt (the original's
code for the countdown of +136 and the +773 writes; the world still: the target at rest, the cops 200 m away and unseen).

Cases: every class x +29 0 / 1 x 1 - 3 cops x the four dt of arrest_reference.py; set values fTrackTgtTime 25, -1, 0, 1, 20, a
whole number of steps, its f32 neighbours, a NaN, an infinity and random ones; a second set during the run (as SetChaseProperties
and the Search's re-sighting do) in half the cases.

  CASE <class 0 Pursuit / 1 Double / 2 Arrow / 3 pull-over> <+29> <cops> <dt bits> <sets: k:bits,...>
  <k> <+136 bits> <+773 mask (bit i = cop i)>        (lines where +136's sign class or the mask changed, every 100th and the last)
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp
import arrest_reference as A

P = A.P
IMG = A.IMG
CLASSES = [IMG + o for o in (0x0C8704, 0x0C81BC, 0x0C8514, 0x0C8B54)]
N = 900

MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
static uint8_t lb(uint32_t a){return (uint8_t)(load(a&~3u)>>((3-(a&3))*8));}
static const uint32_t M=0x40800000u, TGT=0x40A10000u, G=0x40A50000u, IN=0x40D00000u, VTM=0x40C30000u;
static uint32_t CAR(int i){return i<0?TGT:0x40B00000u+(uint32_t)i*0x10000u;}
static void mkcar(uint32_t c, uint32_t vt, bool brain, uint32_t tgt){
 store(c,vt); store(c+48,c+0x1000); store(c+0x1000+8,c+0x2000); store(c+0x1000+20,c+0x3000); store(c+0x1000+28,c+0x5000);
 store(c+0x3000+8,c+0x4000); sf(c+0x4000+24,100.0f); store(c+656,3u);
 if(brain){ uint32_t B=c+0x8000; store(c+876,B); store(B+4,c); store(B+680,tgt); sb(B+773,0); sf(B+596,0.0f);}
}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t AI=strtoul(argv[3],0,16), PL=strtoul(argv[4],0,16); std::string tok;
 static const uint32_t CV[4]={CLS0,CLS1,CLS2,CLS3};
 while(in>>tok){ if(tok!="case") break; int cls,p29,nc,ns; uint32_t dtb; in>>cls>>p29>>nc>>std::hex>>dtb>>std::dec>>ns;
  int sk[4]; uint32_t sv[4]; for(int i=0;i<ns;i++){ in>>sk[i]>>std::hex>>sv[i]>>std::dec; }
  rt_reset();
  sf(TUNE+192,30.0f); sf(TUNE+196,15.0f); sf(TUNE+212,1.5f); sf(TUNE+304,5.0f); sf(TUNE+360,1.0f); sf(TUNE+364,1.75f); sf(TUNE+368,10.0f); sf(TUNE+372,50.0f);
  store(MGRG,0x40C20000u);
  Context c=fresh(); c.r3.u32=M; CTOR(c,nullptr);
  c=fresh(); c.r3.u32=M; RESET(c,nullptr);
  mkcar(TGT,PL,false,0); store(TGT+656,3u);
  uint32_t arr=0x40C10000u; store(M+72,(uint32_t)nc); store(M+76,arr);
  for(int i=0;i<nc;i++){ mkcar(CAR(i),AI,true,TGT); store(arr+4*i,CAR(i)); store(CAR(i)+1076,1u);
   sf(CAR(i)+0x5000+64,200.0f+10.0f*i); sf(CAR(i)+0x5000+68,0.0f); sf(CAR(i)+0x5000+72,200.0f); sf(CAR(i)+0x5000+76,0.0f); }
  sf(TGT+0x5000+64,0.0f); sf(TGT+0x5000+68,0.0f); sf(TGT+0x5000+72,0.0f); sf(TGT+0x5000+76,0.0f);
  sf(TGT+0x2000+224,0.0f); sf(TGT+0x3000+36,10.0f);
  store(M+32,TGT); store(M+36,G); store(M+20,(load(M+20)&0x00ffffffu)|0x01000000u); sf(M+164,100.0f);
  c=fresh(); c.r3.u32=M; BEGIN(c,nullptr);
  for(int i=0;i<24;i++) store(VTM+4*i,load(CV[cls]+4*i));
  store(VTM+72,0x7F000010u); store(VTM+76,0x7F000011u); store(VTM+80,0x7F000010u); store(VTM+84,0x7F000011u); store(M,VTM);
  sb(M+29,(uint8_t)p29);
  store(IN,0); store(IN+4,0); store(IN+8,0); store(IN+12,0); sf(IN+16,50.0f);
  for(int i=0;i<nc;i++) sb(load(CAR(i)+876)+773,0);
  for(int k=0;k<NN;k++){
   for(int i=0;i<ns;i++) if(sk[i]==k){ c=fresh(); c.r3.u32=M; uint32_t u=sv[i]; float f; memcpy(&f,&u,4); c.f1.f64=(double)f; TRACK(c,nullptr); }
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)([](uint32_t u){float f;memcpy(&f,&u,4);return f;})(dtb); COMMON(c,nullptr);
   uint32_t m=0; for(int i=0;i<nc;i++) m|=(uint32_t)(lb(load(CAR(i)+876)+773)&1)<<i;
   printf("%d %08x %u\n",k,load(M+136),m); }
  printf("end\n");}
}
'''


def fb(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def cases():
    g = random.Random(79519)
    out = []
    for cls in range(4):
        for p29 in (0, 1):
            for nc in (1, 2, 3):
                for dtb in A.DT_BITS:
                    dt = struct.unpack('>f', struct.pack('>I', dtb))[0]
                    t, sums = 0.0, []
                    for _ in range(N):
                        t = f32(t + dt)
                        sums.append(t)
                    s = sums[g.randrange(5, 600)]
                    vals = [fb(25.0), fb(-1.0), fb(0.0), fb(1.0), fb(20.0), fb(s), fb(s) + 1, fb(s) - 1, 0x7FC00000, 0x7F800000,
                            fb(f32(g.uniform(0.0, 25.0)))]
                    for v in vals:
                        sets = [(0, v)]
                        if g.random() < 0.5:
                            sets.append((g.randrange(1, 700), g.choice([fb(1.0), fb(20.0), fb(-1.0), fb(f32(g.uniform(0.0, 5.0)))])))
                        out.append((cls, p29, nc, dtb, sets))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    a.more = []
    world = [f for f in P.WORLD if f not in ('F-36806', 'F-63286', 'F-41260', 'F-14415', 'F-29545')]
    ids = (A.MAIN_IDS + ['F-28114', 'F-36806', 'F-63286', 'F-74492', 'F-66453', 'F-41284', 'F-79519'] + P.NOCOMPILE + world + P.TARGETS
           + A.TARGETS + A.OWN_STUBS)
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    ai, pl = rdisp.vtables_of(img, 'mcAiRacer')[0][0], rdisp.vtables_of(img, 'mcPlayer')[0][0]
    stubs = A.stubs_for(name, a)
    fakes = {0x7F000010: 'ctx.r3.u64=0;', 0x7F000011: ''}
    vtabs = [(IMG + 0x0C8764, 16)] + [(v, [i for i in range(21) if i != 15]) for v, _ in P.VTABLES[1:]]
    vtabs += [(c, [i for i in range(18) if i != 15]) for c in CLASSES if all(c != v for v, _ in P.VTABLES)]
    tgts = [addr[f] for f in P.TARGETS + A.TARGETS + ['F-66453', 'F-41284']] + A.RAW_TARGETS + [rdisp.image_u32(img, c + 28) for c in CLASSES]
    src = (MAIN.replace('CTOR', name('F-55508')).replace('RESET', name('F-20606')).replace('BEGIN', name('F-89627'))
           .replace('TRACK(', name('F-79519') + '(').replace('COMMON(', name('F-63286') + '(').replace('TUNE', '0x%08Xu' % P.TUNE)
           .replace('MGRG', '0x%08Xu' % (IMG + 0x86D8E0)).replace('NN', str(N)))
    for i, c in enumerate(CLASSES):
        src = src.replace('CLS%d' % i, '0x%08Xu' % c)
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in A.MAIN_IDS] + [name('F-63286'), name('F-79519')], src, a.out,
                      'lost_flag', image=img, vtables=vtabs, targets=tgts, limit=900, stubs=stubs, fakes=fakes)
    cs = cases()
    assert len(cs) < 2000
    cf = a.out / 'lost_cases.txt'
    cf.write_text(''.join('case %d %d %d %x %d %s\n' % (cl, p, nc, d, len(s), ' '.join('%d %x' % kv for kv in s)) for cl, p, nc, d, s in cs)
                  + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf), '%08X' % ai, '%08X' % pl], capture_output=True, text=True, timeout=600)
    blocks = r.stdout.split('end\n')[:-1]
    if r.returncode or len(blocks) != len(cs):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    ref, n = [], 0
    for (cl, p, nc, d, s), blk in zip(cs, blocks):
        rows = [x for x in blk.split('\n') if x]
        ref.append('CASE %d %d %d %08x %s' % (cl, p, nc, d, ','.join('%d:%08x' % kv for kv in s)))
        prev = None
        for k, row in enumerate(rows):
            v = row.split()
            f = struct.unpack('>f', bytes.fromhex(v[1]))[0]
            key = (f > 0.0, f <= 0.0, v[2])
            if key != prev or k % 100 == 0 or k == len(rows) - 1:
                ref.append(row)
            prev = key
        n += len(rows)
    a.ref.write_text('\n'.join(ref) + '\n')
    print('cases', len(cs), 'updates', n, '->', a.ref)


if __name__ == '__main__':
    main()
