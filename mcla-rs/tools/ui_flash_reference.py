"""The original's Flash runtime helpers run offline (ui_closure / recomp_closure): unchanged generated code on a flat guest memory.
F-#####s by address through `mcla-tool registry id`. Writes ui_flash_ref.txt, one case per line, numbers in hex:
  CX  local[8] parent[8] tint -> out[8]                      world colour transform (combine_cx)
  GR  radial n ratios.. colors.. m[6] x y -> argb           gradient colour (gradient_color)
  FV  grad(0/1) solid cx[8] alpha + GR fields -> argb        fill colour of a vertex (fill_vertex)
  RM  angle -> m[6]                                          rotation matrix from degrees (compose)
  CM  rot placed_rot xs ys pxs pys k[6] x px y py -> m[6]    instance matrix compose (compose)
  TB  type payload -> byte                                   value to bool (to_bool)
  OB  type payload -> word                                   value as object (obj)
  HS  text -> hash                                           string value's hash (value::hash)
  I2  type payload text-hex -> word                         value to int (to_i32; a string's bytes in hex, - for none)
  F2  type payload text-hex -> f32                          value to float (to_f32)
  MA  k x -> f32                                            Math native k (MATH order: sin cos tan atan asin acos sqrt abs) of x
  IC  -> word@offset ..                                      the instance constructor (Inst::new), words +0..+224
Floats are f32 bit patterns."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from ui_closure import index, build

FUNCS = {'combine': 'F-57014', 'gradient': 'F-54894', 'fill': 'F-50749', 'rot': 'F-98882', 'compose': 'F-74625',
         'to_bool': 'F-18546', 'obj': 'F-70327', 'str': 'F-53993', 'ctor': 'F-44617', 'to_i32': 'F-68750', 'to_f32': 'F-90005'}
MATH = ['F-25737', 'F-86511', 'F-18756', 'F-13576', 'F-57109', 'F-54807', 'F-58006', 'F-84682']
# the C library's per-thread data getter returns none (errno then lives in its static word) and its invalid-parameter
# handler throws (a null string is never passed here)
CRT_STUBS = {'F-62587': 'ctx.r3.u64=0;', 'F-37557': 'throw std::runtime_error("invalid parameter");'}

def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
static float bf(uint32_t b){float v;std::memcpy(&v,&b,4);return v;}
static uint32_t fb(float v){uint32_t b;std::memcpy(&b,&v,4);return b;}
static void putb(uint32_t a,uint8_t v){*pointer(a)=v;}
static uint8_t getb(uint32_t a){return *pointer(a);}
static void clear(){std::memset(objects,0,sizeof objects);}
static Context fresh(){Context c{};c.r1.u32=0x2f00;return c;}
// gradient object at 0x1000: m +0..20, ratios ptr +24 (0x1100), colors ptr +28 (0x1200), n +32, radial +33
static void grad(int radial,int n,const uint32_t*r,const uint32_t*c,const uint32_t*m){
  for(int i=0;i<6;i++)store(0x1000+4*i,m[i]);store(0x1018,0x1100);store(0x101c,0x1200);putb(0x1020,n);putb(0x1021,radial);
  for(int i=0;i<n;i++){putb(0x1100+i,r[i]);store(0x1200+4*i,c[i]);}}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 char tag[4];
 while(scanf("%3s",tag)==1){
  std::string t(tag);
  try{
  if(t=="CX"){uint32_t l[8],p[8],tint;for(auto&x:l)scanf("%x",&x);for(auto&x:p)scanf("%x",&x);scanf("%x",&tint);
    clear();for(int i=0;i<8;i++){putb(0x1000+i,l[i]);putb(0x1010+i,p[i]);}
    Context c=fresh();c.r3.u32=0x1020;c.r4.u32=0x1000;c.r5.u32=0x1010;c.r6.u32=tint;c.r7.u32=tint>>24;
    // r6 carries R G B in bits 16..23 / 8..15 / 0..7, r7 the alpha byte
    c.r6.u32=tint&0xffffff;sub_COMBINE(c,nullptr);
    printf("CX");for(int i=0;i<8;i++)printf(" %x",l[i]);for(int i=0;i<8;i++)printf(" %x",p[i]);printf(" %x ->",tint);for(int i=0;i<8;i++)printf(" %x",getb(0x1020+i));printf("\n");}
  else if(t=="GR"||t=="FV"){uint32_t g=1,solid=0,cx[8]={},alpha=255;
    if(t=="FV"){scanf("%x %x",&g,&solid);for(auto&x:cx)scanf("%x",&x);scanf("%x",&alpha);}
    uint32_t radial,n,r[8],col[8],m[6],x,y;scanf("%x %x",&radial,&n);for(uint32_t i=0;i<n;i++)scanf("%x",&r[i]);for(uint32_t i=0;i<n;i++)scanf("%x",&col[i]);
    for(auto&v:m)scanf("%x",&v);scanf("%x %x",&x,&y);
    clear();grad(radial,n,r,col,m);Context c=fresh();c.f1.f64=bf(x);c.f2.f64=bf(y);uint32_t out;
    if(t=="GR"){c.r3.u32=0x1000;sub_GRADIENT(c,nullptr);out=c.r3.u32;}
    else{store(0x1304,solid);store(0x1324,g?0x1000:0);for(int i=0;i<8;i++)putb(0x1400+56+i,cx[i]);putb(0x1400+71,alpha);
      c.r3.u32=0x1300;c.r4.u32=0x1400;sub_FILL(c,nullptr);out=load(0x828396F4);
      printf("FV %x %x",g,solid);for(int i=0;i<8;i++)printf(" %x",cx[i]);printf(" %x ",alpha);}
    if(t=="GR")printf("GR ");
    printf("%x %x",radial,n);for(uint32_t i=0;i<n;i++)printf(" %x",r[i]);for(uint32_t i=0;i<n;i++)printf(" %x",col[i]);for(auto v:m)printf(" %x",v);printf(" %x %x -> %x\n",x,y,out);}
  else if(t=="RM"){uint32_t a;scanf("%x",&a);clear();Context c=fresh();c.r3.u32=0x1000;c.f1.f64=bf(a);sub_ROT(c,nullptr);
    printf("RM %x ->",a);for(int i=0;i<6;i++)printf(" %x",load(0x1000+4*i));printf("\n");}
  else if(t=="CM"){uint32_t v[16];for(auto&x:v)scanf("%x",&x);clear();
    // +80 rotation +76 placed +100/+104 _xscale/_yscale +92/+96 placed scale +116..+136 the placed matrix
    // +84 +108 / +88 +112 the two translation parts
    const int off[16]={80,76,100,104,92,96,116,120,124,128,132,136,84,108,88,112};
    for(int i=0;i<16;i++)store(0x1000+off[i],v[i]);Context c=fresh();c.r3.u32=0x1000;sub_COMPOSE(c,nullptr);
    printf("CM");for(auto x:v)printf(" %x",x);printf(" ->");for(int i=0;i<6;i++)printf(" %x",load(0x1000+4*i));printf(" %x\n",getb(0x1000+73));}
  else if(t=="TB"||t=="OB"){uint32_t ty,p;scanf("%x %x",&ty,&p);clear();store(0x1000,p);store(0x1004,0);store(0x1008,ty);
    Context c=fresh();c.r3.u32=0x1000;if(t=="TB")sub_TO_BOOL(c,nullptr);else sub_OBJ(c,nullptr);
    printf("%s %x %x -> %x\n",tag,ty,p,t=="TB"?c.r3.u32&0xff:c.r3.u32);}
  else if(t=="HS"){char s[256];scanf("%255s",s);clear();std::strcpy((char*)pointer(0x1100),s);
    Context c=fresh();c.r3.u32=0x1000;c.r4.u32=0x1100;sub_STR(c,nullptr);printf("HS %s -> %x %x %x\n",s,load(0x1000),load(0x1004),load(0x1008));}
  else if(t=="I2"||t=="F2"){uint32_t ty,p;char hx[512];scanf("%x %x %511s",&ty,&p,hx);clear();
    if(hx[0]!='-'){for(int i=0;hx[2*i];i++){unsigned b;sscanf(hx+2*i,"%2x",&b);putb(0x1100+i,b);}p=0x1100;}
    store(0x1000,p);store(0x1004,0);store(0x1008,ty);Context c=fresh();c.r3.u32=0x1000;
    if(t=="I2"){sub_TO_I32(c,nullptr);printf("I2 %x %x %s -> %x\n",ty,p,hx,c.r3.u32);}
    else{sub_TO_F32(c,nullptr);printf("F2 %x %x %s -> %x\n",ty,p,hx,fb(float(c.f1.f64)));}}
  else if(t=="MA"){uint32_t k,x;scanf("%x %x",&k,&x);clear();store(0x1000,x);store(0x1008,4);Context c=fresh();c.r5.u32=0x1000;c.r6.u32=0x1010;
    MATH_CALLS
    printf("MA %x %x -> %x\n",k,x,load(0x1010));}
  else if(t=="IC"){clear();for(int i=0;i<0x100;i+=4)store(0x1000+i,0xaaaaaaaa);Context c=fresh();c.r3.u32=0x1000;sub_CTOR(c,nullptr);
    printf("IC ->");for(int i=0;i<0xe0;i+=4)printf(" %x@%x",load(0x1000+i),i);printf("\n");}
  }catch(std::exception&e){printf("%s TRAP\n",tag);}
 }
}
'''


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(rnd):
    out = []
    edge = [0x80, 0x81, 0xff, 0x00, 0x01, 0x3f, 0x40, 0x41, 0x7f]
    bytes8 = lambda: [rnd.choice(edge) if rnd.random() < 0.3 else rnd.randrange(256) for _ in range(8)]
    for _ in range(3000):
        tint = rnd.choice([0xffffffff, 0, 0x80808080]) if rnd.random() < 0.2 else rnd.getrandbits(32)
        out.append('CX ' + ' '.join('%x' % b for b in bytes8() + bytes8()) + ' %x' % tint)

    def gfields():
        n = rnd.choice([1, 2, 2, 3, 4, 8])
        r = sorted(rnd.sample(range(256), n))
        if rnd.random() < 0.3 and n >= 2:
            r[0], r[-1] = 0, 255
        cols = [rnd.getrandbits(32) for _ in range(n)]
        s = rnd.choice([1 / 20, 0.05, 1.0, 0.37, 2.5, 13.0, 820.0, 1638.4, 0.001])
        m = [rnd.uniform(-1, 1) * s, rnd.uniform(-1, 1) * s * rnd.choice([0, 1]), rnd.uniform(-1, 1) * s * rnd.choice([0, 1]),
             rnd.uniform(-1, 1) * s, rnd.uniform(-20000, 20000), rnd.uniform(-20000, 20000)]
        x, y = rnd.uniform(-8000, 8000), rnd.uniform(-8000, 8000)
        return '%x %x ' % (rnd.random() < 0.4, n) + ' '.join('%x' % v for v in r + cols) + ' ' + ' '.join('%x' % fb(v) for v in m + [x, y])
    for _ in range(4000):
        out.append('GR ' + gfields())
    for _ in range(3000):
        g = rnd.random() < 0.5
        out.append('FV %x %x ' % (g, rnd.getrandbits(32)) + ' '.join('%x' % b for b in bytes8()) + ' %x ' % rnd.choice([255, 0, 128, rnd.randrange(256)]) + gfields())
    angles = [0.0, 90.0, -90.0, 180.0, 270.0, 360.0, 45.0, 30.0, 1e-3, -0.5, 720.0, 1e6, 12345.678]
    angles += [rnd.uniform(-360, 360) for _ in range(3000)] + [float(rnd.randrange(-720, 721)) for _ in range(500)]
    out += ['RM %x' % fb(a) for a in angles]
    for _ in range(4000):
        rot = rnd.choice([0.0, 0.0, rnd.uniform(-360, 360), float(rnd.randrange(-180, 181))])
        prot = rnd.choice([0.0, rnd.uniform(-180, 180)])
        xs, ys = (100.0, 100.0) if rnd.random() < 0.4 else (rnd.choice([100.0, rnd.uniform(-300, 300)]), rnd.choice([100.0, rnd.uniform(-300, 300)]))
        pxs, pys = rnd.choice([100.0, rnd.uniform(-300, 300)]), rnd.choice([100.0, rnd.uniform(-300, 300)])
        k = [rnd.uniform(-1, 1) for _ in range(4)] if rnd.random() < 0.6 else [1.0, 0.0, 0.0, 1.0]
        k += [rnd.uniform(-500, 500) * rnd.choice([0, 1]), rnd.uniform(-500, 500) * rnd.choice([0, 1])]
        t = [rnd.uniform(-1000, 1000) for _ in range(4)]
        out.append('CM ' + ' '.join('%x' % fb(v) for v in [rot, prot, xs, ys, pxs, pys] + k + t))
    words = [0, 1, 0xff, 0x100, 0x80000000, fb(0.0), fb(-0.0), fb(1.0), fb(float('nan')), 0x1234, 0x01000000]
    for ty in range(6):
        for w in words:
            out += ['TB %x %x' % (ty, w), 'OB %x %x' % (ty, w)]
    texts = ['a', 'A', '_root', '_global', 'cur_visibility', 'PowerUptype_0', 'fs_measure_width', 'Zz09_', 'UPPER_lower', '__x__']
    texts += [''.join(rnd.choice('abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.') for _ in range(rnd.randrange(1, 40))) for _ in range(800)]
    out += ['HS ' + s for s in texts]
    out.append('IC')
    xs = [0.0, -0.0, 1.0, -1.0, 0.5, 1e-8, 1e-30, 3.14159265, 1.5707964, 6.2831855, 100.0, 1e4, 1e6, 1e10, float('inf'), float('-inf'), float('nan'), 0.99999994, 1.0000001]
    xs += [rnd.uniform(-7, 7) for _ in range(1500)] + [rnd.uniform(-1, 1) for _ in range(1000)] + [rnd.uniform(-1e5, 1e5) for _ in range(300)] + [rnd.uniform(0, 400) for _ in range(300)]
    out += ['MA %x %x' % (k, fb(x)) for k in range(8) for x in xs]
    nums = ['0', '1', '-1', '+7', '  42', '	-13', '12abc', 'abc', '', ' ', '-', '+', '.5', '-.5', '5.', '1e3', '1E-3', '1e', '1e+', '2.5e+2x',
            '2147483647', '2147483648', '-2147483648', '-2147483649', '99999999999', '0x1A', '007', '1.0000001', '3.4028235e38', '3.5e38',
            '1e-46', '1.17549435e-38', '0.1', '0.3', '123456789', '16777217', '1.5', '2.5', '-0', '-0.0', 'inf', 'nan', '1,5', '٣']
    nums += [str(rnd.randrange(-10**6, 10**6)) for _ in range(300)] + ['%.*f' % (rnd.randrange(0, 9), rnd.uniform(-1e4, 1e4)) for _ in range(500)]
    nums += ['%.9g' % rnd.uniform(-1e30, 1e30) for _ in range(300)] + ['%.17g' % rnd.uniform(-10, 10) for _ in range(300)]
    nums += ['%de%d' % (rnd.randrange(1, 100), rnd.randrange(-50, 50)) for _ in range(200)]
    for t in nums:
        hx = t.encode('utf-8').hex() + '00'
        out += ['I2 1 0 ' + hx, 'F2 1 0 ' + hx]
    fw = [0, 1, 0xff, 0x80000000, 0x7fffffff, fb(0.5), fb(-0.5), fb(1.5), fb(-1.5), fb(2.5), fb(1e10), fb(-1e10), fb(2147483520.0), fb(2147483648.0),
          fb(-2147483648.0), fb(float('inf')), fb(float('-inf')), fb(float('nan')), fb(-0.0), fb(0.99999994), fb(1e-45)]
    fw += [fb(rnd.uniform(-3e9, 3e9)) for _ in range(300)] + [rnd.getrandbits(32) for _ in range(300)]
    for ty in (2, 3, 4, 5, 6):
        for w in fw:
            out += ['I2 %x %x -' % (ty, w), 'F2 %x %x -' % (ty, w)]
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True, help='build directory')
    ap.add_argument('--out', type=Path, required=True, help='ui_flash_ref.txt')
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    funcs = {k: addr(a.tool, v) for k, v in FUNCS.items()}
    maths = [addr(a.tool, v) for v in MATH]
    stubs = {'sub_' + addr(a.tool, k): v for k, v in CRT_STUBS.items()}
    src = MAIN
    for k, v in funcs.items():
        src = src.replace('sub_' + k.upper(), 'sub_' + v)
    src = src.replace('MATH_CALLS', ''.join('if(k==%d)sub_%s(c,nullptr);' % (i, v) for i, v in enumerate(maths)))
    exe = build(idx, ['sub_' + v for v in list(funcs.values()) + maths], src, a.work, 'ui_flash', stubs=stubs, vector=True)
    inp = '\n'.join(cases(random.Random(27))) + '\n'
    r = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True, check=True)
    # (a case line with a word that reads as an address of the original's code is left out: tracked files cite IDs, and
    # `registry check` takes such a word for a raw citation)
    keep = [l for l in r.stdout.splitlines() if not any(len(w) == 8 and 0x82000000 <= int(w, 16) < 0x83000000 for w in l.split() if all(c in '0123456789abcdef' for c in w))]
    a.out.write_text(''.join(l + '\n' for l in keep))
    print(len(keep), 'cases kept of', len(r.stdout.splitlines()), ',', r.stdout.count('TRAP'), 'traps')


if __name__ == '__main__':
    main()
