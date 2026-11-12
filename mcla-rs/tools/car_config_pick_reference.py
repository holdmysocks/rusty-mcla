"""mcCharacterConfigData_GetCarConfigDataByVehicleName (F-46809) as the original runs it, offline: which of a character's
cars the original gives an opponent for the car drawn, the player's performance P and the difficulty d.

The character's car map (+36) and the sample map (the global atMap the sample lookup F-39251 reads) are filled with the
original's own insert F-14328 (find F-18909 first, as their loaders do; keys from the name hash F-58929), the vehicle table
(the global hash table F-52155 reads: count, buckets, nodes {key, -, record, next}) by hand with one bucket, the career's
scale and steps (+8468 .. +8492 of the object at the career global, read by F-83091 / F-56889) set to the port's constants
(DIFF_STEPS, scale 1). The player's performance F-92180 is stubbed to return P (r5 non-zero; P kept at 0x4030C000); the log F-47121 and the
difficulty name F-46414 are stubbed empty.

Each case: 1 .. 8 cars of a 40-name pool, each with or without a vehicle record (class 0 .. 9), all with samples (every car
of Opponent.lst has samples; a car without one is not a case: F-83091 would read its record at 0); the drawn name one of
them or another; the class mask 255, -1 or random; d -1 .. 5; P 0.2 .. 1.2; the flag (r8) 0 or 1.
Writes <out-dir>/car_config_pick_ref.txt: "case" P d mask flag drawn n {name class s0 s50 s100}... "->" the index of the car
returned (map order of the insert, -1 for none) and each car's +76 word after the call (decimal, -1 = 0xffffffff)."""
import argparse
import random
import re
import struct
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr

