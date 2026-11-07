"""Memory cap for the offline harnesses: puts this Python process (and so every program it starts - the compiled
recomp harnesses, g++) into a Windows job object whose processes may each commit at most MCLA_HARNESS_MEM_GB
(default 4) GB. A harness that reads a bad case file and asks for hundreds of GB then fails with an allocation
error instead of exhausting RAM and the page file (2026-11-06: a level-query harness did that and the machine
blue-screened). Imported by recomp_closure, so every harness built on it is capped. No-op off Windows.
"""
import os
import sys

LIMIT_GB = float(os.environ.get('MCLA_HARNESS_MEM_GB', '4'))
_job = None


def apply(limit_gb=LIMIT_GB):
    global _job
    if _job is not None or sys.platform != 'win32':
        return
    import ctypes
    from ctypes import wintypes as w
    k = ctypes.WinDLL('kernel32', use_last_error=True)

    class BASIC(ctypes.Structure):
        _fields_ = [('PerProcessUserTimeLimit', ctypes.c_int64), ('PerJobUserTimeLimit', ctypes.c_int64),
                    ('LimitFlags', w.DWORD), ('MinimumWorkingSetSize', ctypes.c_size_t),
                    ('MaximumWorkingSetSize', ctypes.c_size_t), ('ActiveProcessLimit', w.DWORD),
                    ('Affinity', ctypes.c_size_t), ('PriorityClass', w.DWORD), ('SchedulingClass', w.DWORD)]

    class IO(ctypes.Structure):
        _fields_ = [(n, ctypes.c_uint64) for n in ('r', 'w', 'o', 'rb', 'wb', 'ob')]

    class EXT(ctypes.Structure):
        _fields_ = [('Basic', BASIC), ('Io', IO), ('ProcessMemoryLimit', ctypes.c_size_t),
                    ('JobMemoryLimit', ctypes.c_size_t), ('PeakProcessMemoryUsed', ctypes.c_size_t),
                    ('PeakJobMemoryUsed', ctypes.c_size_t)]

    k.CreateJobObjectW.restype = w.HANDLE
    job = k.CreateJobObjectW(None, None)
    if not job:
        raise OSError(ctypes.get_last_error(), 'CreateJobObjectW')
    info = EXT()
    # JOB_OBJECT_LIMIT_PROCESS_MEMORY 0x100 | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE 0x2000
    info.Basic.LimitFlags = 0x100 | 0x2000
    info.ProcessMemoryLimit = int(limit_gb * (1 << 30))
    if not k.SetInformationJobObject(w.HANDLE(job), 9, ctypes.byref(info), ctypes.sizeof(info)):
        raise OSError(ctypes.get_last_error(), 'SetInformationJobObject')
    k.GetCurrentProcess.restype = w.HANDLE
    if not k.AssignProcessToJobObject(w.HANDLE(job), w.HANDLE(k.GetCurrentProcess())):
        raise OSError(ctypes.get_last_error(), 'AssignProcessToJobObject')
    _job = job


apply()
