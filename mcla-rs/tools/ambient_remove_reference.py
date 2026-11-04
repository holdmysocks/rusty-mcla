"""The ambient manager's removal message (F-99126) and the three enable natives that send it (F-26788 traffic,
F-74501 people, F-54264 parked cars), run offline: the generated code whole and unchanged.

Part 1 - F-99126 over the manager's two chains of its three type buckets (S+12 -> 52-byte buckets, heads +8 and +32,
nodes {object, next}), with each object's kind byte from the original's F-93575 -> F-30914 (the object's +44 parking
spot and its type +448: byte +108, vfuncs 24 and 28 answered from the type through a vtable written for it). The
removal request F-76814 logs the object it is given (and its reason); the request's other pieces (F-86587, F-57042,
F-40221, F-75521, F-29339, F-28982) and the pending requests' end F-42105 are empty.
Part 2 - the natives with F-99126 and the threaded queue F-60395 logging the message byte: the argument, the manager
pointer (0x8286D8E4: null or set), its threaded byte (+0x64259) and the stack byte the message is made from.

Writes <out-dir>/ambient_remove_ref.txt:
  "K <msg> <n> <object words...> -> <removed object indices>" with per object four words: chain (0..5: bucket 0..2
  head +8, then bucket 0..2 head +32), spot word, type byte +108, vfunc 24 and vfunc 28 answers (all hex).
  "E <native 0|1|2> <arg> <mgr set> <threaded> <stack byte> -> <stored byte or --> <M xx | Q xx | ->"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n')
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def kcases(rnd, count):
    out = []
    for _ in range(count):
        msg = rnd.choice((0x60, 0x10, 0x20, 0x40, 0x70, 0x60 | rnd.randrange(16), rnd.randrange(256), 0x80, 0))
        objs = []
        for _k in range(rnd.choice((0, 1, 2, 3, 5, 8, 12))):
            objs.append([rnd.randrange(6), rnd.choice((0, 0, 1, 0x50000000, 0xFFFFFFFF)), rnd.choice((0, 0, 1, 2, 3, 0xFF)),
                         rnd.choice((0, 1, 1, 0x100, 0xFFFFFF01)), rnd.choice((0, 0, 1, 2, 3, 0xFFFFFFFF))])
        out.append((msg, objs))
    return out


def ecases(rnd, count):
    out = []
    for _ in range(count):
        out.append([rnd.randrange(3), rnd.choice((0, 0, 1, 0x100, 0xFFFFFF00, 0xFF, 0x80000000, rnd.randrange(1 << 32))),
                    rnd.choice((1, 1, 1, 0)), rnd.choice((0, 0, 0, 1, 0xFF)), rnd.randrange(256)])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    ap.add_argument('--ecases', type=int, default=1500)
    a = ap.parse_args()
    idx = index(a.recomp)
    fids = ('F-99126', 'F-93575', 'F-30914', 'F-26788', 'F-74501', 'F-54264', 'F-60395', 'F-76814', 'F-86587', 'F-57042', 'F-40221',
            'F-75521', 'F-29339', 'F-28982', 'F-42105')
    nm = {f: addr(a.tool, f) for f in fids}
    kc = kcases(random.Random(99126), a.cases)
    ec = ecases(random.Random(26788), a.ecases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    words = []
    for msg, objs in kc:
        words += [0, msg, len(objs)] + [w for o in objs for w in o]
    for e in ec:
        words += [1] + e
    inp = a.out_dir / 'ambient_remove_inputs.bin'
    inp.write_bytes(struct.pack('<%dI' % len(words), *words))
    s = lambda f: 'sub_' + nm[f]
    # part 1 runs F-99126 for real with its request pieces empty; part 2 runs the natives with F-99126 / F-60395 logging
    stubs1 = {s('F-76814'): 'printf(" X%x/%u",ctx.r4.u32,ctx.r5.u32);'}
    for f in ('F-86587', 'F-57042', 'F-40221', 'F-75521', 'F-29339', 'F-28982', 'F-42105', 'F-60395'):
        stubs1[s(f)] = ''
    disp = r'''static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==0x3900){ctx.r3.u32=load(ctx.r3.u32+200);return;}
 if(a==0x3904){ctx.r3.u32=load(ctx.r3.u32+204);return;}
 throw std::runtime_error("unexpected indirect call");}
static int mode2=0;
'''
    body = r'''FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);
size_t i=0;
while(i<in.size()){
 memset(objects,0,0xF000);
 if(in[i]==0){uint32_t msg=in[i+1],n=in[i+2];const uint32_t*o=in.data()+i+3;i+=3+5*n;
  const uint32_t S=0x4000,P=0x4100;store(S+12,P);
  store(0x3800+24,0x3900);store(0x3800+28,0x3904);
  uint32_t tail[6]={0,0,0,0,0,0};
  for(uint32_t k=0;k<n;k++){uint32_t ob=0x5000+0x200*k,ty=0xC000+0x100*k,nd=0xB000+16*k;
   store(ob+44,o[5*k+1]);store(ob+448,ty);store(ty,0x3800);*pointer(ty+108)=uint8_t(o[5*k+2]);store(ty+200,o[5*k+3]);store(ty+204,o[5*k+4]);
   store(nd,ob);store(nd+4,0);uint32_t ch=o[5*k];uint32_t head=P+52*(ch%3)+(ch<3?8:32);
   if(tail[ch])store(tail[ch]+4,nd);else store(head,nd);tail[ch]=nd;}
  *pointer(0xF000)=uint8_t(msg);
  Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x8000;ctx.r3.u32=S;ctx.r4.u32=0xF000;
  printf("K");sub_F99126(ctx,nullptr);printf("\n");
 } else {const uint32_t*e=in.data()+i+1;i+=6;
  const uint32_t X=0xA000u-0x64259u;
  store(0x8286D8E4,e[2]?X:0);*pointer(0xA000)=uint8_t(e[3]);
  const uint8_t sen=uint8_t(~e[1]);store(0x8282A7C8,sen*0x01010101u);
  *pointer(0xFEA0+80)=uint8_t(e[4]);
  Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x8000;ctx.r3.u32=e[1];
  printf("E");
  if(e[0]==0)sub_F26788(ctx,nullptr);else if(e[0]==1)sub_F74501(ctx,nullptr);else sub_F54264(ctx,nullptr);
  uint8_t b=*pointer(0x8282A7C8+e[0]);printf(" %s",b==sen?"--":"");if(b!=sen)printf("%02x",b);printf("\n");}
}
'''
    for f in ('F-99126', 'F-26788', 'F-74501', 'F-54264'):
        body = body.replace('sub_' + f.replace('-', ''), s(f))
    # F-99126 and F-60395 log in part 2: the natives call them through these wrappers
    main_src = (disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    # one exe: F-99126 real; the natives' call of it is rerouted by renaming in their bodies (see below)
    fs = recomp_closure.closure(idx, [s('F-99126'), s('F-26788'), s('F-74501'), s('F-54264')], stubs1)
    for f in ('F-26788', 'F-74501', 'F-54264'):
        src = fs[s(f)]
        assert src.count(s('F-99126') + '(ctx, base);') == 1 and src.count(s('F-60395') + '(ctx, base);') == 1
        fs[s(f)] = src.replace(s('F-99126') + '(ctx, base);', 'log_msg(ctx,base,"M");').replace(s('F-60395') + '(ctx, base);', 'log_msg(ctx,base,"Q");')
    protos = ''.join(f'void {n}(Context &ctx,const uint8_t *base);' + nl for n in list(fs) + list(stubs1))
    stubdefs = ''.join('void %s(Context &ctx,const uint8_t *base){%s}' % (n, b) + nl for n, b in stubs1.items())
    shim = recomp_closure.shim(0x10000)
    # (the queued message is the byte at r4 too: F-60395 takes (queue, &byte))
    log = 'static void log_msg(Context&ctx,const uint8_t*base,const char*k){printf(" %s %02x",k,*pointer(ctx.r4.u32));}' + nl
    cpp = a.out_dir / 'ambient_remove.cpp'
    cpp.write_text(shim + recomp_closure.EXTRA + protos + stubdefs + log + ''.join(fs.values()) + '#include <vector>' + nl + main_src)
    exe = a.out_dir / 'ambient_remove.exe'
    r = subprocess.run(['g++', '-std=c++17', '-O2', '-ffp-contract=off', '-static', str(cpp), '-o', str(exe)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:3000])
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(kc) + len(ec), (len(out), len(kc), len(ec))
    lines = []
    for (msg, objs), o in zip(kc, out[:len(kc)]):
        assert o.startswith('K')
        rem = []
        for t in o[1:].split():
            ad, why = t[1:].split('/')
            rem.append('%d/%s' % ((int(ad, 16) - 0x5000) // 0x200, why))
        lines.append('K %02x %d ' % (msg, len(objs)) + ' '.join('%x %08x %x %08x %08x' % tuple(ob) for ob in objs) + ' -> ' + ' '.join(rem))
    for e, o in zip(ec, out[len(kc):]):
        assert o.startswith('E')
        t = o[1:].split()
        lines.append('E %x %08x %x %x %02x -> ' % tuple(e) + ' '.join([t[-1]] + (t[:-1] or ['-'])))
    bad = [x for x in lines if any(len(t) == 8 and t.startswith('82') for t in x.split())]
    keep = [x for x in lines if x not in bad]
    (a.out_dir / 'ambient_remove_ref.txt').write_text(nl.join(keep) + nl)
    print(len(keep), 'lines kept of', len(lines), '|', sum(x.startswith('K') and x.split(' -> ')[1] != '' for x in keep), 'runs removing,',
          sum(x.split(' -> ')[1].count('/') for x in keep if x.startswith('K')), 'objects removed,',
          sum(' M ' in x or ' Q ' in x for x in keep if x.startswith('E')), 'natives sending')


if __name__ == '__main__':
    main()
