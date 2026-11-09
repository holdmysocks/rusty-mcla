"""The cameras' ball at a point F-87651 run whole offline on the original's code: its query object's ctor F-81308, the set-up
F-38096 (the record count's limits, the material mask 0), the sphere driver F-49353 with everything below it (the culler,
the level walk over one root node, the frame step, the dispatch, the commit, F-72810) - level_query_reference.py's
machinery and level (the city's grid, prop composites, walker capsules), called as the cameras call it: r3 the level,
r4 the centre, f1 the radius, r6 the records, r7 the skipped object, r8 the include mask, r9 the type mask, r10 the states,
the caller's sp+84 the record count and sp+92 the excluded types.

A case: the centre, the radius, the skipped object (an object's index, or a foreign word), include, types, states, the
record count (signed), the excluded types; then the objects as level_query_reference.py writes them.
Out: the answer (r3), the query object's list (+36, +44 .. +84) and the records the count allows (at most 64).
Writes <out-dir>/camera_ball_ref.txt. Test `camprobe::tests::camera_ball_matches_the_original`."""
import argparse
import random
import re
import struct
from pathlib import Path
import level_query_reference as lq  # (patches recomp_dispatch's shim and g++ as that harness needs)

rd = lq.rd


def _shim():
    # (level_query_reference's shim, without its second copy of VectorMaskR when the shared shim already has one)
    s = lq._shim()
    first = s.find('VectorMaskR[]')
    last = s.rfind('alignas(16) static uint8_t VectorMaskR[] = {')
    return s[:last] if last > first >= 0 else s


