"""The light trails' free-record pick F-58134 run offline (the generated code as it is), for the port's test
(crates/game/src/testdata/trail_free_reference.txt): the 96 records at their place in the image (+304 the count, +316
the drawn byte), random counts (mostly held, a few or no empty ones) and drawn bytes.

  CASE <n> <96 counts> | <96 drawn bytes>
  OUT <returned index> <96 counts after> | <96 drawn bytes after>

python trail_free_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import random
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

REC, N = 0x82883970, 96


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-58134')
    rnd = random.Random(58134)
    rows = []
    for k in range(2000):
        p = rnd.choice((0.0, 0.002, 0.01, 0.05, 0.3, 1.0))
        counts = [0 if rnd.random() < p else rnd.choice((1, 2, 3, 8, 0xFFFFFFFF, 0x80000000)) for _ in range(N)]
        drawn = [rnd.choice((0, 1, 1, 0xFF)) for _ in range(N)]
        rows.append(counts + drawn)
    nl, NL = chr(10), chr(92) + 'n'
    body = ('static const uint32_t C[][%d]={' % (2 * N) + ','.join('{' + ','.join('%uu' % v for v in r) + '}' for r in rows) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};' + nl +
            'auto get=[](uint32_t a){uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|uint32_t(p[1])<<16|uint32_t(p[2])<<8|p[3];};' + nl +
            'int k=0;for(auto&c:C){printf("CASE %d",k++);for(int i=0;i<' + str(N) + ';i++)printf(" %u",c[i]);printf(" |");' +
            'for(int i=0;i<' + str(N) + ';i++)printf(" %u",c[' + str(N) + '+i]);printf("' + NL + '");' +
            ('for(int i=0;i<%d;i++){uint32_t b=%du+336*i;put(b+304,c[i]);image[b+316-0x82000000u]=uint8_t(c[%d+i]);}' % (N, REC, N)) +
            'Context ctx{};ctx.r1.u32=0x2F00;' + ('%s(ctx,nullptr);' % fn) + 'printf("OUT %d",ctx.r3.s32);' +
            ('for(int i=0;i<%d;i++)printf(" %%u",get(%du+336*i+304));printf(" |");for(int i=0;i<%d;i++)printf(" %%u",unsigned(image[%du+336*i+316-0x82000000u]));' % (N, REC, N, REC)) +
            'printf("' + NL + '");}}' + nl)
    out = run(build(index(a.recomp), [fn], body, a.out, 'trail_free'), a.image)
    assert out.count('OUT') == len(rows), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(rows), 'cases', sum(1 for l in out.splitlines() if l.startswith('OUT -1')), 'with none free')


if __name__ == '__main__':
    main()
