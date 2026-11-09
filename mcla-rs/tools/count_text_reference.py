"""The mission log's "(n/target)" of the Experience, TallyRace and TallyMission conditions run offline: their vt#10
F-34957, F-62132 and F-95375 as generated. The condition object at 0x3000: +40 complete byte; Experience: +120 target,
+124 the base armed with it, the experience record found by the name at +88 (the look-up F-35527 stubbed: the record at
0x4000 with the value at +8, or 0 = not found); TallyRace: +88 target, +192 count; TallyMission: +120 target, +156
count. The output buffer at 0x2000 holds a prefix. The formatter F-32585 stubbed by a host printf (%s from guest memory,
%d as a signed word; the arguments read before the buffer is written). Writes <out-dir>/count_text_ref.txt:
"kind|prefix|complete count target base found -> text" (kind 0 Experience, 1 TallyRace, 2 TallyMission; for
Experience count is the experience value)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

VALS = [0, 1, 2, 3, 5, 9, 10, 99, 100, 1000, 25000, 123456, -1, -50, 2147483647, -2147483647]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]
    fs = [addr('F-34957'), addr('F-62132'), addr('F-95375')]
    nl = chr(10)
    recomp_closure.EXTRA += ('static std::string cstr(uint32_t a){std::string s;while(*pointer(a))s+=char(*pointer(a++));return s;}' + nl +
           'static std::string pf(const std::string&f,const uint32_t*args){std::string o;int k=0;std::vector<std::string> v;'
           'for(size_t i=0;i<f.size();i++){if(f[i]==37&&i+1<f.size()){char c=f[++i];if(c==115)v.push_back(cstr(args[k++]));'
           'else if(c==100)v.push_back(std::to_string((int32_t)args[k++]));else throw std::runtime_error("fmt");}}'
           'k=0;for(size_t i=0;i<f.size();i++){if(f[i]==37&&i+1<f.size()){i++;o+=v[k++];}else o+=f[i];}return o;}' + nl +
           'static void put(uint32_t a,std::string s,uint32_t n){if(n&&s.size()>=n)s.resize(n-1);for(size_t i=0;i<=s.size();i++)*pointer(a+i)=i<s.size()?s[i]:0;}' + nl +
           'static int g_found;' + nl)
    stubs = {
        'F-35527': 'ctx.r3.u32=g_found?0x4000:0;',
        'F-32585': 'uint32_t ar[3]={ctx.r6.u32,ctx.r7.u32,ctx.r8.u32};std::string s=pf(cstr(ctx.r5.u32),ar);put(ctx.r3.u32,s,ctx.r4.u32);ctx.r3.u32=s.size();',
    }
    stubs = {'sub_' + addr(k): v for k, v in stubs.items()}
    rnd = random.Random(34957)
    cs = []
    for kind in range(3):
        for _ in range(300):
            cs.append((kind, rnd.choice(('Win 3 races', 'Earn rep', '')), rnd.choice((0, 0, 1)), rnd.choice(VALS), rnd.choice(VALS),
                       rnd.choice(VALS) if kind == 0 else 0, rnd.choice((1, 1, 1, 0)) if kind == 0 else 1))
    rows = (',' + nl).join('{%d,"%s",%d,%d,%d,%d,%d}' % c for c in cs)
    NL = chr(92) + 'n'
    body = ('struct K{int kind;const char*p;int done;int32_t count,target,base;int found;};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));*pointer(0x3000+40)=c.done;g_found=c.found;put(0x2000,c.p,0);'
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x3000;ctx.r4.u32=0x2000;ctx.r5.u32=256;' + nl +
            'if(c.kind==0){store(0x3000+120,(uint32_t)c.target);store(0x3000+124,(uint32_t)c.base);store(0x4000+8,(uint32_t)c.count);put(0x3000+88,"rep",0);sub_' + fs[0] + '(ctx,nullptr);}' + nl +
            'else if(c.kind==1){store(0x3000+88,(uint32_t)c.target);store(0x3000+192,(uint32_t)c.count);sub_' + fs[1] + '(ctx,nullptr);}' + nl +
            'else{store(0x3000+120,(uint32_t)c.target);store(0x3000+156,(uint32_t)c.count);sub_' + fs[2] + '(ctx,nullptr);}' + nl +
            'printf("%s' + NL + '",cstr(0x2000).c_str());}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f for f in fs], body, a.out_dir, 'count_text', stubs=stubs, scratch_end=0x8000)
    out = run(exe, a.image).split(nl)[:len(cs)]
    assert len(out) == len(cs)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'count_text_ref.txt').write_text(''.join('%d|%s|%d %d %d %d %d -> %s' % (*c, o) + nl for c, o in zip(cs, out)))
    print(len(cs), 'cases')


if __name__ == '__main__':
    main()
