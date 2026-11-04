"""A driver's wind and car lights run offline on the original's code: the draw hooks' update F-31704 (each hook of a manager:
a driver in a vehicle gets the wind's phase +112, strength +116 and the vehicle body's velocity +128) and then the hook's
F-42959 on the same hook (the shader values of a driver in a car: neon place / colour, gauge place / colour, cone
direction, floor plane, wind plane and wind params), both whole, unchanged generated code on flat guest memory in vector
mode. Stubs answer from the case: F-90764 (the player is in a vehicle), F-54299 (the vehicle is a bike), F-78526 (0),
F-43393 (the per-frame roll of the instance: nothing the wind reads); F-95266 / F-15028 give the shader set, whose
setters F-60271 (vector), F-33681 (vector, the cone direction) and F-52215 (two floats) record what they are given by
handle. The two shadow imposter handles are 0, as for every character effect (none declares them). The start-up vector
constants the image holds as zero are filled in as the running game has them (docs/reference/ai/raceline.md).
Guest memory: the manager 0x40800C00 (+72 the list, +76 its count 1), the hook 0x40800000 (+16 the instance, +28 0: the
handles cached, +32 .. +68 the handles 1 .. 8 then 0, 0; +112 / +116 / +128 the wind), the instance 0x40801000 (+96 the
player, +129 0, +132 0, +147 0, +188 0x40810000 whose +24 is 0), the player 0x40802000 (+48 the vehicle), the vehicle
0x40803000 (+8 the chassis, +12 the lamp object + 16, +76 the model, +132 the record, +144 the matrix rows X, Y, Z, place),
the record 0x40804000 (+96 IntNeonColor, +112 DashGaugeColor, +372 IntNeonIntensity), the lamp object 0x40805000 (+168 the
head state, +268, +408 the flash), the model 0x40806000 (+96 the tune 0x40807000: +144 / +160 the gauge / neon offsets),
the chassis 0x4080A000 (+128 the body 0x4080B000 with its velocity at +272, +224 the speed).
Lines: `DW <inputs> -> <hook +112 +116 +128..+140> <the eight values set, 4 words each>`, floats as hex bits; inputs: matrix
(16), gauge offset (3), neon offset (3), gauge colour (3), neon colour (3), intensity, +408, +268, head, bike, phase, speed,
velocity (4), paused, dt.
Usage: driver_wind_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

UPDATE, CABIN = 'F-31704', 'F-42959'
INCAR, BIKE, ROLL, GATE, SET1, SET2 = 'F-90764', 'F-54299', 'F-43393', 'F-78526', 'F-95266', 'F-15028'
SETV, SETCONE, SET2F, SETTEX, SETMTX, GETTEX, GETMTX = 'F-60271', 'F-33681', 'F-52215', 'F-35010', 'F-11356', 'F-31209', 'F-35033'
NIN = 16 + 3 + 3 + 3 + 3 + 4 + 1 + 2 + 4 + 2

MAIN = r'''
static void st8(uint32_t a,uint32_t v){*pointer(a)=uint8_t(v);}
static void put(uint32_t a,uint32_t v){store(a,v);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);uint32_t x[NIN_];
 const uint32_t H=0x40800000,MG=0x40800C00,LS=0x40800D00,I=0x40801000,P=0x40802000,V=0x40803000,REC=0x40804000,L=0x40805000,
  MD=0x40806000,T=0x40807000,SG=0x40808000,GR=0x40809000,CH=0x4080A000,BD=0x4080B000,PM=0x4080E000,OUT=0x4080F000,DR=0x40810000;
 for(;;){for(int k=0;k<NIN_;k++)if(!(in>>std::hex>>x[k]))return 0;rt_reset();
  for(uint32_t a=H;a<DR+0x100;a++)st8(a,0);
  for(int k=0;k<3;k++)put(0x82860950+4*k,0xFFFFFFFF);put(0x8286095C,0);
  for(int k=0;k<3;k++)put(0x82860930+4*k,0);put(0x8286093C,0x3F800000);
  for(int k=0;k<4;k++){put(0x828609E0+4*k,0x7F800000);put(0x828608C0+4*k,0);}
  int j=0;for(int k=0;k<16;k++)put(V+144+4*k,x[j++]);
  for(int k=0;k<3;k++)put(T+144+4*k,x[j++]);for(int k=0;k<3;k++)put(T+160+4*k,x[j++]);
  for(int k=0;k<3;k++)put(REC+112+4*k,x[j++]);for(int k=0;k<3;k++)put(REC+96+4*k,x[j++]);
  put(REC+372,x[j++]);put(L+408,x[j++]);put(L+268,x[j++]);put(L+168,x[j++]);uint32_t bike=x[j++];
  put(H+112,x[j++]);put(CH+224,x[j++]);for(int k=0;k<4;k++)put(BD+272+4*k,x[j++]);uint32_t paused=x[j++];put(0x827D7508,x[j++]);
  st8(0x82908D5D,paused);put(0x828CD5AC,0);put(0x828CCEA0,PM);st8(PM+4,paused);st8(PM+10,0);
  put(MG+72,LS);*pointer(MG+76)=0;*pointer(MG+77)=1;put(LS,H);
  put(H+16,I);for(int k=0;k<8;k++)put(H+32+4*k,k+1);
  put(I+96,P);put(I+188,DR);put(P+48,V);put(V+8,CH);put(V+12,L+16);put(V+76,MD);put(V+132,REC);put(MD+96,T);put(CH+128,BD);
  put(SG+8,GR);store(0x40809F00,1);store(0x40809F04,bike);
  Context c=fresh();c.r3.u32=MG;sub_UPDATE(c,nullptr);
  Context d=fresh();d.r3.u32=H;sub_CABIN(d,nullptr);
  printf("%08x %08x",load(H+112),load(H+116));for(int k=0;k<4;k++)printf(" %08x",load(H+128+4*k));
  for(int h=1;h<=8;h++)for(int k=0;k<4;k++)printf(" %08x",load(OUT+16*h+4*k));printf("\n");}
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def bits(x):
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
    sub = lambda f: 'sub_' + fid_addr(a.tool, f)
    rec = 'uint8_t*s=pointer(ctx.r5.u32);uint8_t*o=pointer(0x4080F000u+16*ctx.r4.u32);for(int k=0;k<%d;k++)o[k]=s[k];'
    stubs = {sub(INCAR): 'ctx.r3.u64=load(0x40809F00u);', sub(BIKE): 'ctx.r3.u64=load(0x40809F04u);', sub(ROLL): '',
             sub(GATE): 'ctx.r3.u64=0;', sub(SET1): 'ctx.r3.u64=0x40808000u;', sub(SET2): 'ctx.r3.u64=0x40808000u;',
             sub(SETV): rec % 16, sub(SETCONE): rec % 16, sub(SET2F): rec % 8,
             sub(SETTEX): 'throw std::runtime_error("texture set");', sub(SETMTX): 'throw std::runtime_error("matrix set");',
             sub(GETTEX): 'ctx.r3.u64=0;', sub(GETMTX): 'ctx.r3.u64=0;'}
    main_src = MAIN.replace('sub_UPDATE', sub(UPDATE)).replace('sub_CABIN', sub(CABIN)).replace('NIN_', str(NIN))
    exe = rd.build(idx, [sub(UPDATE), sub(CABIN)], main_src, a.work, 'driver_wind', image=img, stubs=stubs)
    rnd = random.Random(20261104)
    cases = []

    def rot():
        q = [rnd.gauss(0, 1) for _ in range(4)]
        n = math.sqrt(sum(v * v for v in q))
        w, x, y, z = (v / n for v in q)
        return [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)],
                [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
                [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]

    for i in range(6000):
        r = rot()
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 300), rnd.uniform(-3000, 3000)]
        m = r[0] + [0.0] + r[1] + [0.0] + r[2] + [0.0] + place + [1.0]
        off = [rnd.uniform(-2, 2) for _ in range(6)]
        col = [rnd.choice((rnd.random(), rnd.randrange(256) / 255.0)) for _ in range(6)]
        inten = rnd.choice((0.0, 1.0, rnd.random()))
        flash = rnd.choice((1.0, rnd.random()))
        head = rnd.choice((0.0, 1.0, rnd.random()))
        bike = int(rnd.random() < 0.6)
        phase = rnd.choice((0.0, rnd.uniform(0, 50), rnd.uniform(0, 5000)))
        speed = rnd.choice((0.0, rnd.uniform(0, 80), 40.233 + rnd.uniform(-0.01, 0.01), rnd.uniform(0, 1)))
        d = [rnd.gauss(0, 1) for _ in range(3)]
        dn = math.sqrt(sum(v * v for v in d)) or 1.0
        mag = rnd.choice((0.0, rnd.uniform(0, 70), math.sqrt(1e-5) * rnd.uniform(0.9, 1.1), rnd.uniform(0, 0.01)))
        vel = [v / dn * mag for v in d] + [rnd.choice((0.0, rnd.uniform(-1, 1)))]
        paused = int(rnd.random() < 0.05)
        dt = rnd.choice((1.0 / 60.0, 1.0 / 30.0, rnd.uniform(0.0, 0.05)))
        cases.append([bits(v) for v in m + off + col + [inten, flash, 1.0, head]] + [bike] +
                     [bits(v) for v in [phase, speed] + vel] + [paused, bits(dt)])
    cf = a.work / 'driver_wind_cases.txt'
    cf.write_text(''.join(' '.join(f'{v:08x}' for v in c) + '\n' for c in cases))
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = [f'DW {" ".join(f"{v:08x}" for v in c)} -> {o}' for c, o in zip(cases, r.stdout.splitlines())]
    if len(out) != len(cases):
        raise SystemExit(f'{len(out)} of {len(cases)} cases ran: {r.stderr[-400:]}')
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
