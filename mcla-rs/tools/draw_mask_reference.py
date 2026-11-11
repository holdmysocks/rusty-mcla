"""The Flash draw traversal F-92070's clip-depth masks run offline (generated code unchanged): random instance trees
built in guest memory, the draw run from the top instance, the render-state calls and the shape draws it makes logged.

Real code: F-92070 and the stencil begin / end F-90239 / F-71617 (their 6-bit counter at 0x8281B7C8 included). Stubs: the
render-state setter F-33110 logs "S<state>,<value>"; the owner draw (the indirect call through +4's vtable slot 3) logs
"D<node>" (the instance it was called for); the colour combine F-57014, the matrix compose F-12960 and the hooks F-54245 /
F-54006 / F-74625 do nothing (each instance's world alpha byte +67 is set by the case, +81 / +148 = 0, +152 = -1, the
focus global 0, so they are not reached or change nothing the log shows).

A tree: a top instance (depth 0..3, or 65500.. so the 16-bit sums wrap), up to 4 levels, 0..4 children an instance in
ascending depth (+188 / +192), each a mask (+168 = its depth + 1..12) one time in four and of alpha 0 one time in twelve;
the end-depth global (0x82901F9C) cleared before each case. Writes <out>: "MK <n> (<depth> <clip> <alpha> <parent>)* ->
<log>" per case; then 4,000 world matrices: "WM <local a b c d tx ty> <parent's world a .. ty> -> <world a .. ty>" (float
bits; the instance's alpha byte 0, so the draw returns after storing them); then 3,000 texture trees: "TX <n> (<depth> <tex>
<alpha> <parent>)* -> <log>", no masks, an instance's +148 a texture one time in four (the texture stub F-12960 logs
"T<texture>"), alpha 0 one time in twelve, each draw logged "D<node>,<+80>" (the swap flag the shape draw F-19892 reads).
The port: `draw::Builder` (draw.rs) through the tests `masks_match_the_original`, `world_matrix_matches_the_original`,
`texture_swaps_match_the_original` (testdata/flash_mask_ref.txt).
"""
import argparse
import subprocess
from pathlib import Path
from recomp_closure import index, build

