"""Run the original's StartTactic F-33550 offline over every combination of the asked kind, the tactic already on the target
(none or its type, its may-escalate byte +28, its duration +104), the law record (none or its score), the request's show flag
(r6), the target being a racer and the tutorial byte, and record what it does, for the port's test
(crates/game/src/testdata/start_tactic_ref.txt; test police::tests::start_tactic_matches_the_original).

The manager's tactic slots (+44, one per type) are objects with fake vtables: start (vt+8: marks the slot active +20 and
returns 1), end (vt+12: clears +20), type (vt+56). The tactic on the target (F-59036) is the slot of its type, active. Stubbed
and recorded: the phone's interrupt F-51256, the stat gCPA + 1 through the stats manager's vt+48 (logged as "radio", code 2), the event POLICE_PURSUIT F-41015 (code 3), the
HUD text F-76838 (form 109 via F-53542), the pursuit tutorial F-24639. The gates at the top (network / camera / cutscene
objects, the menu byte) are zero, so they pass. The kind's floor table (tune at the image's 8282CAC8 region) gets 4001 .. 4004.

Line per case: <kind> <old type or -1> <b28> <rec: -1 none / score> <show> <racer> <tut>
  | ret: K (the slot of that type) / 0 | ended old | started | interrupt | radio | hud modes | tutorial | score | +104 bits
  | mgr +52 +64 +68 +72 +76 +80 +84
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
G_A, G_B, G_C = IMG + 0x8CCEA0, IMG + 0x86D8F0, IMG + 0x86D8D4     # the gates' objects (made null)
G_CAM, G_SPEECH, G_TUT = IMG + 0x86D804, IMG + 0x8CCF68, IMG + 0x8CCFC4
FLOORS = IMG + 0x82CAC8

MAIN = r'''
static uint32_t bits(float f){uint32_t u;memcpy(&u,&f,4);return u;}
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static void sb(uint32_t a,uint32_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((b&0xffu)<<sh);store(a&~3u,w);}
static uint32_t lb(uint32_t a){return (load(a&~3u)>>((3-(a&3))*8))&0xffu;}
static const uint32_t M=0x40800000u, SL=0x40810000u, VT=0x40820000u, TGT=0x40830000u, TVT=0x40831000u, REQ=0x40840000u,
 REC=0x40850000u, CAM=0x40860000u, SPO=0x40870000u, SPVT=0x40871000u, TUTO=0x40880000u, HUDO=0x40890000u, LOG=0x408A0000u;
static void logv(uint32_t v){ uint32_t n=load(LOG); store(LOG+4+4*n,v); store(LOG,n+1); }
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int kind=0;kind<5;kind++) for(int old=-1;old<5;old++) for(int b28=0;b28<2;b28++) for(int rs=-1;rs<3;rs++) for(int show=0;show<2;show++)
 for(int racer=0;racer<2;racer++) for(int tut=0;tut<2;tut++){
  if(old<0&&b28) continue;
  rt_reset();
  // (objects placed by hand are outside what rt_reset clears: zero them per case)
  for(uint32_t o=0;o<0x200;o+=4){ store(M+o,0); store(REC+o,0); store(TUTO+o,0); for(int k=0;k<6;k++) store(0x40900000u+0x1000u*k+o,0); }
  store(GA,0); store(GB,0); store(GC,0); store(GCAM,CAM); store(CAM+48,0x40861000u); store(GSPEECH,SPO); store(SPO,SPVT);
  store(SPVT+48,0x7F000031u); store(GTUT,TUTO); sb(TUTO+76,tut?0x40u:0u);
  for(int i=0;i<4;i++) store(FLOORS+4*i,4001u+i);
  store(M+44,SL);
  for(int k=0;k<6;k++){ uint32_t o=0x40900000u+0x1000u*k, v=VT+0x100u*k; store(SL+4*k,o); store(o,v);
   for(int s=0;s<24;s++) store(v+4*s,0x7F000040u); store(v+8,0x7F000041u); store(v+12,0x7F000042u); store(v+56,0x7F000050u+k);
   store(o+72,1); store(o+76,0x40A00000u+k); store(0x40A00000u+k,0x40B00000u+k); }
  store(TGT,TVT); store(TVT+40,racer?0x7F000061u:0x7F000060u);
  store(REQ,0); store(REQ+4,TGT); store(REQ+8,rs<0?0:REC); store(REC+8,(uint32_t)(rs==0?0:rs==1?4002:9999));
  uint32_t OLD=0;
  if(old>=0){ OLD=load(SL+4*old); sb(OLD+20,1); sb(OLD+28,b28); sf(OLD+104,7.25f); }
  store(0x40C00000u,OLD);
  for(int k=0;k<6;k++) sf(load(SL+4*k)+104,k==old?7.25f:-3.0f);
  store(M+52,0xEEEEEEEEu); for(int o=64;o<=84;o+=4) store(M+o,0xEEEEEEEEu);
  store(LOG,0);
  Context c=fresh(); c.r3.u32=M; c.r4.u32=(uint32_t)kind; c.r5.u32=REQ; c.r6.u32=(uint32_t)show; c.r7.u32=0; START(c,nullptr);
  uint32_t ret=c.r3.u32; int rk=-2; for(int k=0;k<6;k++) if(ret==load(SL+4*k)) rk=k; if(ret==0) rk=-1;
  printf("%d %d %d %d %d %d %d |",kind,old,b28,rs<0?-1:(int)load(REC+8),show,racer,tut);
  printf(" ret %d | log",rk); uint32_t n=load(LOG); for(uint32_t i=0;i<n;i++) printf(" %x",load(LOG+4+4*i));
  printf(" | +104 %08x | mgr",rk>=0?load(ret+104):0u); printf(" %08x",load(M+52)); for(int o=64;o<=84;o+=4) printf(" %08x",o==64?(load(M+64)>>24):load(M+o));
  printf("\n"); }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-33550', 'F-59036', 'F-12404', 'F-52539', 'F-28033', 'F-51256', 'F-72241', 'F-86113', 'F-24671', 'F-41015', 'F-73277',
           'F-23825', 'F-53542', 'F-76838', 'F-24639']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    # log codes: 1 interrupt, 2 radio, 3 career flag, 4x HUD mode x, 5 tutorial, 6k start slot k, 7k end slot k
    stubs = {name('F-59036'): 'ctx.r3.u64=load(0x40C00000u);', name('F-12404'): 'ctx.r3.u64=0;', name('F-52539'): 'ctx.r3.u64=0;',
             name('F-28033'): 'ctx.r3.u64=0;', name('F-51256'): 'logv(1);', name('F-41015'): 'logv(3);',
             name('F-53542'): 'ctx.r3.u64=0x40890000u;', name('F-76838'): 'logv(0x40+ctx.r4.u32);', name('F-24639'): 'logv(5);',
             name('F-73277'): 'logv(0xEE);', name('F-23825'): 'ctx.r3.u64=0;'}
    for f in ['F-72241', 'F-86113', 'F-24671']:
        stubs[name(f)] = 'throw std::runtime_error("unexpected call %s");' % f
    fakes = {0x7F000031: 'logv(2);', 0x7F000040: 'throw std::runtime_error("unexpected tactic slot");',
             0x7F000041: 'uint32_t k=(ctx.r3.u32-0x40900000u)>>12; logv(0x60+k); sb(ctx.r3.u32+20,1); ctx.r3.u64=1;',
             0x7F000042: 'uint32_t k=(ctx.r3.u32-0x40900000u)>>12; logv(0x70+k); sb(ctx.r3.u32+20,0);',
             0x7F000060: 'ctx.r3.u64=0;', 0x7F000061: 'ctx.r3.u64=1;'}
    for k in range(6):
        fakes[0x7F000050 + k] = 'ctx.r3.u64=%d;' % k
    # (stubs and fakes are compiled before main: the log append and the byte store written out)
    lg = lambda v: '{uint32_t n_=load(0x408A0000u); store(0x408A0004u+4*n_,(uint32_t)(%s)); store(0x408A0000u,n_+1);}' % v
    sbx = lambda a_, b_: '{uint32_t a_=%s; uint32_t w_=load(a_&~3u); int sh_=(3-(a_&3))*8; w_=(w_&~(0xffu<<sh_))|((%s&0xffu)<<sh_); store(a_&~3u,w_);}' % (a_, b_)
    import re
    fix = lambda b: re.sub(r'logv\(([^;]*)\);', lambda m: lg(m.group(1)), b)
    stubs = {k: fix(v) for k, v in stubs.items()}
    fakes = {k: fix(v).replace('sb(ctx.r3.u32+20,1);', sbx('ctx.r3.u32+20', '1u')).replace('sb(ctx.r3.u32+20,0);', sbx('ctx.r3.u32+20', '0u')) for k, v in fakes.items()}
    src = MAIN.replace('START', name('F-33550'))
    for k, v in [('GCAM', G_CAM), ('GSPEECH', G_SPEECH), ('GTUT', G_TUT), ('GA', G_A), ('GB', G_B), ('GC', G_C), ('FLOORS', FLOORS)]:
        src = src.replace(k, '0x%08Xu' % v)
    img = rdisp.load_image(a.image)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-33550')], src, a.out, 'start_tactic', image=img, stubs=stubs, fakes=fakes, limit=40)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    # the request blocks the Start* callers build before StartTactic: F-86662 (20 bytes), F-37640 (the Pursuit's, +20 its delay),
    # F-15449 (the dispatch's), each on 64 bytes of A5
    r2 = subprocess.run([a.tool, 'registry', 'addr', 'F-86662', 'F-37640', 'F-15449'], capture_output=True, text=True, check=True).stdout
    rq = {l.split()[0]: int(l.split()[1], 16) for l in r2.splitlines() if l.startswith('F-')}
    bm = (r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t BL=0x40A20000u; void(*f[3])(Context&,const uint8_t*)={B0,B1,B2}; const char*n[3]={"request","pursuit","dispatch"};
 for(int k=0;k<3;k++){ rt_reset(); for(int i=0;i<16;i++) store(BL+4*i,0xA5A5A5A5u); Context c=fresh(); c.r3.u32=BL; f[k](c,nullptr);
  printf("BLOCK %s",n[k]); for(int i=0;i<16;i++) printf(" %08x",load(BL+4*i)); printf("\n"); }
}
'''.replace('B0', 'sub_%08X' % rq['F-86662']).replace('B1', 'sub_%08X' % rq['F-37640']).replace('B2', 'sub_%08X' % rq['F-15449']))
    bexe = rdisp.build(rdisp.index(a.recomp), ['sub_%08X' % rq[f] for f in ['F-86662', 'F-37640', 'F-15449']], bm, a.out, 'request_blocks', image=img, limit=20)
    rb = subprocess.run([str(bexe), str(a.image)], capture_output=True, text=True, check=True)
    a.ref.write_text(rb.stdout + r.stdout)
    print(rb.stdout, len(r.stdout.splitlines()), 'cases ->', a.ref)


if __name__ == '__main__':
    main()
