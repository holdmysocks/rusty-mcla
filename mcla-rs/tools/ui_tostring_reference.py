"""Value to string (F-24664: string copy, "true"/"false", int "%d", float "%0.f" / "%f") run offline with the original's own C library
formatting underneath (recomp_dispatch: the per-thread data, the allocator and the locks around it are the runtime's, the formatting code is the
original's). One line per case:  TS type payload hexbytes -> text-as-hex
(type 0 is the string: hexbytes =  its 12-byte chunks, NUL padded, else '-'). Usage as the other *_reference.py tools."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

TO_STRING = 'F-24664'
# indirect targets the C library's float formatting reaches through its function pointer table (found by running: the trap names the address)
CRT_FLOAT_INIT = 'F-80266'      # the C library's float formatting table set-up (the table in the image holds a 'not loaded' stub until it runs)
CRT_TARGETS = []
MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);uint32_t ty,p;std::string hx;{Context ci=fresh();sub_FPINIT(ci,nullptr);}
 while(in>>std::hex>>ty>>p>>hx){rt_reset();
  if(hx!="-"){size_t n=hx.size()/24;   // a string value is a chain of 16-byte nodes: 12 text bytes, then the next node (0 = last)
   for(size_t k=0;k<n;k++){for(int i=0;i<12;i++){unsigned b;sscanf(hx.c_str()+24*k+2*i,"%2x",&b);*pointer(0x1100+16*k+i)=uint8_t(b);}
    store(0x1100+16*k+12,k+1<n?uint32_t(0x1100+16*(k+1)):0);}p=n?0x1100:0;}
  store(0x1000,p);store(0x1004,0);store(0x1008,ty);
  Context c=fresh();c.r3.u32=0x1000;c.r4.u32=0x1800;c.r5.u32=256;
  try{sub_TOSTR(c,nullptr);std::string s;for(uint32_t a=c.r3.u32;*pointer(a);a++)s+=char(*pointer(a));
   printf("TS %x %x %s ->",ty,p,hx.c_str());for(unsigned char ch:s)printf(" %02x",ch);printf("\n");}
  catch(std::exception&e){printf("TS %x %x %s -> TRAP %s\n",ty,p,hx.c_str(),e.what());}
 }
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases():
    r = random.Random(77)
    out = []
    for ty in range(1, 8):
        out.append((ty, 0, '-'))
        out.append((ty, 0x1234, '-'))     # type 1 and 5-7: any payload gives the empty text
    ints = [0, 1, -1, 7, 255, 256, 99999, -99999, 2**31 - 1, -2**31, 123456789, -123456789] + [r.randint(-2**31, 2**31 - 1) for _ in range(300)]
    out += [(3, i & 0xFFFFFFFF, '-') for i in ints]
    fl = [0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 0.1, 1.5, 2.5, 100.0, 1e6, 123456.0, 16777216.0, 2147483520.0, 2147483648.0, 1e10, -1e10, 3e38,
          1e-5, 1e-7, 123.456, -987.654321, float('inf'), float('-inf'), float('nan')]
    fl += [r.uniform(-1000, 1000) for _ in range(150)] + [float(r.randint(-100000, 100000)) for _ in range(100)] + \
          [struct.unpack('>f', struct.pack('>I', r.getrandbits(32)))[0] for _ in range(150)]
    out += [(4, bits(x), '-') for x in fl]
    # non-finite floats by bit pattern: quiet / signalling NaNs of both signs and payloads
    out += [(4, b, '-') for b in (0x7fc00001, 0xffc00000, 0xffc00001, 0x7fa00000, 0xffa00000, 0x7fffffff, 0xffffffff, 0x7f800001)]
    out += [(2, b, '-') for b in (0, 1, 2, 0x80, 0xFF000000, 0x01000000, 0x00FFFFFF)]
    for s in ('', 'a', 'hello', 'Dialog_Title', 'x' * 100, 'with space', 'exactly12chr', 'thirteen_char', 'a' * 24, 'b' * 251):
        raw = s.encode() + bytes(1)
        raw += bytes(1) * (-len(raw) % 12)               # the last chunk is padded with NULs
        out.append((0, 0, raw.hex()))
    # (payloads in the image's address range would read as a citation of the original in a tracked data file: left out)
    return [c for c in out if not 0x82000000 <= c[1] < 0x83000000]


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
    fn = 'sub_' + fid_addr(a.tool, TO_STRING)
    init = 'sub_' + fid_addr(a.tool, CRT_FLOAT_INIT)
    tg = sorted(set(rd.stored_pointers(idx, init).values()))      # the table's entries: the formatting code calls them indirectly
    exe = rd.build(idx, [fn, 'sub_' + fid_addr(a.tool, CRT_FLOAT_INIT)], MAIN.replace('sub_TOSTR', fn).replace('sub_FPINIT', 'sub_' + fid_addr(a.tool, CRT_FLOAT_INIT)), a.work, 'ui_tostring', image=img, targets=tg)
    cs = cases()
    cf = a.work / 'ui_tostring_cases.txt'
    cf.write_text('\n'.join('%x %x %s' % c for c in cs) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    a.out.write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'cases;', r.stdout.count('TRAP'), 'traps')


if __name__ == '__main__':
    main()
