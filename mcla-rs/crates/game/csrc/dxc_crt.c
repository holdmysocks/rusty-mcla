// DXC, linked into the exe for DX12 (wgpu's static-dxc; render/mod.rs, GitHub #33), was built against MSVC's C runtime
// and calls five of its formatting functions through the import table. The UCRT that the mingw build uses has them
// only as inline functions in its headers, so here they are, on the UCRT's own entry points as those headers have them.
#include <stdarg.h>
#include <stddef.h>

typedef unsigned long long opts_t;
// (the UCRT's legacy null termination for _vsnprintf: _CRT_INTERNAL_PRINTF_LEGACY_VSPRINTF_NULL_TERMINATION)
#define LEGACY_VSPRINTF_NULL_TERMINATION 1ULL

int __stdio_common_vsprintf(opts_t o, char *b, size_t n, const char *f, void *loc, va_list a);
int __stdio_common_vsprintf_s(opts_t o, char *b, size_t n, const char *f, void *loc, va_list a);
int __stdio_common_vsnprintf_s(opts_t o, char *b, size_t n, size_t count, const char *f, void *loc, va_list a);

static int mcla_vsnprintf(char *b, size_t n, const char *f, va_list a) {
    int r = __stdio_common_vsprintf(LEGACY_VSPRINTF_NULL_TERMINATION, b, n, f, NULL, a);
    return r < 0 ? -1 : r;
}

int vsnprintf_s(char *b, size_t n, size_t count, const char *f, va_list a) {
    int r = __stdio_common_vsnprintf_s(0, b, n, count, f, NULL, a);
    return r < 0 ? -1 : r;
}

static int mcla_sprintf_s(char *b, size_t n, const char *f, ...) {
    va_list a;
    va_start(a, f);
    int r = __stdio_common_vsprintf_s(0, b, n, f, NULL, a);
    va_end(a);
    return r < 0 ? -1 : r;
}

static int mcla_snprintf_s(char *b, size_t n, size_t count, const char *f, ...) {
    va_list a;
    va_start(a, f);
    int r = vsnprintf_s(b, n, count, f, a);
    va_end(a);
    return r;
}

int (*__imp__vsnprintf)(char *, size_t, const char *, va_list) = mcla_vsnprintf;
int (*__imp_sprintf_s)(char *, size_t, const char *, ...) = mcla_sprintf_s;
int (*__imp__snprintf_s)(char *, size_t, size_t, const char *, ...) = mcla_snprintf_s;

// DXC's string conversion (CA2WEX / CW2AEX, in DxcCompiler::Compile) does setlocale(LC_ALL, "en_US.UTF-8"), mbstowcs,
// setlocale back - on the process's one C locale. The scene pipelines are built ahead on several threads (render/ms.rs),
// so one thread's setlocale changed or freed the locale under another's mbstowcs or restore, and the UCRT ended the
// process (fail-fast 0xc0000409 in ucrtbase, no Rust message): about 1 cold DX12 start in 5 here (docs/PERFORMANCE.md
// 2026-11-19). Every thread of the process gets a C locale of its own as it starts (a TLS callback: the main thread at
// process attach, every other one at thread attach), so DXC's setlocale changes only its own thread's locale and the
// conversion is the same as on one thread. Nothing else in the exe reads the C locale.
#include <windows.h>
#include <locale.h>

static void NTAPI mcla_thread_locale(PVOID module, DWORD reason, PVOID reserved) {
    (void)module;
    (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        _configthreadlocale(_ENABLE_PER_THREAD_LOCALE);
    }
}

__attribute__((section(".CRT$XLM"), used)) const PIMAGE_TLS_CALLBACK mcla_thread_locale_cb = mcla_thread_locale;
