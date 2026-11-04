"""The night fast collector's view as the original draws it (F-55997's night branch), read from recordings of the
running original at night (gfx recorder, work/lighting-trails/night/cap/g.<n>.bin, the title at 23:00): per frame the
main camera (gViewInverse c12..c15 of the first VS_zPrePass draw after the pass), the night pass's view-projection
(World^-1 x gWorldViewProj of its first VS_ShadowNight draw, the programs' row-vector layout: p' = p VP) and its
ShaderGlobals c63. Writes crates/game/src/testdata/night_collector_ref.txt for the test
render::night_collector_tests::the_view_is_the_originals.

    python tools/night_collector_reference.py D:/MCLA/work/lighting-trails/night/cap/g.24.bin ... > crates/game/src/testdata/night_collector_ref.txt
"""
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gfx_trace import Trace  # noqa: E402

print("# frame | camera x y z | row2 x y z | VP rows 0..3 (16) | c63 (4)")
for path in sys.argv[1:]:
    t = Trace(path)
    vp = c63 = None
    for d in t.draws:
        if d.kind == 6:
            continue
        if vp is None and d.vs_name == "VS_ShadowNight":
            vp = np.linalg.inv(np.array(d.vs_c(0, 4), dtype=np.float64)) @ np.array(d.vs_c(8, 4), dtype=np.float64)
            c63 = d.ps_c(63)
        elif vp is not None and d.vs_name == "VS_zPrePass":
            vi = d.vs_c(12, 4)
            row = [os.path.basename(path)] + ["%.10g" % x for x in vi[3][:3]] + ["%.10g" % x for x in vi[2][:3]] + ["%.10g" % x for x in vp.flatten()] + ["%.10g" % x for x in c63]
            print(" ".join(row))
            break
