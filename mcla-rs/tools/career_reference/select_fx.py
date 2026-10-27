import random
from fx import *
EXTRA=r'''
#include <vector>
static std::vector<std::pair<uint32_t,uint32_t>> calls;   // (vtable slot address, this)
static uint32_t pool=0x4000;
#define REX_CALL_INDIRECT_FUNC(a) calls.emplace_back(uint32_t(a),ctx.r3.u32)
'''
STUB=r'''
void @F-15488@(Context &ctx,const uint8_t*) { uint32_t p=pool; pool+=(ctx.r3.u32+15)&~15u; ctx.r3.u32=p; }   // operator new: bump allocator
'''
MAIN=r'''
int main(int argc,char **argv) {
    std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
    if(image.size()!=0x9e0000)return 3;
    std::ifstream cases(argv[2]);
    std::puts("count,min,max,randomness_bits,lo,carry,out_lo,out_carry,calls");
    int n,mn,mx; uint32_t rb,lo,carry;
    while(cases>>n>>mn>>mx>>rb>>lo>>carry) {
        std::memset(objects,0,sizeof(objects)); calls.clear(); pool=0x4000;
        const uint32_t T=0x1000, ARR=0x1100, VT=0x1800;
        for(int s=0;s<64;s+=4) store(VT+s,0xF0000000u+s);          // fake vtable: slot value = 0xF0000000 + offset
        store(T+4,ARR); *pointer(T+8)=n>>8; *pointer(T+9)=n;
        store(T+28,rb); store(T+32,mn); store(T+36,mx);
        for(int i=0;i<n;i++){ uint32_t o=0x1200+16*i; store(o,VT); store(ARR+4*i,o); }
        store(0x82860890,lo); store(0x82860894,carry);
        Context ctx{}; ctx.r1.u32=0x8000; ctx.r3.u32=T; bool trap=false; try { @F-35087@(ctx,nullptr); } catch(...) { trap=true; }
        if(trap){ std::printf("%d,%d,%d,%u,%u,%u,TRAP,TRAP,\n",n,mn,mx,rb,lo,carry); continue; }
        std::printf("%d,%d,%d,%u,%u,%u,%u,%u,",n,mn,mx,rb,lo,carry,load(0x82860890),load(0x82860894));
        for(size_t i=0;i<calls.size();i++){ uint32_t idx=(calls[i].second-0x1200)/16; std::printf("%s%u:%u",i?" ":"",calls[i].first&0xFF,idx); }
        std::puts("");
    }
}
'''
import struct
rng=random.Random(82685)
cs=[]
fb=lambda f:struct.unpack('<I',struct.pack('<f',f))[0]
for _ in range(20000):
    n=rng.randint(0,8)
    mn=rng.choice([-1,-1,0,1,2,3,n,n+1]); mx=rng.choice([-1,-1,0,1,2,3,n,n+2])
    r=rng.choice([0.0,0.25,0.5,0.9,1.0,1.5])
    cs.append((n,mn,mx,fb(r),rng.getrandbits(32),rng.getrandbits(32)))
text=''.join(' '.join(map(str,c))+'\n' for c in cs)
build('select',['@F-35087@'],MAIN,text,'out',scratch=0xa000,stub_src=STUB,stub=('@F-15488@',),extra=EXTRA)
