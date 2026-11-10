"""Five tiny accessors/setters run offline (generated code unchanged, scalar mode): F-91694 (returns the float at
(object +896)'s +84 in f1: the rider's steering target T), F-56242 (word +684 = 1), F-90946 (word +684 = 20), F-64975
(returns the byte of the showroom test drive flag), F-19354 (ExitHangoutState's body: byte +1077 = 0, word +152 = 0,
word +308 = the address of F-80049). Each runs on `N` objects filled with garbage; the tool records every byte of the
guest scratch memory the call changed, and the result registers.
Writes <out-dir>/engine_stubs_ref.txt: lines "<name> <case> -> ret <hex> | writes <offset>:<byte> ..." (offsets from the
object, bytes in hex; the F-19354 word +308 is printed whole as 'w308=<hex>')."""
import argparse
import random
import subprocess
import struct
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-91694', 'F-56242', 'F-90946', 'F-64975', 'F-19354']
    fa, fb_, fc, fd, fe = (addr(a.tool, i) for i in ids)
    f80049 = int(addr(a.tool, 'F-80049'), 16)
    nl = chr(10)
    NL = chr(92) + 'n'
    body = r'''
uint32_t g=0x9E3779B9u;
auto next=[&](){g=g*1664525u+1013904223u;return g;};
uint32_t flagaddr=(uint32_t)((-32111<<16)-29345);
for(int n=0;n<300;n++){
  memset(objects,0,sizeof(objects));
  for(int k=0;k<0x600;k++)objects[0x100+k]=(uint8_t)(next()>>13);      // the object at 0x1100, 1 KB of garbage
  // F-91694: object +896 points at a block holding the float at +84
  uint32_t blk=0x1800;store(0x1100+896,blk);
  uint32_t fv=next();if(n%40==7)fv=0x7fc00000;if(n%40==9)fv=0x7f800000;store(blk+84,fv);
  for(int k=0;k<0x200;k++)objects[0x800+k]=(uint8_t)(next()>>11);
  store(0x1100+896,blk);store(blk+84,fv);
  static std::vector<uint8_t> snap;snap.assign(objects,objects+sizeof(objects));
  auto diff=[&](){for(size_t i=0;i<sizeof(objects);i++)if(objects[i]!=snap[i])printf(" %d:%02x",(int)i-0x100,objects[i]);};
  auto reset=[&](){memcpy(objects,snap.data(),sizeof(objects));};
  {Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1100;sub_@A@(ctx,nullptr);float v=(float)ctx.f1.f64;uint32_t u;memcpy(&u,&v,4);printf("get896 %d %08x -> ret %08x | writes",n,fv,u);diff();printf("@NL@");}
  reset();
  {Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1100;sub_@B@(ctx,nullptr);printf("set1 %d -> ret - | writes",n);diff();printf("@NL@");}
  reset();
  {Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1100;sub_@C@(ctx,nullptr);printf("set20 %d -> ret - | writes",n);diff();printf("@NL@");}
  reset();
  {Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1100;sub_@E@(ctx,nullptr);printf("exit %d -> ret - | writes",n);diff();printf(" w308=%08x",load(0x1100+308));printf("@NL@");}
  reset();
  // F-64975 reads a static byte of the image: 0, 1 and other values
  for(int val: {0,1,2,255}){
    uint8_t old=*pointer(flagaddr);*pointer(flagaddr)=(uint8_t)val;
    Context ctx{};ctx.r1.u32=0x2F00;sub_@D@(ctx,nullptr);
    printf("flag %d -> ret %08x | writes",val,ctx.r3.u32);diff();printf("@NL@");
    *pointer(flagaddr)=old;
  }
}
'''
    body = body.replace('@NL@', NL).replace('@A@', fa).replace('@B@', fb_).replace('@C@', fc).replace('@D@', fd).replace('@E@', fe)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + x for x in (fa, fb_, fc, fd, fe)], main_src, a.out_dir, 'engine_stubs')
    out = [x for x in run(exe, a.image).split(nl) if x]
    # the written +308 word equals the address of F-80049
    e = [l for l in out if l.startswith('exit ')]
    assert all(l.endswith('w308=%08x' % f80049) for l in e), e[0]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'engine_stubs_ref.txt').write_text(nl.join(out) + nl)
    print(len(out), 'lines')


if __name__ == '__main__':
    main()
