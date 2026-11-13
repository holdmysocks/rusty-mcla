"""The collider's end of tick F-32811, its flag part, run offline on the generated code over every state of its three bytes:
+658 (a deep push this tick), +659 (deep-push ticks in a row), +660 (the "last" matrix is valid; the set-down F-68574 clears
it). F-32811 copies the instance's matrix into +32, and into +96 (the matrix the next tick's collision takes for the last
one) when +658 is clear (then +660 = 1) or when +660 is clear or +659 > 8; then clears +658 if it was set, else +659.
Evidence: docs/reference/vehicle/end_matrix_check.md.

Stubbed: the pending push F-67019 and the instance's re-centre F-18040 (empty: they do not touch the bytes).

Writes <out-dir>/end_matrix_ref.txt, one state a line: "deep count set -> took(0/1) deep count set" (decimal)."""
import argparse
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs

addr = bs.addr

HARNESS = r'''
enum { COL=0x2000, INST=0x3000, STACK=0x7F00 };
int main() {
    int deeps[]={0,1,2,0x80,0xFF};
    for(int d: deeps) for(int c=0;c<256;c++) for(int s: deeps) {
        std::memset(objects,0,sizeof(objects));
        store(COL+16,INST);
        for(int i=0;i<16;i++) store(INST+16+4*i,0x3F800000+i);
        for(int i=0;i<16;i++) { store(COL+32+4*i,0); store(COL+96+4*i,0); }
        *pointer(COL+658)=uint8_t(d); *pointer(COL+659)=uint8_t(c); *pointer(COL+660)=uint8_t(s);
        Context ctx{}; ctx.r1.u32=STACK; ctx.r3.u32=COL;
        FN(ctx,nullptr);
        int took=1, front=1;
        for(int i=0;i<16;i++) { took&=load(COL+96+4*i)==0x3F800000u+i; front&=load(COL+32+4*i)==0x3F800000u+i; }
        if(!front) { std::printf("front not copied\n"); return 3; }
        for(int i=0;i<16&&!took;i++) if(load(COL+96+4*i)!=0) { std::printf("partial\n"); return 4; }
        std::printf("%d %d %d -> %d %d %d %d\n",d,c,s,took,*pointer(COL+658),*pointer(COL+659),*pointer(COL+660));
    }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    sub = lambda fid: 'sub_' + addr(a.tool, fid)
    fn = sub('F-32811')
    stubs = {sub('F-67019'): '', sub('F-18040'): ''}
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], HARNESS.replace('FN(', fn + '('), a.out_dir, 'eref', stubs=stubs, vector=True,
                               scratch_end=0x100000)
    r = subprocess.run([str(exe)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit('reference failed %d: %s %s' % (r.returncode, r.stdout[-300:], r.stderr[-300:]))
    (a.out_dir / 'end_matrix_ref.txt').write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'states')


if __name__ == '__main__':
    main()
