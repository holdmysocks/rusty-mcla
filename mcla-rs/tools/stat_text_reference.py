"""A Stat objective's progress text run offline: the condition's vt#10 F-97850 (the mission log's "(n/target)" after an
objective), the generated code with the money text F-13827 -> F-36073 and its separator inserter. The condition object at
0x3000: +40 complete byte, +120 the statistic type text, +156 the target; the output buffer at 0x2000 holds a prefix.
Stubbed: the progress F-13232 (answers the case's value), the formatter F-32585 and the C runtime's sprintf F-98741
(host snprintf: %s from guest memory, %d as a signed word; every argument read before the buffer is written, as the
condition hands the buffer itself as the first %s). The money object (the word at 0x8286D7FC) at 0x1000 with the
language at +28. Writes <out-dir>/stat_text_ref.txt: "type|prefix|complete progress target lang -> text"."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

TYPES = ['Tally', 'Money', 'Purse', 'None', 'tally', 'MONEY', 'purse', 'Talley', 'Money ', '']
VALS = [0, 1, 2, 5, 9, 10, 25, 999, 1000, 1500, 25000, 100000, 1234567, -1, -1000, 2147483647]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]
    f = addr('F-97850')
    nl = chr(10)
    # a small printf: the format at the given register, arguments from the next registers (r5.. for sprintf, r6.. for the formatter)
    fmt = ('static std::string cstr(uint32_t a){std::string s;while(*pointer(a))s+=char(*pointer(a++));return s;}' + nl +
           'static std::string pf(const std::string&f,const uint32_t*args){std::string o;int k=0;std::vector<std::string> v;'
           'for(size_t i=0;i<f.size();i++){if(f[i]==37&&i+1<f.size()){char c=f[++i];if(c==115)v.push_back(cstr(args[k++]));'
           'else if(c==100)v.push_back(std::to_string((int32_t)args[k++]));else throw std::runtime_error("fmt");}}'
           'k=0;for(size_t i=0;i<f.size();i++){if(f[i]==37&&i+1<f.size()){i++;o+=v[k++];}else o+=f[i];}return o;}' + nl +
           'static void put(uint32_t a,std::string s,uint32_t n){if(n&&s.size()>=n)s.resize(n-1);for(size_t i=0;i<=s.size();i++)*pointer(a+i)=i<s.size()?s[i]:0;}' + nl +
           'static int32_t g_prog;' + nl)
    recomp_closure.EXTRA += fmt
    stubs = {
        'F-13232': 'ctx.r3.u32=(uint32_t)g_prog;',
        'F-32585': 'uint32_t ar[3]={ctx.r6.u32,ctx.r7.u32,ctx.r8.u32};std::string s=pf(cstr(ctx.r5.u32),ar);put(ctx.r3.u32,s,ctx.r4.u32);ctx.r3.u32=s.size();',
        'F-98741': 'uint32_t ar[3]={ctx.r5.u32,ctx.r6.u32,ctx.r7.u32};std::string s=pf(cstr(ctx.r4.u32),ar);put(ctx.r3.u32,s,0);ctx.r3.u32=s.size();',
    }
    stubs = {'sub_' + addr(k): v for k, v in stubs.items()}
    rnd = random.Random(97850)
    cs = []
    for t in TYPES:
        for _ in range(40):
            cs.append((t, rnd.choice(('Win 5 races', 'Earn', '')), rnd.choice((0, 0, 1)), rnd.choice(VALS), rnd.choice(VALS), rnd.choice((0, 0, 1, 2, 3, 4, 5))))
    rows = (',' + nl).join('{"%s","%s",%d,%d,%d,%d}' % c for c in cs)
    NL = chr(92) + 'n'
    body = ('struct K{const char*t;const char*p;int done;int32_t prog,target;int lang;};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x8286D7FC,0x1000);store(0x1000+28,c.lang);'
            '*pointer(0x3000+40)=c.done;put(0x3000+120,c.t,0);store(0x3000+156,(uint32_t)c.target);g_prog=c.prog;put(0x2000,c.p,0);'
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x3000;ctx.r4.u32=0x2000;ctx.r5.u32=256;'
            'sub_' + f + '(ctx,nullptr);printf("%s' + NL + '",cstr(0x2000).c_str());}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'stat_text', stubs=stubs, scratch_end=0x8000)
    out = run(exe, a.image).split(nl)[:len(cs)]
    assert len(out) == len(cs)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'stat_text_ref.txt').write_text(''.join('%s|%s|%d %d %d %d -> %s' % (*c, o) + nl for c, o in zip(cs, out)))
    print(len(cs), 'cases')


if __name__ == '__main__':
    main()
