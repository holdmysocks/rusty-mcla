import random
from fx import *
EXTRA=r'''
static std::string cstr(uint32_t a){ std::string s; while(*pointer(a)) s+=char(*pointer(a++)); return s; }
static void put_cstr(uint32_t a,const std::string&s){ for(size_t i=0;i<=s.size();i++) *pointer(a+i)=i<s.size()?s[i]:0; }
'''
STUB=r'''
// the C run-time's sprintf / the engine's snprintf wrapper, as the host's (only "%d" and "%s%s" are used here)
void @F-98741@(Context &ctx,const uint8_t*) { if(cstr(ctx.r4.u32)!="%d") throw std::runtime_error("fmt"); char b[32]; std::snprintf(b,32,"%d",int(ctx.r5.u32)); put_cstr(ctx.r3.u32,b); ctx.r3.u32=std::strlen(b); }
void @F-32585@(Context &ctx,const uint8_t*) { if(cstr(ctx.r5.u32)!="%s%s") throw std::runtime_error("fmt2"); std::string s=cstr(ctx.r6.u32)+cstr(ctx.r7.u32); if(s.size()>=ctx.r4.u32) s.resize(ctx.r4.u32-1); put_cstr(ctx.r3.u32,s); ctx.r3.u32=s.size(); }
'''
MAIN=r'''
int main(int argc,char **argv) {
    std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
    if(image.size()!=0x9e0000)return 3;
    std::ifstream cases(argv[2]);
    std::puts("kind,a,b,c,d,text");
    int k; long long a,b,c,d;
    while(cases>>k>>a>>b>>c>>d) {
        std::memset(objects,0,sizeof(objects)); Context ctx{}; ctx.r1.u32=0x8000;
        std::string out;
        try {
            if(k==0){ store(0x1000+28,uint32_t(a)); ctx.r3.u32=0x1000; ctx.r4.u32=0x2000; ctx.r5.u32=uint32_t(b); ctx.r6.u32=uint32_t(int(c)); ctx.r7.u32=uint32_t(d); @F-36073@(ctx,nullptr); out=cstr(0x2000); }
            else { ctx.r3.u32=uint32_t(int(a)); ctx.r4.u32=0x2000; ctx.r5.u32=uint32_t(b); ctx.r6.u32=uint32_t(c); @F-35525@(ctx,nullptr); out=cstr(0x2000); }
        } catch(...) { out="TRAP"; }
        std::printf("%d,%lld,%lld,%lld,%lld,%s\n",k,a,b,c,d,out.c_str());
    }
}
'''
rng=random.Random(2217)
vals=[0,1,9,10,99,100,999,1000,1001,9999,10000,12345,99999,100000,999999,1000000,1234567,12345678,123456789,2147483647,-1,-9,-10,-999,-1000,-1001,-123456,-2147483647]
vals+= [rng.randint(-10**9,10**9) for _ in range(500)]+[rng.randint(0,10**5) for _ in range(500)]
cs=[]
for v in vals:
    for lang in range(6):
        for flag in (0,1): cs.append((0,lang,64,v,flag))
    for pre in (0,36): 
        for sep in (44,46,124): cs.append((1,v,pre,sep,0))
text=''.join(' '.join(map(str,c))+'\n' for c in cs)
build('money',['@F-36073@','@F-33429@','@F-35525@'],MAIN,text,'out',scratch=0xa000,stub_src=STUB,stub=('@F-98741@','@F-32585@'),extra=EXTRA)
