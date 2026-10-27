"""Strict equality of two script values of the same type (F-46400, run by the interpreter's StrictEquals after it has compared the type words)
with the original's own code offline (recomp_dispatch): by the receiver's type a string compare (the C string type 1, the chunk chain type 0
flattened by F-39219's neighbour helper), the first byte of a boolean, an integer, a float compare, a pointer compare for an object.
A value is 12 bytes: payload, aux, type. One line per case:  EQ <type> <payload> <text|-> <payload2> <text2|-> -> <0|1>
(both values have the type given; text: the string for types 0 and 1, whose payload is then built here).
Usage as the other *_reference.py tools:  python flash_equals_reference.py --recomp D:/MCLA-static --image <image.bin> --work <dir> --out <ref.txt>"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

EQ = 'F-46400'
CRT_FLOAT_INIT = 'F-80266'      # the C library's float formatting table set-up (as in ui_tostring_reference.py)

MAIN = r'''
#include <sstream>
static uint32_t put_value(uint32_t at,uint32_t ty,uint32_t payload,const std::string&txt){
 if(ty==1){uint32_t b=rt_alloc(txt.size()+16,16);for(size_t i=0;i<=txt.size();i++)*pointer(b+i)=i<txt.size()?uint8_t(txt[i]):0;payload=b;}
 if(ty==0){size_t n=(txt.size()+1+11)/12;uint32_t b=rt_alloc(16*n,16);   // chain of 16-byte nodes: 12 text bytes (NUL padded), then the next node
  for(size_t k=0;k<n;k++){for(int i=0;i<12;i++){size_t j=12*k+i;*pointer(b+16*k+i)=j<txt.size()?uint8_t(txt[j]):0;}store(b+16*k+12,k+1<n?b+16*(k+1):0);}payload=b;}
 store(at,payload);store(at+4,0);store(at+8,ty);return payload;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});image.resize(0x1000000,0);
 std::ifstream in(argv[2]);std::string line;{Context ci=fresh();sub_FPINIT(ci,nullptr);}
 while(std::getline(in,line)){std::istringstream ls(line);rt_reset();std::string tag,t1,t2;uint32_t ty,p1,p2;ls>>tag>>std::hex>>ty>>p1>>t1>>p2>>t2;
  if(t1=="-")t1="";if(t2=="-")t2="";
  printf("%s",line.c_str());
  try{
   uint32_t a=rt_alloc(16,16),b=rt_alloc(16,16);put_value(a,ty,p1,t1);put_value(b,ty,p2,t2);
   Context c=fresh();c.r3.u32=a;c.r4.u32=b;sub_EQ(c,nullptr);printf(" -> %x",c.r3.u32&0xff);
  }catch(std::exception&e){printf(" TRAP %s",e.what());}
  printf("\n");
 }
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases():
    r = random.Random(31)
    out = []
    words = ['', 'a', 'A', 'abc', 'abd', 'ab', 'abcd', 'Hello', 'hello', 'x' * 12, 'x' * 13, 'x' * 24, 'y' * 11, 'text_field', 'eleven_char']
    for ty in (0, 1):
        for a in words:
            for b in words:
                out.append((ty, 0, a or '-', 0, b or '-'))
    ints = [0, 1, 2, 255, 0xFFFFFFFF, 0x80000000, 0x7FFFFFFF, 12345]
    for a in ints:
        for b in ints:
            out.append((3, a, '-', b, '-'))
    fl = [0.0, -0.0, 1.0, -1.0, 2.5, float('inf'), float('-inf'), float('nan'), 1e-30]
    for a in fl:
        for b in fl:
            out.append((4, bits(a), '-', bits(b), '-'))
    out.append((4, 0x7fc00001, '-', 0x7fc00001, '-'))
    out.append((4, 0x3f800000, '-', 0x3f800001, '-'))
    # a boolean is its first byte
    for a in (0, 1, 2, 0x80, 0xFF):
        for b in (0, 1, 2, 0x80, 0xFF):
            out.append((2, a << 24, '-', b << 24, '-'))
    out.append((2, 0x01000000, '-', 0x01FFFFFF, '-'))      # only the first byte counts
    # objects: the pointer; undefined, null-object and the unnamed type 6
    for ty in (5, 6, 7):
        for a in (0, 1, 4, 100):
            for b in (0, 1, 4, 100):
                out.append((ty, a, '-', b, '-'))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fn = 'sub_' + fid_addr(a.tool, EQ)
    init = 'sub_' + fid_addr(a.tool, CRT_FLOAT_INIT)
    tg = sorted(set(rd.stored_pointers(idx, init).values()))
    m = MAIN.replace('sub_EQ', fn).replace('sub_FPINIT', init)
    exe = rd.build(idx, [fn, init], m, a.work, 'flash_equals', image=img, targets=tg)
    cf = a.work / 'flash_equals_cases.txt'
    cf.write_text('\n'.join('EQ %x %x %s %x %s' % c for c in cases()) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    a.out.write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'cases;', r.stdout.count('TRAP'), 'traps')


if __name__ == '__main__':
    main()
