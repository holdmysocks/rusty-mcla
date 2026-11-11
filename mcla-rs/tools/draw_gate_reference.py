"""The Flash draw traversal F-92070's placed-byte gate run offline (generated code unchanged): an instance whose byte +177
is 0 is not drawn - the function returns before it reads or writes anything else, calls nothing and draws none of its
children; any other value goes on to the world matrix (stores to +32 / +36 ..) and the draw calls.

Every callee (F-12960, F-33110, F-54245, F-57014, F-74625, F-90239, F-71617) and every indirect call is a stub that
counts; an instance with no children (+188 = 0) keeps the recursion out. The instance (0x1000, 224 bytes) is random
bytes with +177 set to the case's value; the parent's matrix (0x1800) random floats.

Cases: +177 = 0 .. 255, each with 8 random instances (2,048). Writes <out>: "gate seed -> drawn" per case (drawn = 1 when
the function wrote to the instance, called anything or stopped on a guest access past the scratch memory; 0 when it
returned with the instance and the parent unchanged and nothing called). The port: `Inst::placed` (draw.rs::children
skips a child whose mark is clear)."""
import argparse
import subprocess
from pathlib import Path
from recomp_closure import index, build

CALLEES = ['F-12960', 'F-33110', 'F-54245', 'F-57014', 'F-74625', 'F-90239', 'F-71617']


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
static uint32_t rnd(uint32_t&s){s^=s<<13;s^=s>>17;s^=s<<5;return s;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(uint32_t gate=0;gate<256;gate++)for(uint32_t k=0;k<8;k++){
  uint32_t seed=gate*8+k+1,s=seed*2654435761u|1;
  std::memset(objects,0,sizeof objects);
  for(int i=0;i<224;i++)*pointer(0x1000+i)=uint8_t(rnd(s));
  *pointer(0x1000+177)=uint8_t(gate);
  store(0x1000+188,0);
  for(int i=0;i<16;i++){float f=float(int32_t(rnd(s)%2001)-1000)/100.0f;putf(0x1800+4*i,f);}
  uint8_t before[0x2000];std::memcpy(before,objects,sizeof objects);
  calls=0;int stopped=0;
  Context c{};c.r1.u32=0x2f00;c.r3.u32=0x1000;c.r4.u32=0x1800;
  try{sub_DRAW(c,nullptr);}catch(...){stopped=1;}
  // (the stack below 0x2f00 is the callee's own frame: left out of the comparison)
  int changed=std::memcmp(before,objects,0x1c00)!=0;
  printf("%x %x -> %d\n",gate,seed,(changed||calls||stopped)?1:0);
 }}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True, help='build directory')
    ap.add_argument('--out', type=Path, required=True, help='draw_gate_ref.txt')
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    import recomp_closure
    recomp_closure.EXTRA += 'static int calls;\nstatic void ind(){calls++;}\n#define REX_CALL_INDIRECT_FUNC(a) ind()\n'
    idx = index(a.recomp)
    draw = addr(a.tool, 'F-92070')
    stubs = {'sub_' + addr(a.tool, f): 'calls++;' for f in CALLEES}
    src = MAIN.replace('sub_DRAW', 'sub_' + draw)
    exe = build(idx, ['sub_' + draw], src, a.work, 'draw_gate', stubs=stubs)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, check=True)
    a.out.write_text(r.stdout)
    rows = [l.split() for l in r.stdout.splitlines()]
    drawn = sum(1 for x in rows if x[3] == '1')
    print(len(rows), 'cases,', drawn, 'drawn')


if __name__ == '__main__':
    main()
