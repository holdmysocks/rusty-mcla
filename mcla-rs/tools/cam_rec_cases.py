"""cam_rec_cases.py <cam trace.txt> <out.txt> [first N cases]: the director blends of a camera recording of the running original
(work/recomp-trace src/cam_trace.cpp, MCLA_CAM_TRACE) as replay cases for boomcam's test
`event_blends_match_the_recording`.

Per case (one push of the event camera on, or of the wipeout camera, to the end of its blends):
  case <n> <kind>       kind: event (F-94591's pushes) / wipe (F-37740's push, after the W record)
  push <T>              the push's time (hex bits)
  s <dt> <w> <l>        the blend camera's weight and time left at the step's entry, the step's frame time
  off <t>               F-94591 off with time t (before the next step)
  fin <q>               the step before returned false (F-70013); q = 1 when a camera was queued
All values are the f32 bit patterns of the record.
"""
import sys


def fields(ln):
    return dict(kv.split('=', 1) for kv in ln.split()[1:] if '=' in kv)


def main(src, out, first=None):
    cases, cur, n, wiped = [], None, 0, False
    for ln in open(src):
        k = ln[:1]
        if k not in 'OPRQFSW':
            continue
        f = fields(ln)
        if k == 'W':
            wiped = True
        if k == 'P' and f['type'] == '3':
            # (a push after the wipeout start is its camera's; the others here are the event camera's)
            kind = 'wipe' if wiped else 'event'
            wiped = False
            if kind and (cur is None or kind == 'wipe' or f['c'] != cur['tgt']):
                # an event camera's off push continues its case; any other push starts one
                if cur is not None and cur['kind'] == 'event' and kind == 'event' and cur.get('off'):
                    continue
                if cur is not None:
                    cases.append(cur)
                n += 1
                cur = {'kind': kind, 'tgt': f['c'], 'ops': ['push ' + f['t']]}
        elif cur is None:
            continue
        elif k == 'O' and f['on'] == '0':
            cur['ops'].append('off ' + f['t'])
            cur['off'] = True
        elif k == 'S':
            cur['ops'].append('s %s %s %s' % (f['dt'], f['w'], f['l']))
        elif k == 'F':
            cur['ops'].append('fin %d' % (f['q'] != '00000000'))
            if cur['kind'] == 'wipe' or cur.get('off'):
                cases.append(cur)
                cur = None
    if first is not None:
        cases = cases[:first]
    with open(out, 'w') as o:
        o.write('# director blends recorded in the running original (tools/cam_rec_cases.py)\n')
        for i, c in enumerate(cases):
            o.write('case %d %s\n' % (i, c['kind']))
            o.write('\n'.join(c['ops']) + '\n')
    print(len(cases), 'cases', sum(len(c['ops']) for c in cases), 'ops')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else None)