POOL = ['vp_pick_car_%02d' % i for i in range(40)]
STEPS = [1.0, -0.15, 0.025, 0.15, 0.3, 0.0, 0.45]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def case(rnd):
    n = rnd.randrange(1, 9)
    names = rnd.sample(POOL, n)
    cars = []
    for nm in names:
        cls = rnd.randrange(10) if rnd.random() < 0.85 else -1          # -1: no vehicle record
        a = rnd.uniform(0.2, 1.1)
        b = a + rnd.uniform(0.0, 0.3)
        c = b + rnd.uniform(0.0, 0.3)
        if rnd.random() < 0.05:
            b = a                                                     # equal neighbours (not in the disc's data)
        if rnd.random() < 0.05:
            c = b
        cars.append((nm, cls, fb(a), fb(b), fb(c)))
    drawn = rnd.choice(names) if rnd.random() < 0.35 else rnd.choice(POOL)
    mask = rnd.choice((255, 255, 255, -1, rnd.getrandbits(10)))
    return (fb(rnd.uniform(0.2, 1.2)), rnd.randrange(-1, 6), mask, rnd.randrange(2), drawn, cars)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    pick, ins, find, hsh, perf, log, dname = (addr(a.tool, f) for f in ('F-46809', 'F-14328', 'F-18909', 'F-58929', 'F-92180', 'F-47121', 'F-46414'))
    rnd = random.Random(4680)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    # (the cases as words: P d mask flag drawn-index n, then per car pool-index class s0 s50 s100)
    flat = []
    for p, d, mask, flag, drawn, cars in cases:
        flat += [p, d & 0xffffffff, mask & 0xffffffff, flag, POOL.index(drawn), len(cars)]
        for nm, cls, s0, s1, s2 in cars:
            flat += [POOL.index(nm), cls & 0xffffffff, s0, s1, s2]
    pool = ','.join('"%s"' % s for s in POOL)
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl +
           'static const char*POOL[]={%s};' % pool + nl + r'''
static void put_str(uint32_t a,const char*s){for(;;a++,s++){*pointer(a)=(uint8_t)*s;if(!*s)break;}}
static uint32_t hash_of(uint32_t s){Context x=fresh();x.r3.u32=s;x.r4.u32=0;sub_@HASH@(x,nullptr);return x.r3.u32;}
static void map_insert(uint32_t m,uint32_t key,uint32_t val){
 store(0x40300100,key);store(0x40300104,val);
 Context x=fresh();x.r3.u32=m;x.r4.u32=0x40300100;sub_@FIND@(x,nullptr);
 if(x.r3.u32&&load(x.r3.u32))return;
 Context y=fresh();y.r3.u32=m;y.r4.u32=0x40300100;y.r5.u32=0x40300104;sub_@INS@(y,nullptr);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
const uint32_t*s=CS;
const uint32_t CHAR=0x40300000,CAREER=0x40302000,CFG=0x40305000,REC=0x40306000,VREC=0x40307000,VNODE=0x40308000,VBUCK=0x40309000,DRAWN=0x4030A000,SETUP=0x4030B000;
for(int c=0;c<@N@;c++){rt_reset();
 uint32_t P=*s++;int32_t d=(int32_t)*s++;int32_t mask=(int32_t)*s++;uint32_t flag=*s++;uint32_t dr=*s++;uint32_t n=*s++;
 for(uint32_t o=0;o<0x10000;o+=4)store(0x40300000+o,0);
 for(uint32_t o=0;o<0x10;o+=4){store(0x828CD064+o,0);store(0x828CD0FC+o,0);}
 *pointer(0x828CD0FC+11)=1;*pointer(CHAR+36+11)=1;
 store(0x8286D8DC,CAREER);
 const uint32_t st[7]={@STEPS@};for(int i=0;i<7;i++)store(CAREER+8468+4*i,st[i]);
 // (the vehicle table: one bucket)
 store(0x828CD064,1);store(0x828CD064+8,VBUCK);
 put_str(DRAWN,POOL[dr]);store(SETUP,1);
 printf("case %08x %d %d %u %s %u",P,d,mask,flag,POOL[dr],n);
 uint32_t vn=0;
 for(uint32_t i=0;i<n;i++){uint32_t pi=*s++;int32_t cls=(int32_t)*s++;uint32_t s0=*s++,s1=*s++,s2=*s++;
  printf(" %s %d %08x %08x %08x",POOL[pi],cls,s0,s1,s2);
  uint32_t cfg=CFG+0x60*i,rec=REC+0x40*i;
  put_str(cfg,POOL[pi]);uint32_t h=hash_of(cfg);store(cfg+64,h);store(cfg+76,0);
  map_insert(CHAR+36,h,cfg);
  // (the sample record: +0 hash, slots {mod, performance, speed} at +4 / +16 / +28, +40 name)
  store(rec,h);store(rec+4,0);store(rec+8,s0);store(rec+16,50);store(rec+20,s1);store(rec+28,100);store(rec+32,s2);put_str(rec+40,POOL[pi]);
  map_insert(0x828CD0FC,h,rec);
  if(cls>=0){uint32_t vr=VREC+0x20*i,nd=VNODE+16*vn;store(vr+24,(uint32_t)cls);
   store(nd,h);store(nd+8,vr);store(nd+12,load(VBUCK));store(VBUCK,nd);vn++;}}
 printf(" ->");
 store(0x4030C000,P);
 Context x=fresh();x.r3.u32=CHAR;x.r4.u32=DRAWN;x.r5.u32=SETUP;x.r6.s32=d;x.r7.s32=mask;x.r8.u32=flag;
 sub_@PICK@(x,nullptr);
 int r=-1;for(uint32_t i=0;i<n;i++)if(x.r3.u32==CFG+0x60*i)r=(int)i;
 if(x.r3.u32&&r<0)r=-2;
 printf(" %d",r);for(uint32_t i=0;i<n;i++)printf(" %d",(int32_t)load(CFG+0x60*i+76));
 printf("\n");}}
''')
    for kk, v in (('@N@', str(a.cases)), ('@HASH@', hsh), ('@FIND@', find), ('@INS@', ins), ('@PICK@', pick),
                  ('@STEPS@', ','.join('%du' % fb(x) for x in STEPS))):
        src = src.replace(kk, v)
    stubs = {'sub_' + perf: '{uint32_t b=load(0x4030C000);float f;memcpy(&f,&b,4);ctx.f1.f64=f;}', 'sub_' + log: '', 'sub_' + dname: 'ctx.r3.u32=0;'}
    exe = rd.build(rd.index(a.recomp), ['sub_' + f for f in (pick, ins, find, hsh)], src, a.out_dir, 'carcfg',
                   image=rd.load_image(a.image), stubs=stubs)
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'car_config_pick_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