STUBS = ['F-57014', 'F-54245', 'F-54006', 'F-74625']


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
static uint32_t rnd(uint32_t&s){s^=s<<13;s^=s>>17;s^=s<<5;return s;}
struct N{uint32_t depth,clip,alpha;int parent;std::vector<int> kids;};
static std::vector<N> nodes;
static std::vector<uint32_t> texs;
static uint32_t at(int i){return 0x1000+224*i;}
static void gen(int me,int level,uint32_t&s){
  int n=level>=3?0:int(rnd(s)%5);
  if(level==0&&n==0)n=1;
  uint32_t d=1+rnd(s)%3;
  for(int k=0;k<n&&nodes.size()<40;k++){
    N c{d,0,64,me,{}};
    if(rnd(s)%4==0)c.clip=d+1+rnd(s)%12;
    if(rnd(s)%12==0)c.alpha=0;
    nodes.push_back(c);int id=int(nodes.size())-1;nodes[me].kids.push_back(id);
    gen(id,level+1,s);
    d+=1+rnd(s)%6;
  }
}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(uint32_t seed=1;seed<=3000;seed++){
  uint32_t s=seed*2654435761u|1;
  nodes.clear();
  N top{rnd(s)%10==0?65500+rnd(s)%30:rnd(s)%4,0,64,-1,{}};
  if(rnd(s)%6==0)top.clip=top.depth+1+rnd(s)%12;
  nodes.push_back(top);
  gen(0,0,s);
  std::memset(objects,0,sizeof objects);
  for(size_t i=0;i<nodes.size();i++){
   uint32_t a=at(int(i));const N&n=nodes[i];
   store(a+4,0x7000);
   store16(a+166,uint16_t(n.depth));store16(a+168,uint16_t(n.clip));
   *pointer(a+67)=uint8_t(n.alpha);*pointer(a+177)=1;*pointer(a+81)=0;
   store(a+148,0);store(a+152,0xFFFFFFFFu);
   store(a+184,n.parent<0?0:at(n.parent));
   store(a+188,n.kids.empty()?0:at(n.kids[0]));
  }
  for(size_t i=0;i<nodes.size();i++)for(size_t k=0;k+1<nodes[i].kids.size();k++)store(at(nodes[i].kids[k])+192,at(nodes[i].kids[k+1]));
  store(0x7000,0x7010);
  store16(0x82901F9C,0);store(0x828FF97C,0);
  ev.clear();
  Context c{};c.r1.u32=0x7f00;c.r3.u32=at(0);c.r4.u32=0x7800;
  sub_DRAW(c,nullptr);
  printf("MK %d",int(nodes.size()));
  for(auto&n:nodes)printf(" %u %u %u %d",n.depth,n.clip,n.alpha,n.parent);
  printf(" -> %s\n",ev.c_str());
 }
 // the world matrix (+32 .. +52) of an instance whose alpha byte is 0 (it returns right after): local +8 .. +28, the
 // parent's world at r4 + 24 .. + 44; floats of mixed size, one case in eight raw bit patterns
 for(uint32_t seed=1;seed<=4000;seed++){
  uint32_t s=seed*2246822519u|1;
  std::memset(objects,0,sizeof objects);
  uint32_t a=at(0),in[12];
  for(int i=0;i<12;i++){
   uint32_t r=rnd(s);
   float f=float(int32_t(r%2000001)-1000000)/float(1u<<(rnd(s)%20));
   uint32_t b;std::memcpy(&b,&f,4);
   if(seed%8==0)b=(rnd(s)&0x807FFFFFu)|(uint32_t(100+rnd(s)%56)<<23);
   in[i]=b;
  }
  for(int i=0;i<6;i++){store(a+8+4*i,in[i]);store(0x7800+24+4*i,in[6+i]);}
  store(a+4,0x7000);*pointer(a+177)=1;*pointer(a+67)=0;store(a+152,0xFFFFFFFFu);
  Context c{};c.r1.u32=0x7f00;c.r3.u32=a;c.r4.u32=0x7800;
  sub_DRAW(c,nullptr);
  printf("WM");for(int i=0;i<12;i++)printf(" %08x",in[i]);printf(" ->");
  for(int i=0;i<6;i++)printf(" %08x",load(a+32+4*i));printf("\n");
 }
 // texture swaps: +148 a texture (F-12960 makes it current, +80 = 1; +80 is otherwise the parent's), no masks
 txmode=true;
 for(uint32_t seed=1;seed<=3000;seed++){
  uint32_t s=seed*3266489917u|1;
  nodes.clear();texs.clear();
  N top{rnd(s)%4,0,64,-1,{}};
  nodes.push_back(top);
  gen(0,0,s);
  for(auto&n:nodes){n.clip=0;n.alpha=rnd(s)%12==0?0:64;}
  nodes[0].alpha=64;
  for(size_t i=0;i<nodes.size();i++)texs.push_back(rnd(s)%4==0?1+rnd(s)%250:0);
  std::memset(objects,0,sizeof objects);
  for(size_t i=0;i<nodes.size();i++){
   uint32_t a=at(int(i));const N&n=nodes[i];
   store(a+4,0x7000);
   store16(a+166,uint16_t(n.depth));store16(a+168,0);
   *pointer(a+67)=uint8_t(n.alpha);*pointer(a+177)=1;*pointer(a+81)=0;
   store(a+148,texs[i]);store(a+152,0xFFFFFFFFu);
   store(a+184,n.parent<0?0:at(n.parent));
   store(a+188,n.kids.empty()?0:at(n.kids[0]));
  }
  for(size_t i=0;i<nodes.size();i++)for(size_t k=0;k+1<nodes[i].kids.size();k++)store(at(nodes[i].kids[k])+192,at(nodes[i].kids[k+1]));
  store(0x7000,0x7010);
  store16(0x82901F9C,0);store(0x828FF97C,0);*pointer(0x7800+72)=0;
  ev.clear();
  Context c{};c.r1.u32=0x7f00;c.r3.u32=at(0);c.r4.u32=0x7800;
  sub_DRAW(c,nullptr);
  printf("TX %d",int(nodes.size()));
  for(size_t i=0;i<nodes.size();i++)printf(" %u %u %u %d",nodes[i].depth,texs[i],nodes[i].alpha,nodes[i].parent);
  printf(" -> %s\n",ev.c_str());
 }}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True, help='build directory')
    ap.add_argument('--out', type=Path, required=True, help='flash_mask_ref.txt')
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    import recomp_closure
    recomp_closure.EXTRA += ('#include <string>\n#include <vector>\nstatic std::string ev;\n'
                             'static bool txmode=false;\n'
                             'static void drawn(uint32_t r4){ev+="D"+std::to_string((r4-8-0x1000)/224);'
                             'if(txmode)ev+=","+std::to_string(*pointer(r4+72));ev+=" ";}\n'
                             '#define REX_CALL_INDIRECT_FUNC(a) drawn(ctx.r4.u32)\n')
    idx = index(a.recomp)
    draw = addr(a.tool, 'F-92070')
    stubs = {'sub_' + addr(a.tool, f): '' for f in STUBS}
    stubs['sub_' + addr(a.tool, 'F-12960')] = 'ev+="T"+std::to_string(ctx.r3.u32)+" ";'
    stubs['sub_' + addr(a.tool, 'F-33110')] = 'ev+="S"+std::to_string(ctx.r3.u32)+","+std::to_string(ctx.r4.u32)+" ";'
    src = MAIN.replace('sub_DRAW', 'sub_' + draw)
    exe = build(idx, ['sub_' + draw], src, a.work, 'draw_mask', stubs=stubs, scratch_end=0x8000)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, check=True)
    a.out.write_text(r.stdout)
    rows = [l for l in r.stdout.splitlines() if l.startswith('MK')]
    print(len(rows), 'cases,', sum(1 for l in rows if 'S20,0' in l), 'with the test turned off,',
          sum(1 for l in rows if 'S16,0' in l), 'with a mask;', r.stdout.count('WM '), 'world matrices;',
          r.stdout.count('TX '), 'texture trees,', sum(1 for l in r.stdout.splitlines() if l.startswith('TX') and ',1 ' in l), 'with a swapped draw')


if __name__ == '__main__':
    main()
