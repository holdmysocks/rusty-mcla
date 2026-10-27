"""Small vehicle-physics routines run offline on their generated code (recomp_dispatch):
  ROTX / ROTZ  rotation about X / Z by an angle (F-67091, F-16906)       ->  the 3 rows (9 words)
  ROTU         rotation about a unit axis (F-65964, already verified; the TURN helper)
  INV          3x3 + translation inverse (F-30106; wrapper F-43153)      ->  success flag, the 4 output rows (16 words)
  TURN         a matrix turned by the rotation about an axis (F-42980)   ->  the 3 rows
  SSP          the player input's speed factor (F-40620)
  SSS          speed factor from the tune's min / max / value (F-40603)  ->  the returned float and the stored word
  DMG          damage share ^ 0.3 (F-10364)                               ->  the returned float
Each output line:  OP inputs(hex words) -> outputs(hex words).  Usage: physics_math_reference.py --recomp D:/MCLA-static
--image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = dict(ROTX='F-67091', ROTZ='F-16906', INV='F-30106', INVW='F-43153', TURN='F-42980', ROTU='F-65964', SSS='F-40603', DMG='F-10364', PERL='F-52219', SSP='F-40620', HEAD='F-94892', ROLL='F-83846', G1='F-50173', G2='F-33402', G3='F-26199', G4='F-55749')

MAIN = r'''
static float fl(uint32_t w){float f;memcpy(&f,&w,4);return f;}
static uint32_t bi(float f){uint32_t w;memcpy(&w,&f,4);return w;}
static uint32_t rd32(std::istream&in){uint32_t w;in>>std::hex>>w>>std::dec;return w;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string op;
 {Context g=fresh();sub_G1(g,nullptr);sub_G2(g,nullptr);sub_G3(g,nullptr);sub_G4(g,nullptr);}   // the C runtime's vector constants (zero in the image until its initialisers ran)
 while(in>>op){rt_reset();Context c=fresh();
  if(op=="ROTX"||op=="ROTZ"){uint32_t a=rd32(in);c.r3.u32=0x1000;c.f1.f64=double(fl(a));
   if(op=="ROTX")sub_ROTX(c,nullptr);else sub_ROTZ(c,nullptr);
   printf("%s %08x ->",op.c_str(),a);int o[9]={0,4,8,16,20,24,32,36,40};for(int i=0;i<9;i++)printf(" %08x",load(0x1000+o[i]));printf("\n");}
  else if(op=="INV"||op=="INVW"){uint32_t w[16];for(int i=0;i<16;i++)w[i]=rd32(in);
   for(int i=0;i<16;i++){store(0x1100+4*i,w[i]);store(0x1000+4*i,0x7e7e7e7e);}
   c.r3.u32=0x1000;c.r4.u32=0x1100;
   if(op=="INV")sub_INV(c,nullptr);else{c.r3.u32=0x1100;sub_INVW(c,nullptr);}
   printf("%s",op.c_str());for(int i=0;i<16;i++)printf(" %08x",w[i]);printf(" -> %u",op=="INV"?c.r3.u32:1u);
   uint32_t base_=op=="INV"?0x1000:0x1100;
   for(int i=0;i<16;i++)printf(" %08x",load(base_+4*i));printf("\n");}
  else if(op=="TURN"){uint32_t w[12],ax[3],an;for(int i=0;i<12;i++)w[i]=rd32(in);for(int i=0;i<3;i++)ax[i]=rd32(in);an=rd32(in);
   for(int r=0;r<3;r++)for(int k=0;k<4;k++)store(0x1000+16*r+4*k,w[4*r+k]);
   for(int i=0;i<3;i++)store(0x1200+4*i,ax[i]);store(0x120c,0);
   c.r3.u32=0x1000;c.r4.u32=0x1200;c.f1.f64=double(fl(an));sub_TURN(c,nullptr);
   printf("TURN");for(int i=0;i<12;i++)printf(" %08x",w[i]);for(int i=0;i<3;i++)printf(" %08x",ax[i]);printf(" %08x ->",an);
   for(int r=0;r<3;r++)for(int k=0;k<3;k++)printf(" %08x",load(0x1000+16*r+4*k));printf("\n");}
  else if(op=="ROTU"){uint32_t ax[3],an;for(int i=0;i<3;i++)ax[i]=rd32(in);an=rd32(in);
   for(int i=0;i<3;i++)store(0x1200+4*i,ax[i]);c.r3.u32=0x1000;c.r4.u32=0x1200;c.f1.f64=double(fl(an));sub_ROTU(c,nullptr);
   printf("ROTU");for(int i=0;i<3;i++)printf(" %08x",ax[i]);printf(" %08x ->",an);for(int r=0;r<3;r++)for(int k=0;k<3;k++)printf(" %08x",load(0x1000+16*r+4*k));printf("\n");}
  else if(op=="SSS"){uint32_t mn=rd32(in),mx=rd32(in),val=rd32(in),sp=rd32(in);
   store(0x1000+368,0x1400);store(0x1400+216,mn);store(0x1400+220,mx);store(0x1400+212,val);store(0x1000+568,0x7e7e7e7e);
   c.r3.u32=0x1000;c.f1.f64=double(fl(sp));sub_SSS(c,nullptr);
   printf("SSS %08x %08x %08x %08x -> %08x %08x\n",mn,mx,val,sp,bi(float(c.f1.f64)),load(0x1000+568));}
  else if(op=="SSP"){uint32_t th=rd32(in),val=rd32(in),sp=rd32(in);
   store(0x1000+48,0x1400);store(0x1400+24,th);store(0x1400+28,val);c.r3.u32=0x1000;c.f1.f64=double(fl(sp));sub_SSP(c,nullptr);
   printf("SSP %08x %08x %08x -> %08x\n",th,val,sp,bi(float(c.f1.f64)));}
  else if(op=="HEAD"){uint32_t v[3];for(int i=0;i<3;i++)v[i]=rd32(in);
   for(int i=0;i<3;i++)store(0x1020+4*i,v[i]);store(0x102c,0x3f800000);c.r3.u32=0x1000;sub_HEAD(c,nullptr);
   printf("HEAD %08x %08x %08x -> %08x\n",v[0],v[1],v[2],bi(float(c.f1.f64)));}
  else if(op=="ROLL"){uint32_t v[6],an;for(int i=0;i<6;i++)v[i]=rd32(in);an=rd32(in);
   for(int i=0;i<3;i++){store(0x1000+4*i,v[i]);store(0x1010+4*i,v[3+i]);}store(0x100c,0);store(0x101c,0);
   c.r3.u32=0x1000;c.f1.f64=double(fl(an));sub_ROLL(c,nullptr);
   printf("ROLL");for(int i=0;i<6;i++)printf(" %08x",v[i]);printf(" %08x ->",an);for(int i=0;i<3;i++)printf(" %08x",load(0x1000+4*i));for(int i=0;i<3;i++)printf(" %08x",load(0x1010+4*i));printf("\n");}
  else if(op=="PERL"){uint32_t x=rd32(in);c.f1.f64=double(fl(x));sub_PERL(c,nullptr);printf("PERL %08x -> %08x\n",x,bi(float(c.f1.f64)));}
  else if(op=="DMG"){uint32_t mx=rd32(in),med=rd32(in),dm=rd32(in);
   store(0x1000+8,0x1400);store(0x1400+24,mx);store(0x1400+28,med);store(0x1000+36,dm);
   c.r3.u32=0x1000;sub_DMG(c,nullptr);printf("DMG %08x %08x %08x -> %08x\n",mx,med,dm,bi(float(c.f1.f64)));}
 }
}
'''


def fid_addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def b(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


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
    src = MAIN
    for k, v in addr.items():
        src = src.replace('sub_' + k + '(', v + '(')
    exe = rd.build(idx, list(addr.values()), src, a.work, 'physics_math', image=img)
    r = random.Random(20261027)
    L = []
    angles = [1e-6, 0.5, -0.5, 1.0, 3.14159265, -3.14159265, 6.2831855, 100.0, -100.0, 1e-3, 0.7853982] + [r.uniform(-7, 7) for _ in range(300)]
    for ang in angles:
        L.append(f'ROTX {b(ang):08x}')
        L.append(f'ROTZ {b(ang):08x}')
    # inverse: general matrices at several scales, singular and near-singular ones, translations
    for i in range(1500):
        k = r.choice([1, 1, 1, 0.01, 100, 1e-3])
        m = [[r.uniform(-1, 1) * k for _ in range(3)] for _ in range(3)]
        if i % 10 == 0:
            m[2] = [x * 2 for x in m[0]]
        if i % 10 == 1:
            m[2] = [m[0][j] * 2 + r.uniform(-1, 1) * 1e-7 for j in range(3)]
        if i % 10 == 2:
            m = [[0.0] * 3 for _ in range(3)]
        if i % 10 == 3:
            m = [[1, 0, 0], [0, 1, 0], [0, 0, 1]]
        t = [r.uniform(-500, 500) for _ in range(3)]
        rows = [m[0] + [r.uniform(-1, 1)], m[1] + [r.uniform(-1, 1)], m[2] + [r.uniform(-1, 1)], t + [1.0]]
        w = ' '.join(f'{b(x):08x}' for row in rows for x in row)
        L.append(f'INV {w}')
        if i % 3 == 0:
            L.append(f'INVW {w}')
    for _ in range(800):
        m = [[r.uniform(-1, 1) for _ in range(3)] + [0.0] for _ in range(3)]
        ax = [r.uniform(-1, 1) for _ in range(3)]
        n = sum(x * x for x in ax) ** 0.5
        ax = [x / n for x in ax]
        an = r.uniform(-4, 4)
        L.append('TURN ' + ' '.join(f'{b(x):08x}' for row in m for x in row) + ' ' + ' '.join(f'{b(x):08x}' for x in ax) + f' {b(an):08x}')
        L.append('ROTU ' + ' '.join(f'{b(x):08x}' for x in ax) + f' {b(an):08x}')
    for _ in range(1500):
        mn = r.choice([0.0, 5.0, 10.0, 20.0, r.uniform(0, 40)])
        mx = r.choice([0.0, -1.0, mn + r.uniform(0, 50), mn, mn + 0.001])
        val = r.choice([0.5, 1.0, 1.2, 2.0, r.uniform(0, 3)])
        sp = r.choice([mn, mx, mn - 0.01, (mn + mx) / 2, r.uniform(-5, 80)])
        L.append(f'SSS {b(mn):08x} {b(mx):08x} {b(val):08x} {b(sp):08x}')
    for _ in range(2000):
        mx = r.choice([100.0, 1000.0, r.uniform(10, 5000)])
        med = r.choice([mx * 0.5, mx * 0.9, r.uniform(0, mx)])
        dm = r.choice([0.0, 1.0, -0.1, 1.5, r.uniform(0, 1.2), r.uniform(0, 1.2), med / mx, mx / mx])
        L.append(f'DMG {b(mx):08x} {b(med):08x} {b(dm):08x}')
    for _ in range(1500):
        th = r.choice([0.0, 5.0, 10.0, 20.0, r.uniform(0, 40)])
        val = r.choice([0.5, 1.0, 1.2, 2.0, r.uniform(0, 3)])
        sp = r.choice([th, th - 0.01, th / 2, r.uniform(-5, 80)])
        L.append(f'SSP {b(th):08x} {b(val):08x} {b(sp):08x}')
    for _ in range(1500):
        v = [r.uniform(-1, 1) * r.choice([1, 1, 0.01, 100]) for _ in range(3)]
        if r.random() < 0.3:
            v[1] = 0.0
        if r.random() < 0.1:
            v[0] = 0.0
        if r.random() < 0.05:
            v[2] = 0.0
        L.append('HEAD ' + ' '.join(f'{b(x):08x}' for x in v))
    for _ in range(800):
        rows = [r.uniform(-1, 1) for _ in range(6)]
        L.append('ROLL ' + ' '.join(f'{b(x):08x}' for x in rows) + f' {b(r.uniform(-4, 4)):08x}')
    for _ in range(3000):
        x = r.choice([r.uniform(-300, 300), r.uniform(-4, 4), float(r.randint(-300, 300)), r.randint(-300, 300) + 0.5, r.uniform(-1e4, 1e4)])
        L.append(f'PERL {b(x):08x}')
    cases = a.work / 'physics_math_cases.txt'
    cases.write_text('\n'.join(L) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    import re
    keep = [l for l in p.stdout.splitlines() if not re.search(r'(?<![0-9a-f])82[0-9a-f]{6}(?![0-9a-f])', l)]   # no word that reads as an image address in a tracked data file
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(p.stdout.splitlines()) - len(keep), 'dropped')
    print(len(p.stdout.splitlines()), 'cases')


if __name__ == '__main__':
    main()
