"""Flash host accessors for the movie's global variables run offline with the original's own code (recomp_dispatch): the slot lookup / creation by
name F-43980 and the typed setters (int F-59146, float F-66095, bool F-11028, string F-16600) and the int getter F-94547 with its value conversion.
A context object is built in guest memory (+128 the global object whose member table is at +4, +136 the running flag; the global pointer the
creation path inserts into is the image word the original keeps for the current context). One line per case:
  <F0|F1> <ops..> -> <results..>   ops: SI name int | SF name floatbits | SB name 0/1 | SS name text | GI name   (results: s, or  g<found>:<value>)
F0 / F1: the running flag (+136) clear / set (clear makes the creation path switch the library's current-context globals first).
Usage as the other *_reference.py tools:  python flash_global_reference.py --recomp D:/MCLA-static --image <image.bin> --work <dir> --out <ref.txt>"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = dict(SETI='F-59146', SETF='F-66095', SETB='F-11028', SETS='F-16600', GETI='F-94547')
CUR_GLOBAL_OFF = 0x8FF988          # the image word holding the current context's global object (offset into the image)

MAIN = r'''
#include <sstream>
static uint32_t newobj(uint32_t n){return rt_alloc(n,16);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});image.resize(0x1000000,0);
 std::ifstream in(argv[2]);std::string line;
 while(std::getline(in,line)){std::istringstream ls(line);rt_reset();std::string flag,op;ls>>flag;
  printf("%s",line.c_str());
  try{
   uint32_t ct=newobj(0x200),g=newobj(0x80),nm=newobj(0x100),tx=newobj(0x400),out=newobj(16);
   store(ct+128,g);*pointer(ct+136)=flag=="F1"?1:0;store(IMG_BASE_+CUR_OFF_,g);
   std::vector<std::string> res;
   while(ls>>op){
    std::string name,text;uint32_t v=0;ls>>name;
    if(op=="SI"||op=="SF"||op=="SB")ls>>std::hex>>v;else if(op=="SS")ls>>text;
    for(size_t i=0;i<=name.size();i++)*pointer(nm+i)=i<name.size()?uint8_t(name[i]):0;
    tx=newobj(text.size()+16);for(size_t i=0;i<=text.size();i++)*pointer(tx+i)=i<text.size()?uint8_t(text[i]):0;   // a string value points at the caller's text: one buffer per call
    Context c=fresh();c.r3.u32=ct;c.r4.u32=nm;
    if(op=="SI"){c.r5.u32=v;sub_SETI(c,nullptr);res.push_back("s");}
    else if(op=="SF"){float f;std::memcpy(&f,&v,4);c.f1.f64=f;sub_SETF(c,nullptr);res.push_back("s");}
    else if(op=="SB"){c.r5.u32=v;sub_SETB(c,nullptr);res.push_back("s");}
    else if(op=="SS"){c.r5.u32=tx;sub_SETS(c,nullptr);res.push_back("s");}
    else {c.r5.u32=out;store(out,0xdeadbeef);sub_GETI(c,nullptr);char b[40];snprintf(b,sizeof b,"g%x:%x",c.r3.u32&1,c.r3.u32&1?load(out):0);res.push_back(b);}
   }
   printf(" ->");for(auto&s:res)printf(" %s",s.c_str());
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
    r = random.Random(99)
    names = ['speed', 'Speed', 'rpm', 'd_Speedvalue', 'online', 'lang', 'zone_1', 'x', 'ABC', 'title_text']
    ints = [0, 1, -1, 7, 255, 65536, 2**31 - 1, -2**31, 123456]
    floats = [0.0, 1.0, -1.0, 0.5, 2.5, -2.5, 3.99, -3.99, 1e9, -1e9, 2147483520.0, 3e9, -3e9, 1e20, float('inf'), float('-inf'), float('nan'), 123.456]
    texts = ['0', '1', '42', '-17', '3.75', 'abc', '12abc', 'x', '', '2147483648', '-2147483649', '1e3', '0x10', ' 5', '007']
    out = []
    for flag in ('F0', 'F1'):
        for _ in range(250):
            ops = []
            kind = {}                    # last type set per name: a get of a string value runs the C library's strtol (locale tables set up at start-up): left out
            for _ in range(r.choice([3, 8, 20])):
                n = r.choice(names)
                c = r.random()
                k = n.lower()
                if c < 0.25:
                    ops.append('SI %s %x' % (n, r.choice(ints + [r.getrandbits(31)]) & 0xFFFFFFFF))
                    kind[k] = 'i'
                elif c < 0.4:
                    ops.append('SF %s %x' % (n, bits(r.choice(floats + [r.uniform(-1000, 1000)]))))
                    kind[k] = 'f'
                elif c < 0.5:
                    ops.append('SB %s %x' % (n, r.choice([0, 1])))
                    kind[k] = 'b'
                elif c < 0.6:
                    ops.append('SS %s %s' % (n, r.choice(texts) or '_'))
                    kind[k] = 's'
                elif kind.get(k) != 's':
                    ops.append('GI %s' % n)
            out.append(flag + ' ' + ' '.join(ops))
    return [c for c in out if not any(0x82000000 <= int(w, 16) < 0x83000000 for w in c.split() if len(w) == 8 and all(ch in '0123456789abcdef' for ch in w))]


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
    addr = {k: fid_addr(a.tool, v) for k, v in IDS.items()}
    m = MAIN.replace('IMG_BASE_', hex(rd.IMAGE_BASE) + 'u').replace('CUR_OFF_', hex(CUR_GLOBAL_OFF))
    for k in IDS:
        m = m.replace('sub_' + k, 'sub_' + addr[k])
    exe = rd.build(idx, ['sub_' + v for v in addr.values()], m, a.work, 'flash_global', image=img)
    cf = a.work / 'flash_global_cases.txt'
    cf.write_text('\n'.join(cases()) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    a.out.write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'cases;', r.stdout.count('TRAP'), 'traps')


if __name__ == '__main__':
    main()
