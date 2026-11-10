"""The tab container's step F-87194 (left) / F-12609 (right) run offline as generated: the index +72, the count +68 and the
wrap byte +76 (AllowWrap, F-40820; the constructor F-59174 clears it and no shipped scene sets it). Left at 0 goes to
count - 1 only with the wrap, else stays; right at count - 1 goes to 0 only with the wrap; any other press moves by one. A
move plays UI_MENU_NAV_LEFT / _RIGHT (the sound call stubbed: it records its name) and posts TAB_SELECTION_CHANGED (stubbed:
counted); no move plays UI_MENU_ERROR. The container's vt+540 (has children) is a fake returning 0 and vt+184 (refresh) a
fake that does nothing, so the per-tab select calls are not run.
Line:  <dir 0 left / 1 right> <index> <count> <wrap> -> <index after> <sound 0 none / 1 nav / 2 error> <events>
Usage: tab_step_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t T=0x40010000,V=0x40011000,R=0x40014000;
 uint32_t d,i,n,w;
 while(in>>d>>i>>n>>w){rt_reset();
  for(uint32_t x=T;x<0x40014100;x+=4)store(x,0);
  store(T,V);store(V+540,0x7F000010u);store(V+184,0x7F000020u);
  store(T+68,n);store(T+72,i);store(T+76,w<<24);
  Context cx=fresh();cx.r3.u32=T;if(d)sub_RIGHT(cx,nullptr);else sub_LEFT(cx,nullptr);
  uint32_t s=load(R);
  printf("%u %u %u\n",load(T+72),s==SND_NAV_L||s==SND_NAV_R?1u:(s==SND_ERR?2u:(s?9u:0u)),load(R+4));
 }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    left, right = addr('F-87194'), addr('F-12609')
    img = rd.load_image(a.image)
    # (the three sound names as the handlers load them: lis / addi pairs)
    snd = {'UI_MENU_NAV_LEFT': ((-32247 << 16) - 3648) & 0xFFFFFFFF, 'UI_MENU_NAV_RIGHT': None, 'UI_MENU_ERROR': ((-32253 << 16) - 15540) & 0xFFFFFFFF}
    o = snd['UI_MENU_NAV_LEFT'] - rd.IMAGE_BASE
    assert img[o:o + 17] == b'UI_MENU_NAV_LEFT\0' and img[snd['UI_MENU_ERROR'] - rd.IMAGE_BASE:][:14] == b'UI_MENU_ERROR\0'
    r = img.find(b'\0UI_MENU_NAV_RIGHT\0')
    assert r > 0
    snd['UI_MENU_NAV_RIGHT'] = rd.IMAGE_BASE + r + 1
    # (the callees: the UI sound and the event post)
    idx = rd.index(a.recomp)
    sound, post = addr('F-42664'), addr('F-41407')
    main_src = (MAIN.replace('sub_LEFT(', left + '(').replace('sub_RIGHT(', right + '(')
                .replace('SND_NAV_L', hex(snd['UI_MENU_NAV_LEFT']) + 'u').replace('SND_NAV_R', hex(snd['UI_MENU_NAV_RIGHT']) + 'u')
                .replace('SND_ERR', hex(snd['UI_MENU_ERROR']) + 'u'))
    exe = rd.build(idx, [left, right], main_src, a.work, 'tab_step', image=img,
                   stubs={sound: 'store(0x40014000u,ctx.r3.u32);', post: 'store(0x40014004u,load(0x40014004u)+1);'},
                   fakes={0x7F000010: 'ctx.r3.u32=0;', 0x7F000020: ''})
    rnd = random.Random(87194)
    lines = []
    for d in (0, 1):
        for n in range(0, 7):
            for i in range(0, n + 2):
                for w in (0, 1):
                    lines.append(f'{d} {i} {n} {w}')
    while len(lines) < 440:
        n = rnd.choice([rnd.randint(0, 9), rnd.randint(0, 0xFFFFFFFF)])
        i = rnd.choice([0, n - 1 if n else 0, rnd.randint(0, 0xFFFFFFFF), rnd.randint(0, 12)]) & 0xFFFFFFFF
        # (no number of the line, nor one off it, may read as an address: the public repo's citation check)
        if any(re.fullmatch(r'8[0-9A-Fa-f]{7}', str((v + k) & 0xFFFFFFFF)) for v in (i, n) for k in (-1, 0, 1)):
            continue
        lines.append(f'{rnd.randint(0, 1)} {i} {n} {rnd.randint(0, 1)}')
    cases = a.work / 'tab_step_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines), (len(rows), len(lines))
    a.out.write_text(''.join(l + ' -> ' + r + '\n' for l, r in zip(lines, rows)))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
