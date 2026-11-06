"""The ambient rider's frame F-87488 (the behaviour's update: the state check, the switch on the state +272, then the
tail every state runs) run offline: the generated code whole and unchanged, on flat guest memory, its callees stubbed
with the case's answers and logged in call order. What it shows: which steps run in which state and in which order,
where the pass tracker (behaviour +480) is cleared, and that the tracker F-73744 runs after the switch in every state.

Stubs (each logs its name; answers from the case): the state check F-94409 (nothing), the four behaviour slots 18 / 19 /
20 / 21 (0), the follower's step (states 1 - 3, 6 - 8: the rail's end), the obstacle search, the lane part, the speed
part, the recovery try F-15490 (answers by call: the switch's, the tail's), the tracker clear, the recovery clear, the
backing step F-82779, the stuck tracker's zeroing, the rail unload, the stuck test F-14023, the backing start
(F-56052's thunk), the passing gate F-72757 (an obstacle or 0), the tracker F-73744 (its obstacle or 0) and the passing
try F-28706 (checked to get the gate's obstacle). F-87488's own test of the backing time (behaviour +536 > 0) runs
real. The world byte +0x64251 (the race event) and the obstacle words +432 / +436 from the case. (F-87488 first adds 1 to the
word at r4 + 4 x state: for states 9 and 10 that is r4 +36 - the pointer the event byte is read through - and r4 +40;
the case's byte is put at +0x64251 and +0x64252 so that increment does not move it. r4's block in the game is not
traced.)

Writes <out-dir>/rider_frame_ref.txt, a case a line: state, event, +432, +436, backing time (hex bits), the answers
(follow, rec1, rec2, back_step, stuck, back_start, gate, tracker, pass) ' -> ' the calls in order (indices into
NAMES, comma-terminated) and the state after (test `rider_frame_matches_the_original` in `traffic.rs`)."""
import argparse
import random
import subprocess
from pathlib import Path
import traffic_select_reference as tsr  # noqa: F401 (sets up the vector shim and the indirect-call dispatch)
from traffic_select_reference import addr, fb, NL
import recomp_closure
from recomp_closure import index, build

NAMES = ['follow', 'search', 'lane', 'speed', 'recovery', 'clear_tracker', 'clear_recovery', 'back_step', 'zero_stuck',
         'unload', 'stuck', 'back_start', 'gate', 'tracker', 'pass']
# (the callees without an ID of their own are named by the IDs the registry gives them)
FIDS = {'recovery': 'F-15490', 'stuck': 'F-14023', 'gate': 'F-72757', 'tracker': 'F-73744', 'pass': 'F-28706'}


def callees(tool, fn):
    """F-87488's direct callees in the order the listing names them (the IDs above are checked against these)."""
    from recomp_leaf import extract
    import re
    idx = index(Path(ARGS.recomp))
    s = extract(idx['sub_' + fn], 'sub_' + fn)
    out = []
    for m in re.finditer(r'\b(sub_[0-9A-F]{8})\(ctx, base\)', s):
        if m.group(1) not in out:
            out.append(m.group(1))
    return out


ARGS = None


def main():
    global ARGS
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    ARGS = a
    fn = addr(a.tool, 'F-87488')
    # F-87488's callees in listing order: the state check, then the switch's and the tail's
    cl = callees(a.tool, fn)
    order = ['check', 'follow', 'search', 'lane', 'speed', 'recovery', 'clear_tracker', 'clear_recovery', 'back_step',
             'zero_stuck', 'unload', 'stuck', 'back_start', 'backing', 'gate', 'tracker', 'pass']
    assert len(cl) == len(order), cl
    sub = dict(zip(order, cl))
    for k, v in FIDS.items():
        if v:
            assert addr(a.tool, v) == sub[k][4:], (k, v)
    rnd = random.Random(87488)
    cs = []
    for _ in range(a.cases):
        st = rnd.randrange(0, 11)
        ev = int(rnd.random() < 0.25)
        n432 = rnd.choice((0, 0x7000, 0x7200))
        n436 = rnd.choice((0, 0x7000, 0x7200, n432))
        bt = rnd.choice((0.0, 0.0, 0.5, -0.0, 4.0))
        ans = [int(rnd.random() < 0.5) for _ in range(9)]
        ans[6] = rnd.choice((0, 0x7200))
        ans[7] = rnd.choice((0, 0x7300))
        cs.append([st, ev, n432, n436, fb(bt)] + ans)

    def log(k):
        return 'L(%d);' % NAMES.index(k)
    stubs = {
        sub['check']: '',
        sub['follow']: log('follow') + 'ctx.r3.u64=A[0];',
        sub['search']: log('search'),
        sub['lane']: log('lane'),
        sub['speed']: log('speed'),
        sub['recovery']: log('recovery') + 'ctx.r3.u64=A[1+(nrec++>0)];',
        sub['clear_tracker']: log('clear_tracker'),
        sub['clear_recovery']: log('clear_recovery'),
        sub['back_step']: log('back_step') + 'ctx.r3.u64=A[3];',
        sub['zero_stuck']: log('zero_stuck'),
        sub['unload']: log('unload'),
        sub['stuck']: log('stuck') + 'ctx.r3.u64=A[4];',
        sub['back_start']: log('back_start') + 'ctx.r3.u64=A[5];',
        sub['gate']: log('gate') + 'ctx.r3.u64=A[6];',
        sub['tracker']: log('tracker') + 'ctx.r3.u64=A[7];',
        sub['pass']: log('pass') + 'if(ctx.r5.u32!=A[6])bad=1;ctx.r3.u64=A[8];',
    }
    recomp_closure.EXTRA += ('static uint32_t A[9];static int nrec,bad;static std::string LOG;'
                             'static void L(int k){char b[8];snprintf(b,8,"%d,",k);LOG+=b;}' + NL)
    setup = r'''
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a>=0x9001&&a<=0x9004){ctx.r3.u64=0;return;}
printf("bad call %08x\n",a);throw std::runtime_error("call");}
'''
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));LOG.clear();nrec=0;bad=0;for(int i=0;i<9;i++)A[i]=w[5+i];
uint32_t Bh=0x10000,E=0x20000,P=0x4000;store(Bh,0x2000);for(int i=0;i<4;i++)store(0x2000+72+4*i,0x9001+i);
store(Bh+28,0x5000);store(Bh+272,w[0]);store(Bh+432,w[2]);store(Bh+436,w[3]);store(Bh+536,w[4]);
store(E+36,P);*pointer(P+0x64251)=uint8_t(w[1]);*pointer(P+0x64252)=uint8_t(w[1]);
Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=Bh;ctx.r4.u32=E;ctx.r5.u32=0x6000;%s(ctx,nullptr);
printf("%%s %%u%%s\n",LOG.empty()?"-":LOG.c_str(),load(Bh+272),bad?" bad":"");}
}
''' % ('sub_' + fn)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], setup + body, a.out_dir, 'rider_frame', stubs=stubs, vector=True)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'rider_frame_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs) and not any('bad' in o for o in out), (len(out), r.stdout[-300:])
    lines = []
    for c, o in zip(cs, out):
        line = ' '.join('%x' % x for x in c) + ' -> ' + o
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'rider_frame_ref.txt').write_text(NL.join(lines) + NL)
    tr = NAMES.index('tracker')
    print(len(lines), 'cases;', sum(str(tr) in o.split()[0].split(',') for o in out), 'ran the tracker')


if __name__ == '__main__':
    main()
