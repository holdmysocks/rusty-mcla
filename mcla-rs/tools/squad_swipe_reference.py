"""Run the original's squad swipes offline (static recompilation, native, flat guest memory) and record them for the port's test
(crates/game/src/testdata/squad_swipe_ref.txt, test `squad_swipes_match_the_original` in police.rs).

The two state updates that swipe run unchanged: the Arrow Trap's CatchUp update F-47983 and the Double Pursuit's AlongSide update
F-41674, with every brain test they call (F-42951, F-29545 / F-39842, F-80249, F-99390 / F-53459, F-30098 / F-57435, F-90304 /
F-88594, F-48012, F-37230) and the override setters (F-77791 steer, F-39048 brake, F-10965 handbrake). Their lock is set far
above 0 so the formation never runs (it is compared by tools/formation_reference.py); the radio line's message calls are stubbed
and counted (F-14569); the Arrow Trap's Halt test F-29443 is stubbed false (compared by tools/squad_fsm_reference.py).

World per case: the tactic (+72 cops, +76 the cop list, +164 / +168 the flank slots), each cop a racer (+872 = 1 - cop 0's 0 in cases 2, 3, 10, 11 (as the payback's goons) -, +876 its
brain) with a car (matrix through the instance +48 -> +28 -> +16, velocity +48 -> +8 -> +128 -> +272) and a brain (+4 its car,
+680 the target car, +564 = 0, +774 the head-on flag), the target a car the same way. Before each update the bearing cache
(+764) is cleared as the brain update F-74140 does, and the overrides +748 / +752 / +756 are set to DEADBEEF, so an update
shows only what it writes. Cop geometry: random yaw (rows 0 / 2 slightly tilted), the target at a random bearing 0..180 off
the first row, ahead or behind, 0.5 - 25 m (some 55 - 65 m for the head-on range), the velocities mostly the same way;
each update a cop is re-placed with p 0.06, the target with p 0.03, a head-on flag flips with p 0.005.

  CASE <arrow 0|1> <cops> <a> <b> <law 0|1>   a / b: the cop indices in slots +164 / +168; law: cop 0's +872 (F-37230: the line)
  G <k> <pos row0 row2 vel: 12 f32 hex> <head-on 0|1>
  T <pos vel: 6 f32 hex>
  <per cop: steer brake handbrake hex> <+24 hex> <+32> <lines>   one update, or `=` when the same as the update before
  END
"""
import argparse
import math
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

CASES = 16
N = 300
MAIN_IDS = ['F-47983', 'F-41674']
RADIO = ['F-54152', 'F-23825', 'F-14569', 'F-55001']
OTHER = ['F-29443', 'F-96080', 'F-28114', 'F-21534', 'F-80532']

MAIN = r'''
static void store8(uint32_t a,uint8_t v){*pointer(a)=v;} static uint32_t load8(uint32_t a){return *pointer(a);}
static uint32_t hx(const char*&p){while(*p==' ')p++;uint32_t v=strtoul(p,(char**)&p,16);return v;}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string line; int arrow=0,n=0,law=1; const uint32_t T=0x40800000u,L=0x40810000u,F=0x40820000u,S=0x40830000u,VT=0x40B80000u,CNT=0x40C00020u;
 auto R=[](int k){return 0x40A00000u+0x2000u*k;}; auto B=[](int k){return 0x40A30000u+0x2000u*k;}; auto V=[](int k){return 0x40A60000u+0x2000u*k;};
 auto car=[](uint32_t v){store(v+48,v+0x400);store(v+0x400+28,v+0x800);store(v+0x400+8,v+0xC00);store(v+0xC00+128,v+0x1000);};
 auto setv=[](uint32_t a,const char*&p){for(int i=0;i<3;i++)store(a+4*i,hx(p));};
 const uint32_t TV=0x40B00000u;
 while(std::getline(in,line)){ const char*p=line.c_str()+1;
  if(line[0]=='C'){ int a,b; sscanf(line.c_str(),"C %d %d %d %d %d",&arrow,&n,&a,&b,&law); rt_reset(); store(CNT,0);
   store(T,VT); store(VT+32,0x7F000010u); store(T+72,(uint32_t)n); store(T+76,L); store(T+164,(uint32_t)a); store(T+168,(uint32_t)b);
   store(S+12,F); store(F+20,T); store(S+16,0x4E6E6B28u);   // the lock 1e9: the formation never runs
   car(TV);
   for(int k=0;k<n;k++){ store(L+4*k,R(k)); store8(R(k)+872,k==0?(uint8_t)law:1); store(R(k)+876,B(k)); store(B(k)+4,V(k)); store(B(k)+680,TV); car(V(k)); }
  } else if(line[0]=='G'){ int k=(int)strtol(p,(char**)&p,10); uint32_t v=V(k);
   setv(v+0x800+16+48,p); setv(v+0x800+16,p); setv(v+0x800+16+32,p); setv(v+0x1000+272,p); store8(B(k)+774,(uint8_t)hx(p));
  } else if(line[0]=='T'){ setv(TV+0x800+16+48,p); setv(TV+0x1000+272,p);
  } else if(line[0]=='U'){
   for(int k=0;k<n;k++){ store8(B(k)+764,0); store(B(k)+748,0xDEADBEEFu); store(B(k)+752,0xDEADBEEFu); store(B(k)+756,0xDEADBEEFu); }
   Context c=fresh(); c.r3.u32=S; c.f1.f64=(double)(1.0f/60); if(arrow) CATCHUP(c,nullptr); else ALONG(c,nullptr);
   for(int k=0;k<n;k++) printf("%08x %08x %08x ",load(B(k)+756),load(B(k)+748),load(B(k)+752));
   printf("%08x %d %u\n",load(S+24),(int)load8(S+32),load(CNT));
  } else if(line[0]=='E'){ printf("end\n"); }
 }
}
'''


