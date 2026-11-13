"""The Stealth getter F-54047 run offline on the generated code with its two type getters (F-73113 / F-63149): the racer's
car (+48), its power-up holder (+68), the holder's effect (+236) and the effect's type (+4, -1 .. 30). Evidence:
docs/reference/vehicle/stealth_getter_check.md.

Writes <out-dir>/stealth_getter_ref.txt, one state a line: "car holder effect type -> answer" (decimal)."""
import argparse
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs

addr = bs.addr

HARNESS = r'''
enum { RACER=0x2000, CAR=0x2100, HOLDER=0x2200, EFFECT=0x2400, STACK=0x7F00 };
int main() {
    for(int car=0;car<2;car++) for(int holder=0;holder<2;holder++) for(int effect=0;effect<2;effect++) for(int type=-1;type<=30;type++) {
        std::memset(objects,0,sizeof(objects));
        store(RACER+48,car?CAR:0); store(CAR+68,holder?HOLDER:0); store(HOLDER+236,effect?EFFECT:0); store(EFFECT+4,uint32_t(type));
        Context ctx{}; ctx.r1.u32=STACK; ctx.r3.u32=RACER;
        FN(ctx,nullptr);
        std::printf("%d %d %d %d -> %d\n",car,holder,effect,type,int(ctx.r3.u32&0xFF));
    }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-54047')
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], HARNESS.replace('FN(', fn + '('), a.out_dir, 'gref')
    r = subprocess.run([str(exe)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    (a.out_dir / 'stealth_getter_ref.txt').write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'states,', sum(l.endswith('1') for l in r.stdout.splitlines()), 'true')


if __name__ == '__main__':
    main()
