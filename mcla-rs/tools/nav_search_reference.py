"""The racers' nav-mesh path search run offline on the real city mesh, generated code whole and unchanged, for the port's
test (crates/formats/src/rage/testdata/nav_search_ref.txt; test navmesh::tests::quickest_path_matches_the_original, needs
MCLA_DATA_DIR; MCLA_NAV_SEARCH_REF=<file> checks another run of this tool instead of the kept file).

1. city.xsm (resources/city/sc/0x69d17d57.r26) is loaded as the original's loader lays it out (chunk table F-92072) and
   placed by the mesh's own Place F-10115.
2. The image's static initialisers that call nothing (vector / float constants in BSS the vertex fetch and the search
   read), in table order, then the initialisers of the mesh's two iterator pools F-39825 / F-22830.
3. The level is made by its creation routine F-89841 with its resource load F-57285 stubbed to hand over the placed root;
   the flag word it tests is set, so the level runs without its thread (+584 = 0).
4. A navigation brain (ctor F-86290) with a racing filter object (the mcRacingFilter vtable, +8 start flags, +12 goal
   flags) in its capabilities (F-81317), its query made by F-27233 (the query ctor F-28382 waits for a non-zero low
   timebase word: the harness's timebase is 1).
5. Per case: start / goal points (a random polygon's stored centre, 0 / 2 / 6 / 14 m off in x and z, 0 / +-0.9 / +-4 /
   -12..30 m in y; the goal anywhere or within 60 / 250 / 800 m), start and goal flags from the racers' material masks;
   the brain's submit F-59389, the job body F-76461 run in place of the job (the submit's level update F-91947 would
   queue it), the path printed, the brain's release F-25277.
6. Single thread: the atomic compare-exchange F-62311 / fetch-add F-42502 and the semaphores are modelled; a wait that
   would block throws. F-27315 (the bad-pointer report) prints; F-12484 / F-25937 / F-17496 do nothing here.

Line per case: "case <i> <start xyz> <goal xyz> (f32 bits) <start flags> <goal flags> -> <query +25> <+24> <count> |"
then the path elements (query +152: first +12, count +20, next +48): "<type> <A xyz> <B xyz> <handle +28> <+32> <+36>
<left +40> <entered +44>;" (a type-0 element is the point: +4 the point, +20 the goal node).

Accessor mode (--accessors, test brainturn::tests::mesh_accessors_match_the_original): per case a random polygon,
"poly <node id> <count> | <world vertex xyz>... | n <normal xyz> | m <material word +8>" (F-36421, F-83219, vfunc 60 F-43095),
and a random segment through vfunc 72 F-87322 with the caps the arc test F-38160 builds (F-53025 / F-94095 / F-96012; width 0
or 1.6 - 2.4, min normal-y -1, filter off, the mesh's +76 = 100 as the arc test sets it): "seg <from xyz> <to xyz> <width> -> <result>".

Probe mode (--probe <file of hex x y z>): per point the mesh's containing list (vfunc 32, F-15418), the point list
(F-38131) and the nearest polygon within 10 m (F-83298).
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

SDK = Path('D:/MCLA-static/third_party/rexglue-sdk/include/rex/ppc/intrinsics.h')

# the vector shim as the placement needs it: the condition register's setFromMask, the vector register's lane types, 128
# vector registers, the SDK's mask / shift tables and the vector helpers the mesh code uses, the timebase non-zero
_vs0 = rd.vector_shim


def _vs(*a, **k):
    cpp = '#include <immintrin.h>' + chr(10) + _vs0(*a, **k)
    old = 'bool eq{}, lt{}, gt{};'
    if old in cpp and 'setFromMask' not in cpp:
        cpp = cpp.replace(old, old + ' bool so{};' + chr(10) + '    void setFromMask(__m128 m,int imm) { int k=_mm_movemask_ps(m); lt=k==imm; gt=0; eq=k==0; so=0; }' + chr(10) + '    void setFromMask(__m128i m,int imm) { int k=_mm_movemask_epi8(m); lt=k==imm; gt=0; eq=k==0; so=0; }', 1)

    oldu = 'union alignas(16) Vector { uint8_t u8[16]; uint32_t u32[4]; float f32[4]; };'
    if oldu in cpp:
        cpp = cpp.replace(oldu, 'union alignas(16) Vector { uint8_t u8[16]; int8_t s8[16]; uint16_t u16[8]; int16_t s16[8]; uint32_t u32[4]; int32_t s32[4]; float f32[4]; };' + chr(10) + 'typedef Vector PPCVRegister;', 1)
    if 'v63;' in cpp and 'v64' not in cpp:
        cpp = cpp.replace('v63;', 'v63,' + ','.join('v%d' % i for i in range(64, 128)) + ';', 1)
    if 'VectorMaskR[]' not in cpp:
        import re as _re
        hdr = SDK.read_text()
        mk = _re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', hdr, _re.S).group()
        cpp += chr(10) + mk.replace('inline uint8_t', 'alignas(16) static uint8_t') + chr(10)
    cpp += chr(10) + 'typedef VReg PPCVRegister;' + chr(10) + _EXTRA + 'static inline __m128i simde_mm_sllv_epi32(__m128i a,__m128i c){alignas(16) uint32_t x[4],y[4];_mm_store_si128((__m128i*)x,a);_mm_store_si128((__m128i*)y,c);for(int i=0;i<4;i++)x[i]=y[i]<32?x[i]<<y[i]:0;return _mm_load_si128((__m128i*)x);}' + chr(10)
    return cpp

_EXTRA = chr(10).join(["namespace rex { namespace ppc {",
"static inline __m128 simde_mm_cvtepu32_ps_(__m128i src1) {",
"    __m128i xmm1 = _mm_add_epi32(src1, _mm_set1_epi32(127));",
"    __m128i xmm0 = _mm_slli_epi32(src1, 31 - 8);",
"    xmm0 = _mm_srli_epi32(xmm0, 31);",
"    xmm0 = _mm_add_epi32(xmm0, xmm1);",
"    xmm0 = _mm_srai_epi32(xmm0, 8);",
"    xmm0 = _mm_add_epi32(xmm0, _mm_set1_epi32(0x4F800000));",
"    __m128 xmm2 = _mm_cvtepi32_ps(src1);",
"    return _mm_blendv_ps(xmm2, _mm_castsi128_ps(xmm0), _mm_castsi128_ps(src1));",
"}",
"static inline __m128i simde_mm_vctsxs(__m128 src1) {",
"  __m128 xmm2 = _mm_cmpunord_ps(src1, src1);",
"  __m128i xmm0 = _mm_cvttps_epi32(src1);",
"  __m128i xmm1 = _mm_cmpeq_epi32(xmm0, _mm_set1_epi32(INT_MIN));",
"  xmm1 = _mm_andnot_si128(_mm_castps_si128(src1), xmm1);",
"  __m128 dest = _mm_blendv_ps(_mm_castsi128_ps(xmm0), _mm_castsi128_ps(_mm_set1_epi32(INT_MAX)), _mm_castsi128_ps(xmm1));",
"  return _mm_andnot_si128(_mm_castps_si128(xmm2), _mm_castps_si128(dest)); }",
"} }",
"#define simde__m128 __m128",
"#define simde_mm_permute_ps(a,i) _mm_shuffle_ps(a,a,i)",
"#define simde_mm_movemask_ps _mm_movemask_ps",
"#define simde_mm_movemask_epi8 _mm_movemask_epi8"] + ['#define simde_mm_%s _mm_%s' % (n, n) for n in ['alignr_epi8', 'blend_ps', 'cmpeq_epi32', 'cmpge_ps', 'cmpgt_ps', 'cvtepi16_epi32', 'cvtepi32_ps', 'max_ps', 'min_ps', 'setzero_si128', 'unpackhi_epi64', 'xor_si128']]) + chr(10)

_VEC_EXTRA = (chr(10).join([
 'namespace rex { namespace ppc {',
 '// the SDK intrinsics.h vperm helper, as written there',
 'static inline __m128i simde_mm_perm_epi8_(__m128i a, __m128i b, __m128i c) {',
 '  __m128i d = _mm_set1_epi8(0xF); __m128i e = _mm_sub_epi8(d, _mm_and_si128(c, d));',
 '  return _mm_blendv_epi8(_mm_shuffle_epi8(a, e), _mm_shuffle_epi8(b, e), _mm_slli_epi32(c, 3)); }',
 '// vslb: each byte shifted left by its count (low 3 bits), as the SDK widening to 16 bits then masking does',
 'static inline __m128i simde_mm_sllv_epi8(__m128i a, __m128i c) { alignas(16) uint8_t x[16], y[16];',
 '  _mm_store_si128((__m128i*)x, a); _mm_store_si128((__m128i*)y, c);',
 '  for (int i = 0; i < 16; i++) { unsigned k = y[i]; x[i] = k < 16 ? uint8_t((unsigned(x[i]) << k) & 0xFF) : 0; }',
 '  return _mm_load_si128((__m128i*)x); }',
 '} }']) + chr(10))

_TB = '#undef REX_QUERY_TIMEBASE' + chr(10) + '#define REX_QUERY_TIMEBASE() (rt_time|1)' + chr(10)


def _vector_shim(*a, **k):
    c = _vs(*a, **k)
    hdr = SDK.read_text()
    tabs = ''.join(re.search(r'inline uint8_t %s\[\] = \{.*?\};' % t, hdr, re.S).group().replace('inline uint8_t', 'alignas(16) static uint8_t') + chr(10)
                   for t in ('VectorShiftTableL', 'VectorShiftTableR'))
    return '#include <atomic>' + chr(10) + c + (chr(10) + tabs if 'VectorShiftTableL[]' not in c else '') + _VEC_EXTRA + _TB


rd.vector_shim = _vector_shim

MAIN = r'''
static const uint32_t HV=0x40800000,TB=0x40010100,DR=0x40010000,ROOTW=0x40000F00,NAMEA=0x40000F10;
struct Ch{uint32_t va,host,size,flat;};
static std::vector<Ch> chs;
static uint64_t lcg=0x5DEECE66Dull*NAVSEED;
static uint32_t rnd(){lcg=lcg*6364136223846793005ull+1442695040888963407ull;return uint32_t(lcg>>33);}
static float frand(float a,float b){return a+(b-a)*(rnd()&0xFFFFFF)/16777216.0f;}
static float ldf(uint32_t a){uint32_t w=load(a);float f;memcpy(&f,&w,4);return f;}
static void stf(uint32_t a,float f){uint32_t w;memcpy(&w,&f,4);store(a,w);}
static uint32_t fbits(float f){uint32_t w;memcpy(&w,&f,4);return w;}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 int ncases=atoi(argv[3]);
 std::ifstream f(argv[2],std::ios::binary);std::vector<char> d((std::istreambuf_iterator<char>(f)),{});
 uint32_t flags=*(uint32_t*)&d[8],vs=*(uint32_t*)&d[12];size_t dn=d.size()-16;
 rt_reset();
 for(uint32_t i=0;i<0x800000;i+=4)store(HV+i,0);
 Context c0=fresh();c0.r3.u32=flags;c0.r4.u32=TB;F_92072(c0,nullptr);
 uint32_t nv=load16(TB),np=load16(TB+2);
 uint32_t cur=HV,flat=0;
 for(uint32_t i=0;i<nv+np;i++){uint32_t e=TB+4+12*i;Ch ch;ch.va=load(e);ch.size=load(e+8);
  if(i==nv){cur=(cur+0xFFFF)&~0xFFFFu;cur+=0x10000;flat=vs;}
  ch.host=cur;ch.flat=flat;store(e+4,ch.host);cur+=ch.size;flat+=ch.size;chs.push_back(ch);}
 for(auto&ch:chs){for(uint32_t k=0;k<ch.size;k++){size_t o=ch.flat+k;*pointer(ch.host+k)=o<dn?uint8_t(d[16+o]):0;}}
 store(DR,TB);store(load(fresh().r13.u32)+60,DR);
 uint32_t root=chs[0].host;
 {Context c=fresh();c.r3.u32=root;c.r4.u32=DR;c.r5.u32=0;F_10115(c,nullptr);}
 store(ROOTW,root);
 INITCALLS
 // the static initialisers of the mesh's two iterator pools (mesh +32 / +36 point at them; BSS in the image)
 {Context c=fresh();F_39825(c,nullptr);}{Context c=fresh();F_22830(c,nullptr);}
 store(NOTHREAD,1);                      // the word F-89841 tests: non-zero -> level +584 = 0 (no nav thread)
 const char*nm="city";for(int i=0;i<5;i++)*pointer(NAMEA+i)=uint8_t(nm[i]);
 uint32_t level;{Context c=fresh();c.r3.u32=NAMEA;F_89841(c,nullptr);level=c.r3.u32;}
 printf("level %08x root %08x space %08x mesh+584 %u\n",level,load(level+656),load(level+512),load(level+584));
 // the mesh's chunks (placed): level object = root+12
 uint32_t lv=load(root+12),chp=load(lv+0x30),nch=load16(lv+0x34),cdt=load(load(lv+0x28)+4);
 int loaded=0;for(uint32_t k=0;k<nch;k++){uint32_t ck=load(chp+4*k);if(ck&&(load(ck+92)&1))loaded++;}
 printf("chunks %u loaded %d\n",nch,loaded);
 // the brain-side objects
 uint32_t R=rt_alloc(512,16),F=rt_alloc(32,16);
 {Context c=fresh();c.r3.u32=R;F_86290(c,nullptr);}
 store(F,FILTERVT);store(F+4,1);
 {Context c=fresh();c.r3.u32=R+132;c.r4.u32=F;F_81317(c,nullptr);}
 {Context c=fresh();c.r3.u32=R;c.r4.u32=0;c.r5.u32=0;F_27233(c,nullptr);}
 uint32_t Q=load(R+48);
 static const uint32_t MATS[9]={0,0x81,0x82,0x9,0xA,0x110,0x84,0xA0,0xC};
 auto pick=[&](float*p,int far,const float*near){
  for(int t=0;t<1000;t++){uint32_t k=rnd()%nch;uint32_t ck=load(chp+4*k);if(!ck)continue;uint32_t np_=load16(ck+0x56);if(!np_)continue;
   uint32_t cd=load(cdt+4*load16(ck+0x50));uint32_t polys=load(cd+0x10);uint32_t pi=rnd()%np_;uint32_t pp=polys+28*pi;
   float l[3];for(int j=0;j<3;j++)l[j]=int16_t(load16(pp+2*j))*0.05f;
   float w[3];for(int j=0;j<3;j++)w[j]=ldf(ck+0x10+48+4*j)+l[0]*ldf(ck+0x10+4*j)+l[1]*ldf(ck+0x10+16+4*j)+l[2]*ldf(ck+0x10+32+4*j);
   if(near){float dx=w[0]-near[0],dz=w[2]-near[2];if(dx*dx+dz*dz>far*far)continue;}
   uint32_t m=rnd()%8;float r=m<3?0:m<5?2:m<7?6:14;
   p[0]=w[0]+frand(-r,r);p[2]=w[2]+frand(-r,r);uint32_t yk=rnd()%6;p[1]=w[1]+(yk<2?0:yk<4?frand(-0.9f,0.9f):yk<5?frand(-4,4):frand(-12,30));
   return;}
  p[0]=near?near[0]:0;p[1]=near?near[1]:0;p[2]=near?near[2]:0;};
 if(ACCMODE){
  uint32_t M=load(level+512);
  for(int ci=0;ci<ncases;ci++){
   // a random polygon: its world vertices (F-36421: the chunk matrix at chunk +16, at most r7 = 16), its normal (F-83219), its material word +8 (mesh vfunc 60)
   uint32_t k,ck,np_;for(;;){k=rnd()%nch;ck=load(chp+4*k);if(!ck)continue;np_=load16(ck+0x56);if(np_)break;}
   uint32_t cd=load(cdt+4*load16(ck+0x50)),pi=rnd()%np_,pp=load(cd+0x10)+28*pi,OUT=rt_alloc(16*16,16);
   {Context c=fresh();c.r3.u32=pp;c.r4.u32=cd;c.r5.u32=ck+16;c.r6.u32=OUT;c.r7.u32=16;F_36421(c,nullptr);uint32_t nv=c.r3.u32;
    printf("poly %x %u |",(k<<12)|pi,nv);for(uint32_t v=0;v<nv;v++)for(int j=0;j<3;j++)printf(" %08x",load(OUT+16*v+4*j));}
   {Context c=fresh();c.r3.u32=ck;c.r4.u32=pi;c.r5.u32=OUT;F_83219(c,nullptr);printf(" | n");for(int j=0;j<3;j++)printf(" %08x",load(OUT+4*j));}
   {Context c=fresh();c.r3.u32=M;c.r4.u32=(k<<12)|pi;F_43095(c,nullptr);uint32_t mo=c.r3.u32;printf(" | m %08x\n",mo?load(mo+8):0xFFFFFFFFu);}
   // a random segment through the mesh's vfunc 72 (F-87322), the caps built as the arc test F-38160 builds them:
   // F-53025 at frame +128 (+28 width, +8 min normal-y -1), F-94095 at +176, F-96012 copies the caps to +192
   float s[3],e[3];pick(s,0,nullptr);uint32_t dk=rnd()%4;
   if(dk==0)pick(e,30,s);else{float ang=frand(0,6.2831853f),len=dk==1?frand(0.5f,5):dk==2?frand(5,20):frand(20,60);
    e[0]=s[0]+len*cosf(ang);e[2]=s[2]+len*sinf(ang);e[1]=s[1]+frand(-1,1);}
   uint32_t wk=rnd()%3;float w=wk==0?0.0f:frand(1.6f,2.4f);
   uint32_t FR=rt_alloc(512,16),PA=rt_alloc(32,16);
   {Context c=fresh();c.r3.u32=FR+128;F_53025(c,nullptr);}stf(FR+156,w);stf(FR+136,-1.0f);
   {Context c=fresh();c.r3.u32=FR+176;F_94095(c,nullptr);}
   {Context c=fresh();c.r3.u32=FR+192;c.r4.u32=FR+128;F_96012(c,nullptr);}
   for(int j=0;j<3;j++){stf(PA+4*j,s[j]);stf(PA+16+4*j,e[j]);}stf(PA+12,0);stf(PA+28,0);
   // (the arc test sets the mesh's +76, the point test's window above a polygon, to 100 around the call)
   uint32_t o76=load(M+76);stf(M+76,100.0f);
   Context c=fresh();c.r3.u32=M;c.r4.u32=PA;c.r5.u32=PA+16;c.r6.u32=FR+176;c.r7.u32=0;c.r8.u32=1;F_87322(c,nullptr);
   store(M+76,o76);
   printf("seg %08x %08x %08x %08x %08x %08x %08x -> %u\n",fbits(s[0]),fbits(s[1]),fbits(s[2]),fbits(e[0]),fbits(e[1]),fbits(e[2]),fbits(w),c.r3.u32&0xFF);
  }
  return 0;}
 for(int ci=0;ci<ncases;ci++){
  float s[3],g[3];pick(s,0,nullptr);uint32_t kind=rnd()%4;if(kind==0)pick(g,0,nullptr);else pick(g,kind==1?60:kind==2?250:800,s);
  uint32_t sf=MATS[rnd()%9],gf=MATS[rnd()%9];
  store(F+8,sf);store(F+12,gf);
  for(int j=0;j<3;j++){stf(R+16+4*j,s[j]);stf(R+32+4*j,g[j]);}stf(R+28,0);stf(R+44,0);
  *pointer(level+644)=0;   // (the submit's level update F-91947 would hand the queue to a job: the harness runs that job's body itself)
  {Context c=fresh();c.r3.u32=R;F_59389(c,nullptr);}
  {Context c=fresh();c.r3.u32=level;F_76461(c,nullptr);}
  uint32_t P=Q+152;uint32_t n=load(P+20);
  printf("case %d %08x %08x %08x %08x %08x %08x %x %x -> %u %u %u |",ci,fbits(s[0]),fbits(s[1]),fbits(s[2]),fbits(g[0]),fbits(g[1]),fbits(g[2]),sf,gf,
   (unsigned)*pointer(Q+25),(unsigned)*pointer(Q+24),n);
  uint32_t e=load(P+12);for(uint32_t i=0;i<n&&e;i++){
   printf(" %u",load(e));for(int j=0;j<3;j++)printf(" %08x",load(e+4+4*j));for(int j=0;j<3;j++)printf(" %08x",load(e+16+4*j));
   printf(" %08x %08x %08x %08x %08x;",load(e+28),load(e+32),load(e+36),load(e+40),load(e+44));e=load(e+48);}
  printf("\n");
  {Context c=fresh();c.r3.u32=R;F_25277(c,nullptr);}   // the brain's release (query off the queue) before the next submit
  }
 // probe mode: argv[4] = file of points (hex x y z): the mesh's containing list (vfunc 32), nearest within 10 m (vfunc 56)
 // and the point list (vfunc 36) as the start / goal lookup builds them
 if(argc>4){std::ifstream pf(argv[4]);uint32_t x,y,z;uint32_t M=load(level+512),PT=rt_alloc(16,16);
  while(pf>>std::hex>>x>>y>>z){store(PT,x);store(PT+4,y);store(PT+8,z);store(PT+12,0);
   printf("probe %08x %08x %08x",x,y,z);
   for(int k=0;k<2;k++){store(M+16,0);*pointer(M+21)=0;Context c=fresh();c.r3.u32=M;c.r4.u32=PT;c.r5.u32=0;c.r6.u32=Q+28;
    if(k==0)F_15418(c,nullptr);else F_38131(c,nullptr);
    uint32_t n=load(M+16);printf(" | %s %u:",k?"list":"contain",n);for(uint32_t i=0;i<n;i++)printf(" %x",load(load(M+8)+4*i));}
   {Context c=fresh();c.r3.u32=M;c.r4.u32=PT;c.r5.u32=0;c.r6.u32=Q+28;c.f1.f64=10.0;F_83298(c,nullptr);printf(" | nearest %x\n",c.r3.u32);}}}
}
'''

# vtables the generated code calls through (address, slots): the level, its handler, the quickest-path goal, the
# navPoint, the racing filter, the query, the brain's nav part, the iterator, the mesh space, capabilities, dimensions,
# the simple goal; then the nav classes those reach
VT = [(0x820C47A4, 24), (0x820D5704, 16), (0x820D62A4, 16), (0x820C2F20, 12), (0x82038A58, 5), (0x820D4EB0, 16),
      (0x820C4D5C, 12), (0x820D5350, 8), (0x820D53C4, 32), (0x8207BD84, 8), (0x8207C2D4, 8), (0x820D5564, 24)]
VT_NAV = [
    (0x8207B8D0, 7), (0x8207C07C, 6), (0x8207C5F4, 4), (0x8207C608, 4), (0x8207CB04, 5), (0x820C2F20, 8),
    (0x820C47A4, 20), (0x820CB418, 6), (0x820D4450, 4), (0x820D4A00, 3), (0x820D4BB4, 7), (0x820D4E40, 32),
    (0x820D5310, 21), (0x820D53AC, 36), (0x820D5560, 29), (0x820D5700, 57), (0x820D57E8, 7), (0x820D5808, 21),
    (0x820D59B0, 16), (0x820D5B20, 4), (0x820D62A0, 26), (0x820D6420, 13), (0x820D66B8, 42), (0x820D6890, 36),
    (0x820D6A60, 7), (0x820D6BA0, 5),
]
INIT_TABLE = (0x827CD054, 0x827D1420)   # the image's static initialiser table
NOTHREAD, FILTER_VT = 0x8290A08C, 0x82038A58

# place, the pool initialisers, chunk table, level create, brain ctor, capability set, query make, submit, job body,
# release; probe: containing list, point list, nearest
FIDS = ['F-10115', 'F-39825', 'F-22830', 'F-92072', 'F-89841', 'F-86290', 'F-81317', 'F-27233', 'F-59389', 'F-76461',
        'F-25277', 'F-15418', 'F-38131', 'F-83298',
        # accessor mode: polygon vertices, normal, material (vfunc 60), segment test (vfunc 72), the caps constructors
        'F-36421', 'F-83219', 'F-43095', 'F-87322', 'F-53025', 'F-94095', 'F-96012']
# stubs: bad-pointer report, resource load, three no-ops, compare-exchange, fetch-add
STUB_FIDS = ['F-27315', 'F-57285', 'F-12484', 'F-25937', 'F-17496', 'F-62311', 'F-42502']
# indirect targets: the nav job's entry, the u32 compare the id-list sort passes
TARGET_FIDS = ['F-96452', 'F-37154']


def addrs(tool, fids):
    r = subprocess.run([tool, 'registry', 'addr', *fids], capture_output=True, text=True, check=True).stdout
    m = {l.split()[0]: l.split()[1].upper().replace('0X', '') for l in r.splitlines() if l.startswith('F-')}
    assert all(f in m for f in fids), r
    return [m[f] for f in fids]


def initialisers(idx, img):
    """The table's entries whose generated body calls nothing, in table order."""
    out = []
    for a in range(*INIT_TABLE, 4):
        n = 'sub_%08X' % rd.image_u32(img, a)
        if n not in idx:
            continue
        b = rc.extract(idx[n], n)
        if re.search(r'\(ctx, base\)|INDIRECT', b[b.index('{'):]):
            continue
        out.append(n)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--mesh', type=Path, required=True, help='resources/city/sc/0x69d17d57.r26')
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=300)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('--probe', type=Path)
    ap.add_argument('--accessors', action='store_true', help='polygon / segment accessor cases instead of searches')
    a = ap.parse_args()
    fn = ['sub_' + x for x in addrs(a.tool, FIDS)]
    st = ['sub_' + x for x in addrs(a.tool, STUB_FIDS)]
    tg = [int(x, 16) for x in addrs(a.tool, TARGET_FIDS)]
    img = rd.load_image(a.image)
    idx = rd.index(a.recomp)
    inits = initialisers(idx, img)
    src = MAIN.replace('INITCALLS', ''.join('{Context c=fresh();%s(c,nullptr);}' % f for f in inits))
    src = src.replace('ACCMODE', '1' if a.accessors else '0')
    src = src.replace('NAVSEED', '%dull' % a.seed).replace('NOTHREAD', '0x%08Xu' % NOTHREAD).replace('FILTERVT', '0x%08Xu' % FILTER_VT)
    for f, n in zip(FIDS, fn):
        src = src.replace(f.replace('-', '_') + '(', n + '(')
    stubs = {st[0]: 'printf("BADPTR %x %x%c",ctx.r5.u32,ctx.r3.u32,10);',
             st[1]: 'store(ctx.r3.u32,load(0x40000F00u));ctx.r3.u32=load(0x40000F00u);',
             st[2]: '', st[3]: '', st[4]: '',
             st[5]: 'uint32_t o=load(ctx.r3.u32);if(o==ctx.r5.u32)store(ctx.r3.u32,ctx.r4.u32);ctx.r3.u64=o;',
             st[6]: 'uint32_t o=load(ctx.r3.u32);store(ctx.r3.u32,o+ctx.r4.u32);ctx.r3.u64=o;',
             # semaphores: counts per handle in guest words 0x40000E00 + 4 h (h 1..31)
             '__imp__NtCreateSemaphore': 'uint32_t n=load(0x40000E00u)+1;store(0x40000E00u,n);store(0x40000E00u+4*n,ctx.r5.u32);store(ctx.r3.u32,0x100u+n);ctx.r3.u64=0;',
             '__imp__NtReleaseSemaphore': 'uint32_t a=0x40000E00u+4*((ctx.r3.u32-0x100u)&31);if(ctx.r5.u32)store(ctx.r5.u32,load(a));store(a,load(a)+ctx.r4.u32);ctx.r3.u64=0;',
             '__imp__NtWaitForSingleObjectEx': 'uint32_t a=0x40000E00u+4*((ctx.r3.u32-0x100u)&31);if(load(a)){store(a,load(a)-1);ctx.r3.u64=0;}else if(ctx.r6.u32&&!load(ctx.r6.u32)&&!load(ctx.r6.u32+4)){ctx.r3.u64=0x102;}else throw std::runtime_error("wait would block");'}
    a.out.mkdir(parents=True, exist_ok=True)
    exe = rd.build(idx, inits + fn, src, a.out, 'nav_search', image=img, vtables=VT + VT_NAV, targets=tg, stubs=stubs, limit=4000)
    r = subprocess.run([str(exe), str(a.image), str(a.mesh), str(a.cases)] + ([str(a.probe)] if a.probe else []),
                       capture_output=True, text=True, timeout=3000)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout)
    print(r.stdout.count('case ') + r.stdout.count('seg '), 'cases ->', a.ref)


if __name__ == '__main__':
    main()
