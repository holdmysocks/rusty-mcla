"""Flash instance helpers run offline with the original's own code (recomp_dispatch): child by depth F-75150 (one-entry cache), play F-38672
(forward to the parent through its vtable), the after-frame play / stop step F-48063, goto F-85946 (frame loop, wrap, child reset), nextFrame
F-41374 / prevFrame F-90378 (target, then the real goto through its vtable slot), glyph page by height F-23783, AVM1 value pop F-39219.
Instances, definition objects and vtables are built in guest memory from the offsets the code shows; the display-only frame pass (F-28424)
and the 'full' pass are host stubs that log the frame number they run for. One line per case, `<TAG> inputs -> outputs` (hex words).
Usage as the other *_reference.py tools:  python flash_inst_reference.py --recomp D:/MCLA-static --image <image.bin> --work <dir> --out <ref.txt>"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = dict(CHILD='F-75150', PLAY='F-38672', SPRITE='F-48063', GOTO='F-85946', NEXT='F-41374', PREV='F-90378', GLYPH='F-23783', POP='F-39219',
           DISPLAY='F-28424')
# the AVM1 value stack the pop reads (the stack pointer word and its 12-byte entries): offsets into the executable image
SP_OFF, STACK_OFF = 0x903710, 0x904E90

MAIN = r'''
#include <sstream>
static uint32_t newobj(uint32_t n){return rt_alloc(n,16);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});image.resize(0x1000000,0);   // the executable's zero-initialised globals (the AVM1 stack) lie beyond the file image
 std::ifstream in(argv[2]);std::string line;
 const uint32_t VT=0x1800,DVT=0x1900;                  // instance vtable, definition-object vtable (small fixed scratch words)
 while(std::getline(in,line)){std::istringstream ls(line);rt_reset();std::string tag;ls>>tag;
  for(int i=0;i<0x600;i++)*pointer(0x1000+i)=0;
  store(VT+4,ADDR_GOTO);store(VT+8,ADDR_NEXT);store(VT+16,ADDR_PLAY);store(DVT+16,0x7F000021);
  auto rd32=[&](){uint32_t v;ls>>std::hex>>v;return v;};
  printf("%s",line.c_str());
  try{
  if(tag=="CH"){  // n d0..dn-1 cache(ffffffff none) q
   uint32_t n=rd32();std::vector<uint32_t> d(n),nd(n);uint32_t inst=newobj(0x300);
   for(uint32_t i=0;i<n;i++){d[i]=rd32();nd[i]=newobj(0x300);store16(nd[i]+166,d[i]);}
   uint32_t cache=rd32(),q=rd32();
   for(uint32_t i=0;i<n;i++)store(nd[i]+192,i+1<n?nd[i+1]:0);
   store(inst+188,n?nd[0]:0);
   if(cache==0xffffffff){store16(inst+200,0xFFFF);store(inst+204,0);}else{store16(inst+200,d[cache]);store(inst+204,nd[cache]);}
   Context c=fresh();c.r3.u32=inst;c.r4.u32=q;sub_CHILD(c,nullptr);
   int idx=-1;for(uint32_t i=0;i<n;i++)if(nd[i]==c.r3.u32)idx=int(i);
   if(c.r3.u32&&idx<0)throw std::runtime_error("unknown node");
   printf(" -> %d %x",idx,load16(inst+200));
  }else if(tag=="PL"){  // n t0..tn-1 : element 0 is called, element i+1 is the parent of element i; t = definition type byte
   uint32_t n=rd32();std::vector<uint32_t> in_(n);std::vector<uint32_t> df(n);
   for(uint32_t i=0;i<n;i++){in_[i]=newobj(0x300);df[i]=newobj(0x40);}
   for(uint32_t i=0;i<n;i++){uint32_t t=rd32();*pointer(df[i]+8)=uint8_t(t);store(in_[i]+0,VT);store(in_[i]+4,df[i]);store(in_[i]+184,i+1<n?in_[i+1]:0);}
   Context c=fresh();c.r3.u32=in_[0];sub_PLAY(c,nullptr);
   printf(" ->");for(uint32_t i=0;i<n;i++)printf(" %x",*pointer(in_[i]+173));
  }else if(tag=="SP"){  // playing play_req stop_req
   uint32_t inst=newobj(0x300),df=newobj(0x40);store(df,DVT);store16(df+16,2);store(inst+4,df);uint32_t p=rd32(),r=rd32(),s=rd32();store(inst,VT);*pointer(inst+172)=p;*pointer(inst+173)=r;*pointer(inst+174)=s;
   Context c=fresh();c.r3.u32=inst;sub_SPRITE(c,nullptr);
   printf(" -> %x %x %x %x",*pointer(inst+172),*pointer(inst+173),*pointer(inst+174),load16(inst+164));
  }else if(tag=="GO"||tag=="NX"||tag=="PV"){  // GO n cur target flag nkids | NX n cur nkids | PV n cur nkids
   uint32_t n=rd32(),cur=rd32(),target=0,flag=0;if(tag=="GO"){target=rd32();flag=rd32();}uint32_t nk=rd32();
   uint32_t inst=newobj(0x300),df=newobj(0x40);store(inst,VT);store(inst+4,df);store(df,DVT);store16(df+16,n);store16(inst+164,cur);
   std::vector<uint32_t> kids;for(uint32_t i=0;i<nk;i++){uint32_t k=newobj(0x300);*pointer(k+177)=1;store(k+180,0x1234+i);kids.push_back(k);}
   for(uint32_t i=0;i<nk;i++)store(kids[i]+192,i+1<nk?kids[i+1]:0);store(inst+188,nk?kids[0]:0);
   Context c=fresh();c.r3.u32=inst;
   if(tag=="GO"){c.r4.u32=target;c.r5.u32=flag;sub_GOTO(c,nullptr);}
   else if(tag=="NX"){c.r4.u32=0;sub_NEXT(c,nullptr);}else{c.r4.u32=0;sub_PREV(c,nullptr);}
   printf(" -> %x |",load16(inst+164));uint32_t cnt=load(0x1000);for(uint32_t i=0;i<cnt;i++)printf(" %c%x",load(0x1004+8*i)==1?'d':'f',load(0x1008+8*i));
   uint32_t cl=0;for(auto k:kids)if(*pointer(k+177)==0&&load(k+180)==0)cl++;printf(" | %x %x",cl,*pointer(inst+172));
  }else if(tag=="GL"){  // count h0.. q   (count is a signed 16 bit word, q a sign-extended height)
   uint32_t cnt=rd32();uint32_t inst=newobj(0x300);store16(inst+152,cnt);std::vector<uint32_t> pg;
   for(uint32_t i=0;i<(cnt&0x8000?0u:cnt);i++){uint32_t h=rd32();uint32_t p=newobj(0x40);store16(p+8,h);pg.push_back(p);store(inst+164+4*i,p);}
   uint32_t q=rd32();Context c=fresh();c.r3.u32=inst;c.r4.u32=q;sub_GLYPH(c,nullptr);
   int idx=-1;for(size_t i=0;i<pg.size();i++)if(pg[i]==c.r3.u32){idx=int(i);break;}
   printf(" -> %d",idx);
  }else if(tag=="PO"){  // ops: P w0 type | Q    (a pop prints w0 and the type)
   uint32_t sp=IMG_BASE_+SP_OFF_,st=IMG_BASE_+STACK_OFF_;store(sp,0);std::string op;
   printf(" ->");
   while(ls>>op){
    if(op=="P"){uint32_t w=rd32(),t=rd32();uint32_t n=load(sp);store(st+12*n,w);store(st+12*n+4,0);store(st+12*n+8,t);store(sp,n+1);}
    else{Context c=fresh();uint32_t out=newobj(16);c.r3.u32=out;sub_POP(c,nullptr);printf(" %x:%x",load(out),load(out+8));}
   }
  }
  }catch(std::exception&e){printf(" TRAP %s",e.what());}
  printf("\n");
 }
}
'''

def logw(tag, v):
    """the log: word count at scratch 0x1000, then (tag, frame) pairs (inline: stubs and fakes are compiled before MAIN)"""
    return '{uint32_t n=load(0x1000);store(0x1004+8*n,%d);store(0x1008+8*n,%s);store(0x1000,n+1);}' % (tag, v)


FAKES = {
    0x7F000021: logw(2, 'load16(ctx.r4.u32+164)'),     # definition vtable slot 4: the full frame pass
}


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def ok(x):
    return not 0x82000000 <= x < 0x83000000


def cases():
    r = random.Random(2718)
    out = []
    # child by depth: sorted distinct depths, cache empty or on one node, queries on and off the list
    for _ in range(400):
        n = r.choice([0, 1, 2, 3, 5, 9])
        ds = sorted(r.sample([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 100, 1000, 0x7FFF, 0x8000, 0xFFFE], n))
        cache = 'ffffffff' if (not n or r.random() < 0.4) else '%x' % r.randrange(n)
        for q in r.sample(ds, min(3, len(ds))) + [r.choice([0, 1, 5, 6, 11, 99, 0xFFFE, 0xFFFF, 0x10000 + r.choice(ds or [3])])]:
            out.append('CH %x %s %s %x' % (n, ' '.join('%x' % d for d in ds), cache, q) if n else 'CH 0 %s %x' % (cache, q))
    # a child at the last depth
    for cache in ('ffffffff', '0', '1'):
        for q in (0xFFFF, 0xFFFE):
            out.append('CH 2 5 ffff %s %x' % (cache, q))
    # play: chains of sprites (type 2) and non-sprites
    for _ in range(120):
        n = r.choice([1, 2, 3, 4])
        out.append('PL %x %s' % (n, ' '.join('%x' % r.choice([2, 2, 1, 3, 5]) for _ in range(n))))
    for p in (0, 1):
        for q in (0, 1):
            for s in (0, 1):
                out.append('SP %x %x %x' % (p, q, s))
    # goto, nextFrame, prevFrame
    for _ in range(500):
        n = r.choice([0, 1, 2, 3, 4, 6, 10])
        cur = r.randrange(n) if n else 0
        out.append('GO %x %x %x %x %x' % (n, cur, r.randrange(n + 2), r.choice([0, 0, 1]), r.choice([0, 2])))
    for _ in range(100):
        n = r.choice([0, 1, 2, 3, 6, 10])
        cur = r.randrange(n) if n else 0
        out.append('NX %x %x %x' % (n, cur, r.choice([0, 2])))
        out.append('PV %x %x %x' % (n, cur, r.choice([0, 2])))
    # glyph page by height
    for _ in range(300):
        cnt = r.choice([0, 1, 2, 4, 8, 0xFFFF, 0x8000])
        hs = [r.choice([8, 10, 12, 16, 24, 32, 0xFFF0, 0x7FFF, 0x8000, 0]) for _ in range(cnt if cnt < 0x8000 else 0)]
        q = r.choice(hs + [12, 0xFFF0, 20, 0xFFFFFFF0, 0xFFFF8000, 0x7FFF, 0, 0x10008]) if hs else r.choice([12, 0])
        out.append('GL %x %s %x' % (cnt, ' '.join('%x' % h for h in hs), q & 0xFFFFFFFF))
    # AVM1 value pop: pushes and pops of ints, floats, booleans, with underflows
    for _ in range(200):
        ops = []
        for _ in range(r.choice([3, 10, 40])):
            if r.random() < 0.55:
                t = r.choice([3, 3, 4, 2])
                w = r.getrandbits(32)
                while not ok(w):
                    w = r.getrandbits(32)
                ops.append('P %x %x' % (w, t))
            else:
                ops.append('Q')
        out.append('PO ' + ' '.join(ops))
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
    addr = {k: fid_addr(a.tool, v) for k, v in IDS.items()}
    m = MAIN.replace('SP_OFF_', hex(SP_OFF)).replace('STACK_OFF_', hex(STACK_OFF)).replace('IMG_BASE_', hex(rd.IMAGE_BASE) + 'u')
    for k in ('GOTO', 'PLAY', 'NEXT'):
        m = m.replace('ADDR_' + k, '0x' + addr[k])
    for k in IDS:
        m = m.replace('sub_' + k, 'sub_' + addr[k])
    names = ['sub_' + addr[k] for k in IDS if k != 'DISPLAY']
    stubs = {'sub_' + addr['DISPLAY']: logw(1, 'load16(ctx.r4.u32+164)')}
    exe = rd.build(idx, names, m, a.work, 'flash_inst', image=img, stubs=stubs,
                   targets=[int(addr['GOTO'], 16), int(addr['PLAY'], 16), int(addr['NEXT'], 16)], fakes=FAKES)
    cf = a.work / 'flash_inst_cases.txt'
    cf.write_text('\n'.join(cases()) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    a.out.write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'cases;', r.stdout.count('TRAP'), 'traps')


if __name__ == '__main__':
    main()
