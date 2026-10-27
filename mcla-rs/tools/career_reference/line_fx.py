import random,struct
from fx import *
MAIN=r'''
static uint32_t bits(float v){ uint32_t b; std::memcpy(&b,&v,4); return b; }
int main(int argc,char **argv) {
    std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
    if(image.size()!=0x9e0000)return 3;
    std::ifstream cases(argv[2]);
    std::puts("ax,ay,az,bx,by,bz,half_length_bits,cx_bits,cy_bits,cz_bits");
    float v[6];
    while(cases>>v[0]>>v[1]>>v[2]>>v[3]>>v[4]>>v[5]) {
        std::memset(objects,0,sizeof(objects)); Context ctx{}; ctx.r1.u32=0x8000;
        const uint32_t L=0x2000;                       // the trigger line object: A at +64, B at +80 (mcTriggerLine)
        for(int i=0;i<3;i++){ putf(L+64+4*i,v[i]); putf(L+80+4*i,v[3+i]); }
        // the constant at 0x827D84B0 is in the image
        ctx.r3.u32=L; @F-21032@(ctx,nullptr); uint32_t hl=bits(float(ctx.f1.f64));
        Context c2{}; c2.r1.u32=0x8000; c2.r3.u32=0x3000; c2.r4.u32=L; @F-73482@(c2,nullptr);
        std::printf("%.9g,%.9g,%.9g,%.9g,%.9g,%.9g,%u,%u,%u,%u\n",v[0],v[1],v[2],v[3],v[4],v[5],hl,bits(getf(0x3000)),bits(getf(0x3004)),bits(getf(0x3008)));
    }
}
'''
rng=random.Random(8268)
rows=[]
for _ in range(30000):
    sc=rng.choice([1,10,100,1000,5000])
    rows.append([float(struct.unpack('<f',struct.pack('<f',rng.uniform(-sc,sc)))[0]) for _ in range(6)])
text=''.join(' '.join(repr(x) for x in r)+'\n' for r in rows)
build('line',['@F-21032@','@F-73482@'],MAIN,text,'out',scratch=0xa000)
