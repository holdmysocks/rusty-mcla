"""A sound object's play and updates run offline: the original's common Play F-91431 and update driver F-68220 with what they
call - the settings combine F-63029, the play offsets F-90115, the attack F-37133, the release F-31923, the stop request
F-58485, the child test F-13519 and the prepare test F-72128 - the generated code whole and unchanged, on random sounds
(one node, a parent or none, a request entry or none) over random timelines. Stubbed, as inputs of the case: the type's
methods (Play and Prepare record the call; Update records it and returns the case's value: 0 once at a random update,
the type ending; Kill and ChildStop record it), the stop of the children F-84762, the fold F-61321 and the stream slot
lookup F-20252. Laid out as the original's: the pool global (+196 slot size, +200 request size, +232 bucket table),
bucket 0 (+11440 sound slots, +11444 request entries), the request table word the settings read (entry +199 = 0, so
the entry's fields at their own offsets), the type tables (Prepare, Kill, Play, ChildStop, Update, Duration) for type 12.
Not covered: variables (+12 .. +24, +92, +100 null), percent offsets (+57 bit 0x08, tristate 0), the deferred Prepare
(tristate 3 unset all the way up), ChildStop (+56 bit 0x08), the fold (+58 bit 0x80), children (slots +72 none) and
the parent predelay +96 = 0 (the port has none: the request's create parameters never set it).
Writes <out-dir>/sound_node_ref.txt: per case an 'N' line with the inputs, then per call ('p' the play, 'u' an update,
's' a stop before the update) the time, then '->' the return value, +6, +66, +40 bits, +44 bits, +54, +52, +48, +50 and
the stubs called (P play, U update, K kill, X children stopped)."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static char CALLS[16];static int NC,URET;static void rec(char c){if(NC<15)CALLS[NC++]=c;CALLS[NC]=0;}' + chr(10)

BT = 0x40100000      # bucket 0
SND = 0x40200000     # its sound slots, 256 bytes each: slot 0 the node, slot 1 its parent
REQ = 0x40300000     # its request entries, 256 bytes each
SLOT = 256
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def f32(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def gen(rnd):
    """one case: the node's fields, the parent's (or None), the request entry's (or None), the timeline"""
    pick = rnd.choice
    node = {
        8: (1, pick((0, 0, 0x80, 0x40, 0x20, 0x10, 0x08, 0xf8, rnd.randrange(256)))),
        10: (2, pick((-1, -1, 0, 90, 180, 359, rnd.randrange(360)))),
        28: (4, f32(pick((0.0, 0.0, -6.0, 3.5, rnd.uniform(-40, 10))))),
        32: (2, pick((0, 0, 100, -1200, rnd.randrange(-2400, 2400)))),
        34: (2, pick((0, 0, 0, 50, rnd.randrange(-600, 600)))),
        36: (4, f32(pick((0.0, 0.0, 0.0, -3.0, rnd.uniform(-20, 5))))),
        58: (1, pick((0x50, 0x50, 0x50, 0x40))),
        68: (4, pick((0, 0, 10, 100, 250, 750, 2000, rnd.randrange(1, 3000)))),
        88: (4, pick((0, 0, 0, 16, 100, 250, rnd.randrange(0, 2000)))),
        104: (4, pick((0, 0, 0, 20, 100, rnd.randrange(0, 3000)))),
        108: (4, pick((0, 0, 0, 10, 250, 1000, rnd.randrange(0, 4000)))),
        124: (2, pick((-1, -1, -1, 0, 45, rnd.randrange(360)))),
        136: (4, pick((-1, -1, 0, 100, 500, 1500, 2000, rnd.randrange(0, 3000))) & 0xffffffff),
    }
    par = None
    if rnd.random() < 0.6:
        par = {
            40: (4, f32(pick((0.0, -12.0, rnd.uniform(-60, 10))))),
            44: (4, f32(pick((0.0, rnd.uniform(-20, 5))))),
            48: (2, pick((23900, 23900, 8000, rnd.randrange(100, 24000)))),
            50: (2, pick((0, 0, 200, rnd.randrange(0, 3000)))),
            52: (2, pick((-1, -1, 0, 270, rnd.randrange(360)))),
            54: (2, pick((0, 0, 300, rnd.randrange(-2400, 2400)))),
        }
    req = None
    if rnd.random() < 0.5:
        req = {
            68: (4, f32(pick((0.0, rnd.uniform(-20, 5))))),
            80: (2, pick((0, 0, -300, rnd.randrange(-1200, 1200)))),
            82: (2, pick((-1, -1, 0, 30, rnd.randrange(360)))),
            88: (2, pick((23900, 23900, 5000, rnd.randrange(100, 24000)))),
            90: (2, pick((0, 0, 400, rnd.randrange(0, 2000)))),
            192: (4, pick((-1, -1, -1, 0, 300, rnd.randrange(0, 2500))) & 0xffffffff),
        }
    now = rnd.randrange(0, 100000)
    times = [now]
    n = rnd.randrange(3, 120)
    step = pick((16, 17, 33, 0))
    for _ in range(n):
        now += step if step else rnd.randrange(0, 120)
        if step == 16 and rnd.random() < 0.3:
            now += 1
        times.append(now)
    stop = rnd.randrange(1, n + 1) if rnd.random() < 0.7 else 0
    end = rnd.randrange(1, n + 1) if rnd.random() < 0.3 else 0
    return node, par, req, times, stop, end


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    play, upd, stop, kids_stop, fold, slot_lookup, onstop = (addr(a.tool, f) for f in (
        'F-91431', 'F-68220', 'F-58485', 'F-84762', 'F-61321', 'F-20252', 'F-23434'))
    # stand-ins for the type's methods (any functions; they are stubbed)
    m_prep, m_kill, m_play, m_cstop, m_upd, m_dur = (addr(a.tool, f) for f in (
        'F-68514', 'F-24532', 'F-78769', 'F-41741', 'F-79698', 'F-80575'))
    idx = index(a.recomp)
    src = lambda n: idx['sub_' + n].read_text(errors='replace')
    o = src(onstop)
    m = re.search(r'// lis (r\d+),(-?\d+)\s*\n(?:.*\n){0,3}?\s*// addi r7,\1,(-?\d+)', o[o.index('DEFINE_REX_FUNC(sub_%s)' % onstop):])
    pool = ((int(m.group(2)) << 16) + int(m.group(3))) & 0xffffffff
    # the type tables the update driver indexes by the type byte: lis -32125 / addi 6384 (Prepare), 6488 (Kill),
    # 6592 (Play), 6696 (ChildStop), 6800 (Update); the play offsets' 6904 (Duration)
    u = src(upd)
    u = u[u.index('DEFINE_REX_FUNC(sub_%s)' % upd):]
    u = u[:u.index('DEFINE_REX_FUNC', 20)]
    hi = int(re.search(r'// lis r9,(-?\d+)', u).group(1))
    tabs = sorted({int(x) for x in re.findall(r'// addi r1[01],r(?:9|10),(\d+)', u)})
    assert tabs == [6384, 6488, 6592, 6696, 6800], tabs
    tb = lambda off: ((hi << 16) + off + 11 * 4) & 0xffffffff     # type 12: entry -4 + 12 * 4
    # the request table word (lis -32125 / addi 26644, +4) the settings read the entry's half from
    rq = ((hi << 16) + 26648) & 0xffffffff
    rnd = random.Random(68220)
    cases = [gen(rnd) for _ in range(a.cases)]
    nl = chr(10)
    rows = []
    for node, par, req, times, st, end in cases:
        w = [(SND + o_, s, v) for o_, (s, v) in node.items()]
        w += [(SND + 5, 1, 1 if par else 255), (SND + 4, 1, 0 if req else 255)]
        if par:
            w += [(SND + SLOT + o_, s, v) for o_, (s, v) in par.items()]
        if req:
            w += [(REQ + o_, s, v) for o_, (s, v) in req.items()]
        rows.append('%d,%d,%d,%d,%s,%s,' % (len(w), len(times), st, end, ','.join('%du,%d,%du' % (x, y, z & 0xffffffff) for x, y, z in w),
                                           ','.join('%du' % t for t in times)))
    setup = ('store(%du+232,%du);store(%du+196,%d);store(%du+200,%d);store(%du+11440,%du);store(%du+11444,%du);store(%du,0);'
             % (pool, BT, pool, SLOT, pool, SLOT, BT, SND, BT, REQ, rq) +
             ''.join('store(%du,0x%su);' % (tb(off), f) for off, f in ((6384, m_prep), (6488, m_kill), (6592, m_play),
                                                                        (6696, m_cstop), (6800, m_upd), (6904, m_dur))))
    stubs = {
        'sub_' + m_prep: 'rec(\'R\');ctx.r3.u32=1;',
        'sub_' + m_kill: 'rec(\'K\');',
        'sub_' + m_play: 'rec(\'P\');',
        'sub_' + m_cstop: 'rec(\'C\');',
        'sub_' + m_upd: 'rec(\'U\');ctx.r3.u32=URET;',
        'sub_' + m_dur: 'ctx.r3.u32=0xffffffffu;',
        'sub_' + kids_stop: 'rec(\'X\');',
        'sub_' + fold: 'rec(\'F\');',
        'sub_' + slot_lookup: 'ctx.r3.u32=0;',
    }
    show = ('printf(" -> %%d %%u %%u %%08x %%08x %%d %%d %%u %%u %%s\\n",r,(unsigned)load16(%du+6),(unsigned)load16(%du+66),load(%du+40),'
            'load(%du+44),(int)(int16_t)load16(%du+54),(int)(int16_t)load16(%du+52),(unsigned)load16(%du+48),(unsigned)load16(%du+50),CALLS);'
            % ((SND,) * 8))
    call = lambda f, extra: '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;ctx.r4.u32=T[k];%ssub_%s(ctx,nullptr);r=ctx.r3.s32;}' % (STACK, SND, extra, f)
    main_src = (
        'static const uint32_t C[]={' + nl + nl.join(rows) + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + setup + nl +
        'for(size_t i=0;i<sizeof(C)/4;){uint32_t n=C[i],nt=C[i+1],st=C[i+2],en=C[i+3];const uint32_t*w=C+i+4;const uint32_t*T=w+3*n;i+=4+3*n+nt;'
        'memset(pointer(%du),0,2*%d);memset(pointer(%du),0,%d);' % (SND, SLOT, REQ, SLOT) +
        # the base sound's constructor values F-79360 for both, the parent a root without a request
        'for(int s=0;s<2;s++){uint32_t b=%du+s*%d;memset(pointer(b+72),255,8);*pointer(b+4)=255;*pointer(b+5)=255;store16(b+60,0xffff);'
        'store16(b+62,0xffff);*pointer(b+64)=0;*pointer(b+65)=253;*pointer(b+58)=0x50;*pointer(b+59)=12;store(b+112,0xaaaaaaaau);'
        'store16(b+124,0xffff);store(b+132,0xffffffffu);store16(b+52,0xffff);store16(b+48,23900);}' % (SND, SLOT) +
        'printf("N");for(uint32_t k=0;k<n;k++){uint32_t a=w[3*k],s=w[3*k+1],v=w[3*k+2];printf(" %u:%u",a-0x40000000u,v);'
        'if(s==4)store(a,v);else if(s==2)store16(a,v);else *pointer(a)=v;}printf("\\n");' + nl +
        'int r=0;bool dead=false;for(uint32_t k=0;k<nt&&!dead;k++){NC=0;CALLS[0]=0;URET=(en&&k==en)?0:1;'
        'if(k==0){printf("p %u",T[k]);' + call(play, '') + show + 'continue;}'
        'if(st&&k==st){printf("s %u",T[k]);' + call(stop, '') + 'printf("\\n");}'
        'printf("u %u",T[k]);' + call(upd, '') + show + 'if(r==0)dead=true;}'
        'printf("=\\n");}}' + nl)
    targets = [int(x, 16) for x in (m_prep, m_kill, m_play, m_cstop, m_upd, m_dur)]
    exe = rd.build(idx, ['sub_' + play, 'sub_' + upd, 'sub_' + stop], main_src, a.out_dir, 'sound_node', stubs=stubs,
                   image=rd.load_image(a.image), targets=targets)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sound_node_ref.txt').write_text(out)
    print(len(cases), 'cases,', out.count(' -> '), 'calls')


if __name__ == '__main__':
    main()
