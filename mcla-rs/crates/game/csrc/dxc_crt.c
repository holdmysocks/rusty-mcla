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
