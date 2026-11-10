"""Run the original's cop zone set-off (the zone machine's state 1) offline and record it, for the port's `police::zone_set_off`
(crates/game/src/testdata/zone_setoff_ref.txt; test police::tests::zone_set_off_matches_the_original).

F-71212 (r3 the police manager) with +196 = 1 and the player in no police car (the byte at image +0x88E5B4 clear): with a race
(the race manager at image +0x874374, F-37636 true, F-10499 not 5) and the zone +188 in [0, zones +184) it walks the race's
racers (+264, the next by F-76515) in order. A racer whose place (F-35658's matrix +48) is within the zone's radius
(d = racer - zone, dy dy, fma dz, fma dx, < r x r) sets it off: the request (F-35677, F-14569 - the radio's own generator),
then +212 = 5.5, +200 = 0, +196 = 2 and, with no scripted target +208, the target by the kind +204: 1 the racer, 2 the
nearest of the AI list (image +0x86EBE4: count u16 +12, entries +8; entry 0 null - the racer; nulls skipped; d as above, the
first of the equal) within fRaceTgtAiDist (tune +424; negative: any distance), 0 the same after one draw of the shared
generator (image +0x860890, a = 0x5CDCFAA7: (lo & 0x7FFFFF) x 2^-23 < iRaceTgtAiProbPrcnt (+420, int) x 0.01; no AI list:
the racer, no draw), 3 and up none - the next racer inside sets it off again. A target ends the walk. A scripted target +208
(SetRaceChaseTarget F-12315's racer) found in the racer list (image +0x8745E4: entries +0, count +400) is the target, whatever the kind.

  T <state 2 / 3> <target 0 / 1> f<+212> f<dt> <damaged out> f<fCopZoneRdoDelayTime> -> <+196> f<+212> <F-12032 calls> t<F-30913: f1 | not the target> s<StartTactic>
  <kind> <prob> f<dist bits> f<r bits> <zone x y z bits> <racers: x:y:z bits ...> | <ai: null / count, then x:y:z or n ...>
  s<+208: - none / the racer> m<the racers in the racer list, a bit each>
  g<lo> g<carry> -> <+196> <+200: 0 / r<i> / a<j>> f<+212 bits> <requests> g<lo> g<carry>
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
RNG = IMG + 0x860890
RACEG = IMG + 0x874374
RLG = IMG + 0x8745E4            # the racer list (inline entries, count +400)
AIG = IMG + 0x86EBE4
POLB = IMG + 0x88E5B4
TUNE = IMG + 0x82CAC8
MGRG = IMG + 0x86D8E0           # the police manager

MAIN = r'''
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t M=0x40800000u, Z=0x40810000u, R=0x40820000u, AL=0x40830000u, G0=0x40A00000u;
 while(in>>tok){
  if(tok=="tick"){ uint32_t st,tg,tm,dt,wr,dl; in>>st>>tg>>tm>>dt>>wr>>dl;
   rt_reset(); for(uint32_t a=M;a<M+256;a+=4) store(a,0); for(uint32_t a=G0;a<G0+64;a+=4) store(a,0);
   store(MGRG,M); store(M+196,st); store(M+200,tg?0x40B00000u:0u); store(M+212,tm); store(G0+32,wr); store(TUNE+392,dl); sb(POLB,0);
   Context c=fresh(); c.r3.u32=M; float f; memcpy(&f,&dt,4); c.f1.f64=(double)f; ZONE(c,nullptr);
   printf("%u %u %u %u %u %u\n",load(M+196),load(M+212),load(G0+36),load(G0+40),load(G0+44),load(G0+48)); continue; }
  if(tok!="case") break; uint32_t kind,prob,dist,rad,zx,zy,zz,nr,ai,na,lo,ca; in>>kind>>prob>>dist>>rad>>zx>>zy>>zz>>nr;
  uint32_t rp[8][3]; for(uint32_t i=0;i<nr;i++) in>>rp[i][0]>>rp[i][1]>>rp[i][2];
  in>>ai>>na; uint32_t ap[8][4]; for(uint32_t j=0;j<na;j++) in>>ap[j][0]>>ap[j][1]>>ap[j][2]>>ap[j][3];
  uint32_t s208,mask; in>>s208>>mask>>lo>>ca;
  rt_reset(); sb(load(fresh().r13.u32)+72,0);
  for(uint32_t a=M;a<M+256;a+=4) store(a,0); for(uint32_t a=G0;a<G0+64;a+=4) store(a,0);
  store(M+180,Z); store(Z,Z+0x100); store(M+184,(1u<<16)); store(M+188,0); store(M+196,1); store(M+204,kind); store(M+208, s208<nr?0x40B00000u+s208*0x1000u:0u); store(M+212,0x7F7F7F7Fu);
  store(Z+0x100,zx); store(Z+0x104,zy); store(Z+0x108,zz); store(Z+0x110,rad);
  store(TUNE+420,prob); store(TUNE+424,dist); sb(POLB,0);
  store(RACEG,R); store(R+264, nr?0x40B00000u:0u);
  for(uint32_t i=0;i<nr;i++){ uint32_t c=0x40B00000u+i*0x1000u; store(c+0x100, i+1<nr?c+0x1000u:0u);
   store(c+0x230,rp[i][0]); store(c+0x234,rp[i][1]); store(c+0x238,rp[i][2]); }
  { const uint32_t RL=0x40840000u; store(RLG,RL); uint32_t k=0; for(uint32_t i=0;i<nr;i++) if(mask>>i&1) store(RL+4*k++,0x40B00000u+i*0x1000u); store(RL+400,k); }
  store(AIG, ai?AL:0u); store(AL+12,na<<16); store(AL+8,AL+0x100);
  for(uint32_t j=0;j<na;j++){ uint32_t c=0x40C00000u+j*0x1000u; store(AL+0x100+4*j, ap[j][3]?0u:c);
   store(c+0x230,ap[j][0]); store(c+0x234,ap[j][1]); store(c+0x238,ap[j][2]); }
  store(RNG,lo); store(RNG+4,ca);
  Context c=fresh(); c.r3.u32=M; c.f1.f64=0.0; ZONE(c,nullptr);
  uint32_t t=load(M+200); std::string tl="0"; char b[32];
  if(t>=0x40B00000u&&t<0x40C00000u){snprintf(b,32,"r%u",(t-0x40B00000u)>>12);tl=b;} else if(t>=0x40C00000u&&t<0x40D00000u){snprintf(b,32,"a%u",(t-0x40C00000u)>>12);tl=b;}
  else if(t){snprintf(b,32,"x%08x",t);tl=b;}
  printf("%u %s %u %u %u %u\n",load(M+196),tl.c_str(),load(M+212),load(G0),load(RNG),load(RNG+4)); }
 printf("end\n");
}
'''


def fb(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]


def cases():
    g = random.Random(71212)
    seed = lambda: (g.getrandbits(32), g.getrandbits(32) % 0x5CDCFAA7)
    out = []
    pt = lambda s: tuple(fb(g.uniform(-s, s)) for _ in range(3))
    for _ in range(4000):
        kind = g.choice((0, 0, 0, 1, 2, 2, 3, 7))
        prob = g.choice((0, 30, 30, 50, 100, 1, 99, -1, 150, g.randrange(0, 101)))
        dist = g.choice((150.0, 150.0, -1.0, 0.0, 10.0, 40.0, 1e30, float('nan'), g.uniform(0, 80)))
        r = g.choice((20.0, 30.0, 50.0, 0.0, g.uniform(1, 60)))
        zone = tuple(fb(v) for v in (g.uniform(-500, 500), g.uniform(-20, 20), g.uniform(-500, 500)))
        zf = [struct.unpack('>f', struct.pack('>I', w))[0] for w in zone]
        near = lambda s: tuple(fb(zf[k] + g.uniform(-s, s)) for k in range(3))
        racers = [near(g.choice((5, 15, 30, 60, 200))) for _ in range(g.randrange(0, 6))]
        ai = g.random() < 0.9
        na = g.choice((0, 1, 2, 3, 4, 5, 6)) if ai else 0
        ais = [near(g.choice((20, 60, 150, 400))) + (1 if g.random() < 0.12 else 0,) for _ in range(na)]
        if ais and g.random() < 0.1:                      # equal distances: the first of the equal
            ais.append(ais[0][:3] + (0,))
        s208 = g.randrange(len(racers)) if racers and g.random() < 0.25 else 99
        mask = g.getrandbits(6) if g.random() < 0.3 else 63
        out.append((kind, prob & 0xFFFFFFFF, fb(dist), fb(r), zone, racers, ai, ais, s208, mask) + seed())
    return out


def ticks():
    g = random.Random(712122)
    out = []
    for _ in range(2000):
        tm = g.choice((5.5, 0.0, -0.0, 1e-9, 1.0 / 30.0, 1.0 / 60.0, 2.0, g.uniform(-0.1, 6.0), g.uniform(0.0, 0.05)))
        dt = g.choice((1.0 / 30.0, 1.0 / 60.0, 0.0, 0.05, g.uniform(0.0, 0.1)))
        out.append((g.choice((2, 3)), g.randrange(2), fb(tm), fb(dt), g.randrange(2), fb(g.choice((2.0, 0.0, -1.0, 3.5)))))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-71212', 'F-37636', 'F-55479', 'F-10499', 'F-76515', 'F-35658', 'F-47121', 'F-35677', 'F-24338', 'F-27863', 'F-45180',
           'F-23825', 'F-14569', 'F-55001', 'F-12032', 'F-50742', 'F-93781', 'F-86662', 'F-56787', 'F-30913', 'F-33550']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name(f): '' for f in ids[1:]}
    stubs[name('F-37636')] = 'ctx.r3.u64=1;'
    stubs[name('F-10499')] = 'ctx.r3.u64=0;'
    stubs[name('F-76515')] = 'uint32_t n_=load(ctx.r4.u32); store(ctx.r4.u32,load(n_+0x100));'
    stubs[name('F-35658')] = 'ctx.r3.u64=ctx.r3.u32+0x200u;'
    stubs[name('F-24338')] = 'ctx.r3.u64=1;'
    # (the ticks: F-12032 the target damaged out - the case's answer, counted; F-30913 the Arrow Trap's tactic, its f1 recorded;
    # StartTactic F-33550 (r4 type, r6, r7) recorded)
    stubs[name('F-12032')] = 'store(0x40A00024u,load(0x40A00024u)+1); ctx.r3.u64=load(0x40A00020u);'
    stubs[name('F-30913')] = 'float f_=(float)ctx.f1.f64; uint32_t u_; memcpy(&u_,&f_,4); store(0x40A00028u,u_|((ctx.r6.u32==0x40B00000u)?0u:1u)); ctx.r3.u64=0x40A00100u;'
    stubs[name('F-33550')] = 'store(0x40A0002Cu,1u+ctx.r4.u32*16u+(ctx.r5.u32==0x40A00100u?0u:0x100u)+ctx.r6.u32*0x1000u+ctx.r7.u32*0x10000u); store(0x40A00030u,ctx.r3.u32);'
    stubs[name('F-14569')] = 'store(0x40A00000u,load(0x40A00000u)+1);'
    src = (MAIN.replace('ZONE', name('F-71212')).replace('RACEG', '0x%08Xu' % RACEG).replace('AIG', '0x%08Xu' % AIG).replace('RLG', '0x%08Xu' % RLG)
           .replace('POLB', '0x%08Xu' % POLB).replace('TUNE', '0x%08Xu' % TUNE).replace('RNG', '0x%08Xu' % RNG).replace('MGRG', '0x%08Xu' % MGRG))
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-71212')], src, a.out, 'zone_setoff', image=img, stubs=stubs)
    cs = cases()
    cf = a.out / 'setoff_cases.txt'
    with cf.open('w') as f:
        for kind, prob, dist, rad, zone, racers, ai, ais, s208, mask, lo, ca in cs:
            f.write('case %d %d %d %d %d %d %d %d %s %d %d %s %d %d %d %d\n' % (
                kind, prob, dist, rad, *zone, len(racers), ' '.join('%d %d %d' % p for p in racers), ai, len(ais),
                ' '.join('%d %d %d %d' % p for p in ais), s208, mask, lo, ca))
        for t in ticks():
            f.write('tick %d %d %d %d %d %d\n' % t)
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.split('\n')[:-2]
    rows = []
    for (kind, prob, dist, rad, zone, racers, ai, ais, s208, mask, lo, ca), l in zip(cs, lines[:len(cs)]):
        st, tl, t212, nreq, lo2, ca2 = l.split()
        # (every hex word with a letter prefix: no number in the file reads as a code address)
        rows.append('%d %d f%08x f%08x %s %s | %s s%s m%d g%08x g%08x -> %s %s f%08x %s g%08x g%08x' % (
            kind, prob if prob < 0x80000000 else prob - (1 << 32), dist, rad, ':'.join('%08x' % w for w in zone),
            ' '.join(':'.join('%08x' % w for w in p) for p in racers) or '.',
            ('%d ' % len(ais) + ' '.join('n' if p[3] else ':'.join('%08x' % w for w in p[:3]) for p in ais)).strip() if ai else 'null',
            '-' if s208 == 99 else s208, mask, lo, ca, st, tl, int(t212), nreq, int(lo2), int(ca2)))
    for (st, tg, tm, dt, wr, dl), l in zip(ticks(), lines[len(cs):]):
        st2, tm2, nw, tac, start, mgr = (int(x) for x in l.split())
        assert mgr in (0, 0x40800000), mgr       # (StartTactic on the manager)
        # (StartTactic: s<1 + type x 16 + the tactic not F-30913's 0x100 + r6 x 0x1000 + r7 x 0x10000>; t<F-30913's f1 bits | not the target>)
        rows.append('T %d %d f%08x f%08x %d f%08x -> %d f%08x %d t%08x s%x' % (st, tg, tm, dt, wr, dl, st2, tm2, nw, tac, start))
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
