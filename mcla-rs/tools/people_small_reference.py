"""Small PEOPLE_TRAFFIC / AI functions run offline on the original's unchanged code (recomp_dispatch), one case per line:
  HD x z -> heading bits                       the spawn matrix heading (F-10286: atan2 of the z row, wrapped to (0, 2 pi])
  IT n <n nodes: 4 child idx, minx minz maxx maxz> (a tree), then IN qx qz -> node index   the parking-tree descent (F-97347)
  TU <2195 words>, then ST v / AC g v / DC g v    steer scalar / engine accel / engine decel lookups of the racer tune (F-67089, F-45930, F-71244)
  CE nx nz minx minz maxx maxz qx qz -> id ix iz   the object grid cell (F-59709);  KE nx nz ix iz -> id  (F-74216)
  RC start goal flags -> cost bits    the racers' polygon cost (F-86835)
  GR minx minz maxx maxz -> nx nz minx minz maxx maxz   the grid set-up from the level bounds (F-38432; the accumulators start beyond the bounds)
Floats as hex bits. Usage: people_small_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = dict(HEAD='F-10286', ITEM='F-97347', STEER='F-67089', ACCEL='F-45930', DECEL='F-71244', CELL='F-59709', KEY='F-74216', GRID='F-38432', COST='F-86835')

MAIN = r'''
static float fl(uint32_t w){float f;memcpy(&f,&w,4);return f;}
static uint32_t bi(float f){uint32_t w;memcpy(&w,&f,4);return w;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string kind;
 while(in>>kind){if(kind!="IN"&&kind!="ST"&&kind!="AC"&&kind!="DC")rt_reset();
  if(kind=="HD"){uint32_t x,z;in>>std::hex>>x>>z>>std::dec;store(0x1000,x);store(0x1008,z);
   Context c=fresh();c.r3.u32=0x1000;sub_HEAD(c,nullptr);printf("HD %08x\n",bi(float(c.f1.f64)));}
  else if(kind=="IT"){int n;in>>n;
   for(int k=0;k<n;k++){uint32_t base=0x40010000+0x100*k;int ch[4];for(int c=0;c<4;c++){in>>ch[c];store(base+28+4*c,ch[c]<0?0:0x40010000+0x100*ch[c]);}
    uint32_t w[4];in>>std::hex>>w[0]>>w[1]>>w[2]>>w[3]>>std::dec;for(int i=0;i<4;i++)store(base+52+4*i,w[i]);}}
  else if(kind=="IN"){uint32_t qx,qz;in>>std::hex>>qx>>qz>>std::dec;store(0x1100,qx);store(0x1108,qz);store(0x110c,0x3f800000);
   Context c=fresh();c.r3.u32=0x40010000;c.r4.u32=0x1100;sub_ITEM(c,nullptr);printf("IN %u\n",(c.r3.u32-0x40010000)/0x100);}
  else if(kind=="TU"){
   for(int i=0;i<2195;i++){uint32_t w;in>>std::hex>>w>>std::dec;store(0x40020000+4*i,w);}}
  else if(kind=="ST"){uint32_t v;in>>std::hex>>v>>std::dec;Context c=fresh();c.r3.u32=0x40020000;c.f1.f64=double(fl(v));sub_STEER(c,nullptr);printf("ST %08x\n",bi(float(c.f1.f64)));}
  else if(kind=="AC"||kind=="DC"){int g;uint32_t v;in>>g>>std::hex>>v>>std::dec;Context c=fresh();c.r3.u32=0x40020000;c.r4.u32=g;c.f1.f64=double(fl(v));
   if(kind=="AC")sub_ACCEL(c,nullptr);else sub_DECEL(c,nullptr);printf("%s %08x\n",kind.c_str(),bi(float(c.f1.f64)));}
  else if(kind=="CE"){int nx,nz;uint32_t mx,mz,xx,xz,qx,qz;in>>nx>>nz>>std::hex>>mx>>mz>>xx>>xz>>qx>>qz>>std::dec;
   store(0x2000,nx);store(0x2004,nz);store(0x200c,mx);store(0x2010,mz);store(0x2014,xx);store(0x2018,xz);store(0x2100,qx);store(0x2108,qz);
   Context c=fresh();c.r3.u32=0x2000;c.r4.u32=0x2200;c.r5.u32=0x2204;c.r6.u32=0x2100;sub_CELL(c,nullptr);
   printf("CE %d %d %d\n",(int)c.r3.u32,(int)load(0x2200),(int)load(0x2204));}
  else if(kind=="GR"){uint32_t w[4];in>>std::hex>>w[0]>>w[1]>>w[2]>>w[3]>>std::dec;
   store(0x2400,w[0]);store(0x2408,w[1]);store(0x2420,w[2]);store(0x2428,w[3]);
   store(0x2300,0x7149f2cau);store(0x2304,0x7149f2cau);store(0x2310,0xf149f2cau);store(0x2314,0xf149f2cau);
   Context c=fresh();c.r3.u32=0x2500;c.r4.u32=0x2300;c.r5.u32=0x2310;c.r6.u32=0x2400;c.r7.u32=0x2420;sub_GRID(c,nullptr);
   printf("GR %d %d %08x %08x %08x %08x\n",(int)load(0x2500),(int)load(0x2504),load(0x250c),load(0x2510),load(0x2514),load(0x2518));}
  else if(kind=="RC"){uint32_t a,b,f;in>>std::hex>>a>>b>>f>>std::dec;store(0x2608,a);store(0x260c,b);store(0x2708,f);
   Context c=fresh();c.r3.u32=0x2600;c.r4.u32=0x2700;sub_COST(c,nullptr);printf("RC %08x\n",bi(float(c.f1.f64)));}
  else if(kind=="KE"){int nx,nz,ix,iz;in>>nx>>nz>>ix>>iz;store(0x2000,nx);store(0x2004,nz);
   Context c=fresh();c.r3.u32=0x2000;c.r4.u32=ix;c.r5.u32=iz;sub_KEY(c,nullptr);printf("KE %d\n",(int)c.r3.u32);}
 }
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


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
    names = {k: 'sub_' + fid_addr(a.tool, v) for k, v in IDS.items()}
    src = MAIN
    for k, n in names.items():
        src = src.replace('sub_' + k, n)
    exe = rd.build(idx, list(names.values()), src, a.work, 'people_small', image=img)
    rnd = random.Random(20261028)
    lines = []
    # heading
    for _ in range(300):
        s = 10 ** rnd.uniform(-4, 3)
        x, z = rnd.uniform(-s, s), rnd.uniform(-s, s)
        lines.append(f'HD {fb(x):08x} {fb(z):08x}')
    for x, z in [(0.0, 1.0), (0.0, -1.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 0.0), (-0.0, 1.0), (-0.0, -1.0), (1e-30, -1.0), (-1e-30, -1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0)]:
        lines.append(f'HD {fb(x):08x} {fb(z):08x}')
    # item_node: random trees
    for _ in range(120):
        n = rnd.randint(1, 24)
        nodes = []
        for k in range(n):
            if k == 0:
                box = [-100.0, -100.0, 100.0, 100.0]
            else:
                par = rnd.randrange(0, k)
                pb = nodes[par][1]
                box = [pb[0] + rnd.choice([0.0, rnd.uniform(0, 30)]), pb[1] + rnd.choice([0.0, rnd.uniform(0, 30)]), pb[2] - rnd.choice([0.0, rnd.uniform(0, 30)]), pb[3] - rnd.choice([0.0, rnd.uniform(0, 30)])]
                box = [f32(v) for v in box]
            nodes.append([[-1] * 4, box])
        for k in range(1, n):
            par = rnd.randrange(0, k)
            free = [c for c in range(4) if nodes[par][0][c] < 0]
            if free:
                nodes[par][0][rnd.choice(free)] = k
        ds = ' '.join(' '.join(str(c) for c in nd[0]) + ' ' + ' '.join('%08x' % fb(v) for v in nd[1]) for nd in nodes)
        lines.append(f'IT {n} {ds}')
        for _ in range(8):
            qx, qz = rnd.uniform(-110, 110), rnd.uniform(-110, 110)
            if rnd.random() < 0.3:   # on a box edge +- 1
                nb = nodes[rnd.randrange(n)][1]
                qx = f32(rnd.choice([nb[0], nb[2]]) + rnd.choice([-1.0, 1.0]))
                qz = f32(rnd.uniform(nb[1], nb[3]))
            lines.append(f'IN {fb(qx):08x} {fb(qz):08x}')
    # racer tune tables
    for _ in range(6):
        ms = f32(rnd.uniform(20, 100))
        w = [fb(f32(rnd.uniform(0, 30))) for _ in range(2195)]
        w[0] = fb(ms)
        w[11] = fb(f32(127.0 / ms))
        lines.append('TU ' + ' '.join('%08x' % x for x in w))
        for _ in range(60):
            v = rnd.choice([rnd.uniform(-5, 1.2 * ms), rnd.uniform(0, ms), 0.0, ms, rnd.uniform(0, 1)])
            lines.append(f'ST {fb(v):08x}')
            g = rnd.randint(0, 6)
            lines.append(f'AC {g} {fb(v):08x}')
            lines.append(f'DC {g} {fb(v):08x}')
    # grid
    for _ in range(80):
        nx, nz = rnd.randint(1, 300), rnd.randint(1, 300)
        mx, mz = f32(-40.0 * rnd.randint(0, 100)), f32(-40.0 * rnd.randint(0, 100))
        xx, xz = f32(nx * 40.0 + mx), f32(nz * 40.0 + mz)
        for _ in range(10):
            qx = rnd.choice([rnd.uniform(mx - 50, xx + 50), mx, xx, f32(xx - 1e-3), f32(mx + 40 * rnd.randint(0, nx))])
            qz = rnd.choice([rnd.uniform(mz - 50, xz + 50), mz, xz, f32(xz - 1e-3), f32(mz + 40 * rnd.randint(0, nz))])
            lines.append(f'CE {nx} {nz} {fb(mx):08x} {fb(mz):08x} {fb(xx):08x} {fb(xz):08x} {fb(qx):08x} {fb(qz):08x}')
        for _ in range(10):
            lines.append(f'KE {nx} {nz} {rnd.randint(-3, nx + 2)} {rnd.randint(-3, nz + 2)}')
    # grid set-up from level bounds
    for _ in range(300):
        s = 10 ** rnd.uniform(1, 4.5)
        x0, z0 = rnd.uniform(-s, s * 0.2), rnd.uniform(-s, s * 0.2)
        x1, z1 = x0 + rnd.uniform(1, 2 * s), z0 + rnd.uniform(1, 2 * s)
        if rnd.random() < 0.2:
            x0 = z0 = 0.0
        lines.append(f'GR {fb(x0):08x} {fb(z0):08x} {fb(x1):08x} {fb(z1):08x}')
    lines.append(f'GR {fb(-1000.0):08x} {fb(-500.0):08x} {fb(1000.0):08x} {fb(500.0):08x}')
    # racer path cost
    for _ in range(400):
        a_, b_ = rnd.choice([0, 1, 2, 4, 8, 0x10, 0x20, 0x40, rnd.getrandbits(8)]), rnd.choice([0, 1, 2, 4, 8, 0x10, 0x20, 0x40, rnd.getrandbits(8)])
        f = rnd.choice([a_, b_, rnd.getrandbits(8), 1 << rnd.randrange(8), rnd.getrandbits(32)])
        lines.append(f'RC {a_:x} {b_:x} {f:x}')
    cf = a.work / 'people_small_cases.txt'
    cf.write_text('\n'.join(lines) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    res = iter(r.stdout.splitlines())
    out = []
    for cl in lines:
        if cl.startswith('TU') or cl.startswith('IT'):
            out.append(cl)
            continue
        out.append(cl + ' -> ' + next(res).split(' ', 1)[1])
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'lines')


if __name__ == '__main__':
    main()
