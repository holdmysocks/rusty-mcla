"""Run the original's tactic end offline and record what it does to each cop, for the port's `police::tactic_end`
(crates/game/src/testdata/tactic_end_ref.txt; test police::tests::tactic_end_matches_the_original).

F-67316 (vt+12 of every tactic class; r3 the tactic, r4 the flag byte): with the active byte +20 clear it returns 0 and does
nothing. Otherwise it clears a byte of the global object (image +0x86D8E4, +0x642D2) when that object exists, then for each
of the +72 entries of the cop array +76 that is non-null: when the car has a brain (+876) the brain command F-24671 (r4 0,
r5 0), the brain's +773 = 1 and +408 = 0; when the flag's low byte is set the siren stop F-75318, and when the result +24 is
not 1 (arrested) the siren target F-78189 (r4 0) and, with no race current (F-27863), the stream unlock F-93264. Then the
class's vt+56, the debug log (the stripped printf), the class's vt+4, and it returns 1.

The callees are stubs that write an event list into guest memory (the code, the car, the argument); vt+56 and vt+4 are fakes
that log their call too. Cases: active 0 / 1 / 0x80, flag 0 / 1 / 0x100 (high byte only) / 0xFF, result 0 - 4 and 0xFFFFFFFF,
race 0 / 1, the global present or not, 0 - 4 cops, each a car with or without a brain or a null entry, the count +72 also
0 and -1.

  <active> <flag> <result> <race> <global> <count> <cops: n none / c car / b car with brain ...> ->
  <ret> <global byte> <events: code:cop[:arg] ...> <per cop +773 +408 (brain) or ->
  codes: C brain command (arg r4.r5), S siren stop, L siren target (arg), U stream unlock, V56 / V4 the class's slots
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
GLOBAL = IMG + 0x86D8E4         # the object whose byte +0x642D2 the end clears
EV = 0x40A00000                 # event list: count, then (code, car, arg) triples

MAIN = r'''
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
static uint8_t lb(uint32_t a){return (uint8_t)(load(a&~3u)>>((3-(a&3))*8));}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok;
 const uint32_t M=0x40800000u, VT=0x40810000u, ARR=0x40820000u, GO=0x40900000u, EV=0x40A00000u;
 while(in>>tok){ if(tok!="case") break; uint32_t act,flag,res,race,glob,cnt,n; in>>act>>flag>>res>>race>>glob>>cnt>>n;
  std::string ck[8]; for(uint32_t i=0;i<n;i++) in>>ck[i];
  rt_reset(); for(uint32_t a=M;a<M+256;a+=4) store(a,0); for(uint32_t a=EV;a<EV+1024;a+=4) store(a,0);
  store(EV+0x3F0,race);
  for(int i=0;i<24;i++) store(VT+4*i,0x7F000011u); store(VT+4,0x7F000012u); store(VT+56,0x7F000010u); store(M,VT);
  sb(M+20,(uint8_t)act); store(M+24,res); store(M+72,cnt); store(M+76,ARR); store(M+16,0); store(M+104,0);
  store(GLOBALP, glob?GO:0u); sb(GO+0x642D2,0x5A);
  for(uint32_t i=0;i<n;i++){ uint32_t c=0x40B00000u+i*0x10000u; if(ck[i]=="n"){store(ARR+4*i,0);continue;}
   store(ARR+4*i,c); for(uint32_t a=c;a<c+0x1000;a+=4) store(a,0);
   if(ck[i]=="b"){ uint32_t B=c+0x8000; store(c+876,B); sb(B+773,0x33); store(B+408,0x77777777u);} }
  Context c=fresh(); c.r3.u32=M; c.r4.u64=flag; END(c,nullptr);
  printf("%u %u %u %u %u %u",act,flag,res,race,glob,cnt); for(uint32_t i=0;i<n;i++) printf(" %s",ck[i].c_str());
  printf(" -> %u %u",(uint32_t)c.r3.u32,(uint32_t)lb(GO+0x642D2)); uint32_t ne=load(EV); printf(" %u",ne);
  for(uint32_t k=0;k<ne;k++) printf(" %u:%08x:%08x",load(EV+4+12*k),load(EV+8+12*k),load(EV+12+12*k));
  for(uint32_t i=0;i<n;i++){ uint32_t cc=0x40B00000u+i*0x10000u; if(ck[i]=="b"){uint32_t B=cc+0x8000; printf(" %u:%08x",(uint32_t)lb(B+773),load(B+408));} else printf(" -"); }
  printf("\n"); }
 printf("end\n");
}
'''

EVLOG = 'uint32_t n_=load(0x40A00000u); store(0x40A00004u+12*n_,CODE); store(0x40A00008u+12*n_,ctx.r3.u32); store(0x40A0000Cu+12*n_,ARG); store(0x40A00000u,n_+1);'


def cases():
    g = random.Random(67316)
    out = []
    kinds = ['n', 'c', 'b']
    lists = [[]] + [[k] for k in kinds] + [[a, b] for a in kinds for b in kinds] + [['b', 'b', 'b'], ['b', 'n', 'c'], ['c', 'b', 'n', 'b']]
    for act in (0, 1, 0x80):
        for flag in (0, 1, 0x100, 0xFF):
            for res in (0, 1, 2, 3, 4, 0xFFFFFFFF):
                for race in (0, 1):
                    for cl in lists:
                        glob = g.randrange(2)
                        out.append((act, flag, res, race, glob, len(cl), cl))
    for cnt in (0, 0xFFFFFFFF):
        out.append((1, 1, 0, 0, 1, cnt, ['b', 'b']))
    for _ in range(200):
        cl = [g.choice(kinds) for _ in range(g.randrange(0, 5))]
        out.append((g.choice((0, 1)), g.choice((0, 1, 0x100)), g.choice((0, 1, 2, 3)), g.randrange(2), g.randrange(2), len(cl), cl))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-67316', 'F-24671', 'F-75318', 'F-78189', 'F-27863', 'F-93264']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    ev = lambda code, arg: EVLOG.replace('CODE', str(code)).replace('ARG', arg)
    stubs = {name('F-24671'): ev(1, '(ctx.r4.u32<<16)|(ctx.r5.u32&0xffff)'),
             name('F-75318'): ev(2, '0'),
             name('F-78189'): ev(3, 'ctx.r4.u32'),
             name('F-93264'): ev(4, '0'),
             name('F-27863'): 'ctx.r3.u64=load(0x40A003F0u);'}
    fakes = {0x7F000010: ev(5, '0') + 'ctx.r3.u64=0;', 0x7F000012: ev(6, '0'), 0x7F000011: ''}
    src = MAIN.replace('END', name('F-67316')).replace('GLOBALP', '0x%08Xu' % GLOBAL)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-67316')], src, a.out, 'tactic_end', image=img, stubs=stubs, fakes=fakes)
    cs = cases()
    cf = a.out / 'end_cases.txt'
    cf.write_text(''.join('case %d %d %d %d %d %d %d %s\n' % (ac, fl, rs, rc, gl, cn, len(cl), ' '.join(cl)) for ac, fl, rs, rc, gl, cn, cl in cs)
                  + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    names = {1: 'C', 2: 'S', 3: 'L', 4: 'U', 5: 'V56', 6: 'V4'}
    rows = []
    for l in r.stdout.split('\n')[:-2]:
        lhs, rhs = l.split(' -> ')
        v = rhs.split()
        ret, gb, ne = v[0], v[1], int(v[2])
        evs = []
        for e in v[3:3 + ne]:
            code, car, arg = e.split(':')
            car = int(car, 16)
            who = '%d' % ((car - 0x40B00000) >> 16) if 0x40B00000 <= car < 0x40C00000 else 'T'
            s = '%s:%s' % (names[int(code)], who)
            if int(code) in (1, 3):
                s += ':x%x' % int(arg, 16)
            evs.append(s)
        brains = ['-' if x == '-' else 'k%s:x%s' % tuple(x.split(':')) for x in v[3 + ne:]]
        # (words with a letter prefix: no number in the file reads as a code address)
        rows.append('%s -> r%s g%s %s | %s' % (lhs, ret, gb, ' '.join(evs) or '.', ' '.join(brains) or '.'))
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