rd.vector_shim = _shim
MAIN = r'''
static const uint32_t LV=0x40D00000, ENT=0x40D00100, NODE=0x40D00200, INST=0x40D01000, ARCH=0x40D02000, WB=0x40D03000,
 VTP=VTP_, VTC=VTC_, BLK=0x40D05000, CASTO=0x40D06000, STK=0x40DF0000, RECS=0x40D08000;
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static std::vector<uint8_t> blobp, blobc;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);} put(0x8286095C,0); put(0x8286093C,0x3F800000);
 for(int k=0;k<4;k++) put(0x828609E0+4*k,0x7F800000);
 put(LEVEL_GLOBAL_, LV);
 {std::ifstream b(argv[3],std::ios::binary); blobp.assign(std::istreambuf_iterator<char>(b),{});}
 {std::ifstream b(argv[4],std::ios::binary); blobc.assign(std::istreambuf_iterator<char>(b),{});}
 memcpy(pointer(RESP_),blobp.data(),blobp.size()); memcpy(pointer(RESC_),blobc.data(),blobc.size());
 uint32_t GRID=strtoul(argv[5],nullptr,16);
 FILE*f=fopen(argv[2],"r"); unsigned h[12];
 while(fscanf(f,"%x",&h[0])==1){
  for(int i=1;i<11;i++) if(fscanf(f,"%x",&h[i])!=1) return 1;   // (11 header words)
  unsigned n; if(fscanf(f,"%x",&n)!=1) return 1;
  if(n>4096){fprintf(stderr,"bad case file: %u objects\n",n); return 2;}
  std::vector<std::vector<unsigned>> ob(n,std::vector<unsigned>(18));
  for(auto&o:ob) for(int i=0;i<18;i++) if(fscanf(f,"%x",&o[i])!=1) return 1;
  rt_reset();
  for(uint32_t a=LV;a<CASTO+0x100;a+=4) store(a,0);
  for(uint32_t a=STK-0x20000;a<STK+0x100;a+=4) store(a,0);
  store(VTP+48,FAKE_MAT_); store(VTC+48,MATERIAL_);
  store(CASTO+16,8);
  store(LV+112,ENT); store(LV+132,NODE);
  store(NODE+0,0x4B189680); *pointer(NODE+4)=7; *pointer(NODE+24)=0; *pointer(NODE+25)=n?1:0; store16(NODE+26,0);
  for(int k=0;k<8;k++) store16(NODE+6+2*k,0xFFFF);
  for(unsigned i=0;i<n;i++){ auto&o=ob[i]; uint32_t in=INST+0x100*i, ar=ARCH+0x40*i;
   store16(ENT+8*i,i+1<n?i+1:0xFFFF); store(ENT+8*i+4,in|(o[3]&3));
   store(in+4,ar); for(int j=0;j<12;j++) store(in+16+(j/3)*16+(j%3)*4,o[6+j]);
   for(int j=0;j<3;j++) store(in+16+16*j+12,0); store(in+16+48+12,0x3F800000);
   uint32_t b;
   if(o[0]==0) b=GRID; else if(o[0]==1) b=o[1]; else { b=WB+0x100*i; float R,L; memcpy(&R,&o[1],4); memcpy(&L,&o[2],4);
    float br=L*0.5f+R, hl=L*0.5f+R; uint32_t w;
    store(b,VTP); *pointer(b+4)=1; memcpy(&w,&br,4); store(b+8,w);
    float mx[3]={R,hl,R}, mn[3]={-R,-hl,-R};
    for(int j=0;j<3;j++){ memcpy(&w,&mx[j],4); store(b+16+4*j,w); memcpy(&w,&mn[j],4); store(b+32+4*j,w); store(b+48+4*j,0);}
    for(int j=0;j<4;j++){ store(b+128+4*j,o[1]); store(b+144+4*j,o[2]); } }
   store(ar+12,b); store(ar+16,o[4]); store(ar+20,o[5]); }
  for(int j=0;j<3;j++) store(BLK+4*j,h[j]); store(BLK+12,0);
  for(uint32_t a=RECS;a<RECS+96*64;a+=4) store(a,0xDEADBEEF);
  store(STK+84,h[8]); store(STK+92,h[9]);
  int cnt=(int)h[8]; int nrec = (cnt==-1||cnt==0xFFFF)?1:(cnt<0?0:(cnt>64?64:cnt));
  Context c=fresh(); c.r1.u64=STK; c.r3.u64=LV; c.r4.u64=BLK; c.r6.u64=RECS;
  c.r7.u64 = h[4]<0x1000? INST+0x100*h[4] : h[4]; c.r8.u64=h[5]; c.r9.u64=h[6]; c.r10.u64=h[7];
  { float r; memcpy(&r,&h[3],4); c.f1.f64=r; }
  try{ sub_CAM_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  uint32_t q=STK-19072+96;
  printf("%08x",(uint32_t)c.r3.u64);
  for(int o2: {36,44,48,52,64,68,72,76,80,84}) printf(" %08x",load(q+o2));
  for(int k=0;k<nrec;k++){ uint32_t r=RECS+96*k;
   for(int o2: {0,16,20,24,32,36,40,48,52,56,64,68,72}) printf(" %08x",load(r+o2));
   printf(" %04x %04x %02x %04x",load16(r+76),load16(r+78),*pointer(r+80),load16(r+82)); }
  printf("\n"); } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--props', default='D:/MCLA/extracted/cache/resources/prop/0x0e66973c.r131')
    ap.add_argument('--city', default='D:/MCLA/extracted/cache/resources/city/sc/0x3e5fd91d.r1')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    ap.add_argument('--seed', type=int, default=87651)
    a = ap.parse_args()
    fb, f32 = lq.fb, lq.f32
    blobp, types = lq.load_props(a.props)
    blobc, grid, walls = lq.load_city(a.city)
    rnd = random.Random(a.seed)
    anyt = [i for i, t in enumerate(types) if t]
    cases = []
    while len(cases) < a.cases:
        city = rnd.random() < 0.5
        anchor = [w + rnd.gauss(0, 0.8) for w in rnd.choice(walls)] if city else [rnd.uniform(-2000, 2000), rnd.uniform(-5, 60), rnd.uniform(-2000, 2000)]
        objs = []
        if city or rnd.random() < 0.2:
            objs.append([0, 0, 0, 0, 0x900, 0xFFFFFFFF] + [fb(v) for v in (1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0)])
        for _ in range(rnd.choice([0, 1, 1, 2, 3])):
            ti = rnd.choice(anyt)
            b, cen, rad = types[ti]
            rot = lq.rotation(rnd, rnd.random() < 0.7)
            near = [anchor[j] + rnd.gauss(0, 0.6 * rad + 0.5) for j in range(3)]
            place = [near[j] - sum(cen[k] * rot[k][j] for k in range(3)) for j in range(3)]
            # (a prop bound standing in for a car: the cameras' include masks select the cars' type words)
            ty = rnd.choice([2, 0x185, 0x189, 0x1100, 0x185])
            objs.append([1, b, ti, 0, ty, 0xFFFFFFFF] + [fb(f32(v)) for row in rot + [place] for v in row])
        for _ in range(rnd.choice([0, 0, 1])):
            R, L = f32(rnd.uniform(0.2, 0.5)), f32(rnd.uniform(0.6, 1.4))
            rot = lq.rotation(rnd, True)
            place = [anchor[j] + rnd.gauss(0, 0.8) for j in range(3)]
            objs.append([2, fb(R), fb(L), 0, rnd.choice([0x20, 0x185]), 0xFFFFFFFF] + [fb(f32(v)) for row in rot + [place] for v in row])
        if not objs:
            continue
        rnd.shuffle(objs)
        for o in objs:
            o[3] = rnd.choice([0, 0, 0, 1, 2, 3])
            if rnd.random() < 0.05:
                o[5] = rnd.choice([0, 0x10, 0x100])
        R = f32(rnd.choice([0.3, 0.5, 1.0, rnd.uniform(0.05, 2.5)]))
        c = [f32(anchor[j] + rnd.gauss(0, 0.5)) for j in range(3)]
        skip = rnd.randrange(len(objs)) if rnd.random() < 0.5 else rnd.choice([0, 0x5555])
        include = rnd.choice([0x100, 0x100, 0x300, 0x300, 0xFFFFFFFF, 0x900, 0x2])
        tmask = rnd.choice([0xFFFFFFFF] * 6 + [0x100, 0])
        states = rnd.choice([7] * 6 + [4, 3, 1])
        cnt = rnd.choice([10, 10, 1, 1, 12, 0, -1, 0xFFFF, -5, 70, 3])
        xt = rnd.choice([0] * 6 + [0x1000, 0x100, 0x80])
        head = [fb(v) for v in c] + [fb(R), skip, include, tmask, states, cnt & 0xFFFFFFFF, xt, 0]
        cases.append((head, objs))
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'props.bin').write_bytes(blobp)
    (work / 'city.bin').write_bytes(blobc)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%x' % w for w in h) + ' %x ' % len(o) + ' '.join(' '.join('%x' % w for w in x) for x in o) + '\n'
                           for h, o in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    cam = lq.addr(a.tool, 'F-87651')
    cast = lq.addr(a.tool, 'F-75805')
    material = int(lq.addr(a.tool, 'F-10817'), 16)
    main_src = (MAIN.replace('RESP_', '0x%08Xu' % lq.RES_P).replace('RESC_', '0x%08Xu' % lq.RES_C).replace('VTP_', '0x%08Xu' % lq.VT_P)
                .replace('VTC_', '0x%08Xu' % lq.VT_C).replace('FAKE_MAT_', '0x%08Xu' % lq.FAKE_MAT).replace('MATERIAL_', '0x%08Xu' % material)
                .replace('LEVEL_GLOBAL_', '0x828DA464u').replace('CAM_', cam))
    exe = rd.build(idx, ['sub_' + cam], main_src, work, 'camball', image=img,
                   stubs={'sub_' + cast: 'ctx.r3.u64=0x40D06000;'},
                   targets=[material, int(lq.addr(a.tool, 'F-19966'), 16)], fakes={lq.FAKE_MAT: 'ctx.r3.u64=(ctx.r4.u64+1)<<8;'}, limit=1600)
    r = lq._run([str(exe), str(a.image), str(inp), str(work / 'props.bin'), str(work / 'city.bin'), '%x' % grid], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    lines = [' '.join('%x' % w for w in h) + ' | ' + ' | '.join(' '.join('%x' % w for w in x) for x in objs) + ' -> ' + o.strip()
             for (h, objs), o in zip(cases, out)]
    nl = chr(10)
    (a.out_dir / 'camera_ball_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    hit = sum(1 for o in out if len(o.split()) > 11 and o.split()[11] != '00000000')
    print(len(cases), 'cases;', hit, 'with the first record set')


if __name__ == '__main__':
    main()
