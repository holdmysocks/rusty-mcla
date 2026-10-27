"""Flash member hash table (find F-86938, insert F-86805, erase F-14453, set by name record F-99291, set by C string F-52153, bucket growth
F-69872, with the allocator and the size helpers underneath) run offline on random operation sequences with the original's own code
(recomp_dispatch). One line per sequence:  SEQ <ops..> -> <results..>  where an op is  S key n | T text n | G key | U text | E text (D key, a raw-key erase, is supported by the harness but the port has only erase by name)
and its result  s | t | g<found:n> / g- | d0 / d1 .  Usage as the other *_reference.py tools:
  python flash_table_reference.py --recomp D:/MCLA-static --image <image.bin> --work <dir> --out <ref.txt>"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd

FIND, ERASE, SET_REC, SET_STR, HASH = 'F-86938', 'F-14453', 'F-99291', 'F-52153', 'F-58929'
MAIN = r'''
#include <sstream>
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string line;
 while(std::getline(in,line)){std::istringstream ls(line);rt_reset();
  for(int i=0;i<16;i++)*pointer(0x1000+i)=0;
  std::string op,res;printf("SEQ");std::vector<std::string> outs;
  while(ls>>op){
   if(op=="S"||op=="T"||op=="G"||op=="D"||op=="U"||op=="E"){
    std::string txt;uint32_t key=0,n=0;
    if(op=="T"){ls>>txt>>std::hex>>n;}else if(op=="U"||op=="E"){ls>>txt;}else{ls>>std::hex>>key;if(op=="S")ls>>n;}
    printf(" %s",op.c_str());if(op=="T")printf(" %s %x",txt.c_str(),n);else if(op=="U"||op=="E")printf(" %s",txt.c_str());else{printf(" %x",key);if(op=="S")printf(" %x",n);}
    char b[64];
    try{
     store(0x1100,0);store(0x1104,key);store(0x1108,1);          // name record: hash in +4, type 1
     store(0x1200,n);store(0x1204,0);store(0x1208,3);            // value record: int
     for(size_t i=0;i<=txt.size();i++)*pointer(0x1300+i)=i<txt.size()?uint8_t(txt[i]):0;
     Context c=fresh();
     if(op=="S"){c.r3.u32=0x1000;c.r4.u32=0x1100;c.r5.u32=0x1200;sub_SETREC(c,nullptr);outs.push_back("s");}
     else if(op=="T"){c.r3.u32=0x1000;c.r4.u32=0x1300;c.r5.u32=0x1200;sub_SETSTR(c,nullptr);outs.push_back("t");}
     else if(op=="G"||op=="U"){
       if(op=="U"){Context h=fresh();h.r3.u32=0x1300;h.r4.u32=0;sub_HASH(h,nullptr);key=h.r3.u32;}
       store(0x1400,key);Context d=fresh();d.r3.u32=0x1000;d.r4.u32=0x1400;sub_FIND(d,nullptr);
       if(d.r3.u32){snprintf(b,sizeof b,"g%x:%x:%x",load(d.r3.u32),load(d.r3.u32+4),load(d.r3.u32+8));outs.push_back(b);}else outs.push_back("g-");}
     else {if(op=="E"){Context h=fresh();h.r3.u32=0x1300;h.r4.u32=0;sub_HASH(h,nullptr);key=h.r3.u32;}
       store(0x1400,key);c.r3.u32=0x1000;c.r4.u32=0x1400;sub_ERASE(c,nullptr);outs.push_back(c.r3.u32?"d1":"d0");}
    }catch(std::exception&e){outs.push_back(std::string("TRAP:")+e.what());}
   }
  }
  printf(" ->");for(auto&s:outs)printf(" %s",s.c_str());printf("\n");
 }
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def ok(x):
    return not 0x82000000 <= x < 0x83000000


def cases():
    r = random.Random(4242)
    out = []
    names = ['a', 'b', 'x', 'y', '_x', '_y', 'name', 'Name', 'Score_1', 'text_field', 'LevelUp', 'tf0', 'tf1', 'tf2', 'btn_ok', 'a_very_long_member_name_for_a_clip']
    for seq in range(260):
        n_keys = r.choice([2, 5, 12, 40, 120])
        pool = []
        for _ in range(n_keys):
            if pool and r.random() < 0.25:
                pool.append(pool[-1] + r.choice([10, 20, 30, 1]))        # same bucket at 10 buckets
            else:
                pool.append(r.getrandbits(32))
        pool = [k for k in pool if ok(k)] or [1]
        ops = []
        for _ in range(r.choice([5, 30, 100, 300])):
            k = r.choice(pool)
            c = r.random()
            if c < 0.45:
                ops.append('S %x %x' % (k, r.getrandbits(31) | 0 if r.random() < .9 else 0))
            elif c < 0.6:
                nm = r.choice(names)
                ops.append('T %s %x' % (nm, r.getrandbits(31)))
                ops.append('U %s' % r.choice(names))
            elif c < 0.85:
                ops.append('G %x' % k)
            else:
                ops.append('G %x' % k)
        out.append(' '.join(ops))
    # name based: set by string, then read back by the same string's own key (through a G of the hash computed by the original: S/T mix)
    for seq in range(40):
        ops = []
        for _ in range(r.choice([5, 20])):
            ops.append('T %s %x' % (r.choice(names), r.getrandbits(31)))
            ops.append('U %s' % r.choice(names))
        out.append(' '.join(ops))
    nm2 = ['n%d' % i for i in range(150)] + ['Clip_%d' % i for i in range(30)] + ['TF_%c%d' % (chr(65 + i % 26), i) for i in range(30)]
    for seq in range(60):
        pool = r.sample(nm2, r.choice([3, 10, 40, 100, 200]))
        ops = []
        for _ in range(r.choice([20, 80, 300])):
            c = r.random()
            nm = r.choice(pool)
            ops.append('T %s %x' % (nm, r.getrandbits(31)) if c < 0.4 else 'U %s' % nm if c < 0.75 else 'E %s' % nm)
        out.append(' '.join(ops))
    # key 0, key 0xffffffff, growth to many buckets
    out.append(' '.join(['S %x %x' % (k, k & 0xffff) for k in range(0, 700)] + ['G %x' % k for k in range(0, 700, 7)] + ['G %x' % k for k in range(0, 700, 5)]))
    out.append('S 0 1 S ffffffff 2 G 0 G ffffffff S 0 5 G 0 G ffffffff')
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
    f = {k: 'sub_' + fid_addr(a.tool, v) for k, v in (('FIND', FIND), ('ERASE', ERASE), ('SETREC', SET_REC), ('SETSTR', SET_STR), ('HASH', HASH))}
    m = MAIN
    for k, v in f.items():
        m = m.replace('sub_' + k, v)
    exe = rd.build(idx, list(f.values()), m, a.work, 'flash_table', image=img)
    cf = a.work / 'flash_table_cases.txt'
    cf.write_text('\n'.join(cases()) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    a.out.write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'sequences;', r.stdout.count('TRAP'), 'traps')


if __name__ == '__main__':
    main()
