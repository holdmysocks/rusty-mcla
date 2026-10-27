import random,struct
from fx import *
MAIN=r'''
static const float FRAC[13]={0.0f,0.04f,0.1f,0.17f,0.25f,0.34f,0.43f,0.52f,0.6f,0.68f,0.76f,0.84f,1.0f};
static void setup() {
    std::memset(objects,0,sizeof(objects));
    store(0x8287484C,0x1000);                 // global: the experience manager
    const uint32_t M=0x1000, S=0x1400;
    store(M+48,S);                            // the career system (Progression)
    putf(M+92,100.f); putf(M+96,15.f);
    float de[4]={-1.f,0.f,1.f,3.f}; for(int i=0;i<4;i++) putf(M+492+4*i,de[i]);
    for(int i=0;i<13;i++) putf(M+100+4*i,FRAC[i]);
    putf(M+512,250.f); putf(M+516,3000.f);
    float dm[4]={0.8f,1.0f,1.2f,1.4f}, cm[4]={2.0f,1.5f,1.2f,1.0f};
    for(int i=0;i<4;i++){ putf(M+520+4*i,dm[i]); putf(M+536+4*i,cm[i]); }
    // the constructor's loop: threshold[i] = @F-90390@(M, level fraction[i])
    for(int i=0;i<13;i++){ Context c{}; c.r1.u32=0x8000; c.r3.u32=M; c.f1.f64=double(FRAC[i]); @F-90390@(c,nullptr); putf(M+152+4*i,float(c.f1.f64)); }
}
static uint32_t bits(float v){ uint32_t b; std::memcpy(&b,&v,4); return b; }
int main(int argc,char **argv) {
    std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
    if(image.size()!=0x9e0000)return 3;
    std::ifstream cases(argv[2]);
    std::puts("kind,a,b,c,d,out_bits,out_int");
    int k; double a,b,c,d;
    while(cases>>k>>a>>b>>c>>d) {
        setup(); Context ctx{}; ctx.r1.u32=0x8000; const uint32_t M=0x1000, S=0x1400;
        uint32_t ob=0; int oi=0;
        if(k==0){ ctx.r3.u32=M; ctx.f1.f64=double(float(a)); @F-90390@(ctx,nullptr); ob=bits(float(ctx.f1.f64)); }
        else if(k==1||k==2){ store(S+4,0); store(S+8,int(a)); store(S+12,int(b));
            if(k==1){ ctx.r3.u32=M; ctx.r4.u32=int(a); @F-62421@(ctx,nullptr); oi=int(ctx.r3.u32); }
            else { ctx.r3.u32=S; @F-14176@(ctx,nullptr); ob=bits(float(ctx.f1.f64)); } }
        else if(k==3){ store(S+4,0); store(S+8,int(a)); store(S+12,33500); ctx.r3.u32=M; ctx.r5.u32=int(b); @F-31900@(ctx,nullptr); ob=bits(float(ctx.f1.f64)); }
        else if(k==4){ store(S+4,0); store(S+8,int(a)); store(S+12,33500); ctx.r3.u32=M; ctx.r4.u32=int(b); ctx.r5.u32=int(c); ctx.r6.u32=int(d); @F-78976@(ctx,nullptr); ob=bits(float(ctx.f1.f64)); }
        std::printf("%d,%.9g,%.9g,%.9g,%.9g,%u,%d\n",k,a,b,c,d,ob,oi);
    }
}
'''
rng=random.Random(826)
cs=[]
fs=[i/1000 for i in range(0,1101)]+[rng.uniform(-0.1,1.3) for _ in range(2000)]
for f in fs: cs.append((0,f,0,0,0))
amts=list(range(0,34000))+[33499,33500,33501,119080,200000]
for a in amts: cs.append((1,a,33500,0,0)); cs.append((2,a,33500,0,0))
for a in rng.sample(range(0,34000),3000): cs.append((2,a,rng.choice([100,1000,33500,50000]),0,0))
for a in list(range(0,34000,7))+[33500,119080,200000]:
    for d in range(4): cs.append((3,a,d,0,0))
for a in list(range(0,34000,37))+[33500,119080]:
    for cl in range(4):
        for d in range(4):
            for rd in (10,20): cs.append((4,a,cl,d,rd))
text=''.join(f'{k} {a!r} {b!r} {c!r} {d!r}\n' for k,a,b,c,d in cs)
roots=['@F-90390@','@F-62421@','@F-14176@','@F-31900@','@F-78976@']
build('rewards',roots,MAIN,text,'out',scratch=0xa000)
