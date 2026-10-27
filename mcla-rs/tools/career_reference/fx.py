"""Generic offline fixture builder: cut unchanged functions (and the closure of their callees) out of the
static recompilation, add a main(), compile with the repo's vector shim, run, return the CSV text."""
import re,subprocess,sys,pickle,os
from pathlib import Path
sys.path.insert(0,'D:/MCLA/mcla-rs/tools')
from recomp_vector import vector_shim
GEN=Path('D:/MCLA-static/generated/default')
SDK=Path('D:/MCLA-static/third_party/rexglue-sdk')
IMAGE=Path('D:/MCLA/work/2026-10-10/original-image.bin')
CXX='g++'
_idx=None
TOOL='D:/MCLA/mcla-rs/target/release/mcla-tool.exe'
def resolve(text):
    """Replace @F-#####@ (opaque function IDs; the repository does not carry addresses) by the recompilation's name sub_<address>."""
    def one(m):
        out=subprocess.run([TOOL,'registry','addr',m.group(1)],capture_output=True,text=True,check=True).stdout.split()
        return 'sub_'+out[1].upper().replace('0X','')
    return re.sub(r'@(F-\d{5})@',one,text)
def index():
    global _idx
    if _idx: return _idx
    cache=Path('out')/'fidx.pkl'; cache.parent.mkdir(exist_ok=True)
    if cache.exists(): _idx=pickle.load(open(cache,'rb')); return _idx
    idx={}
    for f in sorted(GEN.glob('mcla_recomp.*.cpp')):
        t=f.read_text()
        for m in re.finditer(r'^DEFINE_REX_FUNC\((\w+)\) \{',t,re.M):
            s=m.start(); e=t.index('\n}\n',s)+3
            idx[m.group(1)]=t[s:e]
    pickle.dump(idx,open(cache,'wb')); _idx=idx; return idx
def closure(roots,stub=()):
    idx=index(); seen={}; todo=list(roots)
    while todo:
        n=todo.pop()
        if n in seen or n in stub: continue
        seen[n]=idx[n]
        for m in re.finditer(r'\b(sub_[0-9A-F]{8}|__\w+)\(ctx, base\)',idx[n]):
            todo.append(m.group(1))
    return seen
def build(name,roots,main,cases,out,scratch=0xa000,stub_src='',stub=(),extra=''):
    roots=[resolve(r) for r in roots]; stub=tuple(resolve(r) for r in stub); main=resolve(main); stub_src=resolve(stub_src); extra=resolve(extra)
    fn=closure(roots,stub)
    code=vector_shim(SDK,scratch)
    code=code.replace('uint8_t u8;','uint8_t u8; int8_t s8;',1)
    code=code.replace('#define simde_mm_cvtsd_si64 _mm_cvtsd_si64','#define simde_mm_cvtsd_si64 _mm_cvtsd_si64'+chr(10)+'#define simde_mm_cvttsd_si64 _mm_cvttsd_si64',1)
    code+=extra
    code+=''.join('void '+n+'(Context&,const uint8_t*);\n' for n in sorted(fn))
    code+=stub_src
    code+=''.join(fn[n] for n in sorted(fn))+main
    out=Path(out); out.mkdir(parents=True,exist_ok=True)
    src=out/(name+'_original.cpp'); src.write_text(code)
    inp=out/(name+'_inputs.txt'); inp.write_text(cases)
    exe=out/(name+'_original.exe')
    subprocess.run([CXX,'-std=c++17','-O2','-msse4.1','-ffp-contract=off','-fmax-errors=4','-static',str(src),'-o',str(exe)],check=True)
    r=subprocess.run([str(exe),str(IMAGE),str(inp)],check=True,capture_output=True)
    csv=out/(name+'_reference.csv'); csv.write_bytes(r.stdout)
    print(name,len(cases.splitlines()),'cases ->',csv,'functions:',len(fn))
    return csv
