"""The Roar / EMP dust slots run offline: the original's own code, unchanged generated code on flat guest memory.

The eight slots are built by the slot constructor F-93019 (its model loader F-81760 a host stub: it only loads the shockwave
model once), then random sequences of
START the ability start's call F-85331 (event word, owner car or none, the light flag r7; f1 = 0.4, f2 = 3.0 as F-77604
      passes them) - with its dust start F-17154: the effect create F-78885 (prints the name, hands back an effect object),
      the placement F-11877 and the start F-80289 (print what they are given); the behaviour pool F-58802 hands back a
      fake behaviour (the physics push of the record, not compared);
TICK  the slot update F-22666 (the behaviour reaper F-92378 skipped: pool pointer 0) with each slot's F-98199 and its light
      F-76537; the light adder F-31349 prints position, direction, colour and strength;
RESET the slots' reset F-84687; FORGET the owner removal F-93284;
after a start or a reset every slot's words (+64 owner, +72 rate, +76, +80 flags, +84 life, +88 age, +68, +92 in use) and
the two colour globals are printed, after the others each slot's owner, age and in-use byte. The tick length is the clock object's +8 (0x3c888889, 1/60) or a random one stored there.
Built with -msse4.1 as the recomp itself is.
Usage: pu_dust_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure

START, DUST, CTOR, LOADER, SLOT_TICK, TICK, LIGHT = 'F-85331', 'F-17154', 'F-93019', 'F-81760', 'F-98199', 'F-22666', 'F-76537'
POOL_ADD, CREATE, PLACE, RUN, ADD_LIGHT, REAP = 'F-58802', 'F-78885', 'F-11877', 'F-80289', 'F-31349', 'F-92378'
RESET, FORGET = 'F-84687', 'F-93284'


def addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


# guest layout (scratch 0x1000..0x3000): slots 0x1000 + 0x70 k, their +96 objects 0x1380 + 0x70 k, a spare matrix 0x1700,
# the event record 0x1740, the fake behaviour 0x1760 (+4 -> 0x1780, +4 -> 0x17A0), two owners (the car objects the start
# passes, +48 -> +8 -> +128 -> +272 the velocity, +48 -> +28 -> +16 the matrix), the effect object 0x2400, TLS 0x2C00.
HEAD = r'''
static const uint32_t OWN[2][5]={{0x1800,0x1840,0x1860,0x1900,0x1950},{0x1A80,0x1AC0,0x1AE0,0x1B80,0x1BD0}};
static void fresh(Context&c){c=Context{};c.r1.u32=0x2FF0;c.r13.u32=0x2C00;}
static void dump(int full){
  printf(full?"ST":"SA");for(int k=0;k<8;k++){uint32_t s=load(0x8287E150+4*k);if(!s){printf(" -");continue;}
    uint32_t o=load(s+64);int oi=o==0?0:o==OWN[0][0]?1:o==OWN[1][0]?2:9;
    if(!full){printf(" %d:%08x:%u",oi,load(s+88),*pointer(s+92));continue;}
    printf(" %d:%08x:%08x:%08x:%08x:%08x:%08x:%u",oi,load(s+72),load(s+76),load(s+80),load(s+84),load(s+88),load(s+68),*pointer(s+92));}
  if(!full){printf("\n");return;}
  printf(" A");for(int i=0;i<4;i++)printf(" %08x",load(0x827E1B80+4*i));printf(" B");for(int i=0;i<4;i++)printf(" %08x",load(0x827E1B90+4*i));
  printf("\n");}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    n = {k: addr(a.tool, k) for k in (START, DUST, CTOR, LOADER, SLOT_TICK, TICK, LIGHT, POOL_ADD, CREATE, PLACE, RUN, ADD_LIGHT, REAP, RESET, FORGET)}
    idx = recomp_closure.index(a.recomp)
    rnd = random.Random(53)
    cases = []
    for c in range(40):
        nulls = [rnd.random() < 0.08 for _ in range(8)]
        ops = []
        pos = [[f32(rnd.uniform(-2000, 2000)), f32(rnd.uniform(-20, 120)), f32(rnd.uniform(-2000, 2000))] for _ in range(2)]
        for k in range(rnd.choice([40, 120, 450])):
            r = rnd.random()
            if r < 0.004:
                ops.append(('R', [list(p) for p in pos]))
            elif r < 0.012:
                ops.append(('F', rnd.choice([1, 2]), [list(p) for p in pos]))
            elif rnd.random() < (0.5 if k < 10 else 0.04):
                word = rnd.choice([1, 2, 1, 2, 0, 3])
                owner = rnd.choice([1, 2, 1, 2, 0])
                flags = rnd.choice([8, 8, 8, 0, 1, 9, 12])
                vel = [f32(rnd.uniform(-60, 60)) for _ in range(3)]
                eff = rnd.random() < 0.9
                ops.append(('S', word, owner, flags, vel, eff, [list(p) for p in pos]))
            else:
                dt = 0x3c888889 if rnd.random() < 0.8 else fb(rnd.choice([0.0, rnd.uniform(0, 0.1), 1 / 30]))
                for p in pos:
                    for i in range(3):
                        p[i] = f32(p[i] + rnd.uniform(-1.5, 1.5))
                ops.append(('T', dt, [list(p) for p in pos]))
        cases.append((nulls, ops))
    # ops as words: kind, then for S: word owner flags eff vel(3) pos(6); for T: dt pos(6)
    rows = []
    for nulls, ops in cases:
        w = [sum(1 << i for i, x in enumerate(nulls) if x), len(ops)]
        for op in ops:
            if op[0] == 'S':
                _, word, owner, flags, vel, eff, pos = op
                w += [1, word, owner, flags, int(eff)] + [fb(x) for x in vel] + [fb(x) for p in pos for x in p]
            elif op[0] == 'T':
                _, dt, pos = op
                w += [2, dt, 0, 0, 0, 0, 0, 0] + [fb(x) for p in pos for x in p]
            else:
                w += [3 if op[0] == 'R' else 4, 0 if op[0] == 'R' else op[1], 0, 0, 0, 0, 0, 0] + [fb(x) for p in op[-1] for x in p]
        rows.append(w)
    data = 'static const uint32_t D[]={' + ','.join('0x%x' % x for r in rows for x in r) + '};\nstatic const int NC=%d;\n' % len(rows)
    stubs = {
        n[LOADER]: '',
        n[POOL_ADD]: 'ctx.r3.u32=0x1760;',
        n[CREATE]: 'printf("EFF %s\\n",(const char*)pointer(ctx.r4.u32));ctx.r3.u32=load(0x2BF0);',
        n[PLACE]: 'printf("POS");for(int i=0;i<4;i++)printf(" %08x",load(ctx.r4.u32+4*i));printf(" VEL");'
                  'for(int i=0;i<4;i++)printf(" %08x",load(ctx.r3.u32+416+4*i));printf("\\n");',
        n[RUN]: 'printf("RUN\\n");',
        n[ADD_LIGHT]: 'auto bits=[](double d){float x=float(d);uint32_t u;memcpy(&u,&x,4);return u;};printf("L");'
                      'for(int i=0;i<4;i++)printf(" %08x",load(ctx.r4.u32+4*i));for(int i=0;i<3;i++)printf(" %08x",load(ctx.r5.u32+4*i));'
                      'for(int i=0;i<3;i++)printf(" %08x",load(ctx.r6.u32+4*i));printf(" %08x\\n",bits(ctx.f1.f64));',
        n[REAP]: 'throw std::runtime_error("reaper reached");',
    }
    body = data + r'''
int main(int argc,char**argv){
  std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
  const std::vector<uint8_t> clean=image;
  const uint32_t*p=D;
  for(int c=0;c<NC;c++){
    image=clean;memset(objects,0,sizeof(objects));
    store(0x2C00,0x2C10);store(0x8287E174,0);
    uint32_t nulls=*p++,nops=*p++;
    for(int k=0;k<8;k++){uint32_t s=0x1000+0x70*k;
      if(nulls>>k&1){store(0x8287E150+4*k,0);continue;}
      store(0x8287E150+4*k,s);store(s+96,0x1380+0x70*k);*pointer(s+92)=0;
      Context x;fresh(x);x.r3.u32=s;CTOR(x,nullptr);}
    for(int o=0;o<2;o++){store(OWN[o][0]+48,OWN[o][1]);store(OWN[o][1]+8,OWN[o][2]);store(OWN[o][1]+28,OWN[o][3]);store(OWN[o][2]+128,OWN[o][4]);
      for(int i=0;i<16;i++)store(OWN[o][3]+16+4*i,(i%5==0)?0x3f800000:0);}
    store(0x1760+4,0x1780);store(0x1780+4,0x17A0);
    printf("CASE %d\n",c);dump(1);
    for(uint32_t k=0;k<nops;k++){
      uint32_t kind=p[0];const uint32_t*q=p+1;p+=1+7+6;
      for(int o=0;o<2;o++){for(int i=0;i<3;i++)store(OWN[o][3]+16+48+4*i,q[7+3*o+i]);store(OWN[o][3]+16+60,0x3f800000);}
      if(kind==1){
        uint32_t word=q[0],owner=q[1],flags=q[2],eff=q[3];
        store(0x1740,word);
        if(owner){for(int i=0;i<3;i++)store(OWN[owner-1][4]+272+4*i,q[4+i]);store(OWN[owner-1][4]+284,0);}
        store(0x2BF0,eff?0x2400:0);
        Context x;fresh(x);x.r3.u32=0x1740;x.r4.u32=owner?OWN[owner-1][3]+16:0x1700;x.r7.u32=flags;x.r8.u32=owner?OWN[owner-1][0]:0;
        x.f1.f64=double(getf(0x8201D0CC));x.f2.f64=double(getf(0x820077F8));
        if(!owner)for(int i=0;i<16;i++)store(0x1700+4*i,(i%5==0)?0x3f800000:0);
        printf("START %u %u %u %u",word,owner,flags,eff);for(int i=4;i<13;i++)printf(" %08x",q[i]);printf("\n");
        START(x,nullptr);
        uint32_t r=x.r3.u32;printf("R %d\n",r?int((r-0x1000)/0x70):-1);
      }else if(kind==2){
        store(0x827D7508,q[0]);printf("TICK %08x",q[0]);for(int i=7;i<13;i++)printf(" %08x",q[i]);printf("\n");
        Context x;fresh(x);TICK(x,nullptr);
      }else if(kind==3){
        printf("RESET\n");Context x;fresh(x);RESET(x,nullptr);
      }else{
        printf("FORGET %u\n",q[0]);Context x;fresh(x);x.r3.u32=OWN[q[0]-1][0];FORGET(x,nullptr);
      }
      dump(kind==1||kind==3);
    }
  }
}
'''
    for k, v in (('START', START), ('CTOR', CTOR), ('TICK', TICK), ('RESET', RESET), ('FORGET', FORGET)):
        body = body.replace(k + '(', n[v] + '(')
    exe = recomp_closure.build(idx, [n[START], n[CTOR], n[TICK], n[RESET], n[FORGET]], HEAD + body, a.work, 'pu_dust', stubs=stubs, vector=True,
                               flags=('-msse4.1',))
    res = recomp_closure.run(exe, a.image)
    lines = [l for l in res.split('\n') if l]
    starts = sum(1 for l in lines if l.startswith('START'))
    ticks = sum(1 for l in lines if l.startswith('TICK'))
    lights = sum(1 for l in lines if l.startswith('L '))
    effs = sum(1 for l in lines if l.startswith('EFF'))
    head = '# tools/pu_dust_reference.py: %d cases, %d starts (%d effects), %d ticks, %d lights' % (len(cases), starts, effs, ticks, lights)
    # (words that would read as 82xxxxxx are split as 82xx_xxxx; the test strips the '_')
    out = [head]
    for l in lines:
        out.append(' '.join(w[:4] + '_' + w[4:] if len(w) == 8 and w.startswith('82') else w for w in l.split(' ')))
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text('\n'.join(out) + '\n')
    print(head)


if __name__ == '__main__':
    main()