def f32hex(x):
    return '%08x' % struct.unpack('>I', struct.pack('>f', x))[0]


def cop_geo(rng, tpos, tdir):
    th = rng.uniform(0, 2 * math.pi)
    tilt = lambda: rng.uniform(-0.05, 0.05)
    right = (math.cos(th), tilt(), -math.sin(th))
    back = (math.sin(th), tilt(), math.cos(th))
    fwd = tuple(-x for x in back)
    u = rng.random()
    beta = math.radians(rng.uniform(125, 185) if u < 0.4 else rng.uniform(-5, 55) if u < 0.8 else rng.uniform(0, 180))
    beta = min(max(beta, 0.0), math.pi)
    s = 1 if rng.random() < 0.5 else -1
    d = rng.uniform(55, 65) if rng.random() < 0.1 else rng.uniform(0.5, 25)
    to = [right[i] * math.cos(beta) + fwd[i] * math.sin(beta) * s for i in range(3)]
    to[1] += rng.uniform(-0.1, 0.1)
    pos = [tpos[i] - to[i] * d for i in range(3)]
    sp = rng.uniform(5, 40) * (1 if rng.random() < 0.85 else -1)
    vel = [tdir[i] * sp + rng.uniform(-2, 2) for i in range(3)]
    return pos + list(right) + list(back) + vel


def gen_cases(rng):
    lines = []
    for c in range(CASES):
        arrow = c % 2 == 0
        n = 3 if arrow else 2
        a, b = rng.sample(range(n), 2)
        lines.append(f'C {int(arrow)} {n} {a} {b} {int(c % 8 not in (2, 3))}')

        def new_target():
            tpos = [rng.uniform(-500, 500), rng.uniform(-20, 20), rng.uniform(-500, 500)]
            th = rng.uniform(0, 2 * math.pi)
            tdir = [math.sin(th), 0.0, math.cos(th)]
            sp = rng.uniform(0, 40)
            return tpos, tdir, [tdir[i] * sp for i in range(3)]

        tpos, tdir, tvel = new_target()
        lines.append('T ' + ' '.join(f32hex(x) for x in tpos + tvel))
        hon = [rng.random() < 0.1 for _ in range(n)]
        for k in range(n):
            lines.append(f'G {k} ' + ' '.join(f32hex(x) for x in cop_geo(rng, tpos, tdir)) + f' {int(hon[k])}')
        for _ in range(N):
            if rng.random() < 0.03:
                tpos, tdir, tvel = new_target()
                lines.append('T ' + ' '.join(f32hex(x) for x in tpos + tvel))
                for k in range(n):
                    lines.append(f'G {k} ' + ' '.join(f32hex(x) for x in cop_geo(rng, tpos, tdir)) + f' {int(hon[k])}')
            for k in range(n):
                flip = rng.random() < 0.005
                if flip:
                    hon[k] = not hon[k]
                if rng.random() < 0.06 or flip:
                    lines.append(f'G {k} ' + ' '.join(f32hex(x) for x in cop_geo(rng, tpos, tdir)) + f' {int(hon[k])}')
            lines.append('U')
        lines.append('E')
    return lines


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = MAIN_IDS + RADIO + OTHER
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name(f): '' for f in RADIO}
    stubs[name('F-23825')] = 'ctx.r3.u64=0;'
    stubs[name('F-14569')] = 'store(0x40C00020u,load(0x40C00020u)+1);'
    stubs[name('F-29443')] = stubs[name('F-96080')] = 'ctx.r3.u64=0;'
    for f in ('F-28114', 'F-21534', 'F-80532'):
        stubs[name(f)] = 'throw std::runtime_error("unexpected call %s");' % f
    main_src = MAIN.replace('CATCHUP', name('F-47983')).replace('ALONG', name('F-41674'))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in MAIN_IDS], main_src, a.out, 'squad_swipe', image=img,
                      fakes={0x7F000010: 'ctx.f1.f64=30.0;'}, limit=800, stubs=stubs)
    rng = random.Random(7)
    lines = gen_cases(rng)
    cf = a.out / 'squad_swipe_cases.txt'
    cf.write_text('\n'.join(lines) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    res = iter(r.stdout.splitlines())
    out, prev, stats = [], None, {'upd': 0, 'swipeA': 0, 'swipeB': 0, 'hb': 0, 'wait': 0}
    for l in lines:
        if l[0] in 'CGT':
            out.append(l if l[0] != 'C' else 'CASE' + l[1:])
            if l[0] == 'C':
                prev = None
        elif l == 'U':
            row = next(res)
            stats['upd'] += 1
            f = row.split()
            stats['hb'] += any(f[3 * k + 2] != 'deadbeef' for k in range((len(f) - 3) // 3))
            stats['swipeA'] += any(f[3 * k] == 'bf800000' and f[3 * k + 1] == '3f800000' for k in range((len(f) - 3) // 3))
            stats['swipeB'] += any(f[3 * k] == '3f800000' and f[3 * k + 1] == '3f800000' for k in range((len(f) - 3) // 3))
            stats['wait'] += f[-3] != '00000000'
            out.append('=' if row == prev else row)
            prev = row
        elif l == 'E':
            assert next(res) == 'end'
            out.append('END')
    a.ref.write_text('\n'.join(out) + '\n')
    print(stats, '->', a.ref)


if __name__ == '__main__':
    main()
