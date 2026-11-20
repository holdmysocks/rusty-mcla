"""The ambient manager's race / garage byte step (F-86550), run offline: the generated code whole and unchanged.

Each case sets the garage byte (0x82908D5D), the manager's bytes (+0x64250 .. +0x64253, the threaded byte +0x64259), the
current race (F-27863's answer: none, or an object with its type +3096), the race's two answers F-18769 (low byte) and
F-98822 (word), and the stack byte the message is made from; F-99126 (the removal) and F-60395 (the threaded queue) log
the message byte, the railnet's vfunc 4 (S+0x64140) logs its call.

Writes <out-dir>/garage_latch_ref.txt:
  "G <garage> <latch> <threaded> <race> <type> <done> <word> <race byte> <stack> -> <latch> <M xx | Q xx | -> <+0x64250>
   <+0x64251> <+0x64252> <+0x64254> <V | ->" (hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n')
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases(rnd, count):
    out = []
    for _ in range(count):
        out.append([rnd.choice((0, 0, 1, 1, 0xFF)), rnd.choice((0, 0, 1, 1, 2)), rnd.choice((0, 0, 0, 1)), rnd.choice((0, 1, 1)),
                    rnd.choice((0, 1, 2, 3, 4, 5, 6, 0xFFFFFFFF)), rnd.choice((0, 0, 1, 0x100)), rnd.choice((0, 0, 1, 7)),
                    rnd.choice((0, 0, 1)), rnd.randrange(256)])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    idx = index(a.recomp)
    fids = ('F-86550', 'F-99126', 'F-60395')
    nm = {f: addr(a.tool, f) for f in fids}
    s = lambda f: 'sub_' + nm[f]
    # the race getters: called by address in F-86550's body (their IDs are read from the body itself)
    body86 = idx[s('F-86550')].read_text(errors='replace')
    start = body86.index('DEFINE_REX_FUNC(%s)' % s('F-86550'))
    fn = body86[start:body86.index('DEFINE_REX_FUNC', start + 10)]
    calls = []
    for t in fn.split('sub_')[1:]:
        c = 'sub_' + t[:8]
        if c not in calls and c not in (s('F-99126'), s('F-60395')) and t[8:9] == '(':
            calls.append(c)
    calls = [c for c in calls if not c.startswith('sub_823D9')]
    assert len(calls) == 3, calls
    getter, done, word = calls
    kc = cases(random.Random(86550), a.cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'garage_latch_inputs.bin'
    inp.write_bytes(struct.pack('<%dI' % (9 * len(kc)), *[w for c in kc for w in c]))
    stubs = {getter: 'ctx.r3.u32=g_race;', done: 'ctx.r3.u32=g_done;', word: 'ctx.r3.u32=g_word;',
             s('F-99126'): 'printf(" M %02x",*pointer(ctx.r4.u32));', s('F-60395'): 'printf(" Q %02x",*pointer(ctx.r4.u32));'}
    glob = 'static uint32_t g_race=0,g_done=0,g_word=0;' + nl
    disp = r'''static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==0x3910){printf(" V");return;}
 throw std::runtime_error("unexpected indirect call");}
'''
    main_src = r'''int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});
FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);
const uint32_t S=0x1000,R=0x68000;
for(size_t i=0;i+9<=in.size();i+=9){const uint32_t*c=in.data()+i;
 memset(objects,0,sizeof objects);
 *pointer(0x82908D5D)=uint8_t(c[0]);
 *pointer(S+0x64253)=uint8_t(c[1]);*pointer(S+0x64259)=uint8_t(c[2]);
 g_race=c[3]?R:0;store(R+3096,c[4]);g_done=c[5];g_word=c[6];
 *pointer(S+0x64251)=uint8_t(c[7]);*pointer(S+0x64250)=0xAA;*pointer(S+0x64252)=0xAA;store(S+0x64254,0xAAAAAAAA);
 store(S+0x64140,0x3800);store(0x3800+16,0x3910);
 *pointer(0xFF00-128+80)=uint8_t(c[8]);
 Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x8000;ctx.r3.u32=S;
 printf("G");MAINFN(ctx,nullptr);
 printf(" | %02x %02x %02x %02x %08x\n",*pointer(S+0x64253),*pointer(S+0x64250),*pointer(S+0x64251),*pointer(S+0x64252),load(S+0x64254));}
}
'''.replace('MAINFN', s('F-86550'))
    fs = recomp_closure.closure(idx, [s('F-86550')], stubs)
    protos = ''.join(f'void {n}(Context &ctx,const uint8_t *base);' + nl for n in list(fs) + list(stubs))
    stubdefs = ''.join('void %s(Context &ctx,const uint8_t *base){%s}' % (n, b) + nl for n, b in stubs.items())
    shim = recomp_closure.shim(0x70000)
    cpp = a.out_dir / 'garage_latch.cpp'
    cpp.write_text(shim + recomp_closure.EXTRA + glob + protos + stubdefs + disp + ''.join(fs.values()) + main_src)
    exe = a.out_dir / 'garage_latch.exe'
    r = subprocess.run(['g++', '-std=c++17', '-O2', '-static', str(cpp), '-o', str(exe)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:3000])
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(kc), (len(out), len(kc))
    lines = []
    for c, o in zip(kc, out):
        assert o.startswith('G')
        ev, st = o[1:].split('|')
        ev = ev.split()
        msg = ' '.join(' '.join(ev[ev.index(k):ev.index(k) + 2]) for k in ('M', 'Q') if k in ev) or '-'
        lat, b50, b51, b52, w54 = st.split()
        lines.append('G %x %x %x %x %08x %x %x %x %02x -> %s %s %s %s %s %s %s' % (*c, lat, msg, b50, b51, b52, w54, 'V' if 'V' in ev else '-'))
    (a.out_dir / 'garage_latch_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases |', sum(' M ' in x or ' Q ' in x for x in lines), 'sending')


if __name__ == '__main__':
    main()
